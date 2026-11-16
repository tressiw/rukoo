//! Helpers for the screen process's own tests. The test binary plays two roles besides running
//! the tests: with `VOICE_SCREEN_TEST_ROLE=recorder` it is `rukoo-voice-recorder`, with
//! `VOICE_SCREEN_TEST_ROLE=screen` it is `rukoo-voice-screen`. So each test runs the real
//! recorder and the real screen process as separate processes, as the app does, and can kill
//! either.
#![allow(dead_code)]

use std::cell::Cell;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

pub const ROLE: &str = "VOICE_SCREEN_TEST_ROLE";
pub const MEETING_ID: &str = "m-20261102-120000-abcd";

/// CI machines are slow and the software encoder is busy, so every wait is generous; the
/// tolerances on recorded times are in the tests that use them.
pub const START_TIMEOUT: Duration = Duration::from_secs(30);
pub const WAIT_TIMEOUT: Duration = Duration::from_secs(45);

/// Call first in `main`: when this process was started for a role, play it and exit.
pub fn play_role() {
    let Ok(role) = std::env::var(ROLE) else {
        return;
    };
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match role.as_str() {
        "recorder" => voice_recorder::run(&args).map_err(|e| (e.code, e.message)),
        "screen" => voice_screen::run(&args).map_err(|e| (e.code, e.message)),
        other => Err((2, format!("unknown role {other}"))),
    };
    match result {
        Ok(()) => std::process::exit(0),
        Err((code, message)) => {
            let _ = writeln!(std::io::stderr(), "{role}: {message}");
            std::process::exit(code)
        }
    }
}

/// One test of a `harness = false` binary.
pub struct Test {
    pub name: &'static str,
    /// Runs only with `--ignored` or `--include-ignored`, as `#[ignore]` would.
    pub ignored: bool,
    pub run: fn(),
}

/// Runs the tests the command line selects, one after the other, like libtest: names filter by
/// substring (or exactly with `--exact`), `--skip <name>`, `--ignored`, `--include-ignored`,
/// `--list`. Exits 101 when one failed.
pub fn run_tests(tests: &[Test]) -> ! {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut filters = Vec::new();
    let mut skips = Vec::new();
    let (mut ignored_only, mut include_ignored, mut exact, mut list) = (false, false, false, false);
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--ignored" => ignored_only = true,
            "--include-ignored" => include_ignored = true,
            "--exact" => exact = true,
            "--list" => list = true,
            "--skip" => skips.extend(it.next().cloned()),
            "--test-threads" | "--format" | "--color" | "-Z" | "--logfile" => {
                it.next();
            }
            a if a.starts_with('-') => {}
            name => filters.push(name.to_string()),
        }
    }
    let selected: Vec<&Test> = tests
        .iter()
        .filter(|t| {
            filters.is_empty()
                || filters.iter().any(|f| {
                    if exact {
                        t.name == f
                    } else {
                        t.name.contains(f.as_str())
                    }
                })
        })
        .filter(|t| !skips.iter().any(|s| t.name.contains(s.as_str())))
        .collect();
    if list {
        for t in &selected {
            println!("{}: test", t.name);
        }
        std::process::exit(0);
    }
    println!("\nrunning {} tests", selected.len());
    let (mut passed, mut failed, mut ignored) = (0, Vec::new(), 0);
    for t in selected {
        let runs = if ignored_only {
            t.ignored
        } else {
            include_ignored || !t.ignored
        };
        if !runs {
            println!("test {} ... ignored", t.name);
            ignored += 1;
            continue;
        }
        let started = Instant::now();
        println!("test {} ...", t.name);
        match panic::catch_unwind(AssertUnwindSafe(t.run)) {
            Ok(()) => {
                println!(
                    "test {} ... ok ({:.1} s)",
                    t.name,
                    started.elapsed().as_secs_f64()
                );
                passed += 1;
            }
            Err(_) => {
                println!("test {} ... FAILED", t.name);
                failed.push(t.name);
            }
        }
    }
    let result = if failed.is_empty() { "ok" } else { "FAILED" };
    if !failed.is_empty() {
        println!("\nfailures:");
        for name in &failed {
            println!("    {name}");
        }
    }
    println!(
        "\ntest result: {result}. {passed} passed; {} failed; {ignored} ignored\n",
        failed.len()
    );
    std::process::exit(if failed.is_empty() { 0 } else { 101 })
}

