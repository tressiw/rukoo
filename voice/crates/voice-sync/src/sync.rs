//! The facade the engine uses: one [`BucketSync`] per data folder.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::keys::{KeyStore, SECRET_FILE};
use crate::protect::Protector;
use crate::queue::Queue;
use crate::s3::Bucket;
use crate::worker::{self, Shared};
use crate::{recovery, ScreenFiles, SyncConfig, SyncError, SyncErrorKind, UrlStyle, SYNC_DIR};

/// The worker's clocks, rates and sizes. Tests shorten them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncTiming {
    /// How often meetings that record or haven't synced are scanned.
    pub scan_live: Duration,
    /// How often every meeting is scanned.
    pub scan_all: Duration,
    /// Bytes per second while a meeting records.
    pub live_rate: u64,
    pub backoff_min: Duration,
    pub backoff_max: Duration,
    /// Errors that stop all uploads (auth, bucket, permission, clock, key) wait this long before
    /// Voice tries again, unless the configuration, the secret or the key changes first (2.9).
    pub blocked: Duration,
    /// A live meeting's manifest at most this often.
    pub manifest_live: Duration,
    /// The index this long after the last manifest changed.
    pub index_debounce: Duration,
    /// Bucket retention runs this often.
    pub prune_every: Duration,
    pub connect_timeout: Duration,
    pub request_timeout: Duration,
    /// Multipart part size; every part but the last has exactly this size.
    pub part_size: u64,
    /// Larger ciphertexts upload in parts.
    pub single_max: u64,
}

impl Default for SyncTiming {
    fn default() -> SyncTiming {
        SyncTiming {
            scan_live: Duration::from_secs(10),
            scan_all: Duration::from_secs(10 * 60),
            live_rate: 163_840,
            backoff_min: Duration::from_secs(2),
            backoff_max: Duration::from_secs(5 * 60),
            blocked: Duration::from_secs(15 * 60),
            manifest_live: Duration::from_secs(5 * 60),
            index_debounce: Duration::from_secs(60),
            prune_every: Duration::from_secs(24 * 60 * 60),
            connect_timeout: Duration::from_secs(10),
            request_timeout: Duration::from_secs(120),
            part_size: 8 << 20,
            single_max: 16 << 20,
        }
    }
}

#[derive(Clone)]
pub struct SyncOptions {
    /// <data>; sync keeps its files in <data>/sync.
    pub data_dir: PathBuf,
    /// Library::root() (library.rs:168).
    pub meetings_dir: PathBuf,
    pub protector: Arc<dyn Protector>,
    pub screen: Arc<dyn ScreenFiles>,
    /// Wall ms.
    pub now: Arc<dyn Fn() -> u64 + Send + Sync>,
    /// May this many bytes be written now (reserve_ok, contract 2.2).
    pub reserve: Arc<dyn Fn(u64) -> bool + Send + Sync>,
    pub timing: SyncTiming,
    /// Status changed; called at most twice a second.
    pub on_change: Arc<dyn Fn() + Send + Sync>,
}

