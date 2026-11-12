//! Meeting media for links (spec RUK-110 2.12): `media.open` prepares a file of a meeting's audio,
//! one frame or a stretch of video and names it with an id, and `media.read` hands it to Rukoo in
//! pieces of at most 1 MiB.
//!
//! `open` runs on the bridge's command thread, and a connection's commands run one at a time, so
//! it only checks and looks up and answers within milliseconds. Mixing audio, fetching media back
//! from the user's bucket and cutting a clip run as jobs on at most two `media-prepare` threads in
//! the platform's background mode. Rukoo asks again until the file is ready.
//!
//! Audio mixes live in `<data>/media/`, which is emptied at start and capped at 2 GiB; each goes
//! 10 minutes after its last read. Frames and clips are read where the screen recording keeps them
//! and are never deleted here: clips are the ones `clip.get` cuts too, and the engine's sweep
//! deletes them an hour after their last use.

use std::collections::{HashMap, VecDeque};
use std::fs::{self, File};
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_bridge::ControlError;
use voice_meetings::reader::{segment_samples, PER_MS};
use voice_meetings::{MeetingDir, TrackReader};
use voice_protocol::{ErrorCode, MediaKind, MediaOpenArgs, MAX_CLIP_MS, MAX_MEDIA_READ};
use voice_recorder::timeline::{History, SegmentRecord};
use voice_screen_files::frames::FrameIndex;
use voice_screen_files::media::{self as screen_files, MediaError, ScreenFileKind};
use voice_screen_files::paths as screen_paths;
use voice_screen_files::policy::reserve_ok;
use voice_sync::manifest::is_segment_file;
use voice_sync::{BucketSync, ScreenFile, ScreenFiles, SyncError, SyncErrorKind};

use crate::meetings::controller::Controller;

/// Prepared audio mixes: `<data>/media/<media id>.wav`.
pub const MEDIA_DIR: &str = "media";
/// A prepared file goes this long after its last read.
pub const KEEP_AFTER_READ: Duration = Duration::from_secs(10 * 60);
/// `<data>/media/` holds at most this much.
pub const MEDIA_CAP: u64 = 2 << 30;
/// Jobs that prepare at the same time; more wait.
const PREPARE_THREADS: usize = 2;
/// How long a link's cut may take: `RecordedScreen::cut` waits this long for `cut_clips`.
pub const CLIP_WAIT: Duration = Duration::from_secs(60);
/// Audio mixes are written ten seconds at a time.
const MIX_BLOCK: u64 = 10 * 1000 * PER_MS;

/// The screen recording, for media links.
pub trait ScreenMedia: Send + Sync {
    /// The frame at or before `at`: its path relative to the meeting folder and its `at`.
    fn frame_at(&self, meeting_dir: &Path, at: u64) -> io::Result<Option<(String, u64)>>;
    /// The video files a clip of [start, end) is cut from, relative paths, to fetch back first.
    fn clip_inputs(&self, meeting_dir: &Path, start: u64, end: u64) -> io::Result<Vec<String>>;
    /// An upper bound on the bytes a clip of [start, end] writes, for the free-space rule. By
    /// default the size of its input files, which are on disk by then.
    fn clip_bytes(&self, meeting_dir: &Path, start: u64, end: u64) -> io::Result<u64> {
        Ok(self
            .clip_inputs(meeting_dir, start, end)?
            .iter()
            .filter_map(|p| fs::metadata(meeting_dir.join(p)).ok())
            .map(|m| m.len())
            .sum())
    }
    /// Cuts the first stretch of continuous video in [start, end]; its path and the range it
    /// covers.
    fn cut(
        &self,
        meeting_dir: &Path,
        start: u64,
        end: u64,
    ) -> Result<(PathBuf, u64, u64), ControlError>;
    /// Whether the meeting recorded frames, and video.
    fn recorded(&self, meeting_dir: &Path) -> (bool, bool);
}

/// No screen recording: `recorded()` is `(false, false)`. For tests.
pub struct NoScreenMedia;

impl ScreenMedia for NoScreenMedia {
    fn frame_at(&self, _meeting_dir: &Path, _at: u64) -> io::Result<Option<(String, u64)>> {
        Ok(None)
    }

    fn clip_inputs(&self, _meeting_dir: &Path, _start: u64, _end: u64) -> io::Result<Vec<String>> {
        Ok(Vec::new())
    }

    fn cut(
        &self,
        _meeting_dir: &Path,
        _start: u64,
        _end: u64,
    ) -> Result<(PathBuf, u64, u64), ControlError> {
        Err(ControlError::not_available(NO_VIDEO))
    }

    fn recorded(&self, _meeting_dir: &Path) -> (bool, bool) {
        (false, false)
    }
}

