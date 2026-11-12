//! The meeting clock: wall-clock based across sessions, monotonic within one.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Milliseconds since the Unix epoch.
pub fn wall_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_millis() as u64
}

/// Meeting time in milliseconds, advanced by the monotonic clock.
pub struct MeetingClock {
    base: u64,
    start: Instant,
}

impl MeetingClock {
    pub fn starting_at(t: u64) -> Self {
        Self {
            base: t,
            start: Instant::now(),
        }
    }

    pub fn now(&self) -> u64 {
        self.at(Instant::now())
    }

    /// The meeting time of an earlier or later instant. Instants before the clock's start
    /// count as its start.
    pub fn at(&self, instant: Instant) -> u64 {
        self.base + instant.saturating_duration_since(self.start).as_millis() as u64
    }

    /// Moves the clock forward to `t`; never backward.
    pub fn advance_to(&mut self, t: u64) {
        let now = Instant::now();
        let current = self.at(now);
        if t > current {
            self.base = t;
            self.start = now;
        }
    }
}

/// Where a resumed session's clock starts: the wall time since session 1 started, but never
/// before anything the timeline already holds, even when the wall clock went back.
pub fn resumed_start(first_wall: u64, now_wall: u64, latest_t: u64) -> u64 {
    now_wall.saturating_sub(first_wall).max(latest_t)
}

/// A clock that counts sleep, in 100 ns units, shared by every process. On Windows the interrupt
/// time; on macOS `mach_continuous_time`. `None` on Linux, whose monotonic clock stops while the
/// computer sleeps.
///
/// The recorder writes it next to `t` in its `session` and `heartbeat` lines, so another process,
/// such as the screen recorder, can map any moment to meeting time exactly:
/// `t + (boot_now - boot) / 10_000`.
#[cfg(windows)]
pub fn boot_100ns() -> Option<u64> {
    use windows::Win32::System::WindowsProgramming::QueryInterruptTimePrecise;
    // SAFETY: it only writes the u64 the binding hands it.
    Some(unsafe { QueryInterruptTimePrecise() })
}

/// A clock that counts sleep, in 100 ns units, shared by every process: `mach_continuous_time`,
/// the macOS counterpart of the Windows interrupt time.
#[cfg(target_os = "macos")]
pub fn boot_100ns() -> Option<u64> {
    Some(mach::continuous_100ns())
}

/// `None` on Linux: its monotonic clock stops while the computer sleeps, so there is no shared
/// counter that counts the sleep.
#[cfg(not(any(windows, target_os = "macos")))]
pub fn boot_100ns() -> Option<u64> {
    None
}

/// macOS's `mach_timebase_info`: one tick of `mach_absolute_time` and `mach_continuous_time` is
/// `numer / denom` nanoseconds. Apple Silicon reports 125/3, Intel Macs 1/1. Its layout is the C
/// struct the call fills in.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Timebase {
    pub numer: u32,
    pub denom: u32,
}

impl Timebase {
    /// `ticks` in 100 ns units, the unit of `boot` and of the Windows interrupt time. Whole units
    /// are dropped. An absurd tick count saturates instead of wrapping.
    pub fn to_100ns(self, ticks: u64) -> u64 {
        self.scale(ticks, 100)
    }

    /// `ticks` in whole milliseconds. An absurd tick count saturates instead of wrapping.
    pub fn to_ms(self, ticks: u64) -> u64 {
        self.scale(ticks, 1_000_000)
    }

    /// `ticks * numer / denom` nanoseconds, cut down to units of `unit` nanoseconds. The product
    /// needs more than 64 bits for a long uptime on a fast clock, so it is taken in 128.
    fn scale(self, ticks: u64, unit: u128) -> u64 {
        let denom = (self.denom as u128 * unit).max(1);
        let units = ticks as u128 * self.numer as u128 / denom;
        units.min(u64::MAX as u128) as u64
    }
}

/// The two macOS counters that tell sleep apart. Both count ticks since boot; the timebase turns
/// them into time.
#[cfg(target_os = "macos")]
mod mach {
    use std::sync::OnceLock;

