//! The one writer of a meeting's files: a second recorder can't write into a meeting that one is
//! recording, while readers keep following the timeline and the segments, and a recorder that is
//! hard-killed leaves no lock behind for its successor (`voice/RECORDING.md`).

mod common;

#[cfg(unix)]
mod unix {
    use std::io;

    use serde_json::json;

    use super::common::*;
    use voice_recorder::meeting;
    use voice_recorder::timeline::{self, History, Line, TimelineWriter};

    /// Unix only: it asserts the lock's `WouldBlock`, which is how this platform reports a second
    /// writer. Windows reports its share mode as a sharing violation, which `meeting.rs` covers
    /// in a unit test that runs there too. The exit code is what a second recorder reports to the
    /// app.
    #[test]
    fn a_second_writer_is_busy_but_readers_follow_the_recorder() {
        let m = TempMeeting::new("lock");
        let tracks = ["mic=synthetic:tone:440", "call=synthetic:tone:660"];
        let mut child = m.record(&tracks, &[]);
        m.wait_for_heartbeat(2_000);

        // The live recorder holds the timeline.
        let path = meeting::timeline_path(&m.dir);
        let err = match TimelineWriter::open(&path) {
            Ok(_) => panic!("a second writer opened the live timeline"),
            Err(e) => e,
        };
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock, "{err}");
        let busy = match meeting::open_timeline(&m.dir) {
            Ok(_) => panic!("a second writer opened the live timeline"),
            Err(e) => e,
        };
        assert_eq!(busy.code, voice_recorder::exit::BUSY, "{busy}");

        // Readers take no lock: the timeline parses and the open segment opens as a WAV, which
        // is what live transcription and the app do while the recorder writes.
        let lines = timeline::read(&path).unwrap();
        assert!(
            lines
                .iter()
                .any(|l| matches!(l, Line::Session { session: 1, .. })),
            "{lines:?}"
        );
        let segment = History::from_lines(&lines)
            .segments
            .last()
            .cloned()
            .expect("a segment started");
        let (pcm, rate) = voice_audio::read_wav_i16(&m.dir.join(&segment.file)).unwrap();
        assert_eq!(rate, 16_000);
        assert!(!pcm.is_empty(), "{} holds no samples yet", segment.file);

        // A hard kill releases everything the recorder held.
        child.kill().unwrap();
        child.wait().unwrap();
        let next = TimelineWriter::open(&path).expect("the lock outlived the killed recorder");
        drop(next);

        // And the recorder itself continues the meeting, as the app restarts it.
        let mut child = m.record(&tracks, &["--resume"]);
        let (mut client, token) = m.connect(2);
        let lines = m.lines_so_far();
        let resumed_at = t(of_kind(&lines, "session")
            .into_iter()
            .find(|l| l["session"] == 2)
            .expect("session 2 started"));
        m.wait_for_heartbeat(resumed_at + 2_000);
        let reply = command(&mut client, &token, json!({ "cmd": "stop" }));
        assert_eq!(reply["ok"], true, "{reply}");
        assert!(wait_exit(&mut child).success());
    }
}