const NO_MEETING: &str = "There is no meeting with that id.";
const NO_FRAMES: &str = "This meeting didn't record the screen.";
const NO_VIDEO: &str = "This meeting didn't record video.";
const NO_DISK: &str = "Not enough free disk space to prepare the media.";

/// The screen folder the screen process writes (voice/RECORDING.md, "The screen folder"), for
/// bucket sync and media links. It reads closed files only and writes nothing but the clips
/// `cut_clips` makes.
#[derive(Default)]
pub struct RecordedScreen {
    /// Each meeting folder's frame index, read again when its journal changed length.
    frames: Mutex<HashMap<PathBuf, (u64, Arc<FrameIndex>)>>,
}

impl RecordedScreen {
    fn frame_index(&self, meeting_dir: &Path) -> io::Result<Arc<FrameIndex>> {
        let length = match fs::metadata(screen_paths::journal_path(meeting_dir)) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
            Err(e) => return Err(e),
        };
        let mut frames = self.frames.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((known, index)) = frames.get(meeting_dir) {
            if *known == length {
                return Ok(Arc::clone(index));
            }
        }
        let index = Arc::new(FrameIndex::load(meeting_dir)?);
        frames.insert(meeting_dir.to_path_buf(), (length, Arc::clone(&index)));
        Ok(index)
    }
}

impl ScreenFiles for RecordedScreen {
    /// Every closed screen file: a frame once the journal names it, a video file after its
    /// `video_end`, the journal after its own `stop`. Never clips, `screen.json` or `*.tmp`.
    fn list(&self, meeting_dir: &Path, _stopped: bool) -> io::Result<Vec<ScreenFile>> {
        Ok(screen_files::closed_files(meeting_dir)?
            .into_iter()
            .map(|(path, kind)| ScreenFile {
                path,
                kind: match kind {
                    ScreenFileKind::Journal => voice_sync::ScreenFileKind::Journal,
                    ScreenFileKind::Frame => voice_sync::ScreenFileKind::Frame,
                    ScreenFileKind::Video => voice_sync::ScreenFileKind::Video,
                },
                closed: true,
            })
            .collect())
    }
}

impl ScreenMedia for RecordedScreen {
    fn frame_at(&self, meeting_dir: &Path, at: u64) -> io::Result<Option<(String, u64)>> {
        let index = self.frame_index(meeting_dir)?;
        Ok(index
            .at_or_before(at)
            .filter(|frame| {
                screen_files::capture_since(meeting_dir, at).is_some_and(|since| frame.t >= since)
            })
            .filter(|frame| screen_paths::frame_path(meeting_dir, &frame.file).is_some())
            .map(|frame| (frame.file.clone(), frame.t)))
    }

    fn clip_inputs(&self, meeting_dir: &Path, start: u64, end: u64) -> io::Result<Vec<String>> {
        screen_files::video_inputs(meeting_dir, start, end)
    }

    fn clip_bytes(&self, meeting_dir: &Path, start: u64, end: u64) -> io::Result<u64> {
        screen_files::clip_input_bytes(meeting_dir, start, end)
    }

    fn cut(
        &self,
        meeting_dir: &Path,
        start: u64,
        end: u64,
    ) -> Result<(PathBuf, u64, u64), ControlError> {
        let cut = screen_files::cut_clips(meeting_dir, start, end, Instant::now() + CLIP_WAIT)
            .map_err(|e| match e {
                MediaError::NotFound(m) => ControlError::not_found(m),
                MediaError::NotAvailable(m) => ControlError::not_available(m),
                MediaError::Failed(m) => ControlError::failed(m),
            })?;
        match cut.clips.into_iter().next() {
            Some(clip) => Ok((clip.path, clip.start, clip.end)),
            None if cut.complete => Err(ControlError::not_found("There is no video at that time.")),
            None => Err(ControlError::failed(
                "Voice couldn't cut the video in time. Try again in a moment.",
            )),
        }
    }

    fn recorded(&self, meeting_dir: &Path) -> (bool, bool) {
        screen_files::recorded(meeting_dir)
    }
}

/// What one `media.open` asks for; the same arguments find the same job.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct JobKey {
    meeting: String,
    kind: MediaKind,
    start: u64,
    /// `None` for a frame, which ignores it.
    end: Option<u64>,
}

impl JobKey {
    fn of(args: &MediaOpenArgs) -> JobKey {
        JobKey {
            meeting: args.meeting.as_str().to_owned(),
            kind: args.kind,
            start: args.start,
            end: if args.kind == MediaKind::Frame {
                None
            } else {
                args.end
            },
        }
    }
}

/// A file ready to read.
#[derive(Clone, Debug)]
struct Ready {
    path: PathBuf,
    kind: &'static str,
    size: u64,
    start: u64,
    end: u64,
    /// In `<data>/media/`: ours to delete.
    owned: bool,
    /// A clip of the screen recording's: answering `ready` touches it so the sweep keeps it for
    /// another hour.
    clip: bool,
}

