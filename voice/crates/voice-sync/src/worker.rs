//! The `sync-worker` thread (spec 2.8, 2.9): scans, the queue's next row, one request at a time,
//! the live lane's token bucket, backoff, manifests and the index. Every loop catches panics, logs
//! them and starts again after 5 s.
//!
//! The worker only reads closed files and never talks to the recorder, so nothing it does can cost
//! audio. It runs in the platform's background mode (D20).

use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};
use voice_meetings::{transcript, MeetingDir};
use voice_protocol::MeetingState;
use voice_recorder::timeline::History;
use zeroize::Zeroizing;

use crate::format::{cipher_len, Header, Sealer};
use crate::keys::{KeyStore, MasterKey};
use crate::manifest::{manifest_for, Index, IndexEntry, Manifest};
use crate::names;
use crate::queue::{hex, MeetingRow, NewRow, Progress, Queue, Row, MEDIA_KINDS};
use crate::s3::Bucket;
use crate::scan::{self, Found};
use crate::sync::{LastError, RestoreState, SyncOptions};
use crate::{BucketRetention, SyncConfig, SyncError, SyncErrorKind};

/// Files under this size are hashed again before each upload attempt (2.9).
const REHASH_MAX: u64 = 64 << 20;
/// A panicked loop starts again after this long.
const PANIC_PAUSE: Duration = Duration::from_secs(5);
/// `staging/` holds at most this much: no new transcript snapshot while it holds more (2.3).
const STAGING_MAX: u64 = 512 << 20;
/// Settings learns of changes at most this often.
const NOTIFY_EVERY: Duration = Duration::from_millis(500);
/// The worker looks at its clocks at least this often.
const TICK: Duration = Duration::from_secs(1);
/// A failed bucket retention run tries again after this long.
const PRUNE_RETRY: Duration = Duration::from_secs(15 * 60);
const DAY_MS: u64 = 86_400_000;
/// The staging folder in `<data>/sync`.
pub(crate) const STAGING_DIR: &str = "staging";

/// What the facade, the worker, restores and fetches share.
pub(crate) struct Shared {
    pub(crate) opts: SyncOptions,
    pub(crate) sync_dir: PathBuf,
    pub(crate) staging: PathBuf,
    pub(crate) queue: Queue,
    pub(crate) keys: KeyStore,
    pub(crate) state: Mutex<State>,
    pub(crate) wake: Condvar,
    /// Held while the worker makes a request or changes rows, so `forget_meeting` and a key import
    /// run between two steps.
    pub(crate) busy: Mutex<()>,
    /// Meeting files `ensure_local` is fetching now. Two `media-prepare` threads can ask for the
    /// same segment; the second waits for the first instead of writing the same file with it.
    pub(crate) fetching: Mutex<HashSet<PathBuf>>,
    /// Signalled when a fetch ends.
    pub(crate) fetched: Condvar,
    /// Held while the key is read and made or replaced: the worker and `show_key` can both find no
    /// key at once, and `KeyStore::create` checks for the file before it writes it.
    pub(crate) key_lock: Mutex<()>,
    /// Counts saves and removals of the bucket secret, so a Test connection can tell whether the
    /// secret it read is still the saved one when its result is stored.
    pub(crate) secret_generation: AtomicU64,
    notifier: Notifier,
}

/// The facade's and the worker's shared state.
#[derive(Default)]
pub(crate) struct State {
    pub(crate) cfg: Option<SyncConfig>,
    /// Counts changes of the configuration, the secret and the key: the worker reloads them, and
    /// errors that stopped uploads are forgotten.
    pub(crate) generation: u64,
    pub(crate) nudged: bool,
    /// Scan every meeting at the next chance.
    pub(crate) rescan: bool,
    pub(crate) stopping: bool,
    /// Counts wake-ups, so the worker never sleeps through one that came while it worked.
    pub(crate) wakes: u64,
    pub(crate) worker_exited: bool,
    pub(crate) restoring: bool,
    pub(crate) restore: RestoreState,
    /// An error that stops all uploads until the generation changes or its time is up.
    pub(crate) blocked: Option<Blocked>,
    /// Transient failures in a row.
    pub(crate) failures: u32,
    /// The last failure, `None` after a success.
    pub(crate) last_error: Option<LastError>,
    /// The generation whose remote meetings were counted for `restore.available`.
    pub(crate) listed: Option<u64>,
    /// The place the queue's rows describe ([`SyncConfig::target`]), as `queue.db` has it.
    pub(crate) target: Option<String>,
    /// Counts changes of where Voice reads from and with which key: sync turned on or off, moved
    /// to another bucket, endpoint or folder, or got another key. A restore made before reads
    /// from a place or with a key sync no longer uses ([`Binding`]).
    pub(crate) place: u64,
}