    use super::Timebase;

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
            // SAFETY: the call only writes the two u32 fields of `info`. Should it fail, 1/1 is
            // what its result then stands for: nanoseconds per tick.
            unsafe { mach_timebase_info(&mut info) };
            info
        })
    }

    pub(super) fn continuous_100ns() -> u64 {
        // SAFETY: it only reads the counter.
        let ticks = unsafe { mach_continuous_time() };
        timebase().to_100ns(ticks)
    }

    /// `(awake, elapsed)` for [`Stamp`]: `mach_absolute_time` stops while the computer sleeps and
    /// `mach_continuous_time` keeps counting, as the unbiased and interrupt times do on Windows.
    /// The continuous one is read second, so it is never behind.
    pub(super) fn sleep_clocks() -> (u64, u64) {
        let timebase = timebase();
        // SAFETY: they only read the counters.
        let awake = unsafe { mach_absolute_time() };
        let elapsed = unsafe { mach_continuous_time() };
        (timebase.to_ms(awake), timebase.to_ms(elapsed))
    }
}

/// Heartbeats are written this often.
pub const HEARTBEAT: Duration = Duration::from_secs(1);
/// The computer slept when more time passed than it was awake, by more than this.
pub const SUSPEND_MS: u64 = 3_000;

/// One reading of the clocks that tell sleep apart, in milliseconds. `awake` stops while the
/// computer sleeps and `elapsed` keeps counting; neither follows a change of the wall clock.
/// `wall` is the wall clock at the same moment, for the timeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    pub awake: u64,
    pub elapsed: u64,
    pub wall: u64,
}

impl Stamp {
    pub fn now() -> Stamp {
        let (awake, elapsed) = sleep_clocks();
        Stamp {
            awake,
            elapsed,
            wall: wall_ms(),
        }
    }
}

/// Windows counts interrupt time with and without the time spent asleep; the difference is
/// exactly the sleep. Rust's `Instant` isn't used for this: whether it counts sleep isn't
/// specified, and on Windows it does.
#[cfg(windows)]
fn sleep_clocks() -> (u64, u64) {
    use windows::Win32::System::WindowsProgramming::{
        QueryInterruptTime, QueryUnbiasedInterruptTime,
    };
    // Both count 100 ns units since boot.
    let mut awake = 0u64;
    // SAFETY: both only write the u64 they are given.
    let (ok, elapsed) = unsafe {
        let ok = QueryUnbiasedInterruptTime(&mut awake).as_bool();
        (ok, QueryInterruptTime())
    };
    let elapsed = elapsed / 10_000;
    // Without the unbiased count, no sleep can be seen; that is what the call failing means.
    let awake = if ok { awake / 10_000 } else { elapsed };
    (awake, elapsed)
}

/// macOS has the same pair of counters as Windows under other names: `mach_absolute_time` stops
/// while the computer sleeps and `mach_continuous_time` counts it, so the difference is exactly
/// the sleep.
#[cfg(target_os = "macos")]
fn sleep_clocks() -> (u64, u64) {
    mach::sleep_clocks()
}

/// Elsewhere the monotonic clock stops during sleep on Linux, and the wall clock stands in for the
/// time that passed, so a wall-clock step counts as sleep there.
#[cfg(not(any(windows, target_os = "macos")))]
fn sleep_clocks() -> (u64, u64) {
    use std::sync::OnceLock;
    static BASE: OnceLock<Instant> = OnceLock::new();
    let awake = BASE.get_or_init(Instant::now).elapsed().as_millis() as u64;
    (awake, wall_ms())
}

/// How long the computer slept between two readings, when it slept more than `SUSPEND_MS`.
/// A thread that was stalled advances both clocks alike, and a wall-clock step neither, so
/// neither counts.
pub fn slept(previous: &Stamp, now: &Stamp) -> Option<u64> {
    let passed = now.elapsed.saturating_sub(previous.elapsed);
    let awake = now.awake.saturating_sub(previous.awake);
    let slept = passed.saturating_sub(awake);
    (slept > SUSPEND_MS).then_some(slept)
}

