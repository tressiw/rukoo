//! The master key and the bucket secret in `<data>/sync` (spec 2.3, 2.4).

use std::fmt;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::UNIX_EPOCH;

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use crate::names::{hex, hmac_sha256};
use crate::protect::{
    self, Protector, MASTER_KEY_LABEL, MASTER_KEY_MAGIC, SECRET_LABEL, SECRET_MAGIC,
};
use crate::{SyncError, SyncErrorKind};

/// The DPAPI-protected master key: "RKVK", version 1, the blob.
pub const MASTER_KEY_FILE: &str = "master-key.bin";
/// The key's id and dates, no secret: [`KeyFile`].
pub const KEY_FILE: &str = "key.json";
/// The DPAPI-protected bucket secret: "RKVS", version 1, the blob.
pub const SECRET_FILE: &str = "bucket-secret.bin";

/// Zeroized on drop; its Debug prints only the kid.
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// 32 bytes from `getrandom::fill`.
    pub fn generate() -> Result<MasterKey, SyncError> {
        let mut key = MasterKey([0u8; 32]);
        random(&mut key.0)?;
        Ok(key)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> MasterKey {
        MasterKey(bytes)
    }

    /// 16 hex characters: `hex(HMAC(MK, "rukoo-voice/kid/v1"))[0..16]`.
    pub fn kid(&self) -> String {
        hex(&self.kid_bytes())
    }

    /// The kid as the 8 raw bytes an `RKV1` header carries.
    pub(crate) fn kid_bytes(&self) -> [u8; 8] {
        let mac = Zeroizing::new(hmac_sha256(&self.0, &[b"rukoo-voice/kid/v1"]));
        let mut kid = [0u8; 8];
        kid.copy_from_slice(&mac[..8]);
        kid
    }

    /// `HMAC(MK, "rukoo-voice/names/v1")`.
    pub fn name_key(&self) -> [u8; 32] {
        hmac_sha256(&self.0, &[b"rukoo-voice/names/v1"])
    }

    /// `HMAC(MK, "rukoo-voice/wrap/v1")`. Callers keep it in a `Zeroizing`.
    pub fn wrap_key(&self) -> [u8; 32] {
        hmac_sha256(&self.0, &[b"rukoo-voice/wrap/v1"])
    }

    /// The raw key, for the recovery code and the protected file.
    pub(crate) fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Drop for MasterKey {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl fmt::Debug for MasterKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("MasterKey").field(&self.kid()).finish()
    }
}

/// `key.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyFile {
    pub kid: String,
    pub created_at: u64,
    pub exported_at: Option<u64>,
}

/// <data>/sync: master-key.bin, key.json, bucket-secret.bin.
pub struct KeyStore {
    dir: PathBuf,
    protector: Arc<dyn Protector>,
}

impl KeyStore {
    pub fn new(sync_dir: &Path, protector: Arc<dyn Protector>) -> KeyStore {
        KeyStore {
            dir: sync_dir.to_path_buf(),
            protector,
        }
    }

