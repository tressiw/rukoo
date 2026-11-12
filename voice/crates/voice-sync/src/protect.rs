//! Bytes only this Windows user can read: the master key and the bucket secret (spec 2.3, D4).
//!
//! A protected file is the 4-byte magic, a version byte `1`, then the protector's blob. Files are
//! written to a temporary name and renamed.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::keys::local;
use crate::{SyncError, SyncErrorKind};

/// Magic of `master-key.bin`.
pub const MASTER_KEY_MAGIC: &[u8; 4] = b"RKVK";
/// Magic of `bucket-secret.bin`.
pub const SECRET_MAGIC: &[u8; 4] = b"RKVS";
/// DPAPI entropy of the master key.
pub const MASTER_KEY_LABEL: &str = "rukoo-voice/master-key/v1";
/// DPAPI entropy of the bucket secret.
pub const SECRET_LABEL: &str = "rukoo-voice/bucket-secret/v1";

/// The version byte after the magic.
const FILE_VERSION: u8 = 1;

/// Keeps bytes readable only by this Windows user (DPAPI), or a stand-in in tests.
///
/// `protect` fails with `Local`; `unprotect` fails with `Key` when the blob isn't one this user
/// and label can open.
pub trait Protector: Send + Sync {
    fn protect(&self, plain: &[u8], label: &str) -> Result<Vec<u8>, SyncError>;
    fn unprotect(&self, blob: &[u8], label: &str) -> Result<Zeroizing<Vec<u8>>, SyncError>;
}

/// CryptProtectData / CryptUnprotectData (windows-0.62.2 Security/Cryptography/mod.rs:1659,1872)
/// with CRYPTPROTECT_UI_FORBIDDEN (:8271), the label as entropy, and LocalFree on the output
/// (Win32/Foundation/mod.rs:38). CryptUnprotectData's output is zeroed before LocalFree, which
/// frees without clearing; the copy it returns is Zeroizing.
#[cfg(windows)]
pub struct Dpapi;