enum State {
    Preparing,
    Ready(Ready),
    Failed(ControlError),
}

struct Job {
    key: JobKey,
    state: State,
    /// The last open or read.
    used: Instant,
    /// Bytes this job will put in `<data>/media/`.
    reserved: u64,
    /// At least one open delivered this completed file before a growing live range replaces it.
    answered_ready: bool,
}

/// What a `media-prepare` thread does for a job.
enum Work {
    Audio(AudioPlan),
    Frame {
        meeting: String,
        dir: PathBuf,
        path: String,
        at: u64,
    },
    Video {
        meeting: String,
        dir: PathBuf,
        start: u64,
        end: u64,
    },
}

/// An audio mix: its range, its tracks and the segment files it reads.
struct AudioPlan {
    meeting: String,
    dir: PathBuf,
    start: u64,
    end: u64,
    tracks: Vec<String>,
    segments: Vec<SegmentRecord>,
    /// The files of the segments in the range, on disk or not.
    needed: Vec<String>,
}

impl AudioPlan {
    fn size(&self) -> u64 {
        voice_audio::HEADER_LEN + (self.end - self.start) * PER_MS * 2
    }
}

#[derive(Default)]
struct Inner {
    jobs: HashMap<String, Job>,
    deleted: std::collections::HashSet<String>,
    queue: VecDeque<(String, Work)>,
    threads: usize,
    spawned: u64,
}

/// Free bytes on the volume of a path, `None` when unknown.
pub type FreeBytes = Arc<dyn Fn(&Path) -> Option<u64> + Send + Sync>;

/// `media.open` and `media.read` for the bridge.
pub struct MediaService {
    dir: PathBuf,
    controller: Arc<Controller>,
    sync: Option<Arc<BucketSync>>,
    screen: Arc<dyn ScreenMedia>,
    free_bytes: FreeBytes,
    inner: Mutex<Inner>,
    stopping: AtomicBool,
    this: std::sync::Weak<MediaService>,
}

impl MediaService {
    pub(crate) fn bucket_sync(&self) -> Option<Arc<BucketSync>> {
        self.sync.clone()
    }

