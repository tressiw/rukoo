//! Speaker labels on the 60-second AMI meeting excerpt, with the real recorder, the real
//! `rukoo-voice-speakers` worker and NeMo-Speech.cpp: three voices before the recording stops,
//! at least 90 % of transcript time to the right speaker, and the same ids after the worker is
//! killed.
//!
//! Windows and macOS. It skips unless these are set (`speaker-assets.ps1` fetches them on Windows,
//! `node voice/scripts/speaker-assets.mjs` on macOS):
//!
//! - `RUKOO_VOICE_TEST_NEMO_DIR`: the NeMo-Speech.cpp runtime folder.
//! - `RUKOO_VOICE_TEST_NEMOTRON`: the Nemotron 3 Diarization GGUF, or the folder that holds it.
//! - `RUKOO_VOICE_TEST_AMI`: the folder with `ami_en2002d_2132.wav`, `.rttm` and `.json`, or the
//!   WAV with the other two beside it.
//!
//! Words come from an oracle instead of a speech model: it finds each window in the excerpt and
//! returns the reference transcript's words spread over each utterance, so the score measures
//! the labels alone. A test that needs only `RUKOO_VOICE_TEST_AMI` checks that the oracle hears
//! every word through the real recorder, so a low score is about the labels, not the harness.
#![cfg(any(windows, target_os = "macos"))]

mod common;

use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError};
use std::time::Duration;

use common::score::{accuracy, mapping, Span};
use common::*;
use serde_json::Value;
use voice_bridge::EventLog;
use voice_core::local::{ModelRegistry, ModelSpec};
use voice_core::speakers::{SpeakerLabels, SpeakerOptions};
use voice_core::{Hints, ProviderError};
use voice_meetings::{
    CreateMeeting, LabelState, Labels, Library, MeetingDir, Pipeline, PipelineHandle,
    PipelineOptions, PipelineState, TimedSpeechToText, TimedText, Timing,
};
use voice_protocol::{ScreenMode, StartedBy, Track};
use voice_recorder::control::Client as RecorderClient;
use voice_recorder::meeting::read_recorder_file;

const WAIT: Duration = Duration::from_secs(180);
const EXCERPT_MS: u64 = 60_000;
/// One AMI test at a time. Each plays the excerpt through the real recorder in real time, and
/// two of them run a speaker worker whose four NeMo threads wait for each other at every step:
/// side by side on a four-core CI runner the labels came too late, and only `call-1` arrived
/// before the stop.
fn one_at_a_time() -> MutexGuard<'static, ()> {
    static AMI: Mutex<()> = Mutex::new(());
    AMI.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The three reference speakers with 8 s or more in the excerpt.
const MAIN: [&str; 3] = ["MEE071", "FEO070", "MEE073"];

/// The excerpt, its RTTM turns and its reference words.
struct Ami {
    wav: PathBuf,
    rttm: PathBuf,
    json: PathBuf,
}

/// The excerpt plus what Nemotron needs.
struct Assets {
    runtime: PathBuf,
    model: PathBuf,
    ami: Ami,
}

