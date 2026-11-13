//! Which part of the screen recording stops when the disk fills, the encoder fails or the
//! graphics device is lost, and the free-space rule every other writer follows.
//!
//! Video stops below `VIDEO_MIN_FREE` free on the meeting folder's volume, frames below
//! `FRAMES_MIN_FREE`; video always goes first, and a drop lasts for the rest of the meeting.
//! The audio is never dropped.

use crate::journal::History;

pub const VIDEO_MIN_FREE: u64 = 4 << 30;
pub const FRAMES_MIN_FREE: u64 = 2 << 30;
pub const DISK_CHECK_MS: u64 = 5_000;
pub const ENCODER_RETRY_MS: u64 = 2_000;
pub const DEVICE_RETRIES: u32 = 2;
/// What another writer leaves free on top of the screen recording's floor.
pub const RESERVE_MARGIN: u64 = 2 << 30;

/// Whether another writer may put `bytes` on the meetings volume now: what stays free must
/// cover the floor of the screen recording that could be running (`VIDEO_MIN_FREE` while a
/// meeting records, else `FRAMES_MIN_FREE`) plus `RESERVE_MARGIN`.
pub fn reserve_ok(free: u64, bytes: u64, meeting_live: bool) -> bool {
    let floor = if meeting_live {
        VIDEO_MIN_FREE
    } else {
        FRAMES_MIN_FREE
    };
    bytes <= free && free - bytes >= floor + RESERVE_MARGIN
}

/// A part of the screen recording.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Part {
    Video,
    Frames,
}

impl Part {
    /// The journal's `what`: `video` or `frames`.
    pub fn as_str(self) -> &'static str {
        match self {
            Part::Video => "video",
            Part::Frames => "frames",
        }
    }

    pub fn parse(what: &str) -> Option<Part> {
        match what {
            "video" => Some(Part::Video),
            "frames" => Some(Part::Frames),
            _ => None,
        }
    }
}

/// Why a part was dropped.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    Disk,
    Encoder,
    Device,
    /// The platform's capture failed to start or stopped by itself too many times in a row.
    Capture,
}

impl Reason {
    /// The journal's `reason` and the mark's `detail`: `disk`, `encoder`, `device` or `capture`.
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::Disk => "disk",
            Reason::Encoder => "encoder",
            Reason::Device => "device",
            Reason::Capture => "capture",
        }
    }

    pub fn parse(reason: &str) -> Option<Reason> {
        match reason {
            "disk" => Some(Reason::Disk),
            "encoder" => Some(Reason::Encoder),
            "device" => Some(Reason::Device),
            "capture" => Some(Reason::Capture),
            _ => None,
        }
    }
}

/// What has been dropped so far. Each `on_*` returns the drops it newly decided, video first;
/// the caller journals them in that order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DropState {
    pub video: Option<Reason>,
    pub frames: Option<Reason>,
    encoder_failures: u32,
    device_failures: u32,
    video_on: bool,
}

/// What to do after an encoder error or a device loss.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EncoderStep {
    /// Try again with a new device and a new file.
    Retry,
    Drop(Part, Reason),
    /// Nothing to do: the part is dropped already or was never on.
    Ignore,
}

impl DropState {
    /// Frames-only meetings: `video` stays `None`, but video counts as not recording.
    pub fn new(video_on: bool) -> DropState {
        DropState {
            video: None,
            frames: None,
            encoder_failures: 0,
            device_failures: 0,
            video_on,
        }
    }

    /// The drops a resumed session inherits from the journal.
    pub fn from_history(h: &History, video_on: bool) -> DropState {
        let mut state = DropState::new(video_on);
        state.video = h.dropped_video.as_deref().and_then(Reason::parse);
        state.frames = h.dropped_frames.as_deref().and_then(Reason::parse);
        state
    }

    /// Whether video still records: on for the meeting and not dropped.
    pub fn video_recording(&self) -> bool {
        self.video_on && self.video.is_none()
    }

    pub fn set_video(&mut self, on: bool) {
        self.video_on = on;
    }

    /// Whether frames still record.
    pub fn frames_recording(&self) -> bool {
        self.frames.is_none()
    }

    /// The free bytes on the meeting folder's volume, checked every `DISK_CHECK_MS` and at
    /// capture start: video stops below `VIDEO_MIN_FREE`, frames below `FRAMES_MIN_FREE`.
    pub fn on_free_bytes(&mut self, free: u64) -> Vec<(Part, Reason)> {
        let mut drops = Vec::new();
        if free < VIDEO_MIN_FREE {
            drops.extend(self.drop(Part::Video, Reason::Disk));
        }
        if free < FRAMES_MIN_FREE {
            drops.extend(self.drop(Part::Frames, Reason::Disk));
        }
        drops
    }

