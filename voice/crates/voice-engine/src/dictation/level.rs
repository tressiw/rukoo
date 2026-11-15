//! Ownkey's microphone level meter and activity check (`ownkey.py:250-266`, `3041-3081`;
//! pill-settings-design.md 1.9), with its constants unchanged so the bars feel the same.

use std::time::Duration;

pub const DB_FLOOR: f32 = -52.0;
pub const DB_CEILING: f32 = -11.0;
/// The rolling peak decays slowly and never drops below this, so quiet rooms don't max the bars.
pub const PEAK_DECAY: f32 = 0.9975;
pub const PEAK_MIN: f32 = 0.45;
pub const PEAK_TARGET: f32 = 0.88;
/// Ownkey's starting peak; it is kept across recordings so the meter stays calibrated to the mic.
pub const PEAK_START: f32 = 0.7;
pub const GATE: f32 = 0.04;
pub const ATTACK: f32 = 0.55;
pub const RELEASE: f32 = 0.16;
/// Sound counts as heard at this loudness or level.
pub const ACTIVITY_DB: f32 = -57.0;
pub const ACTIVITY_LEVEL: f32 = 0.05;
/// "No audio detected" after this long without activity since the first buffer.
pub const NO_AUDIO_AFTER: Duration = Duration::from_secs(5);
/// At most 50 level ticks a second.
pub const PUSH_INTERVAL: Duration = Duration::from_millis(20);
/// The floor a peak reading is reported against, so digital silence reads as one number instead
/// of minus infinity. Speech at a normal distance is 20 to 40 dB above it.
pub const SILENT_DBFS: f32 = -96.0;

/// The loudest sample in a block as dBFS, so a recording can say how loud it was at all. A block
/// of digital silence, or no block, reads [`SILENT_DBFS`] instead of minus infinity.
///
/// This is the one number that tells a microphone that delivered nothing from a recognizer that
/// heard words and returned none, without keeping any of the words.
pub fn peak_dbfs(block: &[i16]) -> f32 {
    let peak = block
        .iter()
        .map(|&sample| f32::from(sample.unsigned_abs()))
        .fold(0.0, f32::max);
    let floor = 10f32.powf(SILENT_DBFS / 20.0);
    20.0 * (peak / 32_768.0).max(floor).log10()
}

/// dBFS of a block of 16-bit samples.
pub fn dbfs(block: &[i16]) -> f32 {
    if block.is_empty() {
        return 20.0 * 1e-6_f32.log10();
    }
    let sum: f64 = block.iter().map(|&s| f64::from(s) * f64::from(s)).sum();
    let rms = (sum / block.len() as f64).sqrt() as f32;
    20.0 * (rms / 32_768.0).max(1e-6).log10()
}

/// The smoothed level for the pill and whether a block had sound in it.
#[derive(Clone, Debug)]
pub struct LevelMeter {
    peak: f32,
    level: f32,
}

impl Default for LevelMeter {
    fn default() -> LevelMeter {
        LevelMeter {
            peak: PEAK_START,
            level: 0.0,
        }
    }
}

impl LevelMeter {
    /// A meter for a new recording that keeps the calibration of the last one.
    pub fn restart(&self) -> LevelMeter {
        LevelMeter {
            peak: self.peak,
            level: 0.0,
        }
    }

    #[cfg(test)]
    pub fn level(&self) -> f32 {
        self.level
    }

    /// Feeds one block; returns the new level (0 to 1) and whether the block was heard.
    pub fn feed(&mut self, block: &[i16]) -> (f32, bool) {
        let db = dbfs(block);
        let raw = ((db - DB_FLOOR) / (DB_CEILING - DB_FLOOR)).clamp(0.0, 1.0);
        self.peak = raw.max(self.peak * PEAK_DECAY).max(PEAK_MIN);
        let boosted = (raw * (PEAK_TARGET / self.peak)).min(1.0);
        let target = ((boosted - GATE) / (1.0 - GATE)).max(0.0);
        let blend = if target > self.level { ATTACK } else { RELEASE };
        self.level = (self.level + (target - self.level) * blend).clamp(0.0, 1.0);
        let heard = db >= ACTIVITY_DB || self.level >= ACTIVITY_LEVEL;
        (self.level, heard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tone(amplitude: f32, n: usize) -> Vec<i16> {
        (0..n)
            .map(|i| ((i as f32 * 0.1).sin() * amplitude) as i16)
            .collect()
    }

    #[test]
    fn decibels_follow_ownkeys_formula() {
        assert!(
            (dbfs(&[0; 160]) - -120.0).abs() < 0.01,
            "silence is the floor"
        );
        // A full-scale square wave is 0 dBFS.
        let square: Vec<i16> = (0..160)
            .map(|i| if i % 2 == 0 { 32_767 } else { -32_767 })
            .collect();
        assert!(dbfs(&square).abs() < 0.01);
        assert!((dbfs(&[3277; 160]) - -20.0).abs() < 0.05);
    }

    #[test]
    fn a_recorded_sequence_gives_ownkeys_levels() {
        // Expected values computed with Ownkey's pipeline by hand for constant blocks:
        // -20 dBFS gives raw 0.780; the peak rises to it; boost 0.88; target 0.875.
        let mut meter = LevelMeter::default();
        let (first, heard) = meter.feed(&[3277; 160]);
        assert!(heard);
        assert!((first - 0.875 * ATTACK).abs() < 0.01, "{first}");
        let (second, _) = meter.feed(&[3277; 160]);
        let expected = first + (0.875 - first) * ATTACK;
        assert!((second - expected).abs() < 0.01, "{second}");
        // Silence falls back with the slower release.
        let (falling, heard) = meter.feed(&[0; 160]);
        assert!(!heard || falling >= ACTIVITY_LEVEL);
        assert!(
            (falling - second * (1.0 - RELEASE)).abs() < 0.01,
            "{falling}"
        );
    }

    #[test]
    fn quiet_noise_is_not_activity() {
        let mut meter = LevelMeter::default();
        for _ in 0..50 {
            let (level, heard) = meter.feed(&tone(20.0, 160));
            assert!(!heard, "level {level}");
        }
        let (_, heard) = meter.feed(&tone(8000.0, 160));
        assert!(heard);
    }

    #[test]
    fn the_peak_is_what_tells_silence_from_sound() {
        // Digital silence reads as the floor, so it prints as one number.
        assert_eq!(peak_dbfs(&[]), SILENT_DBFS);
        assert_eq!(peak_dbfs(&[0; 16_000]), SILENT_DBFS);
        // A single loud sample is the peak, wherever it sits.
        assert!((peak_dbfs(&[0, 0, 16_384, 0]) - -6.0).abs() < 0.1);
        assert!(peak_dbfs(&[32_767]) > -0.01);
        // -40 dBFS peak, a quiet headset's normal level.
        assert!((peak_dbfs(&[328; 400]) - -40.0).abs() < 0.05);
    }

    #[test]
    fn the_peak_carries_over_but_the_level_restarts() {
        let mut meter = LevelMeter::default();
        for _ in 0..20 {
            meter.feed(&[30_000; 160]);
        }
        let next = meter.restart();
        assert_eq!(next.level(), 0.0);
        assert!(next.peak > PEAK_START);
    }
}
