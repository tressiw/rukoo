//! The screen process next to the real recorder: crash safety, the drop order, pause, monitor
//! switching, stopping and the frame text. Every test runs both as real processes, the screen
//! process on the synthetic screen, so nothing of the desktop is captured. Video uses the
//! platform's software encoder (`software-encoder`): Microsoft's on Windows, Apple's on macOS,
//! which is what a CI runner has. `hard_kill_with_hardware_encoder` covers NVENC or VideoToolbox's
//! hardware encoder on a machine that has one. The real-capture spike and the Windows sink
//! writer's scaling spike run on Windows only.

mod common;

#[cfg(any(windows, target_os = "macos"))]
mod tests {
    use std::fs;
    use std::time::Duration;

    use serde_json::{json, Value};
    use voice_screen_files::mp4;

    use super::common::*;

    /// Slides change every 1.5 s, so frames keep coming; 1280 x 720 keeps a CI runner's
    /// software encoder light.
    const SLIDES: &str = "synthetic:1280x720,change-every=1500,text=Slide";

    /// The synthetic screen with the software encoder, and `extra` options.
    /// `VOICE_SCREEN_TEST_SPEC_EXTRA` adds options to every such test; `warp` runs them on
    /// Microsoft's software rasterizer, as on a CI runner without a GPU.
    fn spec(extra: &str) -> String {
        with_extra(&format!("{SLIDES},software-encoder"), extra)
    }

    /// `base` with `extra` and `VOICE_SCREEN_TEST_SPEC_EXTRA` added.
    fn with_extra(base: &str, extra: &str) -> String {
        let mut spec = base.to_string();
        for more in [
            extra.to_string(),
            std::env::var("VOICE_SCREEN_TEST_SPEC_EXTRA").unwrap_or_default(),
        ] {
            if !more.is_empty() {
                spec.push(',');
                spec.push_str(&more);
            }
        }
        spec
    }

    /// Starts the recorder, then the screen process of session 1.
    fn start(m: &TempMeeting, mode: &str, test: &str, extra: &[&str]) -> (Proc, Proc) {
        let recorder = m.recorder(&[]);
        m.wait_recorder(1);
        let screen = m.screen(mode, Some(test), extra);
        m.wait_journal("the first capture", |l| first(l, "capture").is_some());
        (recorder, screen)
    }

    /// Stops the recorder, which stops the screen process within a moment.
    fn stop(m: &TempMeeting, recorder: &mut Proc, screen: &mut Proc) -> Vec<Value> {
        let reply = m.recorder_request(json!({ "cmd": "stop" }));
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(recorder.wait_exit(WAIT_TIMEOUT).success());
        let status = screen.wait_exit(WAIT_TIMEOUT);
        assert!(status.success(), "the screen process exited with {status}");
        m.journal()
    }

    fn video_path(m: &TempMeeting, video: &Value) -> std::path::PathBuf {
        m.dir.join(video["file"].as_str().unwrap())
    }

    /// The video media time an index covers, in ms.
    fn media_ms(index: &mp4::Index) -> u64 {
        index.fragments.last().map_or(0, |f| {
            (f.start + f.duration) * 1000 / u64::from(index.timescale)
        })
    }

    /// A finished video file's `video_end` is where the media in it ends, to the millisecond, so a
    /// clip request for the file's last moments finds the file and meeting time maps to media time
    /// with the `video` line's `t` alone. The writer keeps each frame's capture time in the file
    /// on every encoder path (`video::writer`); before, Microsoft's video processor laid the
    /// software path's frames on a 15 fps grid, up to a frame off. Files ended by an error or a
    /// full disk are abandoned and hold no more than their journal says.
    fn assert_video_ends_cover(m: &TempMeeting, lines: &[Value]) {
        for video in of_kind(lines, "video") {
            let Some(end) = lines
                .iter()
                .find(|l| l["kind"] == "video_end" && l["seq"] == video["seq"])
            else {
                continue;
            };
            if ["error", "disk", "crash"].contains(&end["reason"].as_str().unwrap_or("")) {
                continue;
            }
            let index = mp4::index_path(&video_path(m, video)).unwrap();
            let reached = t(video) + media_ms(&index);
            assert_eq!(
                t(end),
                reached,
                "video {} holds media to {reached} ms; its video_end says {}:\n{}",
                video["seq"],
                t(end),
                show(lines)
            );
        }
    }