    /// Revoke prepared media links before deleting their meeting. A running preparation must
    /// finish first, so a late bucket fetch cannot recreate the meeting folder afterwards.
    pub(crate) fn forget_meeting(&self, meeting: &str) -> Result<(), ControlError> {
        let mut inner = self.lock();
        if inner
            .jobs
            .values()
            .any(|job| job.key.meeting == meeting && matches!(job.state, State::Preparing))
        {
            return Err(ControlError::new(
                ErrorCode::Busy,
                "Voice is still preparing this meeting's media. Try deleting it again in a moment.",
            ));
        }
        let ids: Vec<String> = inner
            .jobs
            .iter()
            .filter(|(_, job)| job.key.meeting == meeting)
            .map(|(id, _)| id.clone())
            .collect();
        for id in ids {
            if let Some(job) = inner.jobs.remove(&id) {
                forget_file(&job);
            }
        }
        inner.deleted.insert(meeting.to_owned());
        Ok(())
    }
    /// Empties `<data>/media/`: nothing prepared before this start is still asked for.
    pub fn new(
        data_dir: &Path,
        controller: Arc<Controller>,
        sync: Option<Arc<BucketSync>>,
        screen: Arc<dyn ScreenMedia>,
        free_bytes: FreeBytes,
    ) -> Arc<MediaService> {
        let dir = data_dir.join(MEDIA_DIR);
        let _ = fs::remove_dir_all(&dir);
        Arc::new_cyclic(|this| MediaService {
            dir,
            controller,
            sync,
            screen,
            free_bytes,
            inner: Mutex::new(Inner::default()),
            stopping: AtomicBool::new(false),
            this: this.clone(),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// `{ media, ready: false }` while the file is prepared, then `{ media, ready: true, type,
    /// size, start, end }`. Answers within milliseconds: anything slow runs as a job.
    pub fn open(&self, args: &MediaOpenArgs) -> Result<Value, ControlError> {
        if self.lock().deleted.contains(args.meeting.as_str()) {
            return Err(ControlError::not_found(NO_MEETING));
        }
        self.sweep();
        let key = JobKey::of(args);
        self.rekey_grown_audio(&key, args);
        if let Some(answer) = self.known(&key)? {
            return Ok(answer);
        }
        let dir = self
            .controller
            .library()
            .dir(args.meeting.as_str())
            .ok_or_else(|| ControlError::not_found(NO_MEETING))?;
        match args.kind {
            MediaKind::Audio => {
                let end = args.end.ok_or_else(|| bad_request("Audio needs an end."))?;
                let plan = self.audio_plan(&dir, args.start, end)?;
                let size = plan.size();
                if !self.may_write(size) {
                    return Err(ControlError::failed(NO_DISK));
                }
                self.make_room(size)?;
                self.start_job(key, Work::Audio(plan), size)
            }
            MediaKind::Frame => {
                if !self.screen.recorded(dir.path()).0 {
                    return Err(ControlError::not_available(NO_FRAMES));
                }
                let (path, at) = self
                    .screen
                    .frame_at(dir.path(), args.start)
                    .map_err(|e| {
                        ControlError::failed(format!("Voice couldn't read the frames: {e}"))
                    })?
                    .ok_or_else(|| ControlError::not_found("There is no frame at that time."))?;
                let file = dir.path().join(&path);
                if let Ok(meta) = fs::metadata(&file) {
                    self.used(args.meeting.as_str(), std::slice::from_ref(&path));
                    let ready = Ready {
                        path: file,
                        kind: "image/jpeg",
                        size: meta.len(),
                        start: at,
                        end: at,
                        owned: false,
                        clip: false,
                    };
                    return self.insert_ready(key, ready);
                }
                let meeting = args.meeting.as_str().to_owned();
                if !self.can_fetch(&meeting, std::slice::from_ref(&path)) {
                    return Err(ControlError::not_available(
                        "This frame is no longer on this computer and isn't in a synced bucket.",
                    ));
                }
                let work = Work::Frame {
                    meeting,
                    dir: dir.path().to_path_buf(),
                    path,
                    at,
                };
                self.start_job(key, work, 0)
            }
            MediaKind::Video => {
                let end = args.end.ok_or_else(|| bad_request("Video needs an end."))?;
                if end.saturating_sub(args.start) > MAX_CLIP_MS {
                    return Err(bad_request(format!(
                        "Voice serves at most {MAX_CLIP_MS} ms of video at a time."
                    )));
                }
                if !self.screen.recorded(dir.path()).1 {
                    return Err(ControlError::not_available(NO_VIDEO));
                }
                let work = Work::Video {
                    meeting: args.meeting.as_str().to_owned(),
                    dir: dir.path().to_path_buf(),
                    start: args.start,
                    end,
                };
                self.start_job(key, work, 0)
            }
        }
    }

    /// An audio mix cut short at the end of what was recorded when it was made, while the meeting
    /// records on: once more is recorded, the job is keyed to the range it holds, so these
    /// arguments mix again up to the new end. The old file stays for a link still reading it, and
    /// the sweep removes it once nobody uses it.
    fn rekey_grown_audio(&self, key: &JobKey, args: &MediaOpenArgs) {
        let Some(wanted) = key.end.filter(|_| key.kind == MediaKind::Audio) else {
            return;
        };
        let clamped = self
            .lock()
            .jobs
            .iter()
            .find_map(|(id, job)| match &job.state {
                State::Ready(ready)
                    if job.answered_ready && job.key == *key && ready.end < wanted =>
                {
                    Some((id.clone(), ready.end))
                }
                _ => None,
            });
        let Some((id, held)) = clamped else {
            return;
        };
        let Some(dir) = self.controller.library().dir(args.meeting.as_str()) else {
            return;
        };
        match self.audio_plan(&dir, args.start, wanted) {
            Ok(plan) if plan.end > held => {}
            _ => return,
        }
        if let Some(job) = self.lock().jobs.get_mut(&id) {
            if job.key == *key {
                job.key.end = Some(held);
            }
        }
    }

    /// The answer for arguments Voice already knows, or `None` to start again.
    fn known(&self, key: &JobKey) -> Result<Option<Value>, ControlError> {
        let mut inner = self.lock();
        let Some(id) = inner
            .jobs
            .iter()
            .find(|(_, job)| job.key == *key)
            .map(|(id, _)| id.clone())
        else {
            return Ok(None);
        };
        let job = inner.jobs.get_mut(&id).expect("found above");
        match &job.state {
            State::Preparing => Ok(Some(json!({ "media": id, "ready": false }))),
            State::Ready(ready) if ready.path.is_file() => {
                job.used = Instant::now();
                job.answered_ready = true;
                if ready.clip {
                    let _ = screen_files::touch_clip(&ready.path);
                }
                Ok(Some(ready_answer(&id, ready)))
            }
            // The file went, for instance a clip the sweep removed: prepare it again.
            State::Ready(_) => {
                let job = inner.jobs.remove(&id).expect("found above");
                drop(inner);
                forget_file(&job);
                Ok(None)
            }
            // A failure is answered once; the next open tries again.
            State::Failed(_) => match inner.jobs.remove(&id).map(|job| job.state) {
                Some(State::Failed(error)) => Err(error),
                _ => Ok(None),
            },
        }
    }

    /// Checks an audio range and finds what it needs, without reading any audio.
    fn audio_plan(
        &self,
        dir: &MeetingDir,
        start: u64,
        end: u64,
    ) -> Result<AudioPlan, ControlError> {
        let meeting = dir
            .meeting()
            .map_err(|_| ControlError::not_found(NO_MEETING))?;
        let tracks: Vec<String> = meeting
            .tracks
            .iter()
            .map(|t| t.as_str().to_owned())
            .collect();
        // Only segments that name their own file in this folder: a timeline restored from the
        // bucket could otherwise point a link at any file on this computer.
        let segments: Vec<SegmentRecord> = History::from_lines(&dir.timeline())
            .segments
            .into_iter()
            .filter(|s| tracks.contains(&s.track) && is_segment_file(&s.track, s.seq, &s.file))
            .collect();
        let path = dir.path();
        let samples = |s: &SegmentRecord| segment_length(path, s);
        let recorded_end = segments
            .iter()
            .map(|s| s.t * PER_MS + samples(s))
            .max()
            .unwrap_or(0)
            .div_ceil(PER_MS);
        if start >= recorded_end {
            return Err(ControlError::not_found(
                "Nothing was recorded at that time.",
            ));
        }
        let end = end.min(recorded_end);
        let needed: Vec<String> = segments
            .iter()
            .filter(|s| {
                let from = s.t;
                let to = (s.t * PER_MS + samples(s)).div_ceil(PER_MS);
                from < end && to > start
            })
            .map(|s| s.file.clone())
            .collect();
        let id = dir.id().as_str().to_owned();
        let fetch = missing(path, &needed);
        if !fetch.is_empty() && !self.can_fetch(&id, &fetch) {
            return Err(ControlError::not_available(
                "The audio of this meeting is no longer on this computer and isn't in a synced bucket.",
            ));
        }
        Ok(AudioPlan {
            meeting: id,
            dir: path.to_path_buf(),
            start,
            end,
            tracks,
            segments,
            needed,
        })
    }

    /// Restored media a link uses from disk stays at least 24 hours from now, as after its fetch
    /// (D8). Without that, retention could remove it a day after the fetch while links still use
    /// it, and a mix in progress would lose its audio.
    fn used(&self, meeting: &str, paths: &[String]) {
        if let Some(sync) = &self.sync {
            sync.used(meeting, paths);
        }
    }

    /// Whether bucket sync can bring these files of the meeting back.
    fn can_fetch(&self, meeting: &str, paths: &[String]) -> bool {
        self.sync.as_ref().is_some_and(|sync| {
            sync.can_fetch() && paths.iter().all(|p| sync.in_bucket(meeting, p))
        })
    }

    fn meeting_live(&self) -> bool {
        self.controller.current().is_some()
    }

    /// The free-space rule for every write here; an unknown amount of free space doesn't stop it.
    fn may_write(&self, bytes: u64) -> bool {
        (self.free_bytes)(&self.dir).is_none_or(|free| reserve_ok(free, bytes, self.meeting_live()))
    }

    /// Keeps `<data>/media/` under its cap: forgets the mixes used longest ago until `size` fits.
    fn make_room(&self, size: u64) -> Result<(), ControlError> {
        let mut gone = Vec::new();
        {
            let mut inner = self.lock();
            loop {
                let used: u64 = inner.jobs.values().map(|j| j.reserved).sum();
                if used + size <= MEDIA_CAP {
                    break;
                }
                let oldest = inner
                    .jobs
                    .iter()
                    .filter(|(_, j)| j.reserved > 0 && !matches!(j.state, State::Preparing))
                    .min_by_key(|(_, j)| j.used)
                    .map(|(id, _)| id.clone());
                match oldest {
                    Some(id) => gone.extend(inner.jobs.remove(&id)),
                    None => return Err(ControlError::failed(
                        "Voice is preparing too much media at once. Try again in a few minutes.",
                    )),
                }
            }
        }
        for job in &gone {
            forget_file(job);
        }
        Ok(())
    }

    fn insert_ready(&self, key: JobKey, ready: Ready) -> Result<Value, ControlError> {
        let id = new_id()?;
        let answer = ready_answer(&id, &ready);
        let mut inner = self.lock();
        if inner.deleted.contains(&key.meeting) {
            return Err(ControlError::not_found(NO_MEETING));
        }
        inner.jobs.insert(
            id,
            Job {
                key,
                state: State::Ready(ready),
                used: Instant::now(),
                reserved: 0,
                answered_ready: true,
            },
        );
        Ok(answer)
    }

    /// Queues a job and answers `ready: false`.
    fn start_job(&self, key: JobKey, work: Work, reserved: u64) -> Result<Value, ControlError> {
        let id = new_id()?;
        let spawn = {
            let mut inner = self.lock();
            if inner.deleted.contains(&key.meeting) {
                return Err(ControlError::not_found(NO_MEETING));
            }
            // Another open of the same arguments got here first.
            if let Some(other) = inner
                .jobs
                .iter()
                .find(|(_, j)| j.key == key && matches!(j.state, State::Preparing))
                .map(|(other, _)| other.clone())
            {
                return Ok(json!({ "media": other, "ready": false }));
            }
            inner.jobs.insert(
                id.clone(),
                Job {
                    key,
                    state: State::Preparing,
                    answered_ready: false,
                    used: Instant::now(),
                    reserved,
                },
            );
            inner.queue.push_back((id.clone(), work));
            if inner.threads < PREPARE_THREADS {
                inner.threads += 1;
                inner.spawned += 1;
                Some(inner.spawned)
            } else {
                None
            }
        };
        if let Some(n) = spawn {
            let this = self.this.clone();
            let started = thread::Builder::new()
                .name(format!("media-prepare-{n}"))
                .spawn(move || prepare_loop(this));
            if started.is_err() {
                let mut inner = self.lock();
                inner.threads -= 1;
                inner.jobs.remove(&id);
                inner.queue.retain(|(queued, _)| *queued != id);
                return Err(ControlError::failed(
                    "Voice couldn't start preparing the media.",
                ));
            }
        }
        Ok(json!({ "media": id, "ready": false }))
    }

    fn run(&self, id: &str, work: Work) -> Result<Ready, ControlError> {
        match work {
            Work::Audio(plan) => self.mix(id, &plan),
            Work::Frame {
                meeting,
                dir,
                path,
                at,
            } => {
                self.fetch(&meeting, std::slice::from_ref(&path))?;
                let file = dir.join(&path);
                let size = fs::metadata(&file)
                    .map_err(|e| ControlError::failed(format!("The frame didn't come back: {e}")))?
                    .len();
                Ok(Ready {
                    path: file,
                    kind: "image/jpeg",
                    size,
                    start: at,
                    end: at,
                    owned: false,
                    clip: false,
                })
            }
            Work::Video {
                meeting,
                dir,
                start,
                end,
            } => {
                let inputs = self.screen.clip_inputs(&dir, start, end).map_err(|e| {
                    ControlError::failed(format!("Voice couldn't read the video: {e}"))
                })?;
                if inputs.is_empty() {
                    return Err(ControlError::not_found("There is no video at that time."));
                }
                self.used(&meeting, &inputs);
                let absent = missing(&dir, &inputs);
                if !absent.is_empty() {
                    if !self.can_fetch(&meeting, &absent) {
                        return Err(ControlError::not_available(
                            "This video is no longer on this computer and isn't in a synced bucket.",
                        ));
                    }
                    self.fetch(&meeting, &absent)?;
                }
                // The clip is at most as large as the fragments it is cut from.
                let input_bytes = self.screen.clip_bytes(&dir, start, end).map_err(|e| {
                    ControlError::failed(format!("Voice couldn't read the video: {e}"))
                })?;
                if !self.may_write(input_bytes) {
                    return Err(ControlError::failed(NO_DISK));
                }
                let (path, start, end) = self.screen.cut(&dir, start, end)?;
                let size = fs::metadata(&path)
                    .map_err(|e| ControlError::failed(format!("The clip is missing: {e}")))?
                    .len();
                let _ = screen_files::touch_clip(&path);
                Ok(Ready {
                    path,
                    kind: "video/mp4",
                    size,
                    start,
                    end,
                    owned: false,
                    clip: true,
                })
            }
        }
    }

    /// Brings files of a meeting back from the bucket.
    fn fetch(&self, meeting: &str, paths: &[String]) -> Result<(), ControlError> {
        let Some(sync) = &self.sync else {
            return Err(ControlError::not_available(
                "This media is no longer on this computer and bucket sync is off.",
            ));
        };
        sync.ensure_local(meeting, paths, &self.stopping)
            .map_err(fetch_error)
    }

    /// Writes the mix of every track as a WAV file in `<data>/media/`.
    fn mix(&self, id: &str, plan: &AudioPlan) -> Result<Ready, ControlError> {
        // Held first, then looked at: retention may have removed a segment since `open`.
        self.used(&plan.meeting, &plan.needed);
        let fetch = missing(&plan.dir, &plan.needed);
        if !fetch.is_empty() {
            self.fetch(&plan.meeting, &fetch)?;
        }
        fs::create_dir_all(&self.dir).map_err(write_error)?;
        let part = self.dir.join(format!("{id}.wav.part"));
        let path = self.dir.join(format!("{id}.wav"));
        let result = self.write_mix(&part, plan);
        if let Err(e) = result {
            let _ = fs::remove_file(&part);
            return Err(e);
        }
        fs::rename(&part, &path).map_err(write_error)?;
        Ok(Ready {
            path,
            kind: "audio/wav",
            size: plan.size(),
            start: plan.start,
            end: plan.end,
            owned: true,
            clip: false,
        })
    }

    fn write_mix(&self, part: &Path, plan: &AudioPlan) -> Result<(), ControlError> {
        let dir = &plan.dir;
        let readers: Vec<TrackReader> = plan
            .tracks
            .iter()
            .map(|track| {
                TrackReader::with_lengths(dir, track, &plan.segments, |s| segment_length(dir, s))
            })
            .collect();
        let (from, to) = (plan.start * PER_MS, plan.end * PER_MS);
        let mut out = BufWriter::new(File::create(part).map_err(write_error)?);
        out.write_all(&voice_audio::segment_header(to - from))
            .map_err(write_error)?;
        let mut at = from;
        while at < to {
            if self.stopping.load(Ordering::SeqCst) {
                return Err(ControlError::failed("Voice is quitting."));
            }
            let next = (at + MIX_BLOCK).min(to);
            let mut sum = vec![0i32; (next - at) as usize];
            for reader in &readers {
                let samples = reader.read(at, next).map_err(|e| {
                    ControlError::failed(format!("Voice couldn't read the meeting's audio: {e}"))
                })?;
                for (total, sample) in sum.iter_mut().zip(samples) {
                    *total += i32::from(sample);
                }
            }
            out.write_all(&mix_bytes(&sum)).map_err(write_error)?;
            at = next;
        }
        let file = out.into_inner().map_err(|e| write_error(e.into_error()))?;
        file.sync_all().map_err(write_error)
    }

    /// Up to `length` bytes of a prepared file from `offset`, as `{ offset, data }` in standard
    /// base64; `data` is empty past the end.
    pub fn read(&self, media: &str, offset: u64, length: u64) -> Result<Value, ControlError> {
        let path = {
            let mut inner = self.lock();
            let Some(job) = inner.jobs.get_mut(media) else {
                return Err(ControlError::not_found("There is no media with that id."));
            };
            match &job.state {
                State::Preparing => return Err(bad_request("The media isn't ready yet.")),
                State::Failed(error) => return Err(error.clone()),
                State::Ready(ready) => {
                    job.used = Instant::now();
                    ready.path.clone()
                }
            }
        };
        let length = length.min(MAX_MEDIA_READ);
        let bytes = match read_at(&path, offset, length) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                // Gone from disk: Rukoo opens it again, which prepares it again.
                let job = self.lock().jobs.remove(media);
                if let Some(job) = job {
                    forget_file(&job);
                }
                return Err(ControlError::not_found(
                    "The prepared media is gone. Open it again.",
                ));
            }
            Err(e) => {
                return Err(ControlError::failed(format!(
                    "Voice couldn't read the prepared media: {e}"
                )))
            }
        };
        Ok(voice_protocol::media_read_result(offset, &bytes))
    }

    /// Forgets files 10 minutes after their last use and deletes the ones in `<data>/media/`.
    /// Runs on every open and once a minute from the engine.
    pub fn sweep(&self) {
        let gone: Vec<Job> = {
            let mut inner = self.lock();
            let expired: Vec<String> = inner
                .jobs
                .iter()
                .filter(|(_, j)| {
                    !matches!(j.state, State::Preparing) && j.used.elapsed() >= KEEP_AFTER_READ
                })
                .map(|(id, _)| id.clone())
                .collect();
            expired
                .iter()
                .filter_map(|id| inner.jobs.remove(id))
                .collect()
        };
        for job in &gone {
            forget_file(job);
        }
    }

    /// Stops the jobs that run: a fetch or a mix ends at its next step.
    pub fn shutdown(&self) {
        self.stopping.store(true, Ordering::SeqCst);
        let mut inner = self.lock();
        let queued: Vec<String> = inner.queue.drain(..).map(|(id, _)| id).collect();
        for id in queued {
            inner.jobs.remove(&id);
        }
    }

    /// Jobs known now, for tests: (preparing, ready).
    #[doc(hidden)]
    pub fn counts(&self) -> (usize, usize) {
        let inner = self.lock();
        let preparing = inner
            .jobs
            .values()
            .filter(|j| matches!(j.state, State::Preparing))
            .count();
        let ready = inner
            .jobs
            .values()
            .filter(|j| matches!(j.state, State::Ready(_)))
            .count();
        (preparing, ready)
    }
}

/// A `media-prepare` thread: runs queued jobs until none is left.
fn prepare_loop(service: std::sync::Weak<MediaService>) {
    crate::platform::background_thread();
    loop {
        let Some(this) = service.upgrade() else {
            return;
        };
        let next = {
            let mut inner = this.lock();
            let next = inner.queue.pop_front();
            if next.is_none() {
                inner.threads -= 1;
            }
            next
        };
        let Some((id, work)) = next else {
            return;
        };
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| this.run(&id, work)))
            .unwrap_or_else(|_| {
                log::error!("media: preparing {id} panicked");
                Err(ControlError::failed("Voice couldn't prepare the media."))
            });
        if let Err(error) = &result {
            log::info!("media: {id} failed: {}", error.message);
        }
        let mut inner = this.lock();
        match inner.jobs.get_mut(&id) {
            Some(job) => {
                job.used = Instant::now();
                job.state = match result {
                    Ok(ready) => State::Ready(ready),
                    Err(error) => State::Failed(error),
                };
            }
            // Forgotten while it prepared, at a shutdown: its file goes too.
            None => {
                if let Ok(Ready {
                    path, owned: true, ..
                }) = result
                {
                    let _ = fs::remove_file(path);
                }
            }
        }
    }
}