fn var(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn ami() -> Option<Ami> {
    let ami = var("RUKOO_VOICE_TEST_AMI")?;
    let wav = if ami.is_dir() {
        ami.join("ami_en2002d_2132.wav")
    } else {
        ami
    };
    let ami = Ami {
        rttm: wav.with_extension("rttm"),
        json: wav.with_extension("json"),
        wav,
    };
    for path in [&ami.wav, &ami.rttm, &ami.json] {
        assert!(path.is_file(), "missing {}", path.display());
    }
    Some(ami)
}

fn assets() -> Option<Assets> {
    let (Some(runtime), Some(model), Some(ami)) = (
        var("RUKOO_VOICE_TEST_NEMO_DIR"),
        var("RUKOO_VOICE_TEST_NEMOTRON"),
        ami(),
    ) else {
        eprintln!(
            "skipped: set RUKOO_VOICE_TEST_NEMO_DIR, RUKOO_VOICE_TEST_NEMOTRON and \
             RUKOO_VOICE_TEST_AMI to run the AMI speaker test"
        );
        return None;
    };
    let file = ModelSpec::nemotron().files[0].name.clone();
    let model = if model.is_dir() {
        model.join(file)
    } else {
        model
    };
    assert!(model.is_file(), "missing {}", model.display());
    assert!(runtime.is_dir(), "missing {}", runtime.display());
    Some(Assets {
        runtime,
        model,
        ami,
    })
}

/// Nemotron through the real worker, for a meeting whose data folder is `data`.
fn nemotron(assets: &Assets, data: &Path, exe_dir: PathBuf) -> Labels {
    // The model where Voice's own registry finds it, and the runtime as a development copy
    // would use it.
    let registry = ModelRegistry::new(data).with_ownkey_models(None);
    let spec = ModelSpec::nemotron();
    let own = registry.own_dir(&spec);
    fs::create_dir_all(&own).unwrap();
    let target = own.join(&spec.files[0].name);
    if fs::hard_link(&assets.model, &target).is_err() {
        fs::copy(&assets.model, &target).unwrap();
    }
    std::env::set_var("RUKOO_VOICE_NEMO_DIR", &assets.runtime);
    let mut options = SpeakerOptions::new(exe_dir, registry);
    options.worker_exe = Some(speakers_exe().to_owned());
    options.dev_mode = true;
    let labels = SpeakerLabels::new(options);
    match labels.diarizer() {
        Ok(diarizer) => Labels::Local { diarizer },
        Err(e) => panic!("speaker labels can't run: {e:?}"),
    }
}

/// A program of this workspace, built into this test's target folder when it isn't there, as
/// `tests/live.rs` builds the recorder.
fn workspace_exe(package: &str, bin: &str) -> PathBuf {
    let name = format!("{bin}{}", std::env::consts::EXE_SUFFIX);
    // target/<profile>/deps/speakers_ami-<hash>.exe
    let deps = std::env::current_exe()
        .unwrap()
        .parent()
        .unwrap()
        .to_owned();
    let profile = deps.parent().unwrap().to_owned();
    let exe = profile.join(&name);
    if exe.is_file() {
        return exe;
    }
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut build = Command::new(cargo);
    build
        .current_dir(&workspace)
        .args(["build", "-p", package, "--bin", bin])
        .arg("--target-dir")
        .arg(profile.parent().unwrap());
    if profile.file_name().is_some_and(|n| n == "release") {
        build.arg("--release");
    }
    let status = build.status().expect("run cargo build");
    assert!(status.success(), "building {bin} failed");
    assert!(exe.is_file(), "no {bin} at {}", exe.display());
    exe
}

fn recorder_exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| workspace_exe("voice-recorder", "rukoo-voice-recorder"))
}

fn speakers_exe() -> &'static Path {
    static EXE: OnceLock<PathBuf> = OnceLock::new();
    EXE.get_or_init(|| workspace_exe("voice-speakers", "rukoo-voice-speakers"))
}

/// The excerpt's samples, after the canonical 44-byte header.
fn read_wav(path: &Path) -> Vec<i16> {
    fs::read(path).unwrap()[44..]
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]))
        .collect()
}

/// The RTTM's turns in ms, repeated for each loop of the excerpt from `offset` on.
fn reference(rttm: &Path, offset: u64, loops: u64) -> Vec<Span> {
    let text = fs::read_to_string(rttm).unwrap();
    let mut turns = Vec::new();
    for lap in 0..loops {
        for line in text.lines() {
            let f: Vec<&str> = line.split_whitespace().collect();
            if f.len() < 8 || f[0] != "SPEAKER" {
                continue;
            }
            let start: f64 = f[3].parse().unwrap();
            let length: f64 = f[4].parse().unwrap();
            let at = offset + lap * EXCERPT_MS;
            turns.push(Span {
                start: at + (start * 1000.0).round() as u64,
                end: at + ((start + length) * 1000.0).round() as u64,
                speaker: Some(f[7].to_owned()),
            });
        }
    }
    turns
}

/// Recognition without a model: finds the window in the excerpt by its first 64 sounding
/// samples and returns the reference words heard in it, each utterance's words spread evenly
/// over it.
struct AmiOracle {
    samples: Vec<i16>,
    /// (start s, end s, word) in excerpt time.
    words: Vec<(f64, f64, String)>,
}

