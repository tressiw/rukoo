//! The clocks the screen process reads on macOS. `boot` is `mach_continuous_time`: it counts
//! sleep, every process shares it, and the recorder reads it next to `t` in its `session` and
//! `heartbeat` lines, so `voice_screen_files::clock::ScreenClock` maps it onto the meeting clock.
//! `awake` is `mach_absolute_time`, which stops while the computer sleeps, for the orphan rule.

use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use voice_screen_files::clock::Now;

/// `mach_timebase_info`: one tick of the mach counters is `numer / denom` nanoseconds. Apple
/// Silicon reports 125/3, Intel Macs 1/1. Its layout is the C struct the call fills in.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Timebase {
    numer: u32,
    denom: u32,
}

extern "C" {
    /// Ticks since boot, counting the time the computer slept.
    fn mach_continuous_time() -> u64;
    /// Ticks since boot, stopping while the computer sleeps.
    fn mach_absolute_time() -> u64;
    /// The timebase in nanoseconds per tick; it never changes during a boot.
    fn mach_timebase_info(info: *mut Timebase) -> i32;
}

fn timebase() -> Timebase {
    static TIMEBASE: OnceLock<Timebase> = OnceLock::new();
    *TIMEBASE.get_or_init(|| {
        let mut info = Timebase { numer: 1, denom: 1 };
        // SAFETY: the call only writes the two u32 fields of `info`. Should it fail, 1/1 is what
        // its result then stands for: nanoseconds per tick.
        unsafe { mach_timebase_info(&mut info) };
        info
    })
}

/// Ticks in 100 ns units, the unit of `boot` and of the Windows interrupt time. Whole units are
/// dropped. An absurd tick count saturates instead of wrapping.
fn to_100ns(ticks: u64) -> u64 {
    let base = timebase();
    let denom = (u128::from(base.denom) * 100).max(1);
    let units = u128::from(ticks) * u128::from(base.numer) / denom;
    u64::try_from(units).unwrap_or(u64::MAX)
}

/// Interrupt time in 100 ns units: counts sleep.
pub fn boot_100ns() -> u64 {
    // SAFETY: the call takes no arguments and only returns the counter.
    to_100ns(unsafe { mach_continuous_time() })
}

/// Unbiased interrupt time in 100 ns units: awake time only, for the 60 s orphan rule, so a
/// computer that slept doesn't count as a dead recorder.
pub fn awake_100ns() -> u64 {
    // SAFETY: the call takes no arguments and only returns the counter.
    to_100ns(unsafe { mach_absolute_time() })
}

/// Unix time in ms.
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Both clocks now, for `ScreenClock`.
pub fn now() -> Now {
    Now {
        boot: Some(boot_100ns()),
        wall: wall_ms(),
    }
}

/// The clocks at an earlier boot time: the wall clock moved back by the same age.
pub fn at_boot(boot: u64) -> Now {
    let now_boot = boot_100ns();
    let age_ms = now_boot.saturating_sub(boot) / 10_000;
    Now {
        boot: Some(boot),
        wall: wall_ms().saturating_sub(age_ms),
    }
}

/// The boot time of a frame whose presentation time was `pts_100ns` on the host clock, which is
/// `mach_absolute_time` and stops while the computer sleeps. Only the frame's age crosses over: it
/// is how far the awake counter has moved since, taken off the continuous counter. A frame
/// captured before a sleep and delivered after it lands after the sleep, because the continuous
/// counter counts the sleep and the awake one doesn't; its picture is the same either way. A time
/// in the future counts as now.
pub fn frame_boot(pts_100ns: u64) -> u64 {
    let (boot, awake) = paired(|| {
        // SAFETY: the calls take no arguments and only return the counters.
        unsafe { (mach_continuous_time(), mach_absolute_time()) }
    });
    frame_boot_at(boot, awake, pts_100ns)
}

/// The two counters a frame's age crosses between are read close together: within this many
/// 100 ns units, 100 µs.
const PAIR_GAP: u64 = 1_000;
/// Reads before the closest pair is taken as it is.
const PAIR_TRIES: u32 = 4;

/// The continuous and the absolute counter at one moment, in 100 ns units. A thread preempted
/// between two reads would move the frame by the time it lost, so the continuous counter is read
/// on both sides of the absolute one and the pair is read again when they are more than
/// [`PAIR_GAP`] apart; after [`PAIR_TRIES`] the closest pair counts, with the middle of its two
/// continuous reads.
fn paired(mut read: impl FnMut() -> (u64, u64)) -> (u64, u64) {
    let mut best: Option<(u64, u64, u64)> = None;
    for _ in 0..PAIR_TRIES {
        let (before, awake) = read();
        let (after, _) = read();
        let (before, after, awake) = (to_100ns(before), to_100ns(after), to_100ns(awake));
        let gap = after.saturating_sub(before);
        let boot = before + gap / 2;
        if gap <= PAIR_GAP {
            return (boot, awake);
        }
        if best.is_none_or(|(g, _, _)| gap < g) {
            best = Some((gap, boot, awake));
        }
    }
    best.map_or((0, 0), |(_, boot, awake)| (boot, awake))
}