#[cfg(windows)]
impl Protector for Dpapi {
    fn protect(&self, plain: &[u8], label: &str) -> Result<Vec<u8>, SyncError> {
        use windows::core::PCWSTR;
        use windows::Win32::Security::Cryptography::{
            CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let input = dpapi::blob(plain)?;
        let entropy = dpapi::blob(label.as_bytes())?;
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: `input` and `entropy` point into slices that outlive the call; DPAPI only reads
        // them. On success `out` holds a LocalAlloc'd buffer that `dpapi::take` copies and frees.
        unsafe {
            CryptProtectData(
                &input,
                PCWSTR::null(),
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| {
            SyncError::new(
                SyncErrorKind::Local,
                format!("Windows couldn't protect the data: {e}"),
            )
        })?;
        // SAFETY: `out` came from a successful CryptProtectData.
        Ok(unsafe { dpapi::take(&mut out, false) }.to_vec())
    }

    fn unprotect(&self, blob: &[u8], label: &str) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        use windows::Win32::Security::Cryptography::{
            CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        };

        let input = dpapi::blob(blob)?;
        let entropy = dpapi::blob(label.as_bytes())?;
        let mut out = CRYPT_INTEGER_BLOB::default();
        // SAFETY: as in `protect`.
        unsafe {
            CryptUnprotectData(
                &input,
                None,
                Some(&entropy),
                None,
                None,
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        }
        .map_err(|e| {
            SyncError::new(
                SyncErrorKind::Key,
                format!("Windows can't open the protected data for this user: {e}"),
            )
        })?;
        // SAFETY: `out` came from a successful CryptUnprotectData.
        Ok(unsafe { dpapi::take(&mut out, true) })
    }
}

#[cfg(windows)]
mod dpapi {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB;
    use zeroize::{Zeroize, Zeroizing};

    use crate::{SyncError, SyncErrorKind};

    /// A blob that points at `bytes`; DPAPI never writes through it.
    pub(super) fn blob(bytes: &[u8]) -> Result<CRYPT_INTEGER_BLOB, SyncError> {
        let len = u32::try_from(bytes.len())
            .map_err(|_| SyncError::new(SyncErrorKind::Local, "The data is too large."))?;
        Ok(CRYPT_INTEGER_BLOB {
            cbData: len,
            pbData: bytes.as_ptr().cast_mut(),
        })
    }

    /// Copies DPAPI's output, zeroes the original when `wipe`, and frees it.
    ///
    /// # Safety
    /// `out` must hold a buffer DPAPI allocated with LocalAlloc, or be empty.
    pub(super) unsafe fn take(out: &mut CRYPT_INTEGER_BLOB, wipe: bool) -> Zeroizing<Vec<u8>> {
        if out.pbData.is_null() {
            return Zeroizing::new(Vec::new());
        }
        let len = out.cbData as usize;
        // SAFETY: DPAPI wrote `cbData` bytes at `pbData`.
        let data = unsafe { std::slice::from_raw_parts_mut(out.pbData, len) };
        let copy = Zeroizing::new(data.to_vec());
        if wipe {
            data.zeroize();
        }
        // SAFETY: the buffer came from LocalAlloc and isn't used after this.
        unsafe {
            LocalFree(Some(HLOCAL(out.pbData.cast())));
        }
        out.pbData = std::ptr::null_mut();
        out.cbData = 0;
        copy
    }
}

/// For tests on every platform: XORs with a stream derived from the label and adds a marker, so a
/// test can tell a protected blob from plain bytes. Used only by tests and the fake engine.
///
/// The blob is `TPRT`, the XORed bytes, and 8 bytes of a SHA-256 over the label and the plain
/// bytes, so the wrong label or a changed blob is refused like DPAPI refuses them. It hides
/// nothing from anyone who reads this file.
pub struct TestProtector;

const TEST_MARKER: &[u8; 4] = b"TPRT";
const TEST_TAG: usize = 8;

impl TestProtector {
    fn xor(data: &mut [u8], label: &str) {
        for (block, chunk) in data.chunks_mut(32).enumerate() {
            let mut hash = Sha256::new();
            hash.update(b"rukoo-voice/test-protector/stream\0");
            hash.update(label.as_bytes());
            hash.update((block as u64).to_be_bytes());
            let stream = hash.finalize();
            for (byte, mask) in chunk.iter_mut().zip(stream.iter()) {
                *byte ^= mask;
            }
        }
    }

    fn tag(plain: &[u8], label: &str) -> [u8; TEST_TAG] {
        let mut hash = Sha256::new();
        hash.update(b"rukoo-voice/test-protector/tag\0");
        hash.update(label.as_bytes());
        hash.update([0u8]);
        hash.update(plain);
        let mut tag = [0u8; TEST_TAG];
        tag.copy_from_slice(&hash.finalize()[..TEST_TAG]);
        tag
    }
}

impl Protector for TestProtector {
    fn protect(&self, plain: &[u8], label: &str) -> Result<Vec<u8>, SyncError> {
        let mut blob = Vec::with_capacity(TEST_MARKER.len() + plain.len() + TEST_TAG);
        blob.extend_from_slice(TEST_MARKER);
        blob.extend_from_slice(plain);
        TestProtector::xor(&mut blob[TEST_MARKER.len()..], label);
        blob.extend_from_slice(&TestProtector::tag(plain, label));
        Ok(blob)
    }

    fn unprotect(&self, blob: &[u8], label: &str) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        let refused = || {
            SyncError::new(
                SyncErrorKind::Key,
                "The test protector can't open this blob.",
            )
        };
        if blob.len() < TEST_MARKER.len() + TEST_TAG || !blob.starts_with(TEST_MARKER) {
            return Err(refused());
        }
        let (body, tag) =
            blob[TEST_MARKER.len()..].split_at(blob.len() - TEST_MARKER.len() - TEST_TAG);
        let mut plain = Zeroizing::new(body.to_vec());
        TestProtector::xor(&mut plain, label);
        if TestProtector::tag(&plain, label) != tag {
            return Err(refused());
        }
        Ok(plain)
    }
}

/// Refuses everything: bucket sync uses DPAPI and runs only on Windows.
#[cfg(not(windows))]
struct Unsupported;

#[cfg(not(windows))]
impl Protector for Unsupported {
    fn protect(&self, _plain: &[u8], _label: &str) -> Result<Vec<u8>, SyncError> {
        Err(SyncError::new(
            SyncErrorKind::Local,
            "Bucket sync needs Windows.",
        ))
    }