/// A child process killed and reaped when the test ends, whatever happened.
pub struct Proc(pub Child);

impl Deref for Proc {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl DerefMut for Proc {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Proc {
    fn drop(&mut self) {
        if matches!(self.0.try_wait(), Ok(None)) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
}

impl Proc {
    /// Waits for the process to end on its own.
    pub fn wait_exit(&mut self, timeout: Duration) -> ExitStatus {
        wait_until(timeout, || self.0.try_wait().unwrap())
    }

    /// Kills it now: TerminateProcess, nothing runs after it.
    pub fn hard_kill(&mut self) {
        self.0.kill().unwrap();
        self.0.wait().unwrap();
    }
}

/// A data folder with one meeting folder in it. It's removed after a passing test and kept
/// after a failing one, unless it holds captures of the real screen: those always go.
pub struct TempMeeting {
    pub root: PathBuf,
    pub dir: PathBuf,
    spawned: Cell<u32>,
    always_remove: bool,
}

impl TempMeeting {
    pub fn new(name: &str, screen: &str) -> TempMeeting {
        TempMeeting::new_in(&std::env::temp_dir(), name, screen, false)
    }

    /// For captures of the real screen: under `base`, removed even when the test fails.
    pub fn real_screen(base: &Path, name: &str, screen: &str) -> TempMeeting {
        TempMeeting::new_in(base, name, screen, true)
    }

    fn new_in(base: &Path, name: &str, screen: &str, always_remove: bool) -> TempMeeting {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = base.join(format!(
            "voice-screen-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        let dir = root.join("meetings").join(MEETING_ID);
        fs::create_dir_all(&dir).unwrap();
        fs::create_dir_all(root.join("logs")).unwrap();
        // The app writes meeting.json before it starts the recorder.
        let meeting = json!({
            "id": MEETING_ID,
            "title": "Test meeting",
            "createdAt": 1_793_000_000_000u64,
            "tracks": ["mic"],
            "sources": {},
            "screen": screen,
            "startedBy": "rukoo",
            "context": null,
            "version": 1,
        });
        fs::write(dir.join("meeting.json"), meeting.to_string()).unwrap();
        TempMeeting {
            root,
            dir,
            spawned: Cell::new(0),
            always_remove,
        }
    }

    pub fn arg(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }

    /// The screen process's log, as the app keeps it.
    pub fn screen_log(&self) -> PathBuf {
        self.root
            .join("logs")
            .join(format!("screen-{MEETING_ID}.log"))
    }

    fn spawn(&self, role: &str, args: &[String], envs: &[(&str, &str)], log: File) -> Proc {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(args)
            .env(ROLE, role)
            .env("RUKOO_VOICE_DATA_DIR", &self.root)
            .env_remove("RUST_TEST_THREADS")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log);
        for (key, value) in envs {
            command.env(key, value);
        }
        Proc(
            command
                .spawn()
                .unwrap_or_else(|e| panic!("start the {role}: {e}")),
        )
    }

    /// The real recorder with a synthetic microphone.
    pub fn recorder(&self, extra: &[&str]) -> Proc {
        let n = self.spawned.get();
        self.spawned.set(n + 1);
        let log = File::create(self.root.join("logs").join(format!("recorder-{n}.log"))).unwrap();
        let mut args = vec![
            "record".to_string(),
            "--meeting".into(),
            self.arg(),
            "--track".into(),
            "mic=synthetic:tone:440".into(),
        ];
        args.extend(extra.iter().map(|s| s.to_string()));
        self.spawn("recorder", &args, &[], log)
    }

    /// The screen process: `record --meeting <dir> --mode <mode> [--test <spec>] <extra>`.
    pub fn screen(&self, mode: &str, test: Option<&str>, extra: &[&str]) -> Proc {
        self.screen_with(mode, test, extra, &[])
    }

    pub fn screen_with(
        &self,
        mode: &str,
        test: Option<&str>,
        extra: &[&str],
        envs: &[(&str, &str)],
    ) -> Proc {
        let mut args = vec![
            "record".to_string(),
            "--meeting".into(),
            self.arg(),
            "--mode".into(),
            mode.into(),
        ];
        if let Some(spec) = test {
            args.push("--test".into());
            args.push(spec.into());
        }
        args.extend(extra.iter().map(|s| s.to_string()));
        let log = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.screen_log())
            .unwrap();
        self.spawn("screen", &args, envs, log)
    }

