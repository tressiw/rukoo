//! Syncs to disk. Every sync the recorder makes goes through [`sync`], so the test option
//! `--test-sync-delay-ms` can make all of them slow, as on a busy disk, and the tests can check
//! that no time in the timeline depends on how long a sync takes.

use std::fs::File;
use std::io;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static TEST_DELAY_MS: AtomicU64 = AtomicU64::new(0);

/// Makes every later [`sync`] in this process wait `ms` first. Only tests set it.
pub fn set_test_delay(ms: u64) {
    TEST_DELAY_MS.store(ms, Ordering::Relaxed);
}

/// Asks the OS to put the file's data on the disk.
pub fn sync(file: &File) -> io::Result<()> {
    let ms = TEST_DELAY_MS.load(Ordering::Relaxed);
    if ms > 0 {
        std::thread::sleep(Duration::from_millis(ms));
    }
    file.sync_data()
}