    /// A write failed with a full disk. A full disk under the frames ends the video too, first.
    pub fn on_disk_full(&mut self, part: Part) -> Vec<(Part, Reason)> {
        let mut drops: Vec<(Part, Reason)> =
            self.drop(Part::Video, Reason::Disk).into_iter().collect();
        if part == Part::Frames {
            drops.extend(self.drop(Part::Frames, Reason::Disk));
        }
        drops
    }

    /// First: `Retry`; second: `Drop(Video, Encoder)`.
    pub fn on_encoder_error(&mut self) -> EncoderStep {
        if !self.video_recording() {
            return EncoderStep::Ignore;
        }
        self.encoder_failures += 1;
        if self.encoder_failures == 1 {
            EncoderStep::Retry
        } else {
            self.step(Part::Video, Reason::Encoder)
        }
    }

    /// No video encoder at all: no hardware encoder, or Media Foundation is missing. Drops the
    /// video at once, without a retry.
    pub fn on_no_encoder(&mut self) -> EncoderStep {
        self.step(Part::Video, Reason::Encoder)
    }

    /// `Retry` until `DEVICE_RETRIES` recreations failed, then `Drop(Video, Device)`,
    /// `Drop(Frames, Device)`.
    ///
    /// Call it when the device is lost and again each time creating a new one fails. Once the
    /// retries are spent it returns one drop per call, video first, then `Ignore`; call it until
    /// then. `on_device_recovered` starts the count again.
    pub fn on_device_lost(&mut self) -> EncoderStep {
        if self.all_dropped() {
            return EncoderStep::Ignore;
        }
        self.device_failures = self.device_failures.saturating_add(1);
        // The first call is the loss itself; every later one is a failed recreation.
        if self.device_failures <= DEVICE_RETRIES {
            return EncoderStep::Retry;
        }
        match self.step(Part::Video, Reason::Device) {
            EncoderStep::Ignore => self.step(Part::Frames, Reason::Device),
            step => step,
        }
    }

    /// A new device works but lacks the video support the lost one had, as a basic display
    /// driver does while the real one restarts. It counts as a failed recreation for the video
    /// only: `Retry` until `DEVICE_RETRIES` recreations failed, then `Drop(Video, Device)`. Once
    /// the video is dropped, or never recorded, such a device suits the frames: `Retry`, and
    /// `Ignore` when the frames are dropped too.
    pub fn on_device_without_video(&mut self) -> EncoderStep {
        if !self.video_recording() {
            return match self.frames_recording() {
                true => EncoderStep::Retry,
                false => EncoderStep::Ignore,
            };
        }
        self.device_failures = self.device_failures.saturating_add(1);
        if self.device_failures <= DEVICE_RETRIES {
            return EncoderStep::Retry;
        }
        self.step(Part::Video, Reason::Device)
    }

    /// A new device and capture session work again.
    pub fn on_device_recovered(&mut self) {
        self.device_failures = 0;
    }

    /// Both parts stopped: the process writes `stop` with reason `dropped`.
    pub fn all_dropped(&self) -> bool {
        !self.video_recording() && !self.frames_recording()
    }

    fn step(&mut self, part: Part, reason: Reason) -> EncoderStep {
        match self.drop(part, reason) {
            Some((part, reason)) => EncoderStep::Drop(part, reason),
            None => EncoderStep::Ignore,
        }
    }