    /// Runs a screen command to its end: its status and stdout.
    pub fn run_screen(&self, args: &[&str]) -> (ExitStatus, String) {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(args)
            .env(ROLE, "screen")
            .env("RUKOO_VOICE_DATA_DIR", &self.root)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        (
            out.status,
            String::from_utf8_lossy(&out.stdout).into_owned(),
        )
    }

    /// Every complete line of a JSON-lines file, each of which must parse.
    fn read_lines(path: &Path) -> Vec<Value> {
        let text = fs::read_to_string(path).unwrap_or_default();
        let complete = match text.rfind('\n') {
            Some(end) => &text[..end],
            None => "",
        };
        complete
            .lines()
            .map(|line| {
                serde_json::from_str(line).unwrap_or_else(|e| panic!("bad line {line:?}: {e}"))
            })
            .collect()
    }

    pub fn journal(&self) -> Vec<Value> {
        Self::read_lines(&self.dir.join("screen").join("journal.jsonl"))
    }

    pub fn timeline(&self) -> Vec<Value> {
        Self::read_lines(&self.dir.join("timeline.jsonl"))
    }

    /// Waits until the journal satisfies `done`, and returns it.
    pub fn wait_journal(&self, what: &str, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            let lines = self.journal();
            if done(&lines) {
                return lines;
            }
            if Instant::now() >= deadline {
                panic!("timed out waiting for {what}; journal:\n{}", show(&lines));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Waits until the recorder's timeline has a `session` line numbered `session`.
    pub fn wait_recorder(&self, session: u64) {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            let lines = Self::read_lines(&self.dir.join("timeline.jsonl"));
            if lines
                .iter()
                .any(|l| l["kind"] == "session" && l["session"] == session)
            {
                return;
            }
            assert!(
                Instant::now() < deadline,
                "the recorder never wrote session {session}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn screen_file(&self) -> Option<Value> {
        let bytes = fs::read(self.dir.join("screen").join("screen.json")).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    /// A request to the live screen process of `session`.
    pub fn screen_request(&self, session: u64, body: Value) -> Value {
        let deadline = Instant::now() + START_TIMEOUT;
        loop {
            if let Some(file) = self.screen_file().filter(|f| f["session"] == session) {
                let port = file["port"].as_u64().unwrap() as u16;
                let token = file["token"].as_str().unwrap();
                if let Ok(reply) = voice_screen_files::control::request(
                    port,
                    token,
                    body.clone(),
                    Duration::from_secs(15),
                ) {
                    return reply;
                }
            }
            assert!(
                Instant::now() < deadline,
                "screen process {session} never answered"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The meeting time of the screen process, from its status.
    pub fn screen_t(&self, session: u64) -> u64 {
        let status = self.screen_request(session, json!({ "cmd": "status" }));
        assert_eq!(status["ok"], true, "{status}");
        status["t"].as_u64().unwrap()
    }

    /// Waits until the screen process's meeting time reaches `t`.
    pub fn wait_screen_t(&self, session: u64, t: u64) -> u64 {
        let deadline = Instant::now() + WAIT_TIMEOUT;
        loop {
            let now = self.screen_t(session);
            if now >= t {
                return now;
            }
            assert!(Instant::now() < deadline, "the screen clock stays at {now}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// A request to the recorder's control port.
    pub fn recorder_request(&self, body: Value) -> Value {
        let file: Value =
            serde_json::from_slice(&fs::read(self.dir.join("recorder.json")).unwrap()).unwrap();
        let port = file["port"].as_u64().unwrap() as u16;
        let mut client =
            voice_recorder::control::Client::connect(port, Duration::from_secs(5)).unwrap();
        let mut request = body;
        request["token"] = file["token"].clone();
        client.request(&request).unwrap()
    }
}

impl Drop for TempMeeting {
    fn drop(&mut self) {
        if std::thread::panicking() && !self.always_remove {
            eprintln!("kept the test folder {}", self.root.display());
        } else {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

pub fn wait_until<T>(timeout: Duration, mut check: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = check() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out after {timeout:?}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn t(line: &Value) -> u64 {
    line["t"]
        .as_u64()
        .unwrap_or_else(|| panic!("no t in {line}"))
}

pub fn of_kind<'a>(lines: &'a [Value], kind: &str) -> Vec<&'a Value> {
    lines.iter().filter(|l| l["kind"] == kind).collect()
}

pub fn first<'a>(lines: &'a [Value], kind: &str) -> Option<&'a Value> {
    lines.iter().find(|l| l["kind"] == kind)
}

/// A journal or timeline for a failure message, frame text left out.
pub fn show(lines: &[Value]) -> String {
    lines
        .iter()
        .map(|l| {
            let mut l = l.clone();
            if l["kind"] == "frame" {
                l["text"] = json!(format!(
                    "<{} chars>",
                    l["text"].as_str().map_or(0, |s| s.chars().count())
                ));
            }
            l.to_string()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The index of the first line matching `kind` and `pred`.
pub fn position(lines: &[Value], kind: &str, pred: impl Fn(&Value) -> bool) -> Option<usize> {
    lines.iter().position(|l| l["kind"] == kind && pred(l))
}

/// The recorder's timeline stayed whole while the screen process did whatever it did: no crash,
/// no lost device, no suspend, and the microphone's segments meet end to start.
pub fn assert_timeline_continuous(m: &TempMeeting) {
    let lines = m.timeline();
    for kind in ["crash", "device_lost", "suspend"] {
        assert!(
            of_kind(&lines, kind).is_empty(),
            "the timeline has {kind}:\n{}",
            show(&lines)
        );
    }
    // By `t`, track by track: the recorder writes a `segment_end` once its segment is on disk,
    // which can be after the track's next `segment` line or a pause.
    let pauses: Vec<u64> = of_kind(&lines, "pause").iter().map(|l| t(l)).collect();
    let mut tracks: Vec<&str> = of_kind(&lines, "segment")
        .iter()
        .filter_map(|l| l["track"].as_str())
        .collect();
    tracks.sort_unstable();
    tracks.dedup();
    for track in tracks {
        let segments: Vec<&Value> = of_kind(&lines, "segment")
            .into_iter()
            .filter(|l| l["track"] == track)
            .collect();
        for pair in segments.windows(2) {
            let Some(end) = of_kind(&lines, "segment_end")
                .into_iter()
                .find(|l| l["track"] == track && l["seq"] == pair[0]["seq"])
                .map(t)
            else {
                continue;
            };
            // A pause leaves a gap on purpose.
            if pauses.iter().any(|&p| p + 30 >= end && p <= t(pair[1])) {
                continue;
            }
            let gap = t(pair[1]).abs_diff(end);
            assert!(gap <= 30, "a {gap} ms gap in the audio:\n{}", show(&lines));
        }
    }
    assert!(
        !of_kind(&lines, "segment").is_empty(),
        "the timeline has no audio:\n{}",
        show(&lines)
    );
}
