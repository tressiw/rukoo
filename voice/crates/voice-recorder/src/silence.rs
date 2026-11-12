//! The silence check: does a call track record nothing but exact zeros because the system won't
//! let Voice record it?
//!
//! On macOS the call track is a Core Audio process tap. Without the System Audio Recording grant,
//! or with a grant that went stale after an update, the tap opens without an error and delivers
//! blocks of exact zeros (spike S2). A working tap delivers exact zeros too, whenever nothing
//! plays. So zeros alone prove nothing, and the check pairs them with what can be known:
//!
//! - **Any sample that isn't exactly zero** proves the grant: the tap records what the Mac plays.
//!   From then on the check stays quiet for the rest of the session, and a notice it gave ends.
//! - **Another app plays sound** (Core Audio's process list says some other process runs
//!   output) while the tap has delivered only exact zeros since its first block. After
//!   [`DEFAULT_THRESHOLD_MS`] of that, the check says the call is silent.
//! - **Nobody can tell** whether anything plays (the process list didn't answer): it waits four
//!   times as long.
//! - **Nothing plays**: zeros are what a working tap gives, so they count for nothing.
//!
//! What it can't tell: a denied grant from a call app that plays exact digital silence for the
//! whole threshold while nobody speaks (it then says so too early, and takes it back at the first
//! sound), and a grant switched off in the middle of a meeting after sound was heard (the check is
//! over by then). It never stops or changes the recording; it only reports.

/// Exact zeros while another app plays this long before the check says the call is silent.
pub const DEFAULT_THRESHOLD_MS: u64 = 30_000;
/// When nobody can tell whether anything plays, the check waits this many times as long.
pub const UNKNOWN_FACTOR: u64 = 4;

/// What the check found out with one block.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Only exact zeros since meeting time `since`, while sound played elsewhere.
    Silent { since: u64 },
    /// The first sound after the check said the call was silent.
    Sound,
}

#[derive(Debug, Clone)]
pub struct SilenceCheck {
    threshold_ms: u64,
    /// A sample that wasn't zero arrived: the grant works.
    heard: bool,
    /// The check said the call is silent.
    flagged: bool,
    /// Where the zeros started, in meeting time.
    since: Option<u64>,
    playing_ms: u64,
    unknown_ms: u64,
}

impl SilenceCheck {
    pub fn new(threshold_ms: u64) -> Self {
        Self {
            threshold_ms: threshold_ms.max(1),
            heard: false,
            flagged: false,
            since: None,
            playing_ms: 0,
            unknown_ms: 0,
        }
    }

    /// One block the source delivered: `ms` of audio that arrived at meeting time `at`, all of
    /// it exactly zero or not, while another app played (`Some(true)`), didn't (`Some(false)`) or
    /// nobody could tell (`None`).
    pub fn feed(&mut self, zero: bool, ms: u64, at: u64, playing: Option<bool>) -> Option<Verdict> {
        if self.heard {
            return None;
        }
        if !zero {
            self.heard = true;
            return self.flagged.then_some(Verdict::Sound);
        }
        let since = *self.since.get_or_insert(at.saturating_sub(ms));
        match playing {
            Some(true) => self.playing_ms += ms,
            Some(false) => {}
            None => self.unknown_ms += ms,
        }
        let long_enough = self.playing_ms >= self.threshold_ms
            || self.unknown_ms >= self.threshold_ms * UNKNOWN_FACTOR;
        if !self.flagged && long_enough {
            self.flagged = true;
            return Some(Verdict::Silent { since });
        }
        None
    }

    /// The check says the call is silent now.
    pub fn silent(&self) -> bool {
        self.flagged && !self.heard
    }

    /// The source delivered sound this session, so the system lets Voice record it.
    pub fn heard(&self) -> bool {
        self.heard
    }
}