impl AmiOracle {
    fn new(ami: &Ami) -> AmiOracle {
        let utterances: Value = serde_json::from_slice(&fs::read(&ami.json).unwrap()).unwrap();
        let mut words = Vec::new();
        for u in utterances.as_array().unwrap() {
            let start = u["start_time"].as_f64().unwrap().max(0.0);
            let end = u["end_time"].as_f64().unwrap().max(start);
            let list: Vec<&str> = u["words"].as_str().unwrap().split_whitespace().collect();
            let each = (end - start) / list.len().max(1) as f64;
            for (i, w) in list.iter().enumerate() {
                let at = start + each * i as f64;
                words.push((at, at + each, w.to_string()));
            }
        }
        words.sort_by(|a, b| a.0.total_cmp(&b.0));
        AmiOracle {
            samples: read_wav(&ami.wav),
            words,
        }
    }

    /// Where `pcm[0]` lies in the excerpt, in samples.
    fn locate(&self, pcm: &[i16]) -> Option<i64> {
        let first = pcm.iter().position(|&s| s.unsigned_abs() > 8)?;
        let probe = pcm.get(first..first + 64)?;
        let close = |a: i16, b: i16| (i32::from(a) - i32::from(b)).abs() <= 2;
        let found = (0..self.samples.len().saturating_sub(64)).find(|&p| {
            close(self.samples[p], probe[0])
                && self.samples[p..p + 64]
                    .iter()
                    .zip(probe)
                    .all(|(&a, &b)| close(a, b))
        })?;
        Some(found as i64 - first as i64)
    }
}

impl TimedSpeechToText for AmiOracle {
    fn transcribe_timed(&self, pcm: &[i16], _hints: &Hints) -> Result<TimedText, ProviderError> {
        let mut out = TimedText::default();
        let Some(offset) = self.locate(pcm) else {
            return Ok(out);
        };
        let from = offset as f64 / RATE as f64;
        let to = from + pcm.len() as f64 / RATE as f64;
        for (start, end, word) in &self.words {
            if *start >= from && *start < to {
                out.tokens.push(format!(" {word}"));
                out.timestamps.push(start - from);
                out.durations.push(end - start);
            }
        }
        out.text = out.tokens.concat().trim().to_owned();
        Ok(out)
    }
}

/// One meeting recorded by the real recorder from the excerpt, labelled by the real worker or,
/// without `assets`, not at all.
struct Run {
    dir: MeetingDir,
    log: EventLog,
    recorder: Option<Child>,
    pipeline: PipelineHandle,
    // Last, so the folder goes after the pipeline and the meeting let go of their files: fields
    // drop in declaration order.
    _tmp: TempDir,
}

impl Run {
    fn start(name: &str, ami: &Ami, looped: bool, assets: Option<&Assets>) -> Run {
        let tmp = TempDir::new(name);
        let data = tmp.path().join("data");
        let library = Library::open(&data).unwrap();
        let log = EventLog::open(&data).unwrap();
        let call = format!(
            "file:{}{}",
            ami.wav.display(),
            if looped { ",loop" } else { "" }
        );
        let dir = library
            .create(&CreateMeeting {
                title: "AMI".into(),
                tracks: vec![Track::Mic, Track::Call],
                sources: [
                    ("mic".to_owned(), "synthetic:idle".to_owned()),
                    ("call".to_owned(), call.clone()),
                ]
                .into(),
                screen: ScreenMode::Off,
                started_by: StartedBy::Voice,
                context: None,
            })
            .unwrap();
        let labels = match assets {
            Some(assets) => nemotron(assets, &data, tmp.path().join("exe")),
            None => Labels::Off {
                reason: "off".into(),
            },
        };

        let logs = tmp.path().join("logs");
        fs::create_dir_all(&logs).unwrap();
        let recorder = Command::new(recorder_exe())
            .arg("record")
            .arg("--meeting")
            .arg(dir.path())
            .args(["--track", "mic=synthetic:idle"])
            .args(["--track", &format!("call={call}")])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(fs::File::create(logs.join("recorder.log")).unwrap())
            .spawn()
            .expect("start rukoo-voice-recorder");
        let pipeline = Pipeline::start(
            dir.clone(),
            log.clone(),
            PipelineOptions {
                timed: Some(Arc::new(AmiOracle::new(ami))),
                timing: Timing {
                    poll: Duration::from_millis(50),
                    ..Timing::default()
                },
                labels,
                ..PipelineOptions::new(Arc::new(Offline))
            },
        );
        Run {
            _tmp: tmp,
            dir,
            log,
            recorder: Some(recorder),
            pipeline,
        }
    }

