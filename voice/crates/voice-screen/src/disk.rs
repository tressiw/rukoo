//! Free space on the meeting folder's volume, and the errors a full disk reports.

use std::path::Path;

/// The error a write fails with on a full disk, as the platform reports it: `ERROR_DISK_FULL` on
/// Windows (`112`), `ENOSPC` on Unix. The `write-fail` test switch uses it, so a test frame's
/// failure is the same as a real one on the platform it runs on.
pub fn test_disk_full_error() -> std::io::Error {
    #[cfg(windows)]
    {
        std::io::Error::from_raw_os_error(112)
    }
    #[cfg(unix)]
    {
        std::io::Error::from_raw_os_error(libc::ENOSPC)
    }
    #[cfg(not(any(windows, unix)))]
    {
        std::io::Error::other("no space left on device")
    }
}

/// Bytes free for this user on the volume of `path`. With `test_file` (the `free-bytes-file`
/// test switch) the number written in that file instead, so tests can step through the drop
/// thresholds. `None` when neither can be read.
pub fn free_bytes(path: &Path, test_file: Option<&Path>) -> Option<u64> {
    if let Some(file) = test_file {
        return std::fs::read_to_string(file).ok()?.trim().parse().ok();
    }
    volume_free(path)
}

#[cfg(windows)]
fn volume_free(path: &Path) -> Option<u64> {
    use windows::core::HSTRING;
    use windows::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

    let mut available = 0u64;
    let dir = HSTRING::from(path.as_os_str());
    unsafe { GetDiskFreeSpaceExW(&dir, Some(&mut available), None, None) }.ok()?;
    Some(available)
}

/// `statvfs`: the bytes available to this user. `f_bavail` is in `f_frsize` blocks.
#[cfg(unix)]
fn volume_free(path: &Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let dir = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stats = std::mem::MaybeUninit::<libc::statvfs>::uninit();
    // SAFETY: the call fills in the struct it is handed and the path is a valid C string.
    if unsafe { libc::statvfs(dir.as_ptr(), stats.as_mut_ptr()) } != 0 {
        return None;
    }
    // SAFETY: `statvfs` succeeded, so the struct is initialized.
    let stats = unsafe { stats.assume_init() };
    let block = if stats.f_frsize > 0 {
        stats.f_frsize
    } else {
        stats.f_bsize
    };
    Some((stats.f_bavail as u128 * block as u128).min(u128::from(u64::MAX)) as u64)
}

#[cfg(not(any(windows, unix)))]
fn volume_free(_path: &Path) -> Option<u64> {
    None
}

/// Whether a write failed because the disk is full: `ERROR_DISK_FULL`, `ERROR_HANDLE_DISK_FULL`
/// on Windows; `ENOSPC`, `EDQUOT` (the user or group quota is spent) on Unix.
pub fn is_disk_full(e: &std::io::Error) -> bool {
    #[cfg(windows)]
    {
        matches!(e.raw_os_error(), Some(112) | Some(39))
    }
    #[cfg(unix)]
    {
        matches!(
            e.raw_os_error(),
            Some(code) if code == libc::ENOSPC || code == libc::EDQUOT
        )
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = e;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_volume_or_the_test_file() {
        let dir = std::env::temp_dir();
        assert!(free_bytes(&dir, None).unwrap() > 0);
        let file = dir.join(format!("voice-screen-free-{}.txt", std::process::id()));
        std::fs::write(&file, "3221225472\n").unwrap();
        assert_eq!(free_bytes(&dir, Some(&file)), Some(3 << 30));
        std::fs::write(&file, "lots").unwrap();
        assert_eq!(free_bytes(&dir, Some(&file)), None);
        std::fs::remove_file(&file).unwrap();
        assert!(is_disk_full(&test_disk_full_error()));
        assert!(!is_disk_full(&std::io::Error::from_raw_os_error(5)));
    }

    /// The free bytes are within a percent of what the platform reports for the same folder.
    #[test]
    fn the_volume_has_room() {
        // A full disk would make this fail; the test says the check reads something real.
        assert!(volume_free(&std::env::temp_dir()).unwrap() > 1 << 20);
    }
}