fn ready_answer(id: &str, ready: &Ready) -> Value {
    json!({
        "media": id,
        "ready": true,
        "type": ready.kind,
        "size": ready.size,
        "start": ready.start,
        "end": ready.end,
    })
}

fn new_id() -> Result<String, ControlError> {
    voice_protocol::new_media_id()
        .map_err(|e| ControlError::failed(format!("Voice couldn't name the media: {e}")))
}

fn bad_request(message: impl Into<String>) -> ControlError {
    ControlError::new(ErrorCode::BadRequest, message)
}

fn write_error(error: io::Error) -> ControlError {
    ControlError::failed(format!("Voice couldn't write the media: {error}"))
}

/// A failed fetch from the bucket, in the bridge's codes.
fn fetch_error(error: SyncError) -> ControlError {
    match error.kind {
        SyncErrorKind::Off | SyncErrorKind::NotInBucket | SyncErrorKind::NotFound => {
            ControlError::not_available(
                "This media is no longer on this computer and isn't in a synced bucket.",
            )
        }
        SyncErrorKind::Local => ControlError::failed(NO_DISK),
        _ => ControlError::failed(format!(
            "Voice couldn't fetch the media from the bucket: {}",
            error.message
        )),
    }
}

/// Deletes a forgotten job's file when it is ours.
fn forget_file(job: &Job) {
    if let State::Ready(Ready {
        path, owned: true, ..
    }) = &job.state
    {
        let _ = fs::remove_file(path);
    }
}