impl State {
    /// Sync is on for another bucket, endpoint or folder than the rows describe, and the worker
    /// hasn't started the queue again for it yet: no row says anything about the new place.
    pub(crate) fn moving(&self) -> bool {
        self.cfg
            .as_ref()
            .is_some_and(|cfg| self.target.as_deref() != Some(cfg.target().as_str()))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Blocked {
    pub(crate) error: SyncError,
    /// On the monotonic clock: a wall clock set back after a `clock` error must not stretch the
    /// wait by the size of the error.
    pub(crate) until: Instant,
    pub(crate) generation: u64,
}

/// Calls `on_change` at most twice a second; a change in between is passed on later.
struct Notifier {
    inner: Mutex<(Option<Instant>, bool)>,
}

impl Shared {
    pub(crate) fn new(
        opts: SyncOptions,
        sync_dir: PathBuf,
        queue: Queue,
        keys: KeyStore,
    ) -> Shared {
        let staging = sync_dir.join(STAGING_DIR);
        let state = State {
            target: queue.meta("target").ok().flatten(),
            ..State::default()
        };
        Shared {
            opts,
            sync_dir,
            staging,
            queue,
            keys,
            state: Mutex::new(state),
            wake: Condvar::new(),
            busy: Mutex::new(()),
            fetching: Mutex::new(HashSet::new()),
            fetched: Condvar::new(),
            key_lock: Mutex::new(()),
            secret_generation: AtomicU64::new(0),
            notifier: Notifier {
                inner: Mutex::new((None, false)),
            },
        }
    }

    pub(crate) fn now(&self) -> u64 {
        (self.opts.now)()
    }

    pub(crate) fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn busy(&self) -> MutexGuard<'_, ()> {
        self.busy.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn key_lock(&self) -> MutexGuard<'_, ()> {
        self.key_lock.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// This computer's key, made now when it has none; true when it was made. Only one caller
    /// makes it, so the key on disk is the key every caller got.
    pub(crate) fn key_or_create(&self, now: u64) -> Result<(MasterKey, bool), SyncError> {
        let _key = self.key_lock();
        match self.keys.load()? {
            Some(key) => Ok((key, false)),
            None => Ok((self.keys.create(now)?, true)),
        }
    }

    pub(crate) fn cfg(&self) -> Option<SyncConfig> {
        self.lock().cfg.clone()
    }

    pub(crate) fn stopping(&self) -> bool {
        self.lock().stopping
    }

    /// The configuration, the secret or the key changed.
    pub(crate) fn bump(&self) {
        let mut st = self.lock();
        st.generation += 1;
        st.blocked = None;
        st.failures = 0;
        st.listed = None;
        drop(st);
        self.poke();
    }

    /// Wakes the worker.
    pub(crate) fn poke(&self) {
        self.lock().wakes += 1;
        self.wake.notify_all();
    }

    /// Settings' state changed. Never call this while holding `state`.
    pub(crate) fn changed(&self) {
        let call = {
            let mut inner = self
                .notifier
                .inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if inner.0.is_none_or(|at| at.elapsed() >= NOTIFY_EVERY) {
                *inner = (Some(Instant::now()), false);
                true
            } else {
                inner.1 = true;
                false
            }
        };
        if call {
            (self.opts.on_change)();
        } else {
            self.poke();
        }
    }

    /// Passes on a change that came too soon; how long until it may go, when one waits.
    fn flush(&self) -> Option<Duration> {
        let call = {
            let mut inner = self
                .notifier
                .inner
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            if !inner.1 {
                return None;
            }
            let since = inner.0.map_or(NOTIFY_EVERY, |at| at.elapsed());
            if since < NOTIFY_EVERY {
                return Some(NOTIFY_EVERY - since);
            }
            *inner = (Some(Instant::now()), false);
            true
        };
        if call {
            (self.opts.on_change)();
        }
        None
    }

    /// The wake-up count, read before a round of work.
    pub(crate) fn wakes(&self) -> u64 {
        self.lock().wakes
    }

    /// Sleeps up to `d`, unless a wake-up came since `seen`. True when stopping.
    pub(crate) fn sleep(&self, d: Duration, seen: u64) -> bool {
        let st = self.lock();
        if st.stopping || st.wakes != seen {
            return st.stopping;
        }
        let (st, _) = self
            .wake
            .wait_timeout_while(st, d, |st| !st.stopping && st.wakes == seen)
            .unwrap_or_else(PoisonError::into_inner);
        st.stopping
    }

    /// Records a failure for Settings: the last error, and since when uploads fail.
    pub(crate) fn note_failure(&self, error: &SyncError) {
        let now = self.now();
        if let Err(e) = self.queue.note_error(now, error) {
            log::warn!("sync: can't note an error: {e}");
        }
        // A start in the future came from a clock that ran ahead; it would hide the daily
        // "sync is failing" notification until the clock caught up.
        let since = self.queue.meta_u64("failing_since").ok().flatten();
        if since.is_none_or(|since| since > now) {
            let _ = self.queue.set_meta("failing_since", Some(&now.to_string()));
        }
        self.lock().last_error = Some(LastError {
            at: now,
            code: error.code().to_owned(),
            message: error.message.clone(),
        });
    }

    /// A request succeeded: nothing fails any more.
    fn note_success(&self) {
        let mut st = self.lock();
        let changed = st.failures > 0 || st.last_error.is_some() || st.blocked.is_some();
        st.failures = 0;
        st.last_error = None;
        st.blocked = None;
        drop(st);
        let _ = self.queue.set_meta("failing_since", None);
        if changed {
            self.changed();
        }
    }

    /// The bucket, the key and its derived name key, for restores and fetches, and the binding
    /// that tells whether they are still the ones sync uses.
    pub(crate) fn remote(&self) -> Result<(Remote, Binding), SyncError> {
        // Read together, before the key: any change from here on moves `place` on.
        let (cfg, place) = {
            let st = self.lock();
            (st.cfg.clone(), st.place)
        };
        let cfg = cfg.ok_or_else(|| SyncError::new(SyncErrorKind::Off, "Bucket sync is off."))?;
        let key = self.keys.load()?.ok_or_else(|| {
            SyncError::new(SyncErrorKind::Key, "Voice has no encryption key yet.")
        })?;
        let secret = self.keys.secret()?.ok_or_else(|| {
            SyncError::new(SyncErrorKind::NoSecret, "Save the secret access key first.")
        })?;
        let timing = &self.opts.timing;
        let bucket = Bucket::new(
            &cfg,
            &secret,
            timing.connect_timeout,
            timing.request_timeout,
        )?;
        let remote = Remote::new(bucket, key);
        let binding = Binding {
            place,
            target: cfg.target(),
            kid: remote.kid.clone(),
        };
        Ok((remote, binding))
    }

    /// Err once Voice quits, or once `binding` no longer names the bucket and key sync uses: sync
    /// was turned off, moved to another bucket, endpoint or folder, or got another key (D10).
    pub(crate) fn still(&self, binding: &Binding) -> Result<(), SyncError> {
        let st = self.lock();
        if st.stopping {
            return Err(SyncError::new(SyncErrorKind::Local, "Voice is quitting."));
        }
        let Some(cfg) = &st.cfg else {
            return Err(SyncError::new(SyncErrorKind::Off, "Bucket sync is off."));
        };
        if st.place == binding.place {
            return Ok(());
        }
        let message = if cfg.target() == binding.target {
            "Bucket sync was turned off, or the encryption key changed, while Voice read from the bucket."
        } else {
            "Bucket sync moved to another bucket, endpoint or folder."
        };
        Err(SyncError::new(SyncErrorKind::Off, message))
    }

    /// The meeting's folder in the library.
    pub(crate) fn meeting_dir(&self, meeting: &str) -> PathBuf {
        self.opts.meetings_dir.join(meeting)
    }
}

/// What a restore or a fetch reads with: the bucket sync was on for and the key, when it started.
#[derive(Clone, Debug)]
pub(crate) struct Binding {
    /// [`State::place`] then.
    place: u64,
    /// [`SyncConfig::target`] of that bucket, endpoint and folder.
    pub(crate) target: String,
    pub(crate) kid: String,
}

/// A connection to the bucket with the key that opens it.
pub(crate) struct Remote {
    pub(crate) bucket: Bucket,
    pub(crate) key: MasterKey,
    pub(crate) kid: String,
    pub(crate) name_key: Zeroizing<[u8; 32]>,
}

impl Remote {
    pub(crate) fn new(bucket: Bucket, key: MasterKey) -> Remote {
        let kid = key.kid();
        let name_key = Zeroizing::new(key.name_key());
        Remote {
            bucket,
            key,
            kid,
            name_key,
        }
    }
}

/// Starts the `sync-worker` thread.
pub(crate) fn spawn(shared: Arc<Shared>) -> io::Result<()> {
    thread::Builder::new()
        .name("sync-worker".into())
        .spawn(move || {
            background_thread();
            loop {
                let run = catch_unwind(AssertUnwindSafe(|| Worker::new(shared.clone()).run()));
                match run {
                    Ok(()) => break,
                    Err(panic) => {
                        log::error!("sync worker panicked: {}", panic_text(&panic));
                        if shared.sleep(PANIC_PAUSE, shared.wakes()) {
                            break;
                        }
                    }
                }
            }
            shared.lock().worker_exited = true;
            shared.poke();
        })
        .map(drop)
}

pub(crate) fn panic_text(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = panic.downcast_ref::<&str>() {
        (*s).to_owned()
    } else if let Some(s) = panic.downcast_ref::<String>() {
        s.clone()
    } else {
        "unknown panic".to_owned()
    }
}

/// Lets this thread's disk and CPU yield to everything else (D20). Does nothing off Windows and
/// macOS.
pub(crate) fn background_thread() {
    #[cfg(windows)]
    {
        use windows::Win32::System::Threading::{
            GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN,
        };
        // SAFETY: GetCurrentThread returns a pseudo handle for this thread, valid for the call.
        let set = unsafe { SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN) };
        if let Err(e) = set {
            log::debug!("sync: background mode refused: {e}");
        }
    }
    #[cfg(target_os = "macos")]
    {
        // UTILITY, not BACKGROUND: BACKGROUND throttles disk access as well, and an upload that
        // waits for a stalled read holds the bucket's connection open.
        // SAFETY: the call changes the quality of service of the calling thread and nothing else.
        let result =
            unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0) };
        if result != 0 {
            log::debug!("sync: the system refused a lower quality of service: {result}");
        }
    }
}