fn frame_boot_at(boot_now: u64, awake_now: u64, pts_100ns: u64) -> u64 {
    boot_now.saturating_sub(awake_now.saturating_sub(pts_100ns))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Divides the ticks the way the timebase says; on this Mac one tick is about 41.7 ns.
    #[test]
    fn the_timebase_is_plausible() {
        let base = timebase();
        assert!(base.numer > 0 && base.denom > 0, "{base:?}");
        let ns_per_tick = f64::from(base.numer) / f64::from(base.denom);
        assert!(
            (1.0..=100.0).contains(&ns_per_tick),
            "{ns_per_tick} ns a tick"
        );
        assert_eq!(to_100ns(0), 0);
    }

    /// Appends `<crate> <test>: <reason>` to the file `RUKOO_VOICE_SKIP_LOG` names. A skip prints to
    /// stderr, which libtest hides for a passing test; CI's `skip-log.mjs report` step reads this file
    /// back. Without the variable nothing is written.
    fn log_skip(test: &str, reason: &str) {
        use std::io::Write;
        let Some(path) = std::env::var_os("RUKOO_VOICE_SKIP_LOG").filter(|p| !p.is_empty()) else {
            return;
        };
        // One write of the whole line, so tests running in parallel threads don't interleave.
        let line = format!("{} {test}: {reason}\n", env!("CARGO_PKG_NAME"));
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            let _ = file.write_all(line.as_bytes());
        }
    }

    /// True, after printing why, when CI's host probe found the host too stalled for timing
    /// windows (`voice/scripts/host-probe.mjs` sets `RUKOO_VOICE_SLOW_HOST` for the test step).
    fn skip_on_slow_host(test: &str) -> bool {
        let slow = std::env::var_os("RUKOO_VOICE_SLOW_HOST").is_some_and(|v| !v.is_empty());
        if slow {
            let reason = "RUKOO_VOICE_SLOW_HOST is set, this host stalls longer than its timing windows allow";
            eprintln!("skipped {test}: {reason}");
            log_skip(test, reason);
        }
        slow
    }

    #[test]
    fn both_counters_move_with_the_clock() {
        // A late wake-up of the sleeping thread moves both counters, and the window below
        // can't tell that from a counter that runs wrong.
        if skip_on_slow_host("both_counters_move_with_the_clock") {
            return;
        }
        let (b0, a0) = (boot_100ns(), awake_100ns());
        std::thread::sleep(std::time::Duration::from_millis(200));
        let (b1, a1) = (boot_100ns(), awake_100ns());
        // 100 ns units: 200 ms is 2,000,000, and the sleeps on this class of machine stay
        // within a few ms of it.
        assert!((1_500_000..=2_500_000).contains(&(b1 - b0)), "{}", b1 - b0);
        assert!((1_500_000..=2_500_000).contains(&(a1 - a0)), "{}", a1 - a0);
        // Continuous time counts at least as much as the awake counter; the two are read a
        // moment apart, so that moment is allowed for.
        assert!(b1 + 10_000 >= a1, "{b1} vs {a1}");
    }

    #[test]
    fn at_boot_moves_the_wall_clock_back() {
        let boot = boot_100ns();
        let now = at_boot(boot.saturating_sub(100 * 10_000));
        assert_eq!(now.boot, Some(boot.saturating_sub(100 * 10_000)));
        let age = wall_ms().saturating_sub(now.wall);
        assert!((90..=400).contains(&age), "{age} ms");
    }

    /// The two counters are taken from reads close together: a pair read across a preemption is
    /// read again, and after the last try the closest pair counts.
    #[test]
    fn the_counters_pair_up_close_together() {
        let base = timebase();
        // Ticks for a number of 100 ns units, on this Mac's timebase.
        let ticks = |units: u64| units * 100 * u64::from(base.denom) / u64::from(base.numer);
        // The first pair straddles a 5 ms preemption; the second is close.
        let mut reads = vec![
            (ticks(10_000), ticks(9_000)),
            (ticks(60_000), 0),
            (ticks(70_000), ticks(69_000)),
            (ticks(70_100), 0),
        ]
        .into_iter();
        let (boot, awake) = paired(|| reads.next().unwrap());
        assert!(boot.abs_diff(70_050) <= 2, "{boot}");
        assert!(awake.abs_diff(69_000) <= 2, "{awake}");
        // Never close: the narrowest of the four tries.
        let mut reads = [50_000, 50_000, 2_000, 50_000]
            .into_iter()
            .enumerate()
            .flat_map(|(n, gap)| {
                let before = (n as u64 + 1) * 1_000_000;
                [(ticks(before), ticks(before)), (ticks(before + gap), 0)]
            });
        let (boot, awake) = paired(|| reads.next().unwrap());
        assert!(boot.abs_diff(3_001_000) <= 2, "{boot}");
        assert!(awake.abs_diff(3_000_000) <= 2, "{awake}");
        // The real counters pair up, and the pair is now. The continuous counter is never behind
        // the absolute one; its two reads bracket the absolute read, so their middle may be up to
        // half the gap before it.
        let (boot, awake) = paired(|| unsafe { (mach_continuous_time(), mach_absolute_time()) });
        assert!(boot + PAIR_GAP >= awake, "{boot} vs {awake}");
        assert!(boot_100ns() >= boot);
    }

    /// A frame is as old on the boot clock as it is on the host clock, and never from the future.
    #[test]
    fn a_frame_keeps_its_age() {
        // Awake for 1,000 units, slept for 5,000 of the 6,000 since boot; the frame is 40 old.
        assert_eq!(frame_boot_at(6_000, 1_000, 960), 5_960);
        assert_eq!(frame_boot_at(6_000, 1_000, 1_000), 6_000);
        assert_eq!(frame_boot_at(6_000, 1_000, 2_000), 6_000);
    }
}