    fn unprotect(&self, _blob: &[u8], _label: &str) -> Result<Zeroizing<Vec<u8>>, SyncError> {
        Err(SyncError::new(
            SyncErrorKind::Key,
            "Bucket sync needs Windows.",
        ))
    }
}

/// Whether this platform has a protector that keeps sync's keys from other users: DPAPI on Windows,
/// and nothing yet on macOS (its Keychain protector comes with the port) or on Linux.
///
/// Settings read this to decide whether to offer bucket sync at all (`features.sync` and
/// `sync.available` in voice/ui/API.md), so it changes in the same place as the protector does.
pub const SUPPORTED: bool = cfg!(windows);

/// Dpapi on Windows; elsewhere a protector that refuses ("Bucket sync needs Windows.").
pub fn system() -> Arc<dyn Protector> {
    #[cfg(windows)]
    {
        Arc::new(Dpapi)
    }
    #[cfg(not(windows))]
    {
        Arc::new(Unsupported)
    }
}

/// `RKVK` / `RKVS` files (2.3), written to a temporary name and renamed.
///
/// Ok(None) when the file doesn't exist. A file with another magic or version, or one the
/// protector can't open, is `Key`; a file that can't be read is `Local`.
pub fn read_protected(
    path: &Path,
    magic: &[u8; 4],
    label: &str,
    p: &dyn Protector,
) -> Result<Option<Zeroizing<Vec<u8>>>, SyncError> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(local(&format!("read {}", file_name(path)), &e)),
    };
    if bytes.len() <= magic.len() || &bytes[..magic.len()] != magic {
        return Err(SyncError::new(
            SyncErrorKind::Key,
            format!("{} isn't a protected Voice file.", file_name(path)),
        ));
    }
    if bytes[magic.len()] != FILE_VERSION {
        return Err(SyncError::new(
            SyncErrorKind::Key,
            format!("{} was written by a newer Voice.", file_name(path)),
        ));
    }
    p.unprotect(&bytes[magic.len() + 1..], label).map(Some)
}

pub fn write_protected(
    path: &Path,
    magic: &[u8; 4],
    label: &str,
    plain: &[u8],
    p: &dyn Protector,
) -> Result<(), SyncError> {
    let blob = p.protect(plain, label)?;
    let mut bytes = Vec::with_capacity(magic.len() + 1 + blob.len());
    bytes.extend_from_slice(magic);
    bytes.push(FILE_VERSION);
    bytes.extend_from_slice(&blob);
    write_atomic(path, &bytes).map_err(|e| local(&format!("save {}", file_name(path)), &e))
}

/// Replaces `path` with `bytes` so a crash leaves either the old file or the new one: a temporary
/// file next to it, flushed to disk, then renamed over the old one. Creates the folder.
pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(dir)?;
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let temp: PathBuf = dir.join(format!(
        ".{}-{}-{}-{}.tmp",
        file_name(path),
        std::process::id(),
        nanos,
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_test_protector_round_trips_and_refuses_the_wrong_label() {
        let p = TestProtector;
        let blob = p.protect(b"secret bytes", "a").unwrap();
        assert!(blob.starts_with(TEST_MARKER));
        assert!(!blob.windows(12).any(|w| w == b"secret bytes"));
        assert_eq!(&*p.unprotect(&blob, "a").unwrap(), b"secret bytes");
        assert_eq!(
            p.unprotect(&blob, "b").unwrap_err().kind,
            SyncErrorKind::Key
        );
        let mut changed = blob.clone();
        changed[5] ^= 1;
        assert!(p.unprotect(&changed, "a").is_err());
        assert!(p.unprotect(b"TPRT", "a").is_err());
        assert_eq!(
            &*p.unprotect(&p.protect(b"", "a").unwrap(), "a").unwrap(),
            b""
        );
    }
}
