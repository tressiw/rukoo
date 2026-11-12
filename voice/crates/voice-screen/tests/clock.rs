//! The screen process's meeting time against the recorder's own (spec 2.4): mapped through the
//! recorder's `boot` anchors, a moment read between two interrupt-time readings must fall within
//! 2 ms of the recorder's `status.t`, also after the recorder was killed and resumed.

mod common;

#[cfg(any(windows, target_os = "macos"))]
mod tests {
    use std::time::{Duration, Instant};

    use serde_json::json;
    #[cfg(target_os = "macos")]
    use voice_screen::clock_mac as platform_clock;
    #[cfg(windows)]
    use voice_screen::clockwin as platform_clock;
    use voice_screen_files::clock::{Now, ScreenClock};
    use voice_screen_files::timeline::{TimelineFollower, TimelineLine};

    use super::common::*;

    const TOLERANCE_MS: u64 = 2;
    const READINGS: usize = 20;

    /// The recorder's anchors, fed as the screen process feeds them.
    fn anchored(m: &TempMeeting, session: u64) -> (TimelineFollower, ScreenClock) {
        let mut follower = TimelineFollower::new(&m.dir);
        let mut clock = ScreenClock::new();
        let deadline = Instant::now() + START_TIMEOUT;
        // Wait for a heartbeat after this session's line, so the anchor is fresh.
        let mut in_session = false;
        let mut heartbeat = false;
        while !heartbeat {
            for line in follower.poll().unwrap() {
                match &line {
                    TimelineLine::Session {
                        session: s, boot, ..
                    } => {
                        in_session = *s == session;
                        assert!(
                            boot.is_some(),
                            "the recorder writes no boot in its session line"
                        );
                    }
                    TimelineLine::Heartbeat { boot, .. } if in_session => {
                        assert!(
                            boot.is_some(),
                            "the recorder writes no boot in its heartbeats"
                        );
                        heartbeat = true;
                    }
                    _ => {}
                }
                clock.feed(&line, platform_clock::now());
            }
            assert!(
                Instant::now() < deadline,
                "no heartbeat of session {session}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(clock.uses_boot(), "the mapping fell back to the wall clock");
        (follower, clock)
    }

    fn check(m: &TempMeeting, session: u64) {
        let (mut follower, mut clock) = anchored(m, session);
        for _ in 0..READINGS {
            for line in follower.poll().unwrap() {
                clock.feed(&line, platform_clock::now());
            }
            let b0 = platform_clock::boot_100ns();
            let status = m.recorder_request(json!({ "cmd": "status" }));
            let b1 = platform_clock::boot_100ns();
            let recorder_t = status["t"].as_u64().unwrap();
            let wall = platform_clock::wall_ms();
            let early = clock
                .at(Now {
                    boot: Some(b0),
                    wall,
                })
                .unwrap();
            let late = clock
                .at(Now {
                    boot: Some(b1),
                    wall,
                })
                .unwrap();
            assert!(
                early.saturating_sub(TOLERANCE_MS) <= recorder_t
                    && recorder_t <= late + TOLERANCE_MS,
                "session {session}: the recorder says {recorder_t}, the mapping {early}..{late}"
            );
            std::thread::sleep(Duration::from_millis(150));
        }
    }

    pub fn screen_time_matches_the_recorder() {
        let m = TempMeeting::new("clock", "frames");
        let mut recorder = m.recorder(&[]);
        m.wait_recorder(1);
        check(&m, 1);

        // A resumed session starts its own clock; the mapping follows its anchors.
        recorder.hard_kill();
        let mut recorder = m.recorder(&["--resume"]);
        m.wait_recorder(2);
        check(&m, 2);

        let reply = m.recorder_request(json!({ "cmd": "stop" }));
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(recorder.wait_exit(WAIT_TIMEOUT).success());
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    common::play_role();
    common::run_tests(&[common::Test {
        name: "screen_time_matches_the_recorder",
        ignored: false,
        run: tests::screen_time_matches_the_recorder,
    }]);
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    common::play_role();
    println!("the clock test needs Windows or macOS; skipped");
}