/// The files among `paths` (relative to `dir`) that aren't on disk.
fn missing(dir: &Path, paths: &[String]) -> Vec<String> {
    paths
        .iter()
        .filter(|p| !dir.join(p).is_file())
        .cloned()
        .collect()
}

/// The samples of a segment: its `segment_end` count when it closed, so a segment the retention
/// removed still has its length, else what its header counts now.
fn segment_length(dir: &Path, s: &SegmentRecord) -> u64 {
    s.ended
        .unwrap_or_else(|| segment_samples(&dir.join(&s.file), None))
}

/// Summed samples as 16-bit little-endian PCM, clipped instead of wrapping.
fn mix_bytes(sum: &[i32]) -> Vec<u8> {
    sum.iter()
        .flat_map(|&s| (s.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16).to_le_bytes())
        .collect()
}

fn read_at(path: &Path, offset: u64, length: u64) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    if offset >= size {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut bytes = Vec::with_capacity(length.min(size - offset) as usize);
    file.take(length).read_to_end(&mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mix_clips_instead_of_wrapping() {
        assert_eq!(
            mix_bytes(&[0, 1, -1, 40_000, -40_000]),
            [0, 0, 1, 0, 0xff, 0xff, 0xff, 0x7f, 0x00, 0x80]
        );
    }

    #[test]
    fn reads_stop_at_the_end_of_the_file() {
        let path = std::env::temp_dir().join(format!("voice-media-read-{}", std::process::id()));
        fs::write(&path, b"0123456789").unwrap();
        assert_eq!(read_at(&path, 2, 3).unwrap(), b"234");
        assert_eq!(read_at(&path, 8, 1 << 20).unwrap(), b"89");
        assert!(read_at(&path, 10, 5).unwrap().is_empty());
        assert!(read_at(&path, 50, 5).unwrap().is_empty());
        fs::remove_file(&path).unwrap();
        assert_eq!(
            read_at(&path, 0, 1).unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[test]
    fn a_frame_ignores_the_end_and_other_kinds_keep_it() {
        let meeting = voice_protocol::MeetingId::parse("m-20261030-171203-4f2a").unwrap();
        let args = |kind, end| MediaOpenArgs {
            meeting: meeting.clone(),
            kind,
            start: 1000,
            end,
        };
        assert_eq!(
            JobKey::of(&args(MediaKind::Frame, Some(5))),
            JobKey::of(&args(MediaKind::Frame, None))
        );
        assert_ne!(
            JobKey::of(&args(MediaKind::Audio, Some(5000))),
            JobKey::of(&args(MediaKind::Audio, Some(6000)))
        );
    }
}