    /// Every journaled frame's JPEG opens, at the size its line says.
    fn assert_frames_open(m: &TempMeeting, lines: &[Value]) {
        let frames = of_kind(lines, "frame");
        assert!(!frames.is_empty(), "no frames:\n{}", show(lines));
        for frame in frames {
            let file = frame["file"].as_str().unwrap();
            let bytes = fs::read(m.dir.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
            let size = voice_screen::frames::jpeg::decode_size(&bytes)
                .unwrap_or_else(|e| panic!("{file} doesn't open: {e}"));
            assert_eq!(
                (u64::from(size.0), u64::from(size.1)),
                (
                    frame["width"].as_u64().unwrap(),
                    frame["height"].as_u64().unwrap()
                ),
                "{file}"
            );
        }
    }

    /// The file ends on a fragment boundary and the platform's reader finds every sample in it:
    /// Media Foundation on Windows, the shared box walker on macOS.
    fn assert_video_reads(path: &std::path::Path) -> Value {
        let index = mp4::index_path(path).unwrap();
        assert_eq!(
            index.complete_len,
            index.file_len,
            "{} ends mid-box",
            path.display()
        );
        let report = voice_screen::inspect::inspect(path);
        assert_eq!(report["reader"]["error"], Value::Null, "{report}");
        assert_eq!(
            report["reader"]["samples"], report["samples"],
            "the platform's reader finds fewer samples than the file holds: {report}"
        );
        report
    }

    pub fn hard_kill_keeps_video_and_frames() {
        hard_kill(&spec(""), false);
    }

    /// The hardware encoder, where the machine has one: NVENC through the GPU path on Windows,
    /// VideoToolbox's hardware encoder on macOS. `VOICE_SCREEN_TEST_SIZE=1440x2560` runs it at the
    /// size of a portrait monitor, still on the synthetic screen.
    pub fn hard_kill_with_hardware_encoder() {
        if voice_screen::probe::probe().hardware_encoder.is_none() {
            println!("    skipped: this computer has no hardware H.264 encoder");
            return;
        }
        let size = std::env::var("VOICE_SCREEN_TEST_SIZE").unwrap_or_else(|_| "1920x1080".into());
        // A screen that changes every second, and one that stays still after its first slide, as
        // a slide deck mostly does: then only the re-submits make the samples, and the fragments
        // must still end within 2 s.
        for change in [1_000, 60_000] {
            println!("    a slide every {change} ms");
            hard_kill(
                &format!("synthetic:{size},change-every={change},text=Slide"),
                true,
            );
        }
    }

    fn hard_kill(test: &str, hardware: bool) {
        let m = TempMeeting::new("hard-kill", "video");
        let (mut recorder, mut screen) = start(&m, "video", test, &[]);
        let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
        let capture_t = t(first(&lines, "capture").unwrap());

        // Spike 1: the file the video writer is still writing opens for reading, and clip.get can
        // cut its finished fragments.
        let growing = video_path(&m, first(&lines, "video").unwrap());
        let index = wait_until(WAIT_TIMEOUT, || {
            mp4::index_path(&growing)
                .ok()
                .filter(|i| !i.fragments.is_empty())
        });
        assert!(index.complete_len <= index.file_len);
        let clip = m.root.join("growing-clip.mp4");
        let cut = mp4::cut(&growing, 0, 60_000, &clip)
            .unwrap()
            .expect("nothing to cut from the growing file");
        assert!(cut.bytes > 0 && cut.end_ms > cut.start_ms, "{cut:?}");
        assert_video_reads(&clip);

        let killed_at = m.wait_screen_t(1, capture_t + 6_500);
        screen.hard_kill();

        let lines = m.journal();
        assert!(first(&lines, "stop").is_none(), "{}", show(&lines));
        let video = first(&lines, "video").unwrap().clone();
        assert_eq!(video["hardware"], hardware, "{video}");
        if hardware {
            let probe = voice_screen::probe::probe();
            let encoder = probe.hardware_encoder.expect("no hardware encoder here");
            assert_eq!(video["encoder"], encoder.name.as_str(), "{video}");
        }
        let path = video_path(&m, &video);
        let index = mp4::index_path(&path).unwrap();
        let reached = t(&video) + media_ms(&index);
        println!(
            "    the video reaches {reached} ms, killed at {killed_at}: {} ms lost",
            killed_at.saturating_sub(reached)
        );
        assert!(
            reached + 2_000 >= killed_at,
            "the video ends at {reached} ms, killed at {killed_at}"
        );
        // A kill between two writes of one fragment leaves a torn last box. The promise is the
        // file up to its last complete fragment, which `--resume` and `finalize` cut it back to.
        let report = if index.complete_len == index.file_len {
            assert_video_reads(&path)
        } else {
            println!(
                "    the kill tore the last box: {} of {} bytes complete",
                index.complete_len, index.file_len
            );
            let copy = m.root.join("killed-repaired.mp4");
            fs::copy(&path, &copy).unwrap();
            mp4::repair(&copy).unwrap();
            assert_video_reads(&copy)
        };
        if hardware {
            let fragments: Vec<u64> = serde_json::from_value(report["fragmentMs"].clone()).unwrap();
            println!(
                "    {} by {}, fragments {fragments:?} ms",
                video["width"], video["encoder"]
            );
            let (last, full) = fragments.split_last().unwrap();
            assert!(!full.is_empty() && *last > 0, "{report}");
            for ms in full {
                assert!((1_000..=2_000).contains(ms), "a {ms} ms fragment: {report}");
            }
        }
        assert_frames_open(&m, &lines);
        assert_timeline_continuous(&m);

        // The watchdog starts it again with --resume.
        let mut screen = m.screen("video", Some(test), &["--resume"]);
        let lines = m.wait_journal("frames and video after the resume", |l| {
            let Some(resumed) = l
                .iter()
                .find(|x| x["kind"] == "session" && x["session"] == 2)
            else {
                return false;
            };
            let at = t(resumed);
            l.iter().any(|x| x["kind"] == "video" && x["seq"] == 2)
                && l.iter().any(|x| x["kind"] == "frame" && t(x) > at)
        });
        let resumed = lines
            .iter()
            .find(|l| l["kind"] == "session" && l["session"] == 2)
            .unwrap();
        assert_eq!(resumed["resumed"], true);
        let crash = first(&lines, "crash").expect("no crash line");
        assert_eq!(t(crash), t(resumed));
        let last_seen = crash["lastSeen"].as_u64().unwrap();
        assert!(
            last_seen <= t(resumed) && last_seen + 2_500 >= killed_at,
            "{crash}"
        );
        let closed = lines
            .iter()
            .find(|l| l["kind"] == "video_end" && l["seq"] == 1)
            .expect("the dead session's video never ended");
        assert_eq!(closed["reason"], "crash");

        let lines = stop(&m, &mut recorder, &mut screen);
        let stopped = lines.last().unwrap();
        assert_eq!(stopped["kind"], "stop", "{}", show(&lines));
        assert_eq!(stopped["reason"], "meeting");
        assert!(m.screen_file().is_none(), "screen.json stays after stop");
        for video in of_kind(&lines, "video") {
            assert_video_reads(&video_path(&m, video));
        }
        assert_video_ends_cover(&m, &lines);
        assert_frames_open(&m, &lines);
        assert_timeline_continuous(&m);
    }

    /// `finalize` closes what a killed session left: the open video and a `stop`.
    pub fn finalize_closes_a_killed_session() {
        let m = TempMeeting::new("finalize", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec(""), &[]);
        m.wait_journal("a video file", |l| first(l, "video").is_some());
        m.wait_screen_t(1, 4_000);
        screen.hard_kill();
        let before = m.journal();
        let (status, out) =
            m.run_screen(&["finalize", "--meeting", &m.arg(), "--reason", "failed"]);
        assert!(status.success(), "{out}");
        let report: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(report["stopped"], true, "{report}");
        let lines = m.journal();
        let ended = lines
            .iter()
            .find(|l| l["kind"] == "video_end" && l["seq"] == 1)
            .expect("no video_end");
        assert_eq!(ended["reason"], "crash");
        let stopped = lines.last().unwrap();
        assert_eq!(
            (stopped["kind"].as_str(), stopped["reason"].as_str()),
            (Some("stop"), Some("failed"))
        );
        let last_seen = before.iter().filter_map(|l| l["t"].as_u64()).max().unwrap();
        assert!(t(stopped) >= last_seen);
        assert_eq!(report["t"], stopped["t"]);
        // Again: nothing changes.
        let (status, out) = m.run_screen(&["finalize", "--meeting", &m.arg()]);
        assert!(status.success(), "{out}");
        assert_eq!(m.journal().len(), lines.len());
        assert_video_reads(&video_path(&m, first(&lines, "video").unwrap()));
        let _ = m.recorder_request(json!({ "cmd": "stop" }));
        recorder.wait_exit(WAIT_TIMEOUT);
    }

    /// `finalize` and `--resume` truncate the open video file to its last complete fragment. A
    /// journal whose open `video` line names another file, here the meeting's audio, gets its
    /// `video_end crash` and `stop`, and the file keeps every byte.
    pub fn finalize_repairs_only_video_files() {
        let m = TempMeeting::new("finalize-foreign", "video");
        let wav = m.dir.join("audio").join("mic").join("000001.wav");
        fs::create_dir_all(wav.parent().unwrap()).unwrap();
        let mut audio = b"RIFF".to_vec();
        audio.resize(96_044, 0x11);
        fs::write(&wav, &audio).unwrap();
        let journal = [
            json!({ "kind": "session", "t": 0, "wall": 1_793_000_000_000u64, "boot": 1, "session": 1,
                    "pid": 1, "resumed": false, "version": 1, "mode": "video", "anchor": "boot",
                    "screen": "0.1.0" }),
            json!({ "kind": "video", "t": 1_000, "seq": 1, "file": "audio/mic/000001.wav",
                    "width": 1280, "height": 720, "fps": 15, "encoder": "x", "hardware": false }),
        ];
        let screen = m.dir.join("screen");
        fs::create_dir_all(&screen).unwrap();
        let text: String = journal.iter().map(|l| format!("{l}\n")).collect();
        fs::write(screen.join("journal.jsonl"), text).unwrap();

        let (status, out) =
            m.run_screen(&["finalize", "--meeting", &m.arg(), "--reason", "interrupted"]);
        assert!(status.success(), "{out}");
        assert_eq!(fs::read(&wav).unwrap(), audio, "finalize changed the audio");
        let lines = m.journal();
        let ended = lines
            .iter()
            .find(|l| l["kind"] == "video_end" && l["seq"] == 1)
            .expect("no video_end");
        assert_eq!(ended["reason"], "crash", "{}", show(&lines));
        assert_eq!(lines.last().unwrap()["kind"], "stop", "{}", show(&lines));
    }

    pub fn portrait_monitor_keeps_its_size() {
        let m = TempMeeting::new("portrait", "video");
        let (mut recorder, mut screen) = start(
            &m,
            "video",
            &with_extra(
                "synthetic:1440x2560,change-every=1500,text=Slide,software-encoder",
                "",
            ),
            &[],
        );
        let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
        let video = first(&lines, "video").unwrap().clone();
        assert_eq!(
            (video["width"].as_u64(), video["height"].as_u64()),
            (Some(1440), Some(2560))
        );
        m.wait_screen_t(1, t(&video) + 3_000);
        stop(&m, &mut recorder, &mut screen);
        let report = assert_video_reads(&video_path(&m, &video));
        assert_eq!(
            (
                report["reader"]["width"].as_u64(),
                report["reader"]["height"].as_u64()
            ),
            (Some(1440), Some(2560)),
            "{report}"
        );

        // Over 4096 wide: the sink writer's video processor scales it by one factor.
        let m = TempMeeting::new("ultrawide", "video");
        let (mut recorder, mut screen) = start(
            &m,
            "video",
            &with_extra(
                "synthetic:5120x1440,change-every=1500,text=Slide,software-encoder",
                "",
            ),
            &[],
        );
        let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
        let video = first(&lines, "video").unwrap().clone();
        assert_eq!(
            (video["width"].as_u64(), video["height"].as_u64()),
            (Some(4096), Some(1152))
        );
        m.wait_screen_t(1, t(&video) + 3_000);
        let lines = stop(&m, &mut recorder, &mut screen);
        assert!(first(&lines, "dropped").is_none(), "{}", show(&lines));
        let report = assert_video_reads(&video_path(&m, &video));
        assert_eq!(
            (
                report["reader"]["width"].as_u64(),
                report["reader"]["height"].as_u64()
            ),
            (Some(4096), Some(1152)),
            "{report}"
        );
    }

    pub fn pinned_monitor_is_recorded() {
        let m = TempMeeting::new("pinned", "frames");
        let (mut recorder, mut screen) = start(
            &m,
            "frames",
            "synthetic:640x360,monitors=2",
            &["--monitor", r"\\.\SYNTHETIC2"],
        );
        let lines = m.journal();
        let monitor = first(&lines, "monitor").unwrap();
        assert_eq!(monitor["id"], r"\\.\SYNTHETIC2", "{monitor}");
        assert_eq!(monitor["reason"], "start");
        assert_eq!(
            first(&lines, "capture").unwrap()["monitor"],
            r"\\.\SYNTHETIC2"
        );
        // Pinned, it never records another monitor, not even on request.
        let monitors = m.screen_request(1, json!({ "cmd": "monitors" }));
        let ids: Vec<&str> = monitors["monitors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [r"\\.\SYNTHETIC2"], "{monitors}");
        let moved = m.screen_request(1, json!({ "cmd": "monitor", "id": r"\\.\SYNTHETIC1" }));
        assert_eq!(moved, json!({ "ok": false, "error": "unknown_monitor" }));
        let lines = stop(&m, &mut recorder, &mut screen);
        assert!(
            lines
                .iter()
                .filter(|l| l["kind"] == "monitor" || l["kind"] == "capture")
                .all(|l| l["id"] == r"\\.\SYNTHETIC2" || l["monitor"] == r"\\.\SYNTHETIC2"),
            "{}",
            show(&lines)
        );

        let m = TempMeeting::new("pinned-missing", "frames");
        let mut recorder = m.recorder(&[]);
        m.wait_recorder(1);
        let mut screen = m.screen(
            "frames",
            Some("synthetic:640x360,monitors=2"),
            &["--monitor", r"\\.\SYNTHETIC9"],
        );
        let status = screen.wait_exit(WAIT_TIMEOUT);
        assert_eq!(status.code(), Some(1));
        assert!(
            first(&m.journal(), "session").is_none(),
            "{}",
            show(&m.journal())
        );
        let _ = m.recorder_request(json!({ "cmd": "stop" }));
        recorder.wait_exit(WAIT_TIMEOUT);
    }

    /// `record` into the test meeting on the slides, to its end; its exit code.
    fn record_code(m: &TempMeeting, resume: bool) -> Option<i32> {
        let meeting = m.arg();
        let mut args = vec![
            "record",
            "--meeting",
            meeting.as_str(),
            "--mode",
            "frames",
            "--test",
            SLIDES,
        ];
        if resume {
            args.push("--resume");
        }
        m.run_screen(&args).0.code()
    }

    /// The startup refusals (spec 4.2, step 1): another screen process recording into the
    /// meeting (4, also for `finalize`), a journal without `--resume` (3), a journal with `stop`
    /// (3, also with `--resume`) and frames on disk without journal lines (3). A refused start
    /// writes nothing.
    pub fn refuses_what_the_meeting_does_not_allow() {
        let m = TempMeeting::new("refusals", "frames");
        let (mut recorder, mut screen) = start(&m, "frames", SLIDES, &[]);
        assert_eq!(record_code(&m, true), Some(4));
        assert_eq!(record_code(&m, false), Some(4));
        let (status, out) = m.run_screen(&["finalize", "--meeting", &m.arg()]);
        assert_eq!(status.code(), Some(4), "{out}");

        // A killed process leaves its journal and a stale screen.json behind.
        screen.hard_kill();
        assert_eq!(record_code(&m, false), Some(3));
        let mut screen = m.screen("frames", Some(SLIDES), &["--resume"]);
        m.wait_journal("the resumed session", |l| {
            l.iter()
                .any(|x| x["kind"] == "session" && x["session"] == 2)
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_eq!(of_kind(&lines, "session").len(), 2, "{}", show(&lines));
        assert_eq!(record_code(&m, true), Some(3));
        assert_eq!(record_code(&m, false), Some(3));
        assert_eq!(
            m.journal().len(),
            lines.len(),
            "a refused start wrote to the journal"
        );

        // Frames on disk count as a recording, even without a journal.
        let m = TempMeeting::new("refusals-media", "frames");
        let frames = m.dir.join("screen").join("frames");
        fs::create_dir_all(&frames).unwrap();
        fs::write(frames.join("f-000001000.jpg"), b"jpeg").unwrap();
        assert_eq!(record_code(&m, false), Some(3));
        assert!(m.journal().is_empty());
    }

    pub fn low_disk_drops_video_then_frames_never_audio() {
        let m = TempMeeting::new("low-disk", "video");
        let free = m.root.join("free-bytes.txt");
        fs::write(&free, (10u64 << 30).to_string()).unwrap();
        let test = spec(&format!("free-bytes-file={}", free.display()));
        let (mut recorder, mut screen) = start(&m, "video", &test, &[]);
        let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
        m.wait_screen_t(1, t(first(&lines, "video").unwrap()) + 3_000);
        fs::write(&free, (3u64 << 30).to_string()).unwrap();
        m.wait_journal("dropped video", |l| first(l, "dropped").is_some());
        let frames_before = of_kind(&m.journal(), "frame").len();
        // Frames go on between the two thresholds.
        m.wait_journal("a frame after the video dropped", |l| {
            of_kind(l, "frame").len() > frames_before
        });
        fs::write(&free, (1u64 << 30).to_string()).unwrap();
        let status = screen.wait_exit(WAIT_TIMEOUT);
        assert!(status.success());
        let lines = m.journal();
        let video_end =
            position(&lines, "video_end", |l| l["reason"] == "disk").expect("no video_end disk");
        let video = position(&lines, "dropped", |l| {
            l["what"] == "video" && l["reason"] == "disk"
        })
        .unwrap();
        let frames = position(&lines, "dropped", |l| {
            l["what"] == "frames" && l["reason"] == "disk"
        })
        .expect("frames never dropped");
        let stop = lines.len() - 1;
        assert!(
            video_end < video && video < frames && frames < stop,
            "{}",
            show(&lines)
        );
        assert_eq!(
            (lines[stop]["kind"].as_str(), lines[stop]["reason"].as_str()),
            (Some("stop"), Some("dropped"))
        );
        assert!(m.screen_file().is_none());
        // The audio never noticed.
        let reply = m.recorder_request(json!({ "cmd": "stop" }));
        assert_eq!(reply["ok"], true);
        recorder.wait_exit(WAIT_TIMEOUT);
        assert_timeline_continuous(&m);
        assert_eq!(m.timeline().last().unwrap()["kind"], "stop");
    }

    pub fn encoder_failure_keeps_frames() {
        let m = TempMeeting::new("encoder-failure", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec("encoder-fail-at=3000"), &[]);
        let lines = m.wait_journal("dropped video", |l| first(l, "dropped").is_some());
        let dropped = first(&lines, "dropped").unwrap().clone();
        assert_eq!(
            (dropped["what"].as_str(), dropped["reason"].as_str()),
            (Some("video"), Some("encoder"))
        );
        let videos = of_kind(&lines, "video");
        assert_eq!(
            videos.len(),
            2,
            "one retry with a new file:\n{}",
            show(&lines)
        );
        assert_ne!(videos[0]["seq"], videos[1]["seq"]);
        let ends: Vec<_> = of_kind(&lines, "video_end")
            .into_iter()
            .filter(|l| l["reason"] == "error")
            .collect();
        assert_eq!(ends.len(), 2, "{}", show(&lines));
        assert!(
            t(videos[1]) >= t(ends[0]) + 1_900,
            "the retry waits 2 s:\n{}",
            show(&lines)
        );
        m.wait_journal("frames after the drop", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) > t(&dropped))
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_eq!(lines.last().unwrap()["reason"], "meeting");
        assert_frames_open(&m, &lines);
        assert_timeline_continuous(&m);
    }

    pub fn no_hardware_encoder_means_frames_only() {
        let m = TempMeeting::new("no-hardware", "video");
        let (mut recorder, mut screen) = start(&m, "video", &format!("{SLIDES},no-hardware"), &[]);
        let lines = m.wait_journal("dropped video", |l| first(l, "dropped").is_some());
        let dropped = first(&lines, "dropped").unwrap().clone();
        assert_eq!(
            (dropped["what"].as_str(), dropped["reason"].as_str()),
            (Some("video"), Some("encoder"))
        );
        assert!(first(&lines, "video").is_none(), "{}", show(&lines));
        let capture = position(&lines, "capture", |_| true).unwrap();
        let drop_at = position(&lines, "dropped", |_| true).unwrap();
        assert!(
            drop_at > capture,
            "dropped at the first capture:\n{}",
            show(&lines)
        );
        m.wait_journal("two frames", |l| of_kind(l, "frame").len() >= 2);
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_frames_open(&m, &lines);
        assert!(!m
            .dir
            .join("screen")
            .join("video")
            .join("000001.mp4")
            .exists());
    }

    pub fn disk_full_writing_mp4_drops_video() {
        let m = TempMeeting::new("disk-full-mp4", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec("write-fail=video@3000"), &[]);
        let lines = m.wait_journal("dropped video", |l| first(l, "dropped").is_some());
        let end =
            position(&lines, "video_end", |l| l["reason"] == "disk").expect("no video_end disk");
        let dropped = position(&lines, "dropped", |l| {
            l["what"] == "video" && l["reason"] == "disk"
        })
        .expect("no dropped video disk");
        assert!(end < dropped, "{}", show(&lines));
        let dropped_t = t(&lines[dropped]);
        m.wait_journal("frames after the drop", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) > dropped_t)
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        assert!(of_kind(&lines, "dropped")
            .iter()
            .all(|l| l["what"] == "video"));
        // The writer is dropped without `Finalize`: the file keeps what reached the disk, cut on a
        // box boundary, and reads when a fragment got there.
        let path = video_path(&m, first(&lines, "video").unwrap());
        let index = mp4::index_path(&path).unwrap();
        assert_eq!(index.complete_len, index.file_len);
        if !index.fragments.is_empty() {
            assert_video_reads(&path);
        }
        assert_timeline_continuous(&m);
    }

    pub fn disk_full_writing_jpeg_drops_video_then_frames() {
        let m = TempMeeting::new("disk-full-jpeg", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec("write-fail=frames@3000"), &[]);
        let status = screen.wait_exit(WAIT_TIMEOUT);
        assert!(status.success());
        let lines = m.journal();
        let video = position(&lines, "dropped", |l| {
            l["what"] == "video" && l["reason"] == "disk"
        })
        .expect("no dropped video disk");
        let frames = position(&lines, "dropped", |l| {
            l["what"] == "frames" && l["reason"] == "disk"
        })
        .expect("no dropped frames disk");
        assert!(video < frames, "{}", show(&lines));
        let stop = lines.last().unwrap();
        assert_eq!(
            (stop["kind"].as_str(), stop["reason"].as_str()),
            (Some("stop"), Some("dropped"))
        );
        let leftovers: Vec<_> = fs::read_dir(m.dir.join("screen").join("frames"))
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.ends_with(".tmp"))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        let reply = m.recorder_request(json!({ "cmd": "stop" }));
        assert_eq!(reply["ok"], true);
        recorder.wait_exit(WAIT_TIMEOUT);
        assert_timeline_continuous(&m);
    }

    pub fn device_loss_keeps_frames() {
        let m = TempMeeting::new("device-lost", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec("device-lost-at=3000"), &[]);
        let lines = m.wait_journal("video and frames after the loss", |l| {
            let Some(lost) = position(l, "device_lost", |_| true) else {
                return false;
            };
            let after = &l[lost..];
            first(after, "capture").is_some()
                && first(after, "video").is_some()
                && first(after, "frame").is_some()
        });
        let lost = position(&lines, "device_lost", |_| true).unwrap();
        let ended =
            position(&lines, "video_end", |l| l["reason"] == "error").expect("no video_end error");
        assert!(ended < lost, "{}", show(&lines));
        let videos = of_kind(&lines, "video");
        assert!(videos.len() >= 2 && videos[0]["seq"] != videos[1]["seq"]);
        assert!(first(&lines, "dropped").is_none(), "{}", show(&lines));
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_frames_open(&m, &lines);
        assert_timeline_continuous(&m);

        // Two failed recreations: the screen recording stops, the audio doesn't.
        let m = TempMeeting::new("device-gone", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec("device-lost-at=3000x2"), &[]);
        let status = screen.wait_exit(WAIT_TIMEOUT);
        assert!(status.success());
        let lines = m.journal();
        let video = position(&lines, "dropped", |l| {
            l["what"] == "video" && l["reason"] == "device"
        })
        .expect("no dropped video device");
        let frames = position(&lines, "dropped", |l| {
            l["what"] == "frames" && l["reason"] == "device"
        })
        .expect("no dropped frames device");
        assert!(video < frames, "{}", show(&lines));
        let stop = lines.last().unwrap();
        assert_eq!(
            (stop["kind"].as_str(), stop["reason"].as_str()),
            (Some("stop"), Some("dropped"))
        );
        let reply = m.recorder_request(json!({ "cmd": "stop" }));
        assert_eq!(reply["ok"], true);
        recorder.wait_exit(WAIT_TIMEOUT);
        assert_timeline_continuous(&m);
    }

    /// The graphics devices the screen process's log names, in order.
    fn devices(m: &TempMeeting) -> Vec<String> {
        fs::read_to_string(m.screen_log())
            .unwrap_or_default()
            .lines()
            .filter_map(|l| {
                l.split_once("graphics device: ")
                    .map(|(_, d)| d.to_string())
            })
            .collect()
    }

    /// After a lost device, a new one without video support, as a basic display driver while the
    /// real one restarts, isn't a recovery while the video records: the process tries again and
    /// the video comes back on a device with video support. When only such devices come, the
    /// video goes with `dropped video device` and the frames go on.
    pub fn a_device_without_video_support_is_no_recovery() {
        let m = TempMeeting::new("no-video-device", "video");
        let (mut recorder, mut screen) = start(
            &m,
            "video",
            &spec("device-lost-at=3000,no-video-device=1"),
            &[],
        );
        let lines = m.wait_journal("video and frames after the loss", |l| {
            position(l, "device_lost", |_| true).is_some_and(|lost| {
                first(&l[lost..], "video").is_some() && first(&l[lost..], "frame").is_some()
            })
        });
        assert!(first(&lines, "dropped").is_none(), "{}", show(&lines));
        let lines = stop(&m, &mut recorder, &mut screen);
        assert!(first(&lines, "dropped").is_none(), "{}", show(&lines));
        let devices = devices(&m);
        if devices.first().map(String::as_str) != Some("GPU with video support") {
            // WARP, as on a CI runner: no device ever has video support, so any one recovers.
            println!(
                "    no graphics device with video support here ({devices:?}); skipped the rest"
            );
            return;
        }
        assert!(
            devices
                .iter()
                .any(|d| d.ends_with("without the video support the video needs")),
            "{devices:?}"
        );
        assert_eq!(
            devices.last().map(String::as_str),
            Some("GPU with video support"),
            "{devices:?}"
        );

        let m = TempMeeting::new("no-video-devices", "video");
        let (mut recorder, mut screen) = start(
            &m,
            "video",
            &spec("device-lost-at=3000,no-video-device=3"),
            &[],
        );
        let lines = m.wait_journal("dropped video", |l| first(l, "dropped").is_some());
        let dropped = first(&lines, "dropped").unwrap().clone();
        assert_eq!(
            (dropped["what"].as_str(), dropped["reason"].as_str()),
            (Some("video"), Some("device")),
            "{}",
            show(&lines)
        );
        m.wait_journal("frames after the drop", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) > t(&dropped))
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_eq!(of_kind(&lines, "dropped").len(), 1, "{}", show(&lines));
        assert_eq!(lines.last().unwrap()["reason"], "meeting");
        assert_frames_open(&m, &lines);
        assert_timeline_continuous(&m);
    }