    /// Drops `part` when it still records. Callers drop the video first.
    fn drop(&mut self, part: Part, reason: Reason) -> Option<(Part, Reason)> {
        match part {
            Part::Video if self.video_recording() => {
                self.video = Some(reason);
                Some((Part::Video, reason))
            }
            Part::Frames if self.frames_recording() => {
                self.frames = Some(reason);
                Some((Part::Frames, reason))
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1 << 30;

    #[test]
    fn reserve_ok_keeps_the_screen_floor_and_a_margin() {
        // While a meeting records: 4 GiB for the video plus 2 GiB.
        assert!(reserve_ok(8 * GIB, 2 * GIB, true));
        assert!(reserve_ok(6 * GIB, 0, true));
        assert!(!reserve_ok(6 * GIB - 1, 0, true));
        assert!(!reserve_ok(8 * GIB, 2 * GIB + 1, true));
        // The spec's example: 5 GiB free and a 1.5 GiB clip input is refused, 8 GiB is fine.
        assert!(!reserve_ok(5 * GIB, 3 * GIB / 2, true));
        assert!(reserve_ok(8 * GIB, 3 * GIB / 2, true));
        // Without a meeting only the frames floor counts.
        assert!(reserve_ok(4 * GIB, 0, false));
        assert!(!reserve_ok(4 * GIB - 1, 0, false));
        assert!(reserve_ok(5 * GIB, GIB, false));
        assert!(!reserve_ok(5 * GIB, GIB + 1, false));
        // More bytes than are free never fit, and nothing overflows.
        assert!(!reserve_ok(GIB, 2 * GIB, false));
        assert!(!reserve_ok(0, u64::MAX, true));
        assert!(reserve_ok(u64::MAX, 0, true));
        assert!(reserve_ok(u64::MAX, u64::MAX - 6 * GIB, true));
        assert!(!reserve_ok(u64::MAX, u64::MAX - 6 * GIB + 1, true));
    }

    #[test]
    fn video_goes_below_4_gib_and_frames_below_2_gib() {
        let mut s = DropState::new(true);
        assert_eq!(s.on_free_bytes(10 * GIB), vec![]);
        assert_eq!(s.on_free_bytes(VIDEO_MIN_FREE), vec![]);
        assert_eq!(
            s.on_free_bytes(VIDEO_MIN_FREE - 1),
            vec![(Part::Video, Reason::Disk)]
        );
        assert!(!s.all_dropped());
        // A drop lasts: more space doesn't bring the video back, and nothing is dropped twice.
        assert_eq!(s.on_free_bytes(100 * GIB), vec![]);
        assert_eq!(s.on_free_bytes(3 * GIB), vec![]);
        assert_eq!(s.on_free_bytes(FRAMES_MIN_FREE), vec![]);
        assert_eq!(
            s.on_free_bytes(FRAMES_MIN_FREE - 1),
            vec![(Part::Frames, Reason::Disk)]
        );
        assert!(s.all_dropped());
        assert_eq!(s.on_free_bytes(0), vec![]);
        assert_eq!(s.video, Some(Reason::Disk));
        assert_eq!(s.frames, Some(Reason::Disk));
    }

    #[test]
    fn a_sudden_drop_under_2_gib_ends_video_first() {
        let mut s = DropState::new(true);
        assert_eq!(
            s.on_free_bytes(GIB),
            vec![(Part::Video, Reason::Disk), (Part::Frames, Reason::Disk)]
        );
        // Frames only: nothing to drop at 3 GiB, frames at 1 GiB.
        let mut s = DropState::new(false);
        assert_eq!(s.on_free_bytes(3 * GIB), vec![]);
        assert_eq!(s.video, None);
        assert!(!s.all_dropped());
        assert_eq!(s.on_free_bytes(GIB), vec![(Part::Frames, Reason::Disk)]);
        assert!(s.all_dropped());
    }

    #[test]
    fn a_full_disk_drops_what_it_hit_video_first() {
        let mut s = DropState::new(true);
        assert_eq!(
            s.on_disk_full(Part::Video),
            vec![(Part::Video, Reason::Disk)]
        );
        assert_eq!(s.on_disk_full(Part::Video), vec![]);
        assert!(s.frames_recording());
        let mut s = DropState::new(true);
        assert_eq!(
            s.on_disk_full(Part::Frames),
            vec![(Part::Video, Reason::Disk), (Part::Frames, Reason::Disk)]
        );
        assert!(s.all_dropped());
        let mut s = DropState::new(false);
        assert_eq!(s.on_disk_full(Part::Video), vec![]);
        assert_eq!(
            s.on_disk_full(Part::Frames),
            vec![(Part::Frames, Reason::Disk)]
        );
    }

    #[test]
    fn the_encoder_gets_one_retry() {
        let mut s = DropState::new(true);
        assert_eq!(s.on_encoder_error(), EncoderStep::Retry);
        assert_eq!(
            s.on_encoder_error(),
            EncoderStep::Drop(Part::Video, Reason::Encoder)
        );
        assert_eq!(s.on_encoder_error(), EncoderStep::Ignore);
        assert!(s.frames_recording());
        assert!(!s.all_dropped());
        // Frames only, or video already dropped for the disk: nothing to retry.
        assert_eq!(
            DropState::new(false).on_encoder_error(),
            EncoderStep::Ignore
        );
        let mut s = DropState::new(true);
        s.on_free_bytes(3 * GIB);
        assert_eq!(s.on_encoder_error(), EncoderStep::Ignore);
        assert_eq!(s.video, Some(Reason::Disk));
        // No encoder at all drops the video at once.
        let mut s = DropState::new(true);
        assert_eq!(
            s.on_no_encoder(),
            EncoderStep::Drop(Part::Video, Reason::Encoder)
        );
        assert_eq!(s.on_no_encoder(), EncoderStep::Ignore);
        assert_eq!(DropState::new(false).on_no_encoder(), EncoderStep::Ignore);
    }

    #[test]
    fn a_lost_device_is_retried_twice_then_both_parts_go() {
        let mut s = DropState::new(true);
        // Lost, and both recreations fail.
        assert_eq!(s.on_device_lost(), EncoderStep::Retry);
        assert_eq!(s.on_device_lost(), EncoderStep::Retry);
        assert_eq!(
            s.on_device_lost(),
            EncoderStep::Drop(Part::Video, Reason::Device)
        );
        assert_eq!(
            s.on_device_lost(),
            EncoderStep::Drop(Part::Frames, Reason::Device)
        );
        assert_eq!(s.on_device_lost(), EncoderStep::Ignore);
        assert!(s.all_dropped());

        // Losses that recover each time never drop anything.
        let mut s = DropState::new(true);
        for _ in 0..5 {
            assert_eq!(s.on_device_lost(), EncoderStep::Retry);
            s.on_device_recovered();
        }
        assert!(!s.all_dropped());

        // Frames only: frames go once the retries are spent.
        let mut s = DropState::new(false);
        assert_eq!(s.on_device_lost(), EncoderStep::Retry);
        assert_eq!(s.on_device_lost(), EncoderStep::Retry);
        assert_eq!(
            s.on_device_lost(),
            EncoderStep::Drop(Part::Frames, Reason::Device)
        );
        assert_eq!(s.on_device_lost(), EncoderStep::Ignore);

        // Video dropped for the encoder earlier: the device loss only takes the frames.
        let mut s = DropState::new(true);
        s.on_no_encoder();
        s.on_device_lost();
        s.on_device_lost();
        assert_eq!(
            s.on_device_lost(),
            EncoderStep::Drop(Part::Frames, Reason::Device)
        );
        assert_eq!(s.video, Some(Reason::Encoder));
    }

    #[test]
    fn a_device_without_video_support_costs_only_the_video() {
        // Lost; both recreations bring a device without video support: the video goes, and
        // the frames go on with such a device.
        let mut s = DropState::new(true);
        assert_eq!(s.on_device_lost(), EncoderStep::Retry);
        assert_eq!(s.on_device_without_video(), EncoderStep::Retry);
        assert_eq!(
            s.on_device_without_video(),
            EncoderStep::Drop(Part::Video, Reason::Device)
        );
        assert_eq!(s.on_device_without_video(), EncoderStep::Retry);
        assert!(s.frames_recording());
        // When even that device can't be made, the frames go as well.
        assert_eq!(
            s.on_device_lost(),
            EncoderStep::Drop(Part::Frames, Reason::Device)
        );
        assert_eq!(s.on_device_without_video(), EncoderStep::Ignore);
        assert!(s.all_dropped());

        // One device without video support, then the real one: nothing is dropped.
        let mut s = DropState::new(true);
        assert_eq!(s.on_device_lost(), EncoderStep::Retry);
        assert_eq!(s.on_device_without_video(), EncoderStep::Retry);
        assert!(!s.all_dropped() && s.video_recording());

        // Frames only, or the video dropped earlier: such a device is fine.
        assert_eq!(
            DropState::new(false).on_device_without_video(),
            EncoderStep::Retry
        );
        let mut s = DropState::new(true);
        s.on_no_encoder();
        assert_eq!(s.on_device_without_video(), EncoderStep::Retry);
        assert_eq!(s.video, Some(Reason::Encoder));
    }

    #[test]
    fn a_resumed_session_inherits_the_drops() {
        let h = History {
            dropped_video: Some("encoder".into()),
            ..History::default()
        };
        let s = DropState::from_history(&h, true);
        assert_eq!(s.video, Some(Reason::Encoder));
        assert_eq!(s.frames, None);
        assert!(!s.video_recording());
        assert!(s.frames_recording());
        let h = History {
            dropped_video: Some("disk".into()),
            dropped_frames: Some("disk".into()),
            ..History::default()
        };
        assert!(DropState::from_history(&h, true).all_dropped());
        // Frames only with nothing dropped.
        let s = DropState::from_history(&History::default(), false);
        assert!(!s.video_recording() && s.frames_recording());
        // A reason this version doesn't know isn't a drop.
        let h = History {
            dropped_video: Some("gremlins".into()),
            ..History::default()
        };
        assert!(DropState::from_history(&h, true).video_recording());
    }

    #[test]
    fn parts_and_reasons_read_their_journal_strings() {
        for part in [Part::Video, Part::Frames] {
            assert_eq!(Part::parse(part.as_str()), Some(part));
        }
        for reason in [
            Reason::Disk,
            Reason::Encoder,
            Reason::Device,
            Reason::Capture,
        ] {
            assert_eq!(Reason::parse(reason.as_str()), Some(reason));
        }
        assert_eq!(Part::parse("audio"), None);
        assert_eq!(Reason::parse(""), None);
    }
}
