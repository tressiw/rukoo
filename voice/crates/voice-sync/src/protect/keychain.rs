//! The macOS Keychain protector (spec 2.3, D4): a wrapping key per label and data folder in the
//! login keychain, and the secret sealed with XChaCha20-Poly1305 under it.
//!
//! The Keychain item holds 32 random bytes and nothing else. The item's account names the label
//! and the data folder, so two data folders (a development copy next to the real one) never share
//! a wrapping key, and so Keychain Access shows which file each item opens:
//!
//! ```text
//! service: nl.bvdm.rukoo.voice
//! account: <label>:<hash>
//! ```
//!
//! `<hash>` is the first 16 hex characters of `SHA-256("rukoo-voice/keychain/account/v1\0" ||
//! canonical path of the data folder)`. The canonical path, so the same folder reached through a
//! symlink or a relative path still finds its items; it is read on first use, while the key files
//! are being read or written in that folder, and a folder that can't be reached is a key error
//! instead of a second account. The item is made on the first `protect` and read on `unprotect`;
//! deleting it makes Voice report a key error rather than lose data. Settings can remove the key
//! file or the secret without its item, which then opens nothing and waits for a later `protect`.
//!
//! The blob after the file's magic and version byte is `KCK1`, a random 24-byte nonce, then the
//! sealed secret with the label as associated data. `RKVK` and `RKVS` themselves don't change, and
//! they carry no protector tag: a blob from another protector fails the tag check, which Voice
//! reports as a key error.
//!
//! macOS ties a keychain item's access to the signature of the program that made it, so after an
//! update or a rebuilt development bundle macOS asks whether Voice may read it. It asks per item:
//! each has its own access list, and Security.framework has no read that answers for two items in
//! one dialog. With sync on that is two dialogs, the master key's and then the bucket secret's;
//! Always Allow ends each. Deny lands here as [`StoreError::Denied`], which is a `KeyDenied` error:
//! Settings shows it with Try again, Voice keeps running, and the bucket secret isn't asked for
//! after the master key was refused. A missing item stays a `Key` error, whose way out is the
//! recovery code.
//!
//! A read waits as long as the dialog is open, so Voice never reads an item on a thread that
//! anything else waits for: the sync worker reads them on its own thread, outside its `busy` lock,
//! and Settings show `sync.state` `unlocking` meanwhile.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::{Protector, SECRET_LABEL};
use crate::keys::random;
use crate::names::hex;
use crate::{SyncError, SyncErrorKind};

/// The service of Voice's sync keys in the login keychain, the same on every data folder.
pub const SERVICE: &str = "nl.bvdm.rukoo.voice";

/// What a protected blob starts with, after the file's magic and version byte.
pub const BLOB_MARKER: &[u8; 4] = b"KCK1";

/// The domain separator of the account's folder hash.
const HASH_DOMAIN: &[u8] = b"rukoo-voice/keychain/account/v1\0";
/// Hex characters of the folder hash in an account: 64 bits, enough to keep folders apart.
const HASH_CHARS: usize = 16;
const NONCE_LEN: usize = 24;
const KEY_LEN: usize = 32;
/// XChaCha20-Poly1305's tag.
const TAG_LEN: usize = 16;

// SecBase.h, which security-framework-sys doesn't re-export.
/// The item isn't there.
const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
/// The user said no, or the keychain refused what the item's access control allows.
const ERR_SEC_AUTH_FAILED: i32 = -25293;
/// The user dismissed the access dialog.
const ERR_SEC_USER_CANCELED: i32 = -128;
/// Nobody could answer a dialog, such as a locked keychain in a session without a desktop.
const ERR_SEC_INTERACTION_NOT_ALLOWED: i32 = -25308;
/// The Mac is in dark wake (Power Nap), where no dialog can show.
const ERR_SEC_IN_DARK_WAKE: i32 = -25320;

/// Why [`ItemStore`] couldn't answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// No item for that account.
    Missing,
    /// The user denied access, dismissed the dialog, or macOS couldn't ask now. Carries the
    /// OSStatus. The item is still there.
    Denied(i32),
    /// Anything else the keychain reported.
    Other(i32),
}

/// Where the wrapping keys live: the login keychain, or a fake in tests.
pub trait ItemStore: Send + Sync {
    /// The item's bytes, or [`StoreError::Missing`].
    fn get(&self, account: &str) -> Result<Vec<u8>, StoreError>;
    /// Creates the item, or replaces its bytes.
    fn set(&self, account: &str, secret: &[u8]) -> Result<(), StoreError>;
    /// Removes the item; [`StoreError::Missing`] when it was already gone.
    fn delete(&self, account: &str) -> Result<(), StoreError>;
}

/// The login keychain's generic passwords (security-framework 3.7.0 `passwords.rs`:33,42,84).
pub struct LoginKeychain {
    service: String,
}

impl LoginKeychain {
    /// The items of `service` (normally [`SERVICE`]).
    pub fn new(service: &str) -> LoginKeychain {
        LoginKeychain {
            service: service.to_owned(),
        }
    }
}