    pub fn pause_stops_capture() {
        let m = TempMeeting::new("pause", "video");
        let (mut recorder, mut screen) = start(&m, "video", &spec(""), &[]);
        let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
        m.wait_screen_t(1, t(first(&lines, "video").unwrap()) + 3_000);
        // As the app does: the recorder first, then the screen process at once.
        let paused = m.recorder_request(json!({ "cmd": "pause" }));
        assert_eq!(paused["ok"], true, "{paused}");
        assert_eq!(m.screen_request(1, json!({ "cmd": "pause" }))["ok"], true);
        std::thread::sleep(Duration::from_secs(3));
        let resumed = m.recorder_request(json!({ "cmd": "resume" }));
        assert_eq!(resumed["ok"], true, "{resumed}");
        assert_eq!(m.screen_request(1, json!({ "cmd": "resume" }))["ok"], true);
        let (pause_t, resume_t) = (t(&paused), t(&resumed));
        m.wait_journal("a frame after the resume", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) >= resume_t)
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        let between: Vec<_> = of_kind(&lines, "frame")
            .into_iter()
            .filter(|f| t(f) > pause_t && t(f) < resume_t)
            .collect();
        assert!(between.is_empty(), "frames while paused: {between:?}");
        let ended = of_kind(&lines, "video_end");
        assert!(
            ended.iter().any(|l| l["reason"] == "pause"),
            "{}",
            show(&lines)
        );
        assert!(
            of_kind(&lines, "video").iter().any(|v| t(v) >= resume_t),
            "no video after the resume"
        );
        assert_video_ends_cover(&m, &lines);
        assert_frames_open(&m, &lines);
        assert_timeline_continuous(&m);
    }

    /// A control `resume` the supervisor delivers after a newer pause, while the recorder's
    /// timeline still says paused, records nothing: the timeline decides, and its resume ends
    /// the pause.
    pub fn a_late_resume_keeps_the_pause() {
        let m = TempMeeting::new("late-resume", "frames");
        let (mut recorder, mut screen) = start(&m, "frames", SLIDES, &[]);
        m.wait_journal("a frame", |l| first(l, "frame").is_some());
        let paused = m.recorder_request(json!({ "cmd": "pause" }));
        assert_eq!(paused["ok"], true, "{paused}");
        assert_eq!(m.screen_request(1, json!({ "cmd": "pause" }))["ok"], true);
        let pause_t = t(&paused);
        // The stale resume.
        assert_eq!(m.screen_request(1, json!({ "cmd": "resume" }))["ok"], true);
        std::thread::sleep(Duration::from_secs(4));
        let status = m.screen_request(1, json!({ "cmd": "status" }));
        assert_eq!(status["state"], "paused", "{status}");
        let lines = m.journal();
        let pause_at = lines
            .iter()
            .rposition(|l| l["kind"] == "capture" && t(l) <= pause_t)
            .unwrap_or(0);
        assert!(
            lines[pause_at + 1..]
                .iter()
                .all(|l| l["kind"] != "capture" && !(l["kind"] == "frame" && t(l) > pause_t)),
            "recorded during the pause:\n{}",
            show(&lines)
        );

        // The recorder's resume ends the pause, without the control port.
        let resumed = m.recorder_request(json!({ "cmd": "resume" }));
        assert_eq!(resumed["ok"], true, "{resumed}");
        let resume_t = t(&resumed);
        m.wait_journal("a frame after the resume", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) >= resume_t)
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        let between: Vec<_> = of_kind(&lines, "frame")
            .into_iter()
            .filter(|f| t(f) > pause_t && t(f) < resume_t)
            .collect();
        assert!(between.is_empty(), "frames while paused: {between:?}");
        assert_frames_open(&m, &lines);
    }

    /// A control `pause` the supervisor delivers after the newer resume, while the recorder's
    /// timeline already says resumed, stops nothing: the screen keeps recording.
    pub fn a_late_pause_keeps_recording() {
        let m = TempMeeting::new("late-pause", "frames");
        let (mut recorder, mut screen) = start(&m, "frames", SLIDES, &[]);
        m.wait_journal("a frame", |l| first(l, "frame").is_some());
        let paused = m.recorder_request(json!({ "cmd": "pause" }));
        assert_eq!(paused["ok"], true, "{paused}");
        let resumed = m.recorder_request(json!({ "cmd": "resume" }));
        assert_eq!(resumed["ok"], true, "{resumed}");
        // The resume arrives first, then the stale pause.
        assert_eq!(m.screen_request(1, json!({ "cmd": "resume" }))["ok"], true);
        assert_eq!(m.screen_request(1, json!({ "cmd": "pause" }))["ok"], true);
        let late_t = t(&m.screen_request(1, json!({ "cmd": "status" })));
        std::thread::sleep(Duration::from_secs(4));
        let status = m.screen_request(1, json!({ "cmd": "status" }));
        assert_eq!(status["state"], "recording", "{status}");
        m.wait_journal("a frame after the late pause", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) > late_t)
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_frames_open(&m, &lines);
    }

    pub fn screen_modes_can_stop_and_restart_without_interrupting_audio() {
        let m = TempMeeting::new("modes", "off");
        let mut recorder = m.recorder(&[]);
        m.wait_recorder(1);
        let mut screen = m.screen("off", Some(&spec("monitors=2")), &[]);
        m.wait_journal("the inert screen session", |l| {
            first(l, "session").is_some()
        });
        assert!(first(&m.journal(), "capture").is_none());
        let set = |mode| {
            let reply = m.screen_request(
                1,
                json!({"cmd": "mode", "mode": mode, "monitor": r"\\.\SYNTHETIC2"}),
            );
            assert_eq!(reply["ok"], true, "{reply}");
        };
        set("frames");
        m.wait_journal("a frame after enabling capture", |l| {
            first(l, "frame").is_some()
        });
        set("off");
        let off_at = t(of_kind(&m.journal(), "mode").last().unwrap());
        std::thread::sleep(Duration::from_secs(2));
        assert!(!of_kind(&m.journal(), "frame").iter().any(|f| t(f) > off_at));
        assert_eq!(m.recorder_request(json!({"cmd": "pause"}))["ok"], true);
        set("video");
        assert_eq!(m.recorder_request(json!({"cmd": "resume"}))["ok"], true);
        m.wait_journal("video after capture resumes", |l| {
            first(l, "video").is_some()
        });
        std::thread::sleep(Duration::from_secs(3));
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_eq!(
            of_kind(&lines, "mode")
                .iter()
                .map(|l| l["mode"].as_str().unwrap())
                .collect::<Vec<_>>(),
            ["frames", "off", "video"]
        );
        assert!(of_kind(&lines, "capture")
            .iter()
            .all(|l| l["monitor"] == r"\\.\SYNTHETIC2"));
        assert_frames_open(&m, &lines);
        assert_video_ends_cover(&m, &lines);
        assert_timeline_continuous(&m);
    }

    pub fn monitor_switch_starts_a_new_file() {
        let m = TempMeeting::new("switch", "video");
        let test = spec("monitors=2");
        let (mut recorder, mut screen) = start(&m, "video", &test, &[]);
        let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
        assert_eq!(first(&lines, "monitor").unwrap()["id"], r"\\.\SYNTHETIC1");
        let monitors = m.screen_request(1, json!({ "cmd": "monitors" }));
        let ids: Vec<&str> = monitors["monitors"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, [r"\\.\SYNTHETIC1", r"\\.\SYNTHETIC2"]);
        let reply = m.screen_request(1, json!({ "cmd": "monitor", "id": r"\\.\SYNTHETIC2" }));
        assert_eq!(reply, json!({ "ok": true, "monitor": r"\\.\SYNTHETIC2" }));
        let unknown = m.screen_request(1, json!({ "cmd": "monitor", "id": r"\\.\SYNTHETIC7" }));
        assert_eq!(unknown, json!({ "ok": false, "error": "unknown_monitor" }));
        let lines = m.wait_journal("a new video file", |l| of_kind(l, "video").len() >= 2);
        let moved =
            position(&lines, "monitor", |l| l["reason"] == "user").expect("no monitor user");
        assert_eq!(lines[moved]["id"], r"\\.\SYNTHETIC2");
        let ended = position(&lines, "video_end", |l| l["reason"] == "monitor")
            .expect("no video_end monitor");
        let second = lines
            .iter()
            .enumerate()
            .filter(|(_, l)| l["kind"] == "video")
            .nth(1)
            .unwrap()
            .0;
        assert!(ended < second && moved < second, "{}", show(&lines));
        let status = m.screen_request(1, json!({ "cmd": "status" }));
        assert_eq!(status["monitor"], r"\\.\SYNTHETIC2", "{status}");
        let lines = stop(&m, &mut recorder, &mut screen);
        assert_video_ends_cover(&m, &lines);
    }

    /// A recorded monitor that leaves the monitor list is noticed without the capture's `Closed`
    /// notice, which the synthetic source never sends: the recording moves to the other monitor
    /// with a `monitor` line, reason `lost`. Pinned with `--monitor`, the process waits for its
    /// monitor instead and writes one `no_monitor` line, however often it looks again.
    pub fn a_monitor_that_leaves_the_list_is_noticed() {
        let m = TempMeeting::new("monitor-gone", "video");
        let test = spec("monitors=2,monitor-gone-at=3000");
        let (mut recorder, mut screen) = start(&m, "video", &test, &[]);
        let lines = m.wait_journal("the move to the other monitor", |l| {
            position(l, "monitor", |x| x["reason"] == "lost").is_some_and(|at| {
                l[at..]
                    .iter()
                    .any(|x| x["kind"] == "capture" && x["monitor"] == r"\\.\SYNTHETIC2")
            })
        });
        let moved = position(&lines, "monitor", |l| l["reason"] == "lost").unwrap();
        assert_eq!(lines[moved]["id"], r"\\.\SYNTHETIC2", "{}", show(&lines));
        assert!(t(&lines[moved]) >= 3_000, "{}", show(&lines));
        let ended = position(&lines, "video_end", |l| l["reason"] == "monitor")
            .expect("no video_end monitor");
        assert!(ended < moved, "{}", show(&lines));
        let moved_t = t(&lines[moved]);
        m.wait_journal("a frame of the other monitor", |l| {
            of_kind(l, "frame").iter().any(|f| t(f) > moved_t)
        });
        let lines = stop(&m, &mut recorder, &mut screen);
        assert!(
            of_kind(&lines, "video").iter().any(|v| t(v) >= moved_t),
            "no video after the move:\n{}",
            show(&lines)
        );

        let m = TempMeeting::new("monitor-gone-pinned", "frames");
        let (mut recorder, mut screen) = start(
            &m,
            "frames",
            "synthetic:640x360,change-every=500,text=Slide,monitors=2,monitor-gone-at=3000",
            &["--monitor", r"\\.\SYNTHETIC1"],
        );
        m.wait_screen_t(1, 6_500);
        let status = m.screen_request(1, json!({ "cmd": "status" }));
        assert_eq!(status["ok"], true, "{status}");
        let lines = m.journal();
        assert!(
            of_kind(&lines, "monitor")
                .iter()
                .all(|l| l["reason"] == "start"),
            "{}",
            show(&lines)
        );
        let late: Vec<_> = of_kind(&lines, "frame")
            .into_iter()
            .filter(|f| t(f) > 4_000)
            .collect();
        assert!(late.is_empty(), "frames after the monitor went: {late:?}");
        let waits = of_kind(&lines, "no_monitor");
        assert_eq!(waits.len(), 1, "{}", show(&lines));
        assert!((3_000..4_500).contains(&t(waits[0])), "{}", show(&lines));
        stop(&m, &mut recorder, &mut screen);
    }

    pub fn stops_with_the_recorder() {
        let m = TempMeeting::new("stop", "frames");
        let (mut recorder, mut screen) = start(&m, "frames", SLIDES, &[]);
        m.wait_journal("a frame", |l| first(l, "frame").is_some());
        let status = m.screen_request(1, json!({ "cmd": "status" }));
        assert_eq!(
            (status["state"].as_str(), status["mode"].as_str()),
            (Some("recording"), Some("frames")),
            "{status}"
        );
        assert_eq!(status["video"], false);
        assert_eq!(status["frames"], true);
        let lines = stop(&m, &mut recorder, &mut screen);
        let timeline_stop = m.timeline().last().cloned().unwrap();
        assert_eq!(timeline_stop["kind"], "stop");
        let stopped = lines.last().unwrap();
        assert_eq!(stopped["kind"], "stop");
        assert_eq!(stopped["reason"], "meeting");
        assert_eq!(t(stopped), t(&timeline_stop));
        assert!(m.screen_file().is_none(), "screen.json stays after stop");
        assert!(of_kind(&lines, "frame").iter().all(|f| t(f) <= t(stopped)));
        assert_frames_open(&m, &lines);
    }

    pub fn stops_when_the_recorder_is_gone() {
        let m = TempMeeting::new("orphan", "frames");
        let mut recorder = m.recorder(&[]);
        m.wait_recorder(1);
        let mut screen = m.screen_with(
            "frames",
            Some(SLIDES),
            &[],
            &[("VOICE_SCREEN_TEST_ORPHAN_MS", "3000")],
        );
        m.wait_journal("the first capture", |l| first(l, "capture").is_some());
        // Killed and not restarted; nobody asks the screen process anything.
        recorder.hard_kill();
        let status = screen.wait_exit(WAIT_TIMEOUT);
        assert!(status.success());
        let lines = m.journal();
        let stopped = lines.last().unwrap();
        assert_eq!(
            (stopped["kind"].as_str(), stopped["reason"].as_str()),
            (Some("stop"), Some("interrupted"))
        );
        assert!(m.screen_file().is_none());
    }

    pub fn ocr_reads_demo_text() {
        if voice_screen::frames::languages().is_empty() {
            println!("    skipped: this computer has no OCR language");
            return;
        }
        let m = TempMeeting::new("ocr", "frames");
        let (mut recorder, mut screen) =
            start(&m, "frames", "synthetic:1280x720,text=RUKOO DEMO 42", &[]);
        let lines = m.wait_journal("a frame with text", |l| first(l, "frame").is_some());
        let frame = first(&lines, "frame").unwrap();
        assert_eq!(frame["ocr"], "ok", "{}", show(&lines));
        assert!(frame["language"].is_string());
        let text = frame["text"].as_str().unwrap();
        assert!(text.contains("RUKOO DEMO 42"), "read {text:?}");
        stop(&m, &mut recorder, &mut screen);
        let log = fs::read_to_string(m.screen_log()).unwrap();
        assert!(
            !log.contains("RUKOO") && !log.contains("DEMO"),
            "the log holds frame text:\n{log}"
        );
        assert!(log.contains("frame f-"), "{log}");
    }

    /// Typing on one slide: every keystroke's dirty regions cover 0.03% of the screen, far under
    /// the 0.25% that makes the screen process read a thumbnail, but together they change more
    /// than the 1% that saves a frame. Their dirty regions add up, so the change against the last
    /// saved frame is seen and saved. Before, each keystroke counted as no change on its own and
    /// the first frame stayed the only one.
    pub fn typing_adds_up_to_a_saved_frame() {
        let m = TempMeeting::new("typing", "frames");
        let test = with_extra("synthetic:1280x720,text=Notes,type-every=100,dirty", "");
        let (mut recorder, mut screen) = start(&m, "frames", &test, &[]);
        let capture_t = t(first(&m.journal(), "capture").unwrap());
        // 36 keystrokes change 1% of the thumbnail; typing never settles, so the change is saved
        // 3 s after it was first seen. Twelve seconds type 120.
        m.wait_screen_t(1, capture_t + 12_000);
        let lines = stop(&m, &mut recorder, &mut screen);
        let frames = of_kind(&lines, "frame");
        assert!(
            frames.len() >= 2,
            "typing saved no frame after the first:\n{}",
            show(&lines)
        );
        assert!(t(frames[1]) >= capture_t + 3_000, "{}", show(&lines));
        assert_frames_open(&m, &lines);
    }

    /// A still screen that the platform keeps handing over at 15 frames a second, with a keystroke
    /// every second, once with dirty regions and once without. Before, every frame went to the
    /// encoder: 15 samples a second while nothing changed. Now frames that changed next to nothing
    /// are held, so the file holds about the four a second of the quarter-second re-submit, and the
    /// typed marks, too small to send a frame of their own, reach the video with the held frame.
    pub fn a_still_screen_records_few_frames() {
        for extra in ["", "dirty"] {
            let m = TempMeeting::new("still", "video");
            let test = with_extra(
                "synthetic:1280x720,text=Notes,idle-every=66,type-every=1000,software-encoder",
                extra,
            );
            let (mut recorder, mut screen) = start(&m, "video", &test, &[]);
            let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
            let video = first(&lines, "video").unwrap().clone();
            m.wait_screen_t(1, t(&video) + 8_000);
            let lines = stop(&m, &mut recorder, &mut screen);
            assert_video_ends_cover(&m, &lines);
            let path = video_path(&m, &video);
            let report = assert_video_reads(&path);
            let samples = report["samples"].as_u64().unwrap();
            let ms = report["durationMs"].as_u64().unwrap().max(1);
            let rate = samples as f64 * 1000.0 / ms as f64;
            println!("    {extra:5}: {samples} samples in {ms} ms, {rate:.1} a second");
            assert!(rate <= 6.0, "{rate:.1} samples a second on a still screen");
            // The typed marks are counted by decoding the file, which only the Windows spike
            // has a decoder for; macOS checks the sample rate alone.
            #[cfg(windows)]
            {
                let marks = marks_in_video(&path, 1280, 720);
                println!(
                    "    marks {:?}",
                    marks.iter().map(|m| m.1).collect::<Vec<_>>()
                );
                assert_eq!(marks.first().map(|m| m.1), Some(0), "{marks:?}");
                let typed = marks.last().unwrap().1;
                assert!(
                    typed >= 5,
                    "the video ends with {typed} typed marks: {marks:?}"
                );
                assert!(marks.windows(2).all(|w| w[0].1 <= w[1].1), "{marks:?}");
                // Samples are never more than a re-submit and a tick apart, so each mark shows
                // within about half a second of its keystroke.
                let gaps = marks.windows(2).map(|w| w[1].0 - w[0].0);
                assert!(gaps.clone().all(|g| g <= 600), "{marks:?}");
            }
        }
    }

    /// Decodes a video file through Media Foundation and counts in each sample the synthetic
    /// screen's typed marks, the dark squares `type-every` draws in a row under the text. Returns
    /// each sample's media time in ms with the marks shown.
    #[cfg(windows)]
    fn marks_in_video(path: &std::path::Path, width: u32, height: u32) -> Vec<(u64, u32)> {
        use windows::core::{Interface, HSTRING};
        use windows::Win32::Media::MediaFoundation::*;

        /// `MF_SOURCE_READER_FIRST_VIDEO_STREAM`.
        const FIRST_VIDEO: u32 = 0xFFFF_FFFC;
        let mf = voice_screen::video::mf::shared().expect("Media Foundation");
        let reader = mf
            .create_source_reader(&HSTRING::from(path.as_os_str()))
            .unwrap();
        let nv12 = mf.create_media_type().unwrap();
        unsafe {
            nv12.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video).unwrap();
            nv12.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).unwrap();
            reader
                .SetCurrentMediaType(FIRST_VIDEO, None, &nv12)
                .unwrap();
        }
        // Where `synthetic::Canvas::type_key` puts mark `n`: its centre.
        let per_row = (width - 80) / 24;
        let centre = |n: u32| {
            let x = 40 + (n % per_row) * 24 + 8;
            let y = ((height * 2 / 3) & !7) + (n / per_row % 4) * 24 + 8;
            (x as usize, y as usize)
        };
        let mut out = Vec::new();
        loop {
            let (mut flags, mut time) = (0u32, 0i64);
            let mut sample: Option<IMFSample> = None;
            unsafe {
                reader
                    .ReadSample(
                        FIRST_VIDEO,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut time),
                        Some(&mut sample),
                    )
                    .unwrap();
            }
            if let Some(sample) = sample {
                let buffer = unsafe { sample.GetBufferByIndex(0) }.unwrap();
                let planes: IMF2DBuffer = buffer.cast().unwrap();
                let (mut scan0, mut pitch) = (std::ptr::null_mut(), 0i32);
                unsafe { planes.Lock2D(&mut scan0, &mut pitch) }.unwrap();
                // The luma plane comes first; limited range, so white is 235 and a mark near 43.
                let luma = |(x, y): (usize, usize)| unsafe {
                    *scan0.offset(y as isize * pitch as isize + x as isize)
                };
                let shown = (0..per_row * 4)
                    .take_while(|&n| luma(centre(n)) < 128)
                    .count() as u32;
                unsafe { planes.Unlock2D() }.unwrap();
                out.push(((time / 10_000) as u64, shown));
            }
            if flags & MF_SOURCE_READERF_ENDOFSTREAM.0 as u32 != 0 {
                break;
            }
            assert_eq!(flags & MF_SOURCE_READERF_ERROR.0 as u32, 0);
        }
        out
    }

    /// Spike 2 (spec 4.2), on a PC with a hardware encoder: a monitor over 4096 per side or
    /// 36,864 macroblocks is scaled by the sink writer's video processor, and one at 3840 x 2160
    /// keeps its size. The synthetic screen, so nothing real is captured.
    #[cfg(windows)]
    pub fn spike_hardware_encoder_scales_large_monitors() {
        for (size, expected) in [
            ("5120x1440", (4096, 1152)),
            ("7680x2160", (4096, 1152)),
            ("3840x2160", (3840, 2160)),
        ] {
            let m = TempMeeting::new("spike-scale", "video");
            let test = format!("synthetic:{size},change-every=1000,text=Slide");
            let (mut recorder, mut screen) = start(&m, "video", &test, &[]);
            let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
            let video = first(&lines, "video").unwrap().clone();
            assert_eq!(video["hardware"], true, "{video}");
            assert_eq!(
                (video["width"].as_u64(), video["height"].as_u64()),
                (Some(expected.0), Some(expected.1)),
                "{size}"
            );
            m.wait_screen_t(1, t(&video) + 4_000);
            let lines = stop(&m, &mut recorder, &mut screen);
            // Scaling puts a video processor in front of NVENC, as on the software path.
            assert_video_ends_cover(&m, &lines);
            let report = assert_video_reads(&video_path(&m, &video));
            assert_eq!(
                (
                    report["reader"]["width"].as_u64(),
                    report["reader"]["height"].as_u64()
                ),
                (Some(expected.0), Some(expected.1)),
                "{report}"
            );
            println!(
                "    {size} -> {}x{}: {} samples, fragments {} ms, {} bytes",
                expected.0, expected.1, report["samples"], report["fragmentMs"], report["bytes"]
            );
        }
    }

    /// Spike 3 (spec 4.2): real Windows.Graphics.Capture on each display, NVENC video, and how
    /// many frames a second arrive while the screen changes at 60 Hz. It captures the real
    /// screen, so it runs only with `VOICE_SCREEN_TEST_REAL_SCREEN=1`, demo windows covering
    /// every display and `VOICE_SCREEN_TEST_REAL_DIR` naming a scratch folder; the captures are
    /// deleted at the end, also when it fails.
    #[cfg(windows)]
    pub fn spike_real_capture_cadence() {
        if std::env::var("VOICE_SCREEN_TEST_REAL_SCREEN").as_deref() != Ok("1") {
            println!("    skipped: needs VOICE_SCREEN_TEST_REAL_SCREEN=1 and demo windows on every display");
            return;
        }
        let base = std::path::PathBuf::from(
            std::env::var("VOICE_SCREEN_TEST_REAL_DIR").expect("VOICE_SCREEN_TEST_REAL_DIR"),
        );
        let features = voice_screen::capture::wgc::features();
        println!("    capture features on this Windows: {features:?}");
        let probe = voice_screen::probe::probe();
        for monitor in &probe.monitors {
            let m = TempMeeting::real_screen(&base, "spike-capture", "video");
            let recorder = m.recorder(&[]);
            m.wait_recorder(1);
            let screen = m.screen("video", None, &["--monitor", &monitor.id]);
            let (mut recorder, mut screen) = (recorder, screen);
            let lines = m.wait_journal("a video file", |l| first(l, "video").is_some());
            let video = first(&lines, "video").unwrap().clone();
            let capture = first(&lines, "capture").unwrap().clone();
            m.wait_screen_t(1, t(&video) + 8_000);
            let lines = stop(&m, &mut recorder, &mut screen);
            let report = assert_video_reads(&video_path(&m, &video));
            let samples = report["samples"].as_u64().unwrap();
            let ms = report["durationMs"].as_u64().unwrap().max(1);
            let fps = samples as f64 * 1000.0 / ms as f64;
            println!(
                "    {}: capture {}x{}, video {}x{} by {} (hardware {}), {samples} samples in {ms} ms = {fps:.1} fps, fragments {} ms, {} frames saved",
                monitor.id,
                capture["width"],
                capture["height"],
                video["width"],
                video["height"],
                video["encoder"],
                video["hardware"],
                report["fragmentMs"],
                of_kind(&lines, "frame").len()
            );
            assert!(fps <= 16.5, "{fps:.1} fps: the 15 fps cap doesn't hold");
            assert_eq!(
                (capture["width"].as_u64(), capture["height"].as_u64()),
                (
                    Some(u64::from(monitor.width & !1)),
                    Some(u64::from(monitor.height & !1))
                )
            );
        }
    }
}

