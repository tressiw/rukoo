//! What `rukoo-voice-screen probe` prints, and the video size rule.

use serde::{Deserialize, Serialize};

use crate::monitors::MonitorInfo;

/// A Media Foundation H.264 encoder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncoderInfo {
    pub name: String,
    pub hardware: bool,
    pub vendor: Option<String>,
}

/// What this computer can record. `ocr` holds the OCR languages as BCP-47 tags.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    pub version: u32,
    pub capture: bool,
    /// Whether this process holds the Screen Recording grant. Only macOS has one, so Windows
    /// leaves it out. A computer that can capture but has no grant records no screen: the screen
    /// process refuses to start and says why (exit code 5).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub screen_recording: Option<bool>,
    pub encoders: Vec<EncoderInfo>,
    pub hardware_encoder: Option<EncoderInfo>,
    pub ocr: Vec<String>,
    pub monitors: Vec<MonitorInfo>,
}

/// The longest side the encoder takes (NVENC on Pascal).
pub const MAX_VIDEO_SIDE: u32 = 4096;
/// The most 16 x 16 macroblocks in a frame at H.264 level 5.1.
pub const MAX_MACROBLOCKS: u32 = 36_864;

fn macroblocks(width: u32, height: u32) -> u64 {
    u64::from(width.div_ceil(16)) * u64::from(height.div_ceil(16))
}

/// The video size for a monitor: its own size when each side is at most `MAX_VIDEO_SIDE` and
/// the frame at most `MAX_MACROBLOCKS`, else scaled by one factor to fit both; then both sides
/// rounded down to even. 1440 x 2560 stays; 5120 x 1440 becomes 4096 x 1152.
pub fn video_size(width: u32, height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (0, 0);
    }
    // One factor for both sides, as the long side scaled to `long`, in integers so that a
    // limit that is met exactly stays met.
    let longest = width.max(height);
    let size_at = |long: u32| {
        let side = |s: u32| (u64::from(s) * u64::from(long) / u64::from(longest)).max(1) as u32;
        (side(width), side(height))
    };
    let fits = |(w, h): (u32, u32)| macroblocks(w, h) <= u64::from(MAX_MACROBLOCKS);
    let mut long = longest.min(MAX_VIDEO_SIDE);
    if !fits(size_at(long)) {
        // The largest long side that fits; one pixel always does.
        let (mut lo, mut hi) = (1, long);
        while hi - lo > 1 {
            let mid = lo + (hi - lo) / 2;
            if fits(size_at(mid)) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        long = lo;
    }
    let (w, h) = size_at(long);
    ((w & !1).max(2), (h & !1).max(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitors_keep_their_size_within_the_limits() {
        // This PC's portrait monitor: 90 x 160 = 14,400 macroblocks.
        assert_eq!(video_size(1440, 2560), (1440, 2560));
        assert_eq!(video_size(2560, 1440), (2560, 1440));
        assert_eq!(video_size(1920, 1080), (1920, 1080));
        assert_eq!(video_size(3840, 2160), (3840, 2160));
        // Exactly at both limits.
        assert_eq!(video_size(4096, 2304), (4096, 2304));
    }

    #[test]
    fn larger_monitors_scale_by_one_factor() {
        // A side over 4096.
        assert_eq!(video_size(5120, 1440), (4096, 1152));
        assert_eq!(video_size(7680, 4320), (4096, 2304));
        assert_eq!(video_size(1440, 5120), (1152, 4096));
        // Sides within 4096 but too many macroblocks: 65,536 of them.
        let (w, h) = video_size(4096, 4096);
        assert_eq!((w, h), (3072, 3072));
        // Every result fits, is even and keeps the shape within a pixel.
        for (width, height) in [
            (5120, 2880),
            (6016, 3384),
            (3840, 3840),
            (3440, 1440),
            (5120, 2160),
            (4097, 4097),
            (8192, 8192),
            (4095, 2305),
        ] {
            let (w, h) = video_size(width, height);
            assert!(
                w <= MAX_VIDEO_SIDE && h <= MAX_VIDEO_SIDE,
                "{width}x{height}"
            );
            assert!(
                macroblocks(w, h) <= u64::from(MAX_MACROBLOCKS),
                "{width}x{height}"
            );
            assert_eq!((w % 2, h % 2), (0, 0), "{width}x{height}");
            let ratio = f64::from(width) / f64::from(height);
            assert!(
                (f64::from(w) / f64::from(h) / ratio - 1.0).abs() < 0.005,
                "{width}x{height} -> {w}x{h}"
            );
            // A scaled size isn't shrunk further than the limits ask.
            let scaled = (w, h) != (width & !1, height & !1);
            assert!(
                !scaled
                    || macroblocks(w + 32, h + 32) > u64::from(MAX_MACROBLOCKS)
                    || w + 32 > MAX_VIDEO_SIDE
                    || h + 32 > MAX_VIDEO_SIDE,
                "{width}x{height} -> {w}x{h}"
            );
        }
    }

    #[test]
    fn odd_sides_are_cropped_to_even() {
        assert_eq!(video_size(1215, 2160), (1214, 2160));
        assert_eq!(video_size(1366, 767), (1366, 766));
        assert_eq!(video_size(1, 1), (2, 2));
        assert_eq!(video_size(0, 1080), (0, 0));
    }

    #[test]
    fn capabilities_serialize_in_camel_case() {
        let nvenc = EncoderInfo {
            name: "NVIDIA H.264 Encoder MFT".into(),
            hardware: true,
            vendor: Some("VEN_10DE".into()),
        };
        let caps = Capabilities {
            version: 1,
            capture: true,
            screen_recording: None,
            encoders: vec![nvenc.clone()],
            hardware_encoder: Some(nvenc),
            ocr: vec!["nl-NL".into()],
            monitors: vec![],
        };
        let json = serde_json::to_value(&caps).unwrap();
        // Windows has no grant to report, so the field is left out.
        assert!(json.get("screenRecording").is_none());
        assert_eq!(json["hardwareEncoder"]["vendor"], "VEN_10DE");
        assert_eq!(json["ocr"][0], "nl-NL");
        let back: Capabilities = serde_json::from_value(json).unwrap();
        assert_eq!(back, caps);
    }

    /// A Mac reports its Screen Recording grant, and a probe without the field still reads.
    #[test]
    fn the_screen_recording_grant_is_optional() {
        let json = serde_json::json!({
            "version": 1, "capture": true, "screenRecording": false,
            "encoders": [], "hardwareEncoder": null, "ocr": [], "monitors": []
        });
        let caps: Capabilities = serde_json::from_value(json).unwrap();
        assert_eq!(caps.screen_recording, Some(false));
        let older = serde_json::json!({
            "version": 1, "capture": true,
            "encoders": [], "hardwareEncoder": null, "ocr": [], "monitors": []
        });
        let caps: Capabilities = serde_json::from_value(older).unwrap();
        assert_eq!(caps.screen_recording, None);
    }
}