/// Whether a block is digital silence: every sample exactly zero, as a denied tap delivers.
pub fn all_zero(samples: &[f32]) -> bool {
    samples.iter().all(|&s| s == 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feeds `seconds` of 10 ms blocks from meeting time `from`, returning the verdicts.
    fn run(
        check: &mut SilenceCheck,
        from: u64,
        seconds: u64,
        zero: bool,
        playing: Option<bool>,
    ) -> Vec<(u64, Verdict)> {
        let mut out = Vec::new();
        for n in 1..=seconds * 100 {
            let at = from + n * 10;
            if let Some(v) = check.feed(zero, 10, at, playing) {
                out.push((at, v));
            }
        }
        out
    }

    #[test]
    fn zeros_from_the_start_while_another_app_plays_are_reported_once() {
        let mut check = SilenceCheck::new(DEFAULT_THRESHOLD_MS);
        let verdicts = run(&mut check, 1_000, 60, true, Some(true));
        assert_eq!(verdicts, vec![(31_000, Verdict::Silent { since: 1_000 })]);
        assert!(check.silent() && !check.heard());
    }

    #[test]
    fn zeros_while_nothing_plays_say_nothing() {
        let mut check = SilenceCheck::new(DEFAULT_THRESHOLD_MS);
        assert!(run(&mut check, 0, 600, true, Some(false)).is_empty());
        assert!(!check.silent());
    }

    #[test]
    fn without_knowing_what_plays_it_waits_four_times_as_long() {
        let mut check = SilenceCheck::new(DEFAULT_THRESHOLD_MS);
        let verdicts = run(&mut check, 0, 200, true, None);
        assert_eq!(verdicts, vec![(120_000, Verdict::Silent { since: 0 })]);
    }

    #[test]
    fn real_audio_proves_the_grant() {
        let mut check = SilenceCheck::new(DEFAULT_THRESHOLD_MS);
        assert!(run(&mut check, 0, 5, false, Some(true)).is_empty());
        assert!(check.heard());
        // Zeros after real audio are a quiet call, never a denied grant.
        assert!(run(&mut check, 5_000, 600, true, Some(true)).is_empty());
        assert!(!check.silent());
    }

    #[test]
    fn zeros_after_real_audio_say_nothing_even_from_the_first_block() {
        let mut check = SilenceCheck::new(1_000);
        assert_eq!(check.feed(false, 10, 10, Some(true)), None);
        assert!(run(&mut check, 10, 10, true, Some(true)).is_empty());
    }

    #[test]
    fn the_first_sound_after_the_notice_takes_it_back() {
        let mut check = SilenceCheck::new(1_000);
        assert_eq!(
            run(&mut check, 0, 2, true, Some(true)),
            vec![(1_000, Verdict::Silent { since: 0 })]
        );
        assert_eq!(
            check.feed(false, 10, 2_010, Some(true)),
            Some(Verdict::Sound)
        );
        assert!(check.heard() && !check.silent());
        assert_eq!(check.feed(false, 10, 2_020, Some(true)), None);
        assert!(run(&mut check, 2_020, 10, true, Some(true)).is_empty());
    }

    #[test]
    fn only_the_time_something_played_counts() {
        let mut check = SilenceCheck::new(1_000);
        // Half a second playing, ten seconds not, half a second playing: 1 s in all.
        assert!(run(&mut check, 0, 1, true, Some(false)).is_empty());
        for n in 1..=50 {
            assert_eq!(check.feed(true, 10, 1_000 + n * 10, Some(true)), None);
        }
        assert!(run(&mut check, 1_500, 10, true, Some(false)).is_empty());
        let verdicts = run(&mut check, 11_500, 1, true, Some(true));
        assert_eq!(verdicts, vec![(12_000, Verdict::Silent { since: 0 })]);
    }

    #[test]
    fn a_block_is_silent_only_when_every_sample_is_exactly_zero() {
        assert!(all_zero(&[0.0, -0.0, 0.0]));
        assert!(all_zero(&[]));
        assert!(!all_zero(&[0.0, 1e-9, 0.0]));
    }
}