    /// Ok(None) without a key; Err(Key) when the protector can't open the file, and
    /// Err(KeyDenied) when the system refused Voice the keychain item that opens it.
    ///
    /// The key is written before `key.json`, so a crash between the two leaves a key whose
    /// `key.json` is missing or names the key before it. Loading writes it again then, dated
    /// like the key file and not exported, so Settings asks the user to save the key.
    pub fn load(&self) -> Result<Option<MasterKey>, SyncError> {
        let path = self.dir.join(MASTER_KEY_FILE);
        let Some(plain) = protect::read_protected(
            &path,
            MASTER_KEY_MAGIC,
            MASTER_KEY_LABEL,
            self.protector.as_ref(),
        )?
        else {
            return Ok(None);
        };
        let mut key = MasterKey([0u8; 32]);
        if plain.len() != key.0.len() {
            return Err(SyncError::new(
                SyncErrorKind::Key,
                "The encryption key file on this computer is damaged.",
            ));
        }
        key.0.copy_from_slice(&plain);
        let kid = key.kid();
        if self.info().is_none_or(|info| info.kid != kid) {
            let created_at = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_millis() as u64);
            self.write_info(&KeyFile {
                kid,
                created_at,
                exported_at: None,
            })?;
        }
        Ok(Some(key))
    }

    /// Makes and keeps a new key. Refused when this computer has a key file already, even one
    /// DPAPI can't open: replacing a key is [`replace`](Self::replace)'s job.
    pub fn create(&self, now: u64) -> Result<MasterKey, SyncError> {
        if self.dir.join(MASTER_KEY_FILE).exists() {
            return Err(SyncError::new(
                SyncErrorKind::Local,
                "This computer has an encryption key already.",
            ));
        }
        let key = MasterKey::generate()?;
        self.write_key(&key, now, None)?;
        Ok(key)
    }

    /// Uses `key` from now on, such as one imported from a recovery code. `exportedAt` becomes
    /// `now`: whoever typed the code has it saved.
    pub fn replace(&self, key: &MasterKey, now: u64) -> Result<(), SyncError> {
        self.write_key(key, now, Some(now))
    }

    pub fn info(&self) -> Option<KeyFile> {
        let bytes = fs::read(self.dir.join(KEY_FILE)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn mark_exported(&self, at: u64) -> Result<(), SyncError> {
        let Some(mut info) = self.info() else {
            return Err(SyncError::new(
                SyncErrorKind::Key,
                "This computer has no encryption key yet.",
            ));
        };
        info.exported_at = Some(at);
        self.write_info(&info)
    }

    pub fn save_secret(&self, secret: &str) -> Result<(), SyncError> {
        if secret.is_empty() {
            return Err(SyncError::new(
                SyncErrorKind::NoSecret,
                "The secret access key is empty.",
            ));
        }
        protect::write_protected(
            &self.dir.join(SECRET_FILE),
            SECRET_MAGIC,
            SECRET_LABEL,
            secret.as_bytes(),
            self.protector.as_ref(),
        )
    }

    /// Ok(None) without a saved secret. A secret Windows can't open any more (another user, a
    /// password reset) or a damaged file is `NoSecret`: the user saves it again. A keychain that
    /// refused Voice now stays `KeyDenied`: the secret is fine, and allowing Voice opens it.
    pub fn secret(&self) -> Result<Option<Zeroizing<String>>, SyncError> {
        let mut plain = match protect::read_protected(
            &self.dir.join(SECRET_FILE),
            SECRET_MAGIC,
            SECRET_LABEL,
            self.protector.as_ref(),
        ) {
            Ok(Some(plain)) => plain,
            Ok(None) => return Ok(None),
            Err(error) if error.kind == SyncErrorKind::Key => {
                return Err(SyncError::new(
                    SyncErrorKind::NoSecret,
                    "Voice can't open the saved secret on this computer. Save it again.",
                ))
            }
            Err(error) => return Err(error),
        };
        match String::from_utf8(std::mem::take(&mut *plain)) {
            Ok(text) => Ok(Some(Zeroizing::new(text))),
            Err(error) => {
                error.into_bytes().zeroize();
                Err(SyncError::new(
                    SyncErrorKind::NoSecret,
                    "The saved secret is damaged. Save it again.",
                ))
            }
        }
    }

    pub fn remove_secret(&self) -> Result<(), SyncError> {
        match fs::remove_file(self.dir.join(SECRET_FILE)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(local("remove the saved secret", &e)),
        }
    }

    fn write_key(
        &self,
        key: &MasterKey,
        now: u64,
        exported_at: Option<u64>,
    ) -> Result<(), SyncError> {
        protect::write_protected(
            &self.dir.join(MASTER_KEY_FILE),
            MASTER_KEY_MAGIC,
            MASTER_KEY_LABEL,
            key.bytes(),
            self.protector.as_ref(),
        )?;
        self.write_info(&KeyFile {
            kid: key.kid(),
            created_at: now,
            exported_at,
        })
    }

    fn write_info(&self, info: &KeyFile) -> Result<(), SyncError> {
        let bytes = serde_json::to_vec(info).expect("KeyFile serializes");
        protect::write_atomic(&self.dir.join(KEY_FILE), &bytes)
            .map_err(|e| local("save key.json", &e))
    }
}

/// Fills `buf` from the system's random source.
pub(crate) fn random(buf: &mut [u8]) -> Result<(), SyncError> {
    getrandom::fill(buf).map_err(|e| {
        SyncError::new(
            SyncErrorKind::Local,
            format!("The system's random number source failed: {e}"),
        )
    })
}

pub(crate) fn local(what: &str, error: &io::Error) -> SyncError {
    SyncError::new(SyncErrorKind::Local, format!("Couldn't {what}: {error}"))
}