/// What one step did.
#[derive(Debug, PartialEq)]
enum Step {
    /// Made a request or changed a row: go on at once.
    Did,
    /// Nothing to do for this long.
    Idle(Duration),
}

/// Why the worker can't upload now.
enum Ready {
    Yes,
    No(Duration),
}

/// The worker's own state for one run of its loop. A panic throws it away; the queue keeps
/// everything that matters.
struct Worker {
    shared: Arc<Shared>,
    /// The generation the key, the secret and the bucket below belong to.
    generation: Option<u64>,
    cfg: Option<SyncConfig>,
    remote: Option<Remote>,
    /// The last connection before this one, until the next connect: when it was to another
    /// bucket, the uploads left open there are cancelled with it.
    previous: Option<Remote>,
    scanned_live: Option<Instant>,
    scanned_all: Option<Instant>,
    /// The live lane's token bucket in bytes, never above 0: a request may put it into debt by
    /// one object, and the next waits until that is paid (2.9).
    tokens: f64,
    refilled: Instant,
    /// Multipart rows whose parts this run checked with ListParts.
    listed: HashSet<i64>,
    /// After three failures in a row the whole worker waits until then. The waits of this run
    /// are on the monotonic clock, so setting the wall clock back never lengthens them.
    wait_until: Option<Instant>,
    /// When bucket retention may run again after a failure.
    prune_retry: Option<Instant>,
    /// Counter for staged file names.
    staged: AtomicU64,
    /// Meetings whose last scan found a file it couldn't queue, such as a segment another program
    /// held open. They don't count as synced, so retention keeps holding their media. Every run
    /// starts with a scan of every meeting, which fills this before any manifest goes up.
    unread: HashSet<String>,
}

/// What became of one file a scan found.
#[derive(Debug, PartialEq, Eq)]
enum Queued {
    Added,
    /// Its row is there already.
    Known,
    /// It couldn't be read or changed while it was read: the next scan tries again.
    NotNow,
}

impl Worker {
    fn new(shared: Arc<Shared>) -> Worker {
        Worker {
            shared,
            generation: None,
            cfg: None,
            remote: None,
            previous: None,
            scanned_live: None,
            scanned_all: None,
            tokens: 0.0,
            refilled: Instant::now(),
            listed: HashSet::new(),
            wait_until: None,
            prune_retry: None,
            staged: AtomicU64::new(0),
            unread: HashSet::new(),
        }
    }

    fn run(&mut self) {
        let shared = self.shared.clone();
        loop {
            if shared.stopping() {
                return;
            }
            let seen = shared.wakes();
            let pending = shared.flush();
            let wait = {
                let _busy = shared.busy();
                match self.prepare() {
                    Ready::No(d) => Some(d),
                    Ready::Yes => {
                        self.maintain();
                        match self.step() {
                            Step::Did => None,
                            Step::Idle(d) => Some(d),
                        }
                    }
                }
            };
            if let Some(d) = wait {
                let d = d.min(TICK).min(pending.unwrap_or(TICK));
                if shared.sleep(d, seen) {
                    return;
                }
            }
        }
    }

    /// Loads the configuration, the key, the secret and the bucket when they changed, and says
    /// whether uploads may go.
    fn prepare(&mut self) -> Ready {
        let shared = self.shared.clone();
        let (cfg, generation, blocked) = {
            let st = shared.lock();
            (st.cfg.clone(), st.generation, st.blocked.clone())
        };
        let Some(cfg) = cfg else {
            if self.cfg.take().is_some() {
                self.keep_previous();
                self.generation = None;
                shared.changed();
            }
            return Ready::No(TICK);
        };
        let now = shared.now();
        if let Some(blocked) = &blocked {
            let left = blocked.until.saturating_duration_since(Instant::now());
            if blocked.generation == generation && !left.is_zero() {
                return Ready::No(left);
            }
            let mut st = shared.lock();
            if st
                .blocked
                .as_ref()
                .is_some_and(|b| b.generation == blocked.generation)
            {
                st.blocked = None;
            }
        }
        if self.generation == Some(generation) && self.remote.is_some() {
            return Ready::Yes;
        }
        self.keep_previous();
        self.listed.clear();
        self.wait_until = None;
        // Retries that a clock running ahead scheduled for later than any backoff can.
        self.reschedule_ahead(now);
        match self.connect(&cfg, now) {
            Ok((remote, left_open)) => {
                self.cancel_left_open(left_open);
                self.remote = Some(remote);
                self.cfg = Some(cfg);
                self.generation = Some(generation);
                {
                    let mut st = shared.lock();
                    st.rescan = true;
                }
                shared.changed();
                Ready::Yes
            }
            Err(error) => {
                log::warn!("sync: can't start uploading: {error}");
                shared.note_failure(&error);
                shared.lock().blocked = Some(Blocked {
                    error,
                    until: Instant::now() + shared.opts.timing.blocked,
                    generation,
                });
                self.generation = Some(generation);
                shared.changed();
                Ready::No(TICK)
            }
        }
    }

    /// The bucket and the key, and the multipart uploads a change of bucket left open in the old
    /// one, (object, upload id).
    fn connect(
        &self,
        cfg: &SyncConfig,
        now: u64,
    ) -> Result<(Remote, Vec<(String, String)>), SyncError> {
        let shared = &self.shared;
        let (key, _) = shared.key_or_create(now)?;
        if shared.queue.adopt_kid(&key.kid())? {
            log::info!("sync: the encryption key changed; the queue starts again");
        }
        let secret = shared.keys.secret()?.ok_or_else(|| {
            SyncError::new(SyncErrorKind::NoSecret, "Save the secret access key first.")
        })?;
        let timing = &shared.opts.timing;
        let bucket = Bucket::new(cfg, &secret, timing.connect_timeout, timing.request_timeout)?;
        let target = cfg.target();
        let left_open = shared.queue.adopt_target(&target)?;
        if left_open.is_some() {
            log::info!(
                "sync: the bucket, its endpoint or the folder changed; everything still on this computer goes up again"
            );
        }
        shared.lock().target = Some(target);
        Ok((Remote::new(bucket, key), left_open.unwrap_or_default()))
    }

    /// The connection Voice stops using, so a change of bucket can still cancel the uploads it left
    /// open there.
    fn keep_previous(&mut self) {
        if let Some(remote) = self.remote.take() {
            self.previous = Some(remote);
        }
    }

    /// Cancels, as far as the old bucket lets Voice, the multipart uploads a change of bucket left
    /// open there, so they don't keep taking space. Nothing else depends on it.
    fn cancel_left_open(&mut self, left_open: Vec<(String, String)>) {
        let previous = self.previous.take();
        if left_open.is_empty() {
            return;
        }
        let Some(old) = previous else {
            log::info!(
                "sync: {} uploads stay open in the old bucket until it expires them",
                left_open.len()
            );
            return;
        };
        for (object, upload_id) in left_open {
            if self.shared.stopping() {
                return;
            }
            if let Err(e) = old.bucket.abort_multipart(&object, &upload_id) {
                log::info!("sync: can't cancel an upload in the old bucket: {e}");
            }
        }
    }