#[cfg(any(windows, target_os = "macos"))]
fn main() {
    common::play_role();
    use common::Test;
    #[allow(unused_mut)]
    let mut tests = vec![
        Test {
            name: "screen_modes_can_stop_and_restart_without_interrupting_audio",
            ignored: false,
            run: tests::screen_modes_can_stop_and_restart_without_interrupting_audio,
        },
        Test {
            name: "hard_kill_keeps_video_and_frames",
            ignored: false,
            run: tests::hard_kill_keeps_video_and_frames,
        },
        Test {
            name: "hard_kill_with_hardware_encoder",
            ignored: true,
            run: tests::hard_kill_with_hardware_encoder,
        },
        Test {
            name: "finalize_closes_a_killed_session",
            ignored: false,
            run: tests::finalize_closes_a_killed_session,
        },
        Test {
            name: "finalize_repairs_only_video_files",
            ignored: false,
            run: tests::finalize_repairs_only_video_files,
        },
        Test {
            name: "portrait_monitor_keeps_its_size",
            ignored: false,
            run: tests::portrait_monitor_keeps_its_size,
        },
        Test {
            name: "pinned_monitor_is_recorded",
            ignored: false,
            run: tests::pinned_monitor_is_recorded,
        },
        Test {
            name: "refuses_what_the_meeting_does_not_allow",
            ignored: false,
            run: tests::refuses_what_the_meeting_does_not_allow,
        },
        Test {
            name: "low_disk_drops_video_then_frames_never_audio",
            ignored: false,
            run: tests::low_disk_drops_video_then_frames_never_audio,
        },
        Test {
            name: "encoder_failure_keeps_frames",
            ignored: false,
            run: tests::encoder_failure_keeps_frames,
        },
        Test {
            name: "no_hardware_encoder_means_frames_only",
            ignored: false,
            run: tests::no_hardware_encoder_means_frames_only,
        },
        Test {
            name: "disk_full_writing_mp4_drops_video",
            ignored: false,
            run: tests::disk_full_writing_mp4_drops_video,
        },
        Test {
            name: "disk_full_writing_jpeg_drops_video_then_frames",
            ignored: false,
            run: tests::disk_full_writing_jpeg_drops_video_then_frames,
        },
        Test {
            name: "device_loss_keeps_frames",
            ignored: false,
            run: tests::device_loss_keeps_frames,
        },
        Test {
            name: "a_device_without_video_support_is_no_recovery",
            ignored: false,
            run: tests::a_device_without_video_support_is_no_recovery,
        },
        Test {
            name: "pause_stops_capture",
            ignored: false,
            run: tests::pause_stops_capture,
        },
        Test {
            name: "a_late_resume_keeps_the_pause",
            ignored: false,
            run: tests::a_late_resume_keeps_the_pause,
        },
        Test {
            name: "a_late_pause_keeps_recording",
            ignored: false,
            run: tests::a_late_pause_keeps_recording,
        },
        Test {
            name: "monitor_switch_starts_a_new_file",
            ignored: false,
            run: tests::monitor_switch_starts_a_new_file,
        },
        Test {
            name: "a_monitor_that_leaves_the_list_is_noticed",
            ignored: false,
            run: tests::a_monitor_that_leaves_the_list_is_noticed,
        },
        Test {
            name: "stops_with_the_recorder",
            ignored: false,
            run: tests::stops_with_the_recorder,
        },
        Test {
            name: "stops_when_the_recorder_is_gone",
            ignored: false,
            run: tests::stops_when_the_recorder_is_gone,
        },
        Test {
            name: "ocr_reads_demo_text",
            ignored: false,
            run: tests::ocr_reads_demo_text,
        },
        Test {
            name: "typing_adds_up_to_a_saved_frame",
            ignored: false,
            run: tests::typing_adds_up_to_a_saved_frame,
        },
        Test {
            name: "a_still_screen_records_few_frames",
            ignored: false,
            run: tests::a_still_screen_records_few_frames,
        },
    ];
    // The Windows sink writer's scaling and the real screen capture are Windows spikes.
    #[cfg(windows)]
    {
        tests.push(Test {
            name: "spike_hardware_encoder_scales_large_monitors",
            ignored: true,
            run: tests::spike_hardware_encoder_scales_large_monitors,
        });
        tests.push(Test {
            name: "spike_real_capture_cadence",
            ignored: true,
            run: tests::spike_real_capture_cadence,
        });
    }
    common::run_tests(&tests);
}

#[cfg(not(any(windows, target_os = "macos")))]
fn main() {
    common::play_role();
    println!("the screen process's tests need Windows or macOS; skipped");
}