impl ItemStore for LoginKeychain {
    fn get(&self, account: &str) -> Result<Vec<u8>, StoreError> {
        security_framework::passwords::get_generic_password(&self.service, account)
            .map_err(store_error)
    }

    fn set(&self, account: &str, secret: &[u8]) -> Result<(), StoreError> {
        security_framework::passwords::set_generic_password(&self.service, account, secret)
            .map_err(store_error)
    }

    fn delete(&self, account: &str) -> Result<(), StoreError> {
        security_framework::passwords::delete_generic_password(&self.service, account)
            .map_err(store_error)
    }
}

fn store_error(error: security_framework::base::Error) -> StoreError {
    match error.code() {
        ERR_SEC_ITEM_NOT_FOUND => StoreError::Missing,
        code @ (ERR_SEC_AUTH_FAILED
        | ERR_SEC_USER_CANCELED
        | ERR_SEC_INTERACTION_NOT_ALLOWED
        | ERR_SEC_IN_DARK_WAKE) => StoreError::Denied(code),
        code => StoreError::Other(code),
    }
}

/// The macOS protector: [`LoginKeychain`] on a real data folder, a fake store in tests.
pub struct Keychain<S: ItemStore = LoginKeychain> {
    /// The data folder whose keys these are.
    data_dir: PathBuf,
    /// Its hash, the second half of every account, read on first use so the folder exists then.
    folder: OnceLock<String>,
    store: S,
}

impl Keychain<LoginKeychain> {
    /// The login keychain, holding `data_dir`'s sync keys.
    pub fn new(data_dir: &Path) -> Keychain<LoginKeychain> {
        Keychain::with_store(data_dir, LoginKeychain::new(SERVICE))
    }
}

impl<S: ItemStore> Keychain<S> {
    /// The same, on another store: a fake in tests.
    pub fn with_store(data_dir: &Path, store: S) -> Keychain<S> {
        Keychain {
            data_dir: data_dir.to_path_buf(),
            folder: OnceLock::new(),
            store,
        }
    }

    /// The Keychain Access account of `label` in this data folder: `<label>:<hash>`.
    pub fn account(&self, label: &str) -> Result<String, SyncError> {
        Ok(format!("{label}:{}", self.folder()?))
    }

    /// The data folder's hash, computed once, when the key files are read or written in it.
    ///
    /// A folder Voice can't canonicalize is a `Key` error that names it: hashing the raw path
    /// would make a second account and leave the items the key files need out of reach.
    fn folder(&self) -> Result<&str, SyncError> {
        if let Some(folder) = self.folder.get() {
            return Ok(folder);
        }
        let canonical = fs::canonicalize(&self.data_dir).map_err(|e| {
            SyncError::new(
                SyncErrorKind::Key,
                format!(
                    "Voice can't reach its data folder to open its sync keys: {}: {e}",
                    self.data_dir.display()
                ),
            )
        })?;
        let mut hash = Sha256::new();
        hash.update(HASH_DOMAIN);
        hash.update(canonical.as_os_str().as_encoded_bytes());
        let folder = hex(&hash.finalize()[..HASH_CHARS / 2]);
        Ok(self.folder.get_or_init(|| folder))
    }

    /// The wrapping key of `label`: the item's bytes, made first when `create` and there is none.
    fn wrapping_key(
        &self,
        label: &str,
        create: bool,
    ) -> Result<Zeroizing<[u8; KEY_LEN]>, SyncError> {
        let account = self.account(label)?;
        let bytes = match self.store.get(&account) {
            Ok(bytes) => Zeroizing::new(bytes),
            Err(StoreError::Missing) if create => {
                let mut key = Zeroizing::new([0u8; KEY_LEN]);
                random(&mut *key)?;
                self.store
                    .set(&account, &*key)
                    .map_err(|e| store_failed("save", label, &account, e))?;
                return Ok(key);
            }
            Err(StoreError::Missing) => {
                return Err(SyncError::new(
                    SyncErrorKind::Key,
                    "Voice can't find the login keychain item that opens its encryption key file. Import the key from its recovery code, or save the bucket secret again.",
                ))
            }
            Err(e) => return Err(store_failed("read", label, &account, e)),
        };
        if bytes.len() != KEY_LEN {
            return Err(SyncError::new(
                SyncErrorKind::Key,
                "The login keychain item that opens Voice's encryption key file is damaged. Import the key from its recovery code, or save the bucket secret again.",
            ));
        }
        let mut key = Zeroizing::new([0u8; KEY_LEN]);
        key.copy_from_slice(&bytes);
        Ok(key)
    }
}