    /// Scans, manifests, the index, bucket retention and tidying, each when it is due.
    fn maintain(&mut self) {
        let shared = self.shared.clone();
        let timing = shared.opts.timing.clone();
        let (nudged, rescan) = {
            let mut st = shared.lock();
            let flags = (st.nudged, st.rescan);
            st.nudged = false;
            st.rescan = false;
            flags
        };
        let all_due = rescan
            || self
                .scanned_all
                .is_none_or(|at| at.elapsed() >= timing.scan_all);
        let live_due = nudged
            || self
                .scanned_live
                .is_none_or(|at| at.elapsed() >= timing.scan_live);
        if all_due || live_due {
            let before = shared.queue.waiting().ok();
            if let Err(e) = self.scan(all_due) {
                log::warn!("sync: scan failed: {e}");
            }
            self.scanned_live = Some(Instant::now());
            if all_due {
                self.scanned_all = Some(Instant::now());
                self.tidy();
            }
            if shared.queue.waiting().ok() != before {
                shared.changed();
            }
        }
        if let Err(e) = self.manifests() {
            log::warn!("sync: can't make a manifest: {e}");
        }
        if let Err(e) = self.index() {
            log::warn!("sync: can't make the index: {e}");
        }
        let live = shared.queue.any_live().unwrap_or(true);
        if !live {
            self.prune();
            self.count_remote();
        }
    }

    /// Looks at the meeting folders: every one with `all`, else those that record or haven't
    /// synced and new ones.
    fn scan(&mut self, all: bool) -> Result<(), SyncError> {
        let shared = self.shared.clone();
        let now = shared.now();
        let rows: HashMap<String, MeetingRow> = shared
            .queue
            .meetings()?
            .into_iter()
            .map(|m| (m.id.clone(), m))
            .collect();
        let enabled_at = shared.queue.meta_u64("enabled_at")?.unwrap_or(0);
        let mut present = HashSet::new();
        for dir in meeting_dirs(&shared.opts.meetings_dir) {
            let id = dir.id().as_str().to_owned();
            present.insert(id.clone());
            let row = rows.get(&id);
            // Explicit deletion survives a full rescan, including the short interval before
            // the engine renames the local folder out of the library.
            if row.is_some_and(|r| r.phase == "forgotten") {
                continue;
            }
            if !all
                && row.is_some_and(|r| {
                    matches!(r.phase.as_str(), "synced" | "restored" | "forgotten")
                })
            {
                continue;
            }
            if shared.stopping() {
                return Ok(());
            }
            if let Err(e) = self.scan_meeting(&dir, enabled_at, now) {
                log::warn!("sync: can't scan a meeting: {e}");
            }
        }
        // A meeting whose folder went can't be live any more.
        for row in rows.values() {
            if row.phase == "live" && !present.contains(&row.id) {
                shared.queue.set_phase(&row.id, "after")?;
            }
        }
        Ok(())
    }

    fn scan_meeting(
        &mut self,
        dir: &MeetingDir,
        enabled_at: u64,
        now: u64,
    ) -> Result<(), SyncError> {
        let shared = self.shared.clone();
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let Ok(file) = dir.meeting() else {
            return Ok(());
        };
        let id = dir.id().as_str();
        let mhash = names::meeting_hash(&remote.name_key, id);
        shared
            .queue
            .ensure_meeting(id, &mhash, &file.title, file.created_at)?;
        let before = shared.queue.meeting(id)?;
        let backfill = file.created_at < enabled_at;
        let scan::Scan {
            live,
            found,
            unreadable,
        } = scan::scan(dir, shared.opts.screen.as_ref(), backfill);
        let mut unread = unreadable;
        let outcome = if live {
            None
        } else {
            transcript::stored_outcome(dir.path())
        };
        let state = match (&outcome, live) {
            (_, true) => "live",
            (Some(o), _) if o.state == MeetingState::Done => "done",
            (Some(o), _) if o.state == MeetingState::Failed => "failed",
            _ => "processing",
        };
        let done_at = outcome.as_ref().and_then(|o| match o.state {
            MeetingState::Done => o.done_at.or(Some(o.ended_at)),
            MeetingState::Failed => Some(o.ended_at),
            _ => None,
        });
        let duration_ms = if live {
            None
        } else {
            Some(History::from_lines(&dir.timeline()).latest_t)
        };
        let media_expired = before
            .as_ref()
            .is_some_and(|m| m.media_removed_at.is_some())
            || self.media_expired(done_at, now);
        for found in &found {
            if media_expired && MEDIA_KINDS.contains(&found.kind) {
                continue;
            }
            if shared.stopping() {
                return Ok(());
            }
            match self.add_file(dir, &mhash, found, now) {
                Ok(Queued::Added | Queued::Known) => {}
                Ok(Queued::NotNow) => unread = true,
                Err(e) => {
                    log::warn!("sync: can't queue a file: {e}");
                    unread = true;
                }
            }
        }
        if unread {
            if self.unread.insert(id.to_owned()) {
                log::info!("sync: a file of a meeting can't be read now; its media stays held");
            }
        } else {
            self.unread.remove(id);
        }
        // A meeting that became done needs a manifest that says so.
        if before.as_ref().is_some_and(|b| b.state != state) {
            shared.queue.set_manifest_dirty(id)?;
        }
        let after = shared.queue.meeting(id)?;
        let waiting = shared.queue.meeting_waiting(id, true)?;
        // Synced only once every file the scan found has its row: one it couldn't read has none.
        let complete = !live
            && !unread
            && matches!(state, "done" | "failed")
            && !waiting
            && after
                .as_ref()
                .is_some_and(|m| !m.manifest_dirty && m.uploaded_gen > 0);
        let restored = before.as_ref().is_some_and(|m| m.phase == "restored");
        let phase = match (live, complete, restored) {
            (true, _, _) => "live",
            (false, true, true) => "restored",
            (false, true, false) => "synced",
            (false, false, _) => "after",
        };
        shared.queue.set_progress(
            id,
            &Progress {
                phase,
                state,
                duration_ms,
                done_at,
            },
            now,
        )
    }

    /// Whether bucket retention removes media of a meeting done at `done_at`.
    fn media_expired(&self, done_at: Option<u64>, now: u64) -> bool {
        let days = match self.cfg.as_ref().map(|c| c.retention) {
            Some(BucketRetention::Days365) => 365,
            Some(BucketRetention::Days90) => 90,
            _ => return false,
        };
        done_at.is_some_and(|at| at.saturating_add(days * DAY_MS) <= now)
    }

    /// Queues a found file when it is new or changed.
    fn add_file(
        &self,
        dir: &MeetingDir,
        mhash: &str,
        found: &Found,
        now: u64,
    ) -> Result<Queued, SyncError> {
        let shared = &self.shared;
        let id = dir.id().as_str();
        let known = shared.queue.known(id, &found.path)?;
        if found.kind == "transcript" {
            if known.as_ref().is_some_and(|k| k.1 == Some(found.mtime)) {
                return Ok(Queued::Known);
            }
            return self.add_transcript(dir, mhash, found, known.map(|k| k.2), now);
        }
        if known
            .as_ref()
            .is_some_and(|k| k.0 == found.size && k.1 == Some(found.mtime))
        {
            return Ok(Queued::Known);
        }
        let path = dir.path().join(&found.path);
        let Ok((sha, len)) = sha256_file(&path) else {
            // Another program holds it open, for instance: the next scan tries again.
            return Ok(Queued::NotNow);
        };
        if len != found.size {
            // It changed while it was read; the next scan looks again.
            return Ok(Queued::NotNow);
        }
        if known.as_ref().is_some_and(|k| k.2 == sha) {
            shared.queue.set_mtime(id, &found.path, &sha, found.mtime)?;
            return Ok(Queued::Known);
        }
        let added = self.insert(id, mhash, found, found.size, sha, None, now)?;
        Ok(if added { Queued::Added } else { Queued::Known })
    }