/// `sync` in Settings' state (voice/ui/API.md). The engine adds `screenRecording`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncStatus {
    pub available: bool,
    pub state: SyncState,
    pub waiting: u64,
    pub waiting_bytes: u64,
    pub oldest_waiting: Option<u64>,
    pub last_upload: Option<u64>,
    pub last_error: Option<LastError>,
    pub failing_since: Option<u64>,
    /// Files removed locally before they reached the bucket (2.10).
    pub lost: u64,
    pub key: KeyInfo,
    pub restore: RestoreState,
    pub other_keys: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum SyncState {
    #[default]
    Off,
    Idle,
    Uploading,
    Live,
    Retrying,
    Failing,
    /// Voice can't open its key or the bucket secret: `lastError` says why, `key` for a key
    /// that is missing or damaged, `key_denied` for one the system wouldn't open now.
    Key,
    /// The worker waits for the protector to open the key or the secret: on macOS, for the
    /// answer to the keychain dialog. Nothing else waits for it.
    Unlocking,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct LastError {
    pub at: u64,
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct KeyInfo {
    /// Voice has a key and, the last time it read it, could open it.
    pub exists: bool,
    pub id: Option<String>,
    pub exported: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct RestoreState {
    /// idle | listing | available | restoring | done | failed
    pub state: &'static str,
    pub available: u32,
    pub done: u32,
    pub total: u32,
    pub error: Option<String>,
}

impl Default for RestoreState {
    fn default() -> RestoreState {
        RestoreState {
            state: "idle",
            available: 0,
            done: 0,
            total: 0,
            error: None,
        }
    }
}

/// A meeting in the bucket that isn't on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteMeeting {
    pub id: String,
    pub title: String,
    pub started_at: u64,
    pub duration_ms: Option<u64>,
}

/// What a passed Test connection found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TestReport {
    /// The URL style that worked; the engine stores it as `urlStyle`.
    pub url_style: UrlStyle,
    /// Other encryption keys with meetings in this bucket.
    pub other_keys: u32,
    /// [`BucketSync::secret_generation`] when the test read the secret. A different one now
    /// means the secret was saved or removed during the test, which then tested another secret.
    pub secret_generation: u64,
}

pub struct BucketSync {
    shared: Arc<Shared>,
}

/// `Core::shutdown` waits this long for the worker (2.9).
const SHUTDOWN_WAIT: Duration = Duration::from_secs(2);
/// The key id Settings shows: its first 8 hex characters (voice/ui/API.md).
const SHOWN_KID: usize = 8;

impl BucketSync {
    /// Opens `<data>/sync/queue.db`, removes `.restore-<id>` folders a hard kill left, and starts
    /// the `sync-worker` thread, which stays idle until [`BucketSync::configure`] turns sync on.
    /// Fails when the queue is locked, damaged or from a newer Voice; Voice then runs without sync
    /// and retention holds every meeting with media.
    pub fn open(opts: SyncOptions) -> Result<BucketSync, SyncError> {
        let sync_dir = opts.data_dir.join(SYNC_DIR);
        let queue = Queue::open(&sync_dir)?;
        crate::restore::clear_partial(&opts.meetings_dir);
        let keys = KeyStore::new(&sync_dir, opts.protector.clone());
        let shared = Arc::new(Shared::new(opts, sync_dir, queue, keys));
        worker::spawn(shared.clone())
            .map_err(|e| worker::local("Voice can't start the sync worker", &e))?;
        Ok(BucketSync { shared })
    }

    /// The state the worker shares, for this crate's unit tests.
    #[cfg(test)]
    pub(crate) fn shared(&self) -> &Arc<Shared> {
        &self.shared
    }

    /// None turns sync off. Wakes the worker. Turning it on after it was off makes the meetings
    /// that exist now a backfill, newest first (D7). Settings hear of it only when the
    /// configuration or the on/off state changed, so a start with sync off sends them nothing.
    pub fn configure(&self, cfg: Option<SyncConfig>) {
        let shared = &self.shared;
        let on = cfg.is_some();
        let was_on = shared.queue.meta("on").ok().flatten().as_deref() == Some("1");
        let switched = on != was_on;
        if on && !was_on {
            let now = shared.now().to_string();
            let _ = shared.queue.set_meta("enabled_at", Some(&now));
            let _ = shared.queue.set_meta("on", Some("1"));
        } else if !on && was_on {
            let _ = shared.queue.set_meta("on", Some("0"));
        }
        let changed = {
            let mut st = shared.lock();
            let changed = st.cfg != cfg;
            // On, off or another bucket, endpoint or folder: a restore of the old place stops.
            let target = |cfg: &Option<SyncConfig>| cfg.as_ref().map(SyncConfig::target);
            if target(&st.cfg) != target(&cfg) {
                st.place += 1;
            }
            st.cfg = cfg;
            st.rescan = true;
            changed
        };
        if changed {
            shared.bump();
        } else {
            shared.poke();
        }
        if changed || switched {
            shared.changed();
        }
    }

    /// Probes `cfg.url_style`, then the other style when the first fails with bucket, not found or
    /// endpoint. Uses the saved secret (NoSecret without one).
    pub fn test(&self, cfg: &SyncConfig) -> Result<TestReport, SyncError> {
        let shared = &self.shared;
        // Read before the secret: a save in between then counts as a change, never the reverse.
        let secret_generation = self.secret_generation();
        let secret = shared.keys.secret()?.ok_or_else(|| {
            SyncError::new(SyncErrorKind::NoSecret, "Save the secret access key first.")
        })?;
        let own_kid = shared.keys.load().ok().flatten().map(|k| k.kid());
        let timing = &shared.opts.timing;
        let probe = |style: UrlStyle| -> Result<u32, SyncError> {
            let mut cfg = cfg.clone();
            cfg.url_style = style;
            Bucket::new(
                &cfg,
                &secret,
                timing.connect_timeout,
                timing.request_timeout,
            )?
            .probe(own_kid.as_deref())
        };
        let first = cfg.url_style;
        let (url_style, other_keys) = match probe(first) {
            Ok(n) => (first, n),
            Err(e)
                if matches!(
                    e.kind,
                    SyncErrorKind::Bucket | SyncErrorKind::NotFound | SyncErrorKind::Endpoint
                ) =>
            {
                let other = match first {
                    UrlStyle::Path => UrlStyle::VirtualHost,
                    UrlStyle::VirtualHost => UrlStyle::Path,
                };
                match probe(other) {
                    Ok(n) => (other, n),
                    Err(_) => return Err(e),
                }
            }
            Err(e) => return Err(e),
        };
        let _ = shared
            .queue
            .set_meta("other_keys", Some(&other_keys.to_string()));
        // A passed test of the bucket sync uploads to ends the stop an error caused: the user set
        // the clock right or fixed the key's permissions and tested again, as Settings asked.
        let unblocked = {
            let mut st = shared.lock();
            let active = st.cfg.as_ref().is_some_and(|active| {
                SyncConfig {
                    url_style: active.url_style,
                    ..cfg.clone()
                } == *active
            });
            let unblocked = active && st.blocked.is_some();
            if unblocked {
                st.blocked = None;
                st.failures = 0;
            }
            unblocked
        };
        if unblocked {
            shared.poke();
        }
        shared.changed();
        Ok(TestReport {
            url_style,
            other_keys,
            secret_generation,
        })
    }

    pub fn save_secret(&self, secret: &str) -> Result<(), SyncError> {
        self.shared.keys.save_secret(secret)?;
        self.shared.secret_generation.fetch_add(1, Ordering::SeqCst);
        self.shared.bump();
        self.shared.changed();
        Ok(())
    }

    pub fn remove_secret(&self) -> Result<(), SyncError> {
        self.shared.keys.remove_secret()?;
        self.shared.secret_generation.fetch_add(1, Ordering::SeqCst);
        self.shared.bump();
        self.shared.changed();
        Ok(())
    }

    /// Grows with every save or removal of the bucket secret in this run (see [`TestReport`]).
    pub fn secret_generation(&self) -> u64 {
        self.shared.secret_generation.load(Ordering::SeqCst)
    }

    pub fn secret_saved(&self) -> bool {
        self.shared.sync_dir.join(SECRET_FILE).is_file()
    }

    /// (code, id); creates the key when there is none.
    pub fn show_key(&self) -> Result<(String, String), SyncError> {
        let shared = &self.shared;
        let (key, created) = shared.key_or_create(shared.now())?;
        if created {
            shared.queue.adopt_kid(&key.kid())?;
            shared.bump();
        }
        shared.changed();
        Ok((recovery::encode(&key), shown(&key.kid())))
    }

    pub fn mark_key_saved(&self) -> Result<(), SyncError> {
        self.shared.keys.mark_exported(self.shared.now())?;
        self.shared.changed();
        Ok(())
    }

    /// Refused with KeyInUse once anything was uploaded with this computer's key and that key
    /// still opens (D5).
    pub fn import_key(&self, code: &str) -> Result<String, SyncError> {
        let shared = &self.shared;
        let key = recovery::decode(code)?;
        let kid = key.kid();
        {
            let _busy = shared.busy();
            let _key = shared.key_lock();
            match shared.keys.load() {
                Ok(Some(current)) if current.kid() == kid => return Ok(shown(&kid)),
                Ok(Some(current)) => {
                    if shared.queue.uploaded_with(&current.kid())? {
                        return Err(SyncError::new(
                            SyncErrorKind::KeyInUse,
                            "This computer already uploaded meetings with its own key, so it keeps that key.",
                        ));
                    }
                }
                Ok(None) => {}
                // The old key can't be opened any more: nothing it uploaded can be read here anyway.
                // A key the system only refused now (`KeyDenied`) still opens once allowed, so
                // that error comes back instead: replacing the key would skip the check above.
                Err(e) if e.kind == SyncErrorKind::Key => {}
                Err(e) => return Err(e),
            }
            shared.keys.replace(&key, shared.now())?;
            shared.note_key_read(None);
            shared.queue.adopt_kid(&kid)?;
            let mut st = shared.lock();
            st.restore = RestoreState::default();
            // Under `busy`, so a restore reading with the old key never imports rows after this.
            st.place += 1;
        }
        shared.bump();
        shared.changed();
        Ok(shown(&kid))
    }

    /// Reads the key and the secret again now, and tries the bucket again: Settings' Try again
    /// after the user refused Voice in the keychain dialog, or after anything else stopped
    /// uploads. A refused keychain is asked only then, at a restart, or when the settings, the
    /// secret or the key change.
    pub fn retry(&self) {
        self.shared.bump();
        self.shared.changed();
    }

    pub fn status(&self) -> SyncStatus {
        let shared = &self.shared;
        let (on, blocked, failures, last_error, restore, unlocking, key_unreadable) = {
            let st = shared.lock();
            (
                st.cfg.is_some(),
                st.blocked.clone(),
                st.failures,
                st.last_error.clone(),
                st.restore.clone(),
                st.unlocking,
                st.key_unreadable,
            )
        };
        let queue = &shared.queue;
        let (waiting, waiting_bytes, oldest_waiting) = queue.waiting().unwrap_or((0, 0, None));
        let live = queue.any_live().unwrap_or(false);
        let state = if !on {
            SyncState::Off
        } else if unlocking {
            // A new read is under way; whatever stopped uploads before may be over with it.
            SyncState::Unlocking
        } else if let Some(blocked) = &blocked {
            if blocked.error.kind.is_key() {
                SyncState::Key
            } else {
                SyncState::Failing
            }
        } else if failures > 0 {
            SyncState::Retrying
        } else if live {
            SyncState::Live
        } else if waiting > 0 || queue.pending().unwrap_or(true) {
            // A manifest or the index still to go counts: idle means everything is in the bucket.
            SyncState::Uploading
        } else {
            SyncState::Idle
        };
        let info = shared.keys.info();
        let failing_since = queue.meta_u64("failing_since").ok().flatten();
        SyncStatus {
            available: true,
            state,
            waiting,
            waiting_bytes,
            oldest_waiting,
            last_upload: queue.last_upload().ok().flatten(),
            // After a restart the queue remembers the failure that hasn't been followed by a success.
            last_error: last_error.or_else(|| {
                failing_since?;
                let (at, code, message) = queue.last_error().ok().flatten()?;
                Some(LastError { at, code, message })
            }),
            failing_since,
            lost: queue.lost().unwrap_or(0),
            key: KeyInfo {
                exists: info.is_some() && !key_unreadable,
                id: info.as_ref().map(|k| shown(&k.kid)),
                exported: info.is_some_and(|k| k.exported_at.is_some()),
            },
            restore,
            other_keys: queue
                .meta_u64("other_keys")
                .ok()
                .flatten()
                .and_then(|n| u32::try_from(n).ok())
                .unwrap_or(0),
        }
    }

    /// Look at the meetings now: one stopped or became done.
    pub fn nudge(&self) {
        self.shared.lock().nudged = true;
        self.shared.poke();
    }

    /// Retention's hold (2.10). `sync_on` is the switch in Settings, which stays on while uploads
    /// pause for a new Test connection: repairing the bucket must never let held media go. `true`
    /// on a database error or a poisoned lock: a broken queue never lets media go.
    pub fn holds_media(&self, meeting: &str, sync_on: bool) -> bool {
        let shared = &self.shared;
        let has_media = worker::has_media(&shared.meeting_dir(meeting));
        // Rows of the old bucket say nothing about a new one until the worker starts again.
        if sync_on && has_media && shared.lock().moving() {
            return true;
        }
        shared
            .queue
            .holds_media(meeting, sync_on, has_media, shared.now())
            .unwrap_or(true)
    }

    /// Whether the meeting has files that never reached the bucket, counting transcript and manifest
    /// rows too. A meeting with no row yet counts as waiting. `true` on error, like holds_media.
    pub fn has_waiting(&self, meeting: &str) -> bool {
        self.shared.lock().moving() || self.shared.queue.has_waiting(meeting).unwrap_or(true)
    }

    /// Marks the meeting's waiting rows of these kinds `lost`: retention released them below the
    /// disk floor (2.10). How many.
    pub fn mark_lost(&self, meeting: &str, kinds: &[&str]) -> u64 {
        let n = self.shared.queue.mark_lost(meeting, kinds).unwrap_or(0);
        if n > 0 {
            self.shared.changed();
        }
        n
    }

    /// On, with a readable key and a saved secret.
    pub fn can_fetch(&self) -> bool {
        let shared = &self.shared;
        let ok = {
            let st = shared.lock();
            st.cfg.is_some()
                && !st.moving()
                && !st
                    .blocked
                    .as_ref()
                    .is_some_and(|b| b.error.kind.is_key())
        };
        ok && shared.keys.info().is_some() && self.secret_saved()
    }

    /// A link uses these files of the meeting from disk: the ones a restore or a fetch brought
    /// back stay at least 24 hours from now (D8, 2.10). Others are left alone.
    pub fn used(&self, meeting: &str, paths: &[String]) {
        let shared = &self.shared;
        let now = shared.now();
        for path in paths {
            if let Err(e) = shared.queue.touch_restored(meeting, path, now) {
                log::warn!("sync: can't note a use of restored media: {e}");
            }
        }
    }

    /// Whether the newest version of this file reached the bucket. Never while sync moves to
    /// another bucket and the queue hasn't started again for it.
    pub fn in_bucket(&self, meeting: &str, path: &str) -> bool {
        !self.shared.lock().moving()
            && matches!(self.shared.queue.in_bucket(meeting, path), Ok(Some(_)))
    }

    /// Fetches the files that aren't on disk with their row's size back from the bucket (2.11).
    /// A text file on disk is never replaced. Two calls for one file fetch it once. Fails with
    /// `off` when sync is off, `key` when the key can't open them, `not_in_bucket` without a row
    /// and `local` when `reserve` refuses one.
    pub fn ensure_local(
        &self,
        meeting: &str,
        paths: &[String],
        cancel: &AtomicBool,
    ) -> Result<(), SyncError> {
        crate::restore::ensure_local(&self.shared, meeting, paths, cancel)
    }

    /// The bucket's meetings that aren't on this computer, newest first. Settings' restore count
    /// follows.
    pub fn remote_meetings(&self) -> Result<Vec<RemoteMeeting>, SyncError> {
        let shared = &self.shared;
        let (remote, binding) = shared.remote()?;
        let restoring = shared.lock().restoring;
        if !restoring {
            shared.lock().restore.state = "listing";
            shared.changed();
        }
        let listed = crate::restore::list_remote(shared, &remote, &|| shared.still(&binding));
        if !restoring {
            {
                let mut st = shared.lock();
                match &listed {
                    Ok(meetings) => {
                        st.restore.state = "available";
                        st.restore.available = u32::try_from(meetings.len()).unwrap_or(u32::MAX);
                        st.restore.error = None;
                    }
                    Err(e) => {
                        st.restore.state = "failed";
                        st.restore.error = Some(e.message.clone());
                    }
                }
            }
            shared.changed();
        }
        listed
    }

    /// Restores `ids` (every meeting the bucket has that this computer doesn't, when empty) on the
    /// `sync-restore` thread and returns at once; progress shows in `status().restore`.
    /// `on_restored` gets each meeting's id once its folder is in place.
    pub fn restore(
        &self,
        ids: Vec<String>,
        on_restored: Box<dyn Fn(&str) + Send>,
    ) -> Result<(), SyncError> {
        crate::restore::spawn_restore(self.shared.clone(), ids, on_restored)
    }

    /// With `delete_objects`, deletes every object of the meeting from the bucket and marks it
    /// deleted in the next index; without, only forgets its rows (D15).
    pub fn forget_meeting(&self, meeting: &str, delete_objects: bool) -> Result<(), SyncError> {
        let _busy = self.shared.busy();
        crate::prune::forget(&self.shared, meeting, delete_objects)
    }

    /// A durable deletion, also offline: the next index tombstones the meeting. Encrypted
    /// objects remain subject to the bucket's lifecycle rules; restore cannot select them.
    pub fn mark_deleted(&self, meeting: &str) -> Result<(), SyncError> {
        if voice_protocol::MeetingId::parse(meeting).is_none() {
            return Err(SyncError::new(SyncErrorKind::Local, "Invalid meeting id."));
        }
        let shared = &self.shared;
        let _busy = shared.busy();
        shared.queue.drop_meeting(meeting, true)?;
        shared
            .queue
            .set_meta("index_due", Some(&shared.now().to_string()))?;
        shared.changed();
        shared.poke();
        Ok(())
    }

    /// Stops the worker between two requests and waits for it at most 2 s. An unfinished request
    /// is resumed next time.
    pub fn shutdown(&self) {
        self.signal_stop();
        let deadline = Instant::now() + SHUTDOWN_WAIT;
        let mut st = self.shared.lock();
        while !st.worker_exited {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                log::warn!("sync: the worker didn't stop within 2 s");
                break;
            }
            st = self
                .shared
                .wake
                .wait_timeout(st, left)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    fn signal_stop(&self) {
        self.shared.lock().stopping = true;
        self.shared.poke();
    }
}

impl Drop for BucketSync {
    fn drop(&mut self) {
        self.signal_stop();
    }
}

fn shown(kid: &str) -> String {
    kid.chars().take(SHOWN_KID).collect()
}