/// An item Voice couldn't save or read, as Settings show it under Error details. A refusal is
/// `KeyDenied`, so Settings ask the user to allow Voice instead of to enter the key again.
fn store_failed(what: &str, label: &str, account: &str, error: StoreError) -> SyncError {
    let item = if label == SECRET_LABEL {
        "the key that protects its bucket secret"
    } else {
        "its encryption key"
    };
    let (kind, message) = match error {
        StoreError::Denied(code) => (
            SyncErrorKind::KeyDenied,
            format!(
                "macOS didn't allow Voice to {what} {item} in the login keychain (error {code}). Choose Try again in Settings > Storage, then Always Allow when macOS asks."
            ),
        ),
        StoreError::Other(code) => (
            SyncErrorKind::Key,
            format!("The login keychain couldn't {what} {item} (error {code}). Try again."),
        ),
        StoreError::Missing => (
            SyncErrorKind::Key,
            "Voice can't find the login keychain item that holds its encryption key. Import the key from its recovery code, or save the bucket secret again.".to_owned(),
        ),
    };
    log::warn!("sync: {message} (account {account})");
    SyncError::new(kind, message)
}

impl<S: ItemStore> Protector for Keychain<S> {
    fn protect(&self, plain: &[u8], label: &str) -> Result<Vec<u8>, SyncError> {
        let key = self.wrapping_key(label, true)?;
        let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| {
            SyncError::new(
                SyncErrorKind::Key,
                "The login keychain item that opens Voice's encryption key file is damaged.",
            )
        })?;
        let mut nonce = [0u8; NONCE_LEN];
        random(&mut nonce)?;
        let sealed = cipher
            .encrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: plain,
                    aad: label.as_bytes(),
                },
            )
            .map_err(|_| {
                SyncError::new(
                    SyncErrorKind::Key,
                    "Voice couldn't seal the data with its login keychain key.",
                )
            })?;
        let mut blob = Vec::with_capacity(BLOB_MARKER.len() + NONCE_LEN + sealed.len());
        blob.extend_from_slice(BLOB_MARKER);
        blob.extend_from_slice(&nonce);
        blob.extend_from_slice(&sealed);
        Ok(blob)
    }

    fn unprotect(&self, blob: &[u8], label: &str) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let refused = || {
            SyncError::new(
                SyncErrorKind::Key,
                "Voice can't open this file with the key in the login keychain. The file or the key was changed.",
            )
        };
        if blob.len() < BLOB_MARKER.len() + NONCE_LEN + TAG_LEN || !blob.starts_with(BLOB_MARKER) {
            return Err(refused());
        }
        let key = self.wrapping_key(label, false)?;
        let cipher = XChaCha20Poly1305::new_from_slice(&*key).map_err(|_| refused())?;
        let mut nonce = [0u8; NONCE_LEN];
        nonce.copy_from_slice(&blob[BLOB_MARKER.len()..BLOB_MARKER.len() + NONCE_LEN]);
        let sealed = &blob[BLOB_MARKER.len() + NONCE_LEN..];
        let plain = cipher
            .decrypt(
                &XNonce::from(nonce),
                Payload {
                    msg: sealed,
                    aad: label.as_bytes(),
                },
            )
            .map_err(|_| refused())?;
        Ok(Zeroizing::new(plain))
    }
}

/// The macOS protector for `data_dir`, as [`super::system`] hands it out.
pub fn system(data_dir: &Path) -> Arc<dyn Protector> {
    Arc::new(Keychain::new(data_dir))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protect::SECRET_LABEL;

    /// Enough to name accounts; these tests never reach a store.
    struct NoStore;

    impl ItemStore for NoStore {
        fn get(&self, _account: &str) -> Result<Vec<u8>, StoreError> {
            Err(StoreError::Missing)
        }

        fn set(&self, _account: &str, _secret: &[u8]) -> Result<(), StoreError> {
            Ok(())
        }

        fn delete(&self, _account: &str) -> Result<(), StoreError> {
            Err(StoreError::Missing)
        }
    }

    fn account(dir: &Path) -> String {
        Keychain::with_store(dir, NoStore)
            .account(SECRET_LABEL)
            .expect("the folder exists")
    }

    #[test]
    fn the_folder_hash_is_stable_and_short() {
        let dir = std::env::temp_dir();
        let named = account(&dir);
        let hash = named.strip_prefix("rukoo-voice/bucket-secret/v1:").unwrap();
        assert_eq!(hash.len(), HASH_CHARS);
        assert!(hash.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(named, account(&dir));
        // The canonical path, so a relative path and its absolute folder agree.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(account(Path::new(".")), account(&cwd));
    }

    #[test]
    fn refusals_are_told_from_a_missing_item() {
        use security_framework::base::Error;
        for code in [-128, -25293, -25308, -25320] {
            assert_eq!(
                store_error(Error::from_code(code)),
                StoreError::Denied(code)
            );
        }
        assert_eq!(store_error(Error::from_code(-25300)), StoreError::Missing);
        assert_eq!(
            store_error(Error::from_code(-25291)),
            StoreError::Other(-25291)
        );
    }

    #[test]
    fn a_folder_voice_cant_reach_is_a_key_error_naming_it() {
        let dir = std::env::temp_dir().join(format!("voice-sync-nowhere-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let keychain = Keychain::with_store(&dir, NoStore);
        let error = keychain.account(SECRET_LABEL).unwrap_err();
        assert_eq!(error.kind, SyncErrorKind::Key);
        assert!(
            error.message.contains(&dir.display().to_string()),
            "{}",
            error.message
        );
    }
}