    /// `transcript.db` goes up as a `VACUUM INTO` snapshot in `staging/`, because a database in
    /// WAL mode can't be copied as a file.
    fn add_transcript(
        &self,
        dir: &MeetingDir,
        mhash: &str,
        found: &Found,
        known_sha: Option<String>,
        now: u64,
    ) -> Result<Queued, SyncError> {
        let shared = &self.shared;
        let id = dir.id().as_str();
        if folder_size(&shared.staging) > STAGING_MAX || !(shared.opts.reserve)(found.size) {
            log::info!("sync: no room for a transcript snapshot now");
            return Ok(Queued::NotNow);
        }
        fs::create_dir_all(&shared.staging)
            .map_err(|e| local("Voice can't make its staging folder", &e))?;
        let staged = shared.staging.join(format!(
            "{id}-transcript-{now}-{}.db",
            self.staged.fetch_add(1, Ordering::Relaxed)
        ));
        if let Err(e) = snapshot(&dir.path().join(transcript::FILE_NAME), &staged) {
            let _ = fs::remove_file(&staged);
            return Err(e);
        }
        let (sha, size) = match sha256_file(&staged) {
            Ok(done) => done,
            Err(e) => {
                let _ = fs::remove_file(&staged);
                return Err(local("Voice can't read a transcript snapshot", &e));
            }
        };
        if known_sha.as_deref() == Some(sha.as_str()) {
            let _ = fs::remove_file(&staged);
            shared.queue.set_mtime(id, &found.path, &sha, found.mtime)?;
            return Ok(Queued::Known);
        }
        let added = self.insert(id, mhash, found, size, sha, Some(staged.clone()), now)?;
        if !added {
            let _ = fs::remove_file(&staged);
            return Ok(Queued::Known);
        }
        Ok(Queued::Added)
    }

    /// One new row with a fresh header. Older versions of the file that still wait give way.
    #[allow(clippy::too_many_arguments)]
    fn insert(
        &self,
        meeting: &str,
        mhash: &str,
        found: &Found,
        size: u64,
        sha: String,
        source: Option<PathBuf>,
        now: u64,
    ) -> Result<bool, SyncError> {
        let Some(remote) = &self.remote else {
            return Ok(false);
        };
        let object = names::file_object(
            &remote.kid,
            mhash,
            &remote.name_key,
            meeting,
            &found.path,
            &sha,
        );
        let (header, _) = Sealer::new(&remote.key, &object, size)?;
        let added = self.shared.queue.insert(NewRow {
            meeting: meeting.to_owned(),
            path: found.path.clone(),
            kind: found.kind.to_owned(),
            lane: found.lane,
            size,
            mtime: Some(found.mtime),
            sha256: sha,
            object,
            header: header.to_bytes().to_vec(),
            source,
            track: found.track.clone(),
            seq: found.seq,
            t: found.t,
            samples: found.samples,
            created_at: now,
        })?;
        if added {
            self.supersede(meeting, &found.path)?;
        }
        Ok(added)
    }

    /// Older waiting versions of a file become `gone`, and their staged copies go.
    fn supersede(&self, meeting: &str, path: &str) -> Result<(), SyncError> {
        for source in self.shared.queue.supersede(meeting, path)? {
            let _ = fs::remove_file(source);
        }
        Ok(())
    }

    /// Makes the manifests that are due (2.8): during a meeting every `manifest_live` when a file
    /// went up since the last; after it, once its other rows are done.
    fn manifests(&mut self) -> Result<(), SyncError> {
        let shared = self.shared.clone();
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let now = shared.now();
        let enabled_at = shared.queue.meta_u64("enabled_at")?.unwrap_or(0);
        let live_every = millis(shared.opts.timing.manifest_live);
        for m in shared.queue.dirty_meetings()? {
            let live = m.phase == "live";
            if live {
                // A stamp from a clock that ran ahead and was set right doesn't hold the next live
                // manifest off until the clock catches up: the bucket's manifest would miss the
                // segments that went up meanwhile.
                let recent =
                    m.manifest_at > 0 && m.manifest_at <= now && now - m.manifest_at < live_every;
                if recent {
                    continue;
                }
            } else if shared.queue.meeting_waiting(&m.id, false)? {
                continue;
            }
            if shared.queue.manifest_rows(&m.id)?.is_empty() {
                // Nothing of it reached the bucket: the first upload marks it again.
                shared.queue.clear_manifest_dirty(&m.id)?;
                continue;
            }
            let state = if m.state.is_empty() {
                "processing"
            } else {
                m.state.as_str()
            };
            let manifest = manifest_for(&shared.queue, &m.id, state, now)?;
            if manifest.files.is_empty() {
                continue;
            }
            let text = serde_json::to_vec(&manifest).map_err(|e| {
                SyncError::new(
                    SyncErrorKind::Local,
                    format!("Voice can't write a manifest: {e}"),
                )
            })?;
            let lane = if live {
                0
            } else if m.created_at < enabled_at {
                2
            } else {
                1
            };
            let object = names::manifest_object(&remote.kid, &m.mhash);
            let name = format!("{}-manifest-{}.json", m.id, manifest.gen);
            self.stage(
                &m.id,
                "manifest",
                "manifest",
                lane,
                &text,
                object,
                Some(manifest.gen),
                &name,
                now,
            )?;
            // Only now that its row waits: something is pending at every moment in between.
            shared.queue.clear_manifest_dirty(&m.id)?;
        }
        Ok(())
    }

    /// Makes the index once `index_debounce` passed after the last manifest went up.
    fn index(&mut self) -> Result<(), SyncError> {
        let shared = self.shared.clone();
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let now = shared.now();
        let Some(due) = shared.queue.meta_u64("index_due")? else {
            return Ok(());
        };
        // Due further ahead than the debounce: stamped by a clock that ran ahead and was set
        // right since. The index goes now instead of when the clock catches up.
        let debounce = millis(shared.opts.timing.index_debounce);
        if now < due && due - now <= debounce {
            return Ok(());
        }
        let meetings: Vec<IndexEntry> = shared
            .queue
            .meetings()?
            .into_iter()
            .filter(|m| m.uploaded_gen > 0)
            .map(|m| {
                let deleted = m.phase == "forgotten";
                IndexEntry {
                    id: m.id,
                    m: m.mhash,
                    title: m.title,
                    created_at: m.created_at,
                    duration_ms: m.duration_ms,
                    state: if m.state.is_empty() {
                        "processing".into()
                    } else {
                        m.state
                    },
                    // A deletion outranks the version this computer knew.
                    gen: m.uploaded_gen + u64::from(deleted),
                    media_removed_at: m.media_removed_at,
                    deleted,
                }
            })
            .collect();
        if meetings.is_empty() {
            shared.queue.set_meta("index_due", None)?;
            return Ok(());
        }
        let object = names::index_object(&remote.kid, &remote.name_key, &shared.queue.writer());
        let writer = object.rsplit('/').next().unwrap_or_default().to_owned();
        let index = Index {
            v: crate::manifest::VERSION,
            writer,
            updated_at: now,
            meetings,
        };
        let text = serde_json::to_vec(&index).map_err(|e| {
            SyncError::new(
                SyncErrorKind::Local,
                format!("Voice can't write the index: {e}"),
            )
        })?;
        let name = format!(
            "index-{now}-{}.json",
            self.staged.fetch_add(1, Ordering::Relaxed)
        );
        self.stage("", "index", "index", 1, &text, object, None, &name, now)?;
        shared.queue.set_meta("index_due", None)
    }