    fn timeline(&self) -> Vec<Value> {
        fs::read_to_string(self.dir.path().join("timeline.jsonl"))
            .unwrap_or_default()
            .lines()
            .filter_map(|l| serde_json::from_str(l).ok())
            .collect()
    }

    /// Waits for a heartbeat at or after meeting time `t`.
    fn wait_for(&self, t: u64) {
        wait_until("the recording to reach a time", WAIT, || {
            self.timeline()
                .iter()
                .any(|l| l["kind"] == "heartbeat" && l["t"].as_u64() >= Some(t))
                .then_some(())
        });
    }

    /// Meeting ms where the call's first segment starts: the excerpt's first sample.
    fn call_start(&self) -> u64 {
        self.timeline()
            .iter()
            .find(|l| l["kind"] == "segment" && l["track"] == "call")
            .and_then(|l| l["t"].as_u64())
            .unwrap()
    }

    fn stop(&mut self) {
        let mut child = self.recorder.take().unwrap();
        let file = wait_until("recorder.json", WAIT, || {
            read_recorder_file(self.dir.path()).filter(|f| f.pid == child.id())
        });
        let mut client = RecorderClient::connect(file.port, Duration::from_secs(5)).unwrap();
        let reply = client
            .request(&serde_json::json!({"token": file.token, "cmd": "stop"}))
            .unwrap();
        assert_eq!(reply["ok"], true, "{reply}");
        wait_until("the recorder to exit", WAIT, || child.try_wait().unwrap());
    }

    /// The call passages so far, each at its last revision.
    fn call_passages(&self) -> Vec<Span> {
        let mut last: BTreeMap<String, Span> = BTreeMap::new();
        for p in of(&events(&self.log, self.dir.id().as_str()), "passage") {
            if p["track"] == "call" {
                last.insert(
                    p["id"].as_str().unwrap().to_owned(),
                    Span {
                        start: p["start"].as_u64().unwrap(),
                        end: p["end"].as_u64().unwrap(),
                        speaker: p["speaker"].as_str().map(str::to_owned),
                    },
                );
            }
        }
        last.into_values().collect()
    }