/// The meeting time after a sleep found between a heartbeat at `heartbeat_t` and now: the time
/// that passed since the heartbeat, but never more than `slept` past `current`, the clock's own
/// reading. A clock that already counted the sleep doesn't move again.
pub fn after_sleep(heartbeat_t: u64, current: u64, previous: &Stamp, now: &Stamp) -> u64 {
    let passed = now.elapsed.saturating_sub(previous.elapsed);
    let slept = slept(previous, now).unwrap_or(0);
    (heartbeat_t + passed).min(current + slept).max(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp(awake: u64, elapsed: u64, wall: u64) -> Stamp {
        Stamp {
            awake,
            elapsed,
            wall,
        }
    }

    #[test]
    fn resumed_sessions_follow_the_wall_clock_but_never_go_back() {
        assert_eq!(resumed_start(1_000, 61_000, 30_000), 60_000);
        assert_eq!(resumed_start(1_000, 20_000, 30_000), 30_000);
        assert_eq!(resumed_start(90_000, 20_000, 30_000), 30_000);
    }

    #[test]
    fn a_sleep_of_more_than_three_seconds_counts() {
        let before = stamp(10_000, 10_000, 1_793_000_010_000);
        // One second awake, then a minute asleep.
        let after = stamp(11_000, 71_000, 1_793_000_071_000);
        assert_eq!(slept(&before, &after), Some(60_000));
        assert_eq!(slept(&before, &stamp(11_000, 14_000, 0)), None);
        assert_eq!(slept(&before, &stamp(11_000, 14_001, 0)), Some(3_001));
    }

    #[test]
    fn a_stalled_thread_is_not_a_sleep() {
        // The recorder thread was blocked for 6 s, opening a device: every clock moved 6 s.
        let before = stamp(10_000, 10_000, 1_793_000_010_000);
        let after = stamp(16_000, 16_000, 1_793_000_016_000);
        assert_eq!(slept(&before, &after), None);
    }

    #[test]
    fn a_wall_clock_step_is_not_a_sleep() {
        // A time sync moved the wall clock a minute forward, then back; one second passed.
        let before = stamp(10_000, 10_000, 1_793_000_010_000);
        assert_eq!(
            slept(&before, &stamp(11_000, 11_000, 1_793_000_071_000)),
            None
        );
        assert_eq!(
            slept(&before, &stamp(11_000, 11_000, 1_792_999_951_000)),
            None
        );
    }

    #[test]
    fn the_clock_moves_by_the_sleep_and_never_by_a_wall_step() {
        let before = stamp(10_000, 10_000, 0);
        let asleep = stamp(11_000, 71_000, 0);
        // A monotonic clock that stopped while asleep reads one second past the heartbeat.
        assert_eq!(after_sleep(5_000, 6_000, &before, &asleep), 66_000);
        // One that counted the sleep already is where it should be and stays.
        assert_eq!(after_sleep(5_000, 66_000, &before, &asleep), 66_000);
        // A wall-clock step moves nothing.
        let stepped = stamp(11_000, 11_000, 60_000);
        assert_eq!(after_sleep(5_000, 6_000, &before, &stepped), 6_000);
        // Never back.
        assert_eq!(after_sleep(5_000, 70_000, &before, &asleep), 70_000);
    }

    #[test]
    fn this_computer_reads_its_clocks() {
        let a = Stamp::now();
        std::thread::sleep(Duration::from_millis(30));
        let b = Stamp::now();
        assert!(b.awake >= a.awake && b.elapsed >= a.elapsed);
        assert!(b.elapsed - a.elapsed >= 10, "{a:?} {b:?}");
        assert_eq!(slept(&a, &b), None);
    }

    #[test]
    fn boot_time_is_there_where_the_clock_counts_sleep() {
        let a = boot_100ns();
        std::thread::sleep(Duration::from_millis(30));
        let b = boot_100ns();
        if cfg!(windows) || cfg!(target_os = "macos") {
            let (a, b) = (a.unwrap(), b.unwrap());
            // 30 ms is 300,000 units; allow for a slow scheduler but not for a wrong unit.
            assert!(b - a >= 250_000 && b - a < 50_000_000, "{a} {b}");
        } else {
            assert_eq!((a, b), (None, None));
        }
    }

    /// A second of ticks for the timebase a Mac reports: Apple Silicon counts 24 MHz.
    const SECOND: u64 = 24_000_000;

    fn apple_silicon() -> Timebase {
        Timebase {
            numer: 125,
            denom: 3,
        }
    }

    /// A [`Stamp`] built the way [`sleep_clocks`] builds one on macOS: two tick counts through
    /// one timebase.
    fn mach_stamp(timebase: Timebase, awake: u64, elapsed: u64, wall: u64) -> Stamp {
        Stamp {
            awake: timebase.to_ms(awake),
            elapsed: timebase.to_ms(elapsed),
            wall,
        }
    }

    #[test]
    fn the_timebase_turns_ticks_into_time() {
        // Apple Silicon: a tick is 125/3 ns, so a second is 24,000,000 ticks.
        let apple = apple_silicon();
        assert_eq!(apple.to_100ns(24_000_000), 10_000_000);
        assert_eq!(apple.to_ms(SECOND), 1_000);
        // Intel Macs and the simulator count nanoseconds.
        let intel = Timebase { numer: 1, denom: 1 };
        assert_eq!(intel.to_100ns(10_000), 100);
        assert_eq!(intel.to_ms(2_500_000), 2);
        // Whole units are dropped, never rounded up.
        assert_eq!(apple.to_100ns(2), 0);
        assert_eq!(intel.to_ms(999_999), 0);
    }

    #[test]
    fn a_huge_tick_count_saturates_instead_of_wrapping() {
        // A long uptime on a fast clock needs more than 64 bits for ticks * numer.
        let apple = apple_silicon();
        let exact = u64::MAX as u128 * 125 / 300;
        assert!(exact < u64::MAX as u128);
        assert_eq!(apple.to_100ns(u64::MAX), exact as u64);
        // A timebase that makes every tick more than a second can't fit at all.
        let coarse = Timebase {
            numer: u32::MAX,
            denom: 1,
        };
        assert_eq!(coarse.to_100ns(u64::MAX), u64::MAX);
        assert_eq!(coarse.to_ms(u64::MAX), u64::MAX);
    }

    #[test]
    fn two_awake_seconds_are_not_a_sleep() {
        let timebase = apple_silicon();
        let before = mach_stamp(timebase, SECOND, SECOND, 1_793_000_001_000);
        let after = mach_stamp(timebase, 3 * SECOND, 3 * SECOND, 1_793_000_003_000);
        assert_eq!((before.awake, before.elapsed), (1_000, 1_000));
        assert_eq!((after.awake, after.elapsed), (3_000, 3_000));
        assert_eq!(slept(&before, &after), None);
    }

    #[test]
    fn thirty_seconds_asleep_move_the_continuous_clock_only() {
        let timebase = apple_silicon();
        let before = mach_stamp(timebase, SECOND, SECOND, 1_793_000_001_000);
        // One second awake, then thirty asleep: 31 s on the continuous clock.
        let after = mach_stamp(timebase, 2 * SECOND, 32 * SECOND, 1_793_000_032_000);
        assert_eq!((after.awake, after.elapsed), (2_000, 32_000));
        assert_eq!(slept(&before, &after), Some(30_000));
        // The meeting clock jumps to what the continuous clock says, 36 s after the heartbeat.
        assert_eq!(after_sleep(5_000, 6_000, &before, &after), 36_000);
    }

    /// The macOS counters are real: both move forward and, while the computer is awake, they move
    /// together.
    #[cfg(target_os = "macos")]
    #[test]
    fn this_mac_reads_both_mach_clocks() {
        let a = Stamp::now();
        std::thread::sleep(Duration::from_millis(30));
        let b = Stamp::now();
        assert!(b.awake >= a.awake && b.elapsed >= a.elapsed, "{a:?} {b:?}");
        assert!(b.elapsed - a.elapsed >= 10, "{a:?} {b:?}");
        // They differ by the time this Mac has slept, which doesn't change while it's awake.
        assert!(
            (a.elapsed - a.awake).abs_diff(b.elapsed - b.awake) <= 20,
            "{a:?} {b:?}"
        );
    }

    #[test]
    fn clock_moves_forward_only() {
        let mut clock = MeetingClock::starting_at(5_000);
        assert!(clock.now() >= 5_000);
        clock.advance_to(65_000);
        assert!(clock.now() >= 65_000);
        clock.advance_to(10_000);
        assert!(clock.now() >= 65_000);
        if let Some(before) = clock.start.checked_sub(Duration::from_millis(1)) {
            assert_eq!(clock.at(before), 65_000);
        }
    }
}