    /// Writes `text` to `staging/<name>` and queues it.
    #[allow(clippy::too_many_arguments)]
    fn stage(
        &self,
        meeting: &str,
        path: &str,
        kind: &'static str,
        lane: u8,
        text: &[u8],
        object: String,
        seq: Option<u64>,
        name: &str,
        now: u64,
    ) -> Result<(), SyncError> {
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let shared = &self.shared;
        fs::create_dir_all(&shared.staging)
            .map_err(|e| local("Voice can't make its staging folder", &e))?;
        let staged = shared.staging.join(name);
        fs::write(&staged, text).map_err(|e| local("Voice can't stage a file", &e))?;
        let size = text.len() as u64;
        let (header, _) = Sealer::new(&remote.key, &object, size)?;
        let added = shared.queue.insert(NewRow {
            meeting: meeting.to_owned(),
            path: path.to_owned(),
            kind: kind.to_owned(),
            lane,
            size,
            mtime: None,
            sha256: sha256_hex(text),
            object,
            header: header.to_bytes().to_vec(),
            source: Some(staged.clone()),
            track: None,
            seq,
            t: None,
            samples: None,
            created_at: now,
        })?;
        if added {
            self.supersede(meeting, path)?;
        } else {
            let _ = fs::remove_file(&staged);
        }
        Ok(())
    }

    /// Bucket retention, once per `prune_every` (2.10).
    fn prune(&mut self) {
        let shared = self.shared.clone();
        let (Some(cfg), Some(remote)) = (&self.cfg, &self.remote) else {
            return;
        };
        let now = shared.now();
        if self.prune_retry.is_some_and(|at| Instant::now() < at) {
            return;
        }
        let last = shared
            .queue
            .meta_u64("pruned_at")
            .ok()
            .flatten()
            .unwrap_or(0);
        // A run stamped by a clock that ran ahead doesn't hold the next one off until the clock
        // catches up.
        if last <= now && now - last < millis(shared.opts.timing.prune_every) {
            return;
        }
        match crate::prune::bucket_retention(&shared, remote, cfg.retention, now) {
            Ok(n) => {
                if n > 0 {
                    log::info!("sync: bucket retention removed the media of {n} meetings");
                    shared.changed();
                }
                let _ = shared.queue.set_meta("pruned_at", Some(&now.to_string()));
            }
            Err(e) => {
                log::warn!("sync: bucket retention failed: {e}");
                self.prune_retry = Some(Instant::now() + PRUNE_RETRY);
            }
        }
    }

    /// Counts the bucket's meetings that aren't on this computer, once per configuration.
    fn count_remote(&mut self) {
        let shared = self.shared.clone();
        let Some(remote) = &self.remote else {
            return;
        };
        let generation = self.generation;
        {
            let st = shared.lock();
            if st.listed == generation || st.restoring {
                return;
            }
        }
        // Only while the configuration, the secret and the key the worker connected with hold.
        let check = || {
            let st = shared.lock();
            if st.stopping || Some(st.generation) != generation {
                return Err(SyncError::new(SyncErrorKind::Off, "Bucket sync changed."));
            }
            Ok(())
        };
        match crate::restore::list_remote(&shared, remote, &check) {
            Ok(meetings) => {
                let mut st = shared.lock();
                st.listed = generation;
                if matches!(st.restore.state, "idle" | "available") {
                    st.restore.state = "available";
                    st.restore.available = u32::try_from(meetings.len()).unwrap_or(u32::MAX);
                }
                drop(st);
                shared.changed();
            }
            Err(e) => {
                log::info!("sync: can't list the bucket's meetings now: {e}");
                shared.lock().listed = generation;
            }
        }
    }

    /// Makes rows due now whose retry lies further ahead than any backoff schedules one: they
    /// were scheduled while the wall clock ran ahead, and would otherwise wait for it to catch up.
    fn reschedule_ahead(&self, now: u64) {
        let horizon = millis(self.shared.opts.timing.backoff_max).saturating_mul(2);
        match self.shared.queue.reschedule_ahead(now, horizon) {
            Ok(0) => {}
            Ok(n) => log::info!("sync: {n} retries lay further ahead than any backoff; due now"),
            Err(e) => log::warn!("sync: {e}"),
        }
    }

    /// Aborts uploads nothing will finish, drops rows nobody needs and staged files no row names.
    fn tidy(&mut self) {
        let shared = self.shared.clone();
        self.reschedule_ahead(shared.now());
        if let Some(remote) = &self.remote {
            if let Ok(dangling) = shared.queue.dangling_uploads() {
                for (id, object, upload_id) in dangling {
                    match remote.bucket.abort_multipart(&object, &upload_id) {
                        Ok(()) => {}
                        Err(e)
                            if matches!(
                                e.kind,
                                SyncErrorKind::NoSuchUpload | SyncErrorKind::NotFound
                            ) => {}
                        Err(e) => {
                            log::info!("sync: can't abort an upload now: {e}");
                            continue;
                        }
                    }
                    let _ = shared.queue.clear_upload(id);
                }
            }
        }
        if let Err(e) = shared.queue.tidy() {
            log::warn!("sync: can't tidy the queue: {e}");
        }
        let Ok(needed) = shared.queue.staged() else {
            return;
        };
        let needed: HashSet<PathBuf> = needed.into_iter().collect();
        if let Ok(entries) = fs::read_dir(&shared.staging) {
            for entry in entries.filter_map(Result::ok) {
                let path = entry.path();
                if !needed.contains(&path) {
                    let _ = fs::remove_file(&path);
                }
            }
        }
    }

    /// Sends the next row's next request.
    fn step(&mut self) -> Step {
        let shared = self.shared.clone();
        let now = shared.now();
        if let Some(until) = self.wait_until {
            let left = until.saturating_duration_since(Instant::now());
            if !left.is_zero() {
                return Step::Idle(left);
            }
            self.wait_until = None;
        }
        let live = match shared.queue.any_live() {
            Ok(live) => live,
            Err(e) => {
                log::warn!("sync: can't read the queue: {e}");
                return Step::Idle(TICK);
            }
        };
        if !live {
            self.tokens = 0.0;
        }
        let row = match shared.queue.next(now, live) {
            Ok(Some(row)) => row,
            Ok(None) => {
                let until = shared
                    .queue
                    .next_due(live)
                    .ok()
                    .flatten()
                    .map_or(TICK, |at| {
                        Duration::from_millis(at.saturating_sub(now).max(1))
                    });
                return Step::Idle(until.min(shared.opts.timing.scan_live));
            }
            Err(e) => {
                log::warn!("sync: can't read the queue: {e}");
                return Step::Idle(TICK);
            }
        };
        match self.upload(&row, live, now) {
            Ok(step) => step,
            Err(e) => self.failed(&row, e),
        }
    }

    /// The token bucket of the live lane: how long to wait before the next request, if at all.
    fn throttle(&mut self, live: bool) -> Option<Duration> {
        if !live {
            return None;
        }
        let rate = self.shared.opts.timing.live_rate.max(1) as f64;
        self.tokens = (self.tokens + self.refilled.elapsed().as_secs_f64() * rate).min(0.0);
        self.refilled = Instant::now();
        (self.tokens < 0.0)
            .then(|| Duration::from_secs_f64(-self.tokens / rate).max(Duration::from_millis(1)))
    }

    fn spend(&mut self, live: bool, bytes: u64) {
        if live {
            self.tokens -= bytes as f64;
        }
    }

