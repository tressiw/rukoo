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
/// is how far the awake counter has moved since, taken off the continuous counter, so a frame
/// delivered after a sleep still lands where it happened. A time in the future counts as now.
pub fn frame_boot(pts_100ns: u64) -> u64 {
    frame_boot_at(boot_100ns(), awake_100ns(), pts_100ns)
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

    #[test]
    fn both_counters_move_with_the_clock() {
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

    /// A frame is as old on the boot clock as it is on the host clock, and never from the future.
    #[test]
    fn a_frame_keeps_its_age() {
        // Awake for 1,000 units, slept for 5,000 of the 6,000 since boot; the frame is 40 old.
        assert_eq!(frame_boot_at(6_000, 1_000, 960), 5_960);
        assert_eq!(frame_boot_at(6_000, 1_000, 1_000), 6_000);
        assert_eq!(frame_boot_at(6_000, 1_000, 2_000), 6_000);
    }
}