    fn wait_done(&self) {
        wait_until("done", WAIT, || {
            let state = self.pipeline.state();
            assert!(
                !matches!(state, PipelineState::Failed { .. } | PipelineState::Stopped),
                "{state:?}"
            );
            (state == PipelineState::Done).then_some(())
        });
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if let Some(mut child) = self.recorder.take() {
            if child.try_wait().ok().flatten().is_none() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

/// Which id each main reference speaker got, from the passages in `[from, to)` ms.
fn ids_of_main(
    passages: &[Span],
    reference: &[Span],
    from: u64,
    to: u64,
) -> BTreeMap<String, String> {
    let inside: Vec<Span> = passages
        .iter()
        .filter(|p| p.start >= from && p.end <= to)
        .cloned()
        .collect();
    mapping(&inside, reference)
        .into_iter()
        .filter(|(_, name)| MAIN.contains(&name.as_str()))
        .map(|(id, name)| (name, id))
        .collect()
}

/// The harness: through the real recorder, the oracle finds the windows of the excerpt and hands
/// back their reference words once each, in order, so the label tests below score the labels
/// alone. Not every word: a window quieter than -55 dBFS isn't recognized at all, as in Voice
/// 0.1, and the excerpt has one: a soft stretch from 26.7 s to 29.2 s at -55.7 dBFS.
#[test]
fn ami_oracle_hears_the_excerpt_through_the_recorder() {
    let Some(ami) = ami() else {
        eprintln!("skipped: set RUKOO_VOICE_TEST_AMI to check the AMI oracle");
        return;
    };
    let _one = one_at_a_time();
    let expected: Vec<String> = AmiOracle::new(&ami)
        .words
        .into_iter()
        .map(|(_, _, w)| w)
        .collect();
    let mut run = Run::start("ami-oracle", &ami, false, None);
    run.wait_for(EXCERPT_MS + 2_000);
    run.stop();
    run.wait_done();
    let heard: Vec<String> = of(&events(&run.log, run.dir.id().as_str()), "passage")
        .iter()
        .filter(|p| p["track"] == "call")
        .flat_map(|p| {
            p["text"]
                .as_str()
                .unwrap()
                .split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect();
    eprintln!(
        "AMI oracle: {} of {} reference words",
        heard.len(),
        expected.len()
    );
    // In order, none twice and none made up: each heard word is the next match in the reference.
    let mut rest = expected.iter();
    for (i, word) in heard.iter().enumerate() {
        assert!(
            rest.any(|w| w == word),
            "word {i} {word:?} isn't next in the reference: heard {heard:?}"
        );
    }
    assert!(
        heard.len() * 10 >= expected.len() * 9,
        "the oracle lost words: heard {heard:?}"
    );
}

/// 1 and 2: three voices before the stop, then at least 90 % of transcript time to the right
/// speaker.
#[test]
fn ami_three_speakers_live_and_ninety_percent_right() {
    let Some(assets) = assets() else {
        return;
    };
    let _one = one_at_a_time();
    let mut run = Run::start("ami-score", &assets.ami, false, Some(&assets));
    run.wait_for(EXCERPT_MS + 4_000);
    let reference = reference(&assets.ami.rttm, run.call_start(), 1);
    let live = run.call_passages();
    let ids: HashSet<&str> = live.iter().filter_map(|p| p.speaker.as_deref()).collect();
    assert!(ids.len() >= 3, "call ids before the stop: {ids:?}");
    let main = ids_of_main(&live, &reference, 0, u64::MAX);
    assert_eq!(
        main.len(),
        3,
        "the three main speakers before the stop: {main:?}"
    );
    let distinct: HashSet<&String> = main.values().collect();
    assert_eq!(distinct.len(), 3, "{main:?}");

    run.stop();
    run.wait_done();
    let status = run.pipeline.labels();
    assert_eq!(
        (status.mode.as_str(), status.state),
        ("nemotron", LabelState::Finished)
    );
    let passages = run.call_passages();
    let score = accuracy(&passages, &reference);
    eprintln!(
        "AMI: {:.1} % of transcript time to the right speaker",
        score * 100.0
    );
    assert!(score >= 0.90, "accuracy {score}: {passages:#?}");
}

/// 3: the worker is killed 30 s into the excerpt's second loop; the main speakers keep their
/// ids. The second loop makes sure all three spoke both before and after the kill.
#[test]
fn ami_ids_survive_a_killed_worker() {
    let Some(assets) = assets() else {
        return;
    };
    let _one = one_at_a_time();
    let mut run = Run::start("ami-kill", &assets.ami, true, Some(&assets));
    let kill_at = EXCERPT_MS + 30_000;
    run.wait_for(kill_at);
    let pid = run
        .pipeline
        .labels()
        .worker_pid
        .expect("a worker runs for the meeting");
    #[cfg(windows)]
    let status = Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/F"])
        .stdout(Stdio::null())
        .status()
        .unwrap();
    #[cfg(not(windows))]
    let status = Command::new("kill")
        .args(["-9", &pid.to_string()])
        .status()
        .unwrap();
    assert!(status.success(), "killing worker {pid}");
    wait_until("labels to listen again", WAIT, || {
        let s = run.pipeline.labels();
        (s.worker_pid.is_some_and(|p| p != pid) && s.state == LabelState::Listening).then_some(())
    });
    run.wait_for(2 * EXCERPT_MS + 2_000);
    run.stop();
    run.wait_done();

    let start = run.call_start();
    let reference = reference(&assets.ami.rttm, start, 3);
    let passages = run.call_passages();
    let kill = start + kill_at;
    let before = ids_of_main(&passages, &reference, 0, kill);
    let after = ids_of_main(&passages, &reference, kill + 5_000, u64::MAX);
    eprintln!("ids before the kill {before:?}, after {after:?}");
    assert_eq!(before.len(), 3, "{before:?}");
    assert_eq!(before, after, "the main speakers kept their ids");
    let epochs = voice_meetings::Transcript::open(run.dir.path())
        .unwrap()
        .epochs()
        .unwrap();
    assert!(epochs.len() >= 2, "{epochs:?}");
}