    fn upload(&mut self, row: &Row, live: bool, now: u64) -> Result<Step, SyncError> {
        let shared = self.shared.clone();
        let source = row
            .source
            .clone()
            .unwrap_or_else(|| shared.meeting_dir(&row.meeting).join(&row.path));
        let size = match fs::metadata(&source) {
            Ok(meta) => meta.len(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                return self.gone(row, "its file is gone")
            }
            Err(e) => return Err(local("Voice can't read a file to upload", &e)),
        };
        if size != row.size {
            return self.gone(row, "its file changed");
        }
        let Some(remote) = &self.remote else {
            return Ok(Step::Idle(TICK));
        };
        let header = match row.header.as_deref().map(Header::parse) {
            Some(Ok(header)) if header.plain_len() == row.size => header,
            _ => return self.gone(row, "its header is unusable"),
        };
        let sealer = match Sealer::open(&remote.key, &row.object, &header) {
            Ok(sealer) => sealer,
            Err(_) => return self.gone(row, "its header doesn't open with this key"),
        };
        let total = cipher_len(row.size);
        if total <= shared.opts.timing.single_max {
            if let Some(wait) = self.throttle(live) {
                return Ok(Step::Idle(wait));
            }
            let plain =
                fs::read(&source).map_err(|e| local("Voice can't read a file to upload", &e))?;
            if plain.len() as u64 != row.size || sha256_hex(&plain) != row.sha256 {
                return self.gone(row, "its file changed");
            }
            let body = sealer.read_cipher(&header, &mut Cursor::new(&plain), 0, to_usize(total))?;
            self.spend(live, total);
            let remote = self.remote.as_ref().expect("checked above");
            remote.bucket.put(&row.object, &body)?;
            self.finished(row, Some(&plain), now)?;
            return Ok(Step::Did);
        }
        self.multipart(row, &source, &header, &sealer, total, live, now)
    }

    #[allow(clippy::too_many_arguments)]
    fn multipart(
        &mut self,
        row: &Row,
        source: &Path,
        header: &Header,
        sealer: &Sealer,
        total: u64,
        live: bool,
        now: u64,
    ) -> Result<Step, SyncError> {
        let shared = self.shared.clone();
        let part_size = row.part_size.unwrap_or(shared.opts.timing.part_size).max(1);
        let count = total.div_ceil(part_size);
        let Some(upload_id) = row.upload_id.clone() else {
            if !self.unchanged(row, source)? {
                return self.gone(row, "its file changed");
            }
            let remote = self.remote.as_ref().expect("connected");
            let upload_id = remote.bucket.create_multipart(&row.object)?;
            shared
                .queue
                .set_upload(row.id, Some(&upload_id), Some(part_size))?;
            shared.queue.replace_parts(row.id, &[])?;
            self.listed.insert(row.id);
            return Ok(Step::Did);
        };
        if !self.listed.contains(&row.id) {
            if !self.unchanged(row, source)? {
                return self.gone(row, "its file changed");
            }
            let remote = self.remote.as_ref().expect("connected");
            let found = remote.bucket.list_parts(&row.object, &upload_id)?;
            let keep: Vec<_> = found
                .into_iter()
                .filter(|p| {
                    let n = u64::from(p.number);
                    n >= 1 && n <= count && p.size == part_len(total, part_size, n)
                })
                .collect();
            shared.queue.replace_parts(row.id, &keep)?;
            self.listed.insert(row.id);
            return Ok(Step::Did);
        }
        let have = shared.queue.parts(row.id)?;
        let missing = (1..=count).find(|n| !have.iter().any(|p| u64::from(p.number) == *n));
        match missing {
            Some(n) => {
                let number = u16::try_from(n).map_err(|_| {
                    SyncError::new(
                        SyncErrorKind::Local,
                        "The file needs more than 65,535 parts.",
                    )
                })?;
                let offset = (n - 1) * part_size;
                let len = part_len(total, part_size, n);
                if let Some(wait) = self.throttle(live) {
                    return Ok(Step::Idle(wait));
                }
                let mut file = File::open(source)
                    .map_err(|e| local("Voice can't read a file to upload", &e))?;
                let body = sealer.read_cipher(header, &mut file, offset, to_usize(len))?;
                self.spend(live, len);
                let remote = self.remote.as_ref().expect("connected");
                let etag = remote
                    .bucket
                    .upload_part(&row.object, &upload_id, number, &body)?;
                shared.queue.part_done(row.id, number, &etag, len)?;
                shared.note_success();
                Ok(Step::Did)
            }
            None => {
                let mut have = have;
                have.sort_by_key(|p| p.number);
                let etags: Vec<String> = have.into_iter().map(|p| p.etag).collect();
                let remote = self.remote.as_ref().expect("connected");
                remote
                    .bucket
                    .complete_multipart(&row.object, &upload_id, &etags)?;
                self.listed.remove(&row.id);
                self.finished(row, None, now)?;
                Ok(Step::Did)
            }
        }
    }

    /// Whether a file about to go up in parts still has its row's bytes: its SHA-256 when it is
    /// under 64 MiB, else its size, which `upload` checked.
    fn unchanged(&self, row: &Row, source: &Path) -> Result<bool, SyncError> {
        if row.size >= REHASH_MAX {
            return Ok(true);
        }
        let (sha, len) =
            sha256_file(source).map_err(|e| local("Voice can't read a file to upload", &e))?;
        Ok(len == row.size && sha == row.sha256)
    }

    /// The row reached the bucket.
    fn finished(&mut self, row: &Row, plain: Option<&[u8]>, now: u64) -> Result<(), SyncError> {
        let shared = self.shared.clone();
        if row.kind == "manifest" {
            // Before the row is done, so the index is pending from then on.
            let due = now + millis(shared.opts.timing.index_debounce);
            shared.queue.set_meta("index_due", Some(&due.to_string()))?;
        }
        shared.queue.done(row.id, now)?;
        if let Some(source) = &row.source {
            let _ = fs::remove_file(source);
        }
        if row.kind == "manifest" {
            if let Some(plain) = plain {
                if let Err(e) = self.manifest_uploaded(row, plain) {
                    log::warn!("sync: can't remove replaced objects: {e}");
                }
            }
            self.settle_phase(&row.meeting)?;
        }
        shared.note_success();
        shared.changed();
        Ok(())
    }

    /// A finished meeting whose last manifest went up with nothing left to upload is synced now,
    /// not at the next scan, so retention's hold lets go as soon as Settings says idle.
    fn settle_phase(&self, meeting: &str) -> Result<(), SyncError> {
        let queue = &self.shared.queue;
        let Some(m) = queue.meeting(meeting)? else {
            return Ok(());
        };
        if m.phase == "after"
            && !self.unread.contains(meeting)
            && matches!(m.state.as_str(), "done" | "failed")
            && !m.manifest_dirty
            && !queue.meeting_waiting(meeting, true)?
        {
            queue.set_phase(meeting, "synced")?;
        }
        Ok(())
    }

    /// A manifest went up: objects of older versions of the files it names are deleted (2.8).
    fn manifest_uploaded(&self, row: &Row, plain: &[u8]) -> Result<(), SyncError> {
        let shared = &self.shared;
        let Some(remote) = &self.remote else {
            return Ok(());
        };
        let manifest: Manifest = serde_json::from_slice(plain).map_err(|e| {
            SyncError::new(
                SyncErrorKind::Local,
                format!("A staged manifest is unreadable: {e}"),
            )
        })?;
        let named: HashSet<&str> = manifest.files.iter().map(|f| f.object.as_str()).collect();
        let rows = shared.queue.done_rows(&row.meeting)?;
        let mut named_id: HashMap<&str, i64> = HashMap::new();
        for r in &rows {
            if named.contains(r.object.as_str()) {
                let e = named_id.entry(r.path.as_str()).or_insert(r.id);
                *e = (*e).max(r.id);
            }
        }
        let stale: Vec<&Row> = rows
            .iter()
            .filter(|r| {
                !named.contains(r.object.as_str())
                    && named_id.get(r.path.as_str()).is_some_and(|id| r.id < *id)
            })
            .collect();
        if stale.is_empty() {
            return Ok(());
        }
        let keys: Vec<String> = stale.iter().map(|r| r.object.clone()).collect();
        let failed: HashSet<String> = remote.bucket.delete_many(&keys)?.into_iter().collect();
        let removed: Vec<i64> = stale
            .iter()
            .filter(|r| !failed.contains(&r.object))
            .map(|r| r.id)
            .collect();
        shared.queue.mark_removed(&removed)
    }

    /// The row can't go up as it is: the next scan makes a new one.
    fn gone(&mut self, row: &Row, why: &str) -> Result<Step, SyncError> {
        log::info!("sync: a queued {} isn't uploaded: {why}", row.kind);
        self.shared.queue.gone(row.id)?;
        if let Some(source) = &row.source {
            let _ = fs::remove_file(source);
        }
        self.listed.remove(&row.id);
        self.shared.changed();
        Ok(Step::Did)
    }

    /// A request failed: retry the row later, start its upload again, or stop uploads (2.9).
    fn failed(&mut self, row: &Row, error: SyncError) -> Step {
        let shared = self.shared.clone();
        let now = shared.now();
        log::info!("sync: an upload failed: {error}");
        self.listed.remove(&row.id);
        if error.kind == SyncErrorKind::NoSuchUpload {
            // The bucket also forgets an upload it completed. When Voice was killed after
            // CompleteMultipartUpload, or its answer got lost, the whole object is there already.
            if row.upload_id.is_some() {
                match self.completed_already(row) {
                    Ok(true) => {
                        log::info!("sync: a multipart upload had completed already");
                        return match self.finished(row, None, now) {
                            Ok(()) => Step::Did,
                            Err(e) => {
                                log::warn!("sync: {e}");
                                Step::Idle(TICK)
                            }
                        };
                    }
                    Ok(false) => {}
                    // Can't tell now: retry later as for any failed request, upload id kept.
                    Err(e) => return self.failed(row, e),
                }
            }
            // R2 aborts uploads after 7 days: start this one again.
            if let Err(e) = shared.queue.set_upload(row.id, None, None) {
                log::warn!("sync: {e}");
                return Step::Idle(TICK);
            }
            return Step::Did;
        }
        shared.note_failure(&error);
        let generation = self.generation.unwrap_or(0);
        if error.transient() {
            let failures = {
                let mut st = shared.lock();
                st.failures += 1;
                st.failures
            };
            let _ = shared
                .queue
                .retry(row.id, now + self.backoff(row.attempts), &error);
            if failures >= 3 {
                self.wait_until =
                    Some(Instant::now() + Duration::from_millis(self.backoff(failures - 3)));
            }
        } else {
            match error.kind {
                SyncErrorKind::Local => {
                    let _ = shared
                        .queue
                        .retry(row.id, now + self.backoff(row.attempts), &error);
                }
                SyncErrorKind::Crypto => {
                    let _ = self.gone(row, "it can't be encrypted");
                }
                _ => {
                    shared.lock().blocked = Some(Blocked {
                        error,
                        until: Instant::now() + shared.opts.timing.blocked,
                        generation,
                    });
                }
            }
        }
        shared.changed();
        Step::Did
    }

    /// Whether the row's object is in the bucket with its whole length. The name holds the
    /// plaintext's SHA-256 and the stored header fixes every byte, so such an object is the one
    /// this row's upload would complete.
    fn completed_already(&self, row: &Row) -> Result<bool, SyncError> {
        let Some(remote) = &self.remote else {
            return Ok(false);
        };
        Ok(remote.bucket.head(&row.object)? == Some(cipher_len(row.size)))
    }

    /// `min(backoff_max, backoff_min · 2^n)` with ±20 % jitter, in ms.
    fn backoff(&self, n: u32) -> u64 {
        let timing = &self.shared.opts.timing;
        let min = millis(timing.backoff_min);
        let max = millis(timing.backoff_max);
        let base = min.saturating_mul(1u64 << n.min(40)).min(max);
        let mut r = [0u8; 2];
        let unit = if getrandom::fill(&mut r).is_ok() {
            f64::from(u16::from_le_bytes(r)) / f64::from(u16::MAX)
        } else {
            0.5
        };
        let jitter = 0.8 + 0.4 * unit;
        ((base as f64) * jitter) as u64
    }
}

/// Every meeting folder of the library, newest first: folders named as a meeting id that hold a
/// `meeting.json`. `.restore-<id>` folders don't count.
pub(crate) fn meeting_dirs(root: &Path) -> Vec<MeetingDir> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<MeetingDir> = entries
        .filter_map(Result::ok)
        .filter_map(|e| MeetingDir::at(&e.path()))
        .filter(|d| d.path().join(voice_meetings::MEETING_FILE).is_file())
        .collect();
    dirs.sort_by(|a, b| b.id().as_str().cmp(a.id().as_str()));
    dirs
}

/// Whether the meeting folder holds media retention would remove: `audio/` and the screen media
/// (`MeetingDir::has_media`).
pub(crate) fn has_media(meeting_dir: &Path) -> bool {
    MeetingDir::at(meeting_dir).is_some_and(|dir| dir.has_media())
}

/// `VACUUM INTO` a fresh file: a consistent copy of a database another process writes.
fn snapshot(db: &Path, into: &Path) -> Result<(), SyncError> {
    use rusqlite::{Connection, OpenFlags};
    let fail = |e: rusqlite::Error| {
        SyncError::new(
            SyncErrorKind::Local,
            format!("Voice can't copy a transcript: {e}"),
        )
    };
    let conn = Connection::open_with_flags(
        db,
        OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(fail)?;
    conn.busy_timeout(Duration::from_secs(5)).map_err(fail)?;
    conn.execute("VACUUM INTO ?1", [into.to_string_lossy().as_ref()])
        .map_err(fail)?;
    Ok(())
}

/// The ciphertext bytes of part `n` (from 1).
fn part_len(total: u64, part_size: u64, n: u64) -> u64 {
    let offset = (n - 1) * part_size;
    part_size.min(total.saturating_sub(offset))
}

/// SHA-256 of a file as hex, and how many bytes it had.
pub(crate) fn sha256_file(path: &Path) -> io::Result<(String, u64)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut len = 0u64;
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        len += n as u64;
    }
    Ok((hex(&hasher.finalize()), len))
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn folder_size(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .filter(fs::Metadata::is_file)
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

pub(crate) fn millis(d: Duration) -> u64 {
    u64::try_from(d.as_millis()).unwrap_or(u64::MAX)
}

fn to_usize(n: u64) -> usize {
    usize::try_from(n).unwrap_or(usize::MAX)
}

pub(crate) fn local(what: &str, e: &io::Error) -> SyncError {
    SyncError::new(SyncErrorKind::Local, format!("{what}: {e}"))
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn the_worker_thread_lowers_its_quality_of_service() {
        let class = std::thread::spawn(|| {
            super::background_thread();
            let mut class = libc::qos_class_t::QOS_CLASS_DEFAULT;
            let mut priority = 0;
            // SAFETY: `pthread_self` names this thread, and both out-pointers are valid.
            let result = unsafe {
                libc::pthread_get_qos_class_np(libc::pthread_self(), &mut class, &mut priority)
            };
            assert_eq!(result, 0);
            class
        })
        .join()
        .expect("the thread ran");
        assert_eq!(class as u32, libc::qos_class_t::QOS_CLASS_UTILITY as u32);
    }
}
