//! Bucket sync in the engine (spec RUK-110 2.3, 2.10, 4.5): the connection from `config.json`,
//! moving an old plain secret into DPAPI, Settings' `sync` state, retention's hold with its
//! 10 GiB floor, and the notifications.
//!
//! voice-sync does the work on its own threads; nothing here holds the controller's lock or waits
//! for the network, except the Settings commands the app runs on its blocking pool.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use voice_core::config::Config;
use voice_meetings::{release_held, MediaPart};
use voice_sync::keys::KeyStore;
use voice_sync::protect::Protector;
use voice_sync::{BucketSync, LastError, SyncConfig, SyncError, SyncState, SyncStatus};

use crate::engine::{lock, Core};
use crate::meetings::now_ms;

/// The notification about bucket sync (voice/ui/i18n `app.notify.sync`).
pub const NOTIFY_SYNC: &str = "app.notify.sync";
/// Retention holds media for the bucket only while the meetings volume has this much free.
pub const HOLD_FLOOR: u64 = 10 << 30;
/// Bucket sync's settings in `config.json`, under `extra` (voice/ui/API.md, `storage.sync.*`).
pub const SYNC_KEY: &str = "storage_sync";

const DAY_MS: u64 = 86_400_000;
const DAY: Duration = Duration::from_millis(DAY_MS);

const FAILING_BODY: &str = "Voice couldn't upload to your bucket for a day. Your audio stays on this computer until it can, as long as 10 GB stay free.";
const LOST_BODY: &str = "The disk was almost full, so Voice removed meeting media that hadn't reached your bucket yet. Settings > Storage lists how much.";
const UNREADABLE_BODY: &str = "The disk was almost full, so Voice removed meeting media it couldn't check against your bucket.";

/// What the engine remembers about sync between calls.
#[derive(Default)]
pub(crate) struct SyncRuntime {
    /// When each notification last went out, on the monotonic clock, so a wall clock set back
    /// doesn't silence the next one.
    failing_note: Mutex<Option<Instant>>,
    lost_note: Mutex<Option<Instant>>,
    /// The last sweep held media for the bucket; a sync change may let it go now.
    held: AtomicBool,
    /// Sweep again soon: sync changed while media was held.
    resweep: AtomicBool,
    /// A Settings state for a sync change is on its way.
    pub status_pending: AtomicBool,
    /// Why the sync store didn't open at start, for Error details while Voice runs without it.
    pub open_error: Mutex<Option<LastError>>,
    /// Held while the engine makes, shows or replaces the encryption key. voice-sync's
    /// load-or-create isn't atomic: two calls at once could each make a key, and the code one of
    /// them shows would then open nothing that uploads. An import at that moment could be
    /// overwritten by a new key the same way.
    pub key: Mutex<()>,
    /// Held while Settings save or remove the bucket secret, from clearing the passed Test
    /// connection to changing the secret, and while a passed test commits.
    pub secret: Mutex<()>,
}

impl SyncRuntime {
    /// Sync's status changed: held media may have reached the bucket.
    pub fn changed(&self) {
        if self.held.load(Ordering::SeqCst) {
            self.resweep.store(true, Ordering::SeqCst);
        }
    }

    /// The sync settings changed: retention looks again soon.
    pub fn sweep_soon(&self) {
        self.resweep.store(true, Ordering::SeqCst);
    }

    /// Whether a sweep is due because of a sync change; clears the flag.
    pub fn take_resweep(&self) -> bool {
        self.resweep.swap(false, Ordering::SeqCst)
    }

    /// The sync store didn't open: Settings says why under Error details.
    pub fn store_failed(&self, error: &SyncError) {
        *lock(&self.open_error) = Some(LastError {
            at: now_ms(),
            code: error.code().to_owned(),
            message: error.message.clone(),
        });
    }
}

/// `storage_sync` from `config.json`, or an empty object.
pub fn sync_value(cfg: &Config) -> Value {
    cfg.extra
        .get(SYNC_KEY)
        .filter(|v| v.is_object())
        .cloned()
        .unwrap_or_else(|| json!({}))
}

/// Whether the switch is on in Settings.
pub fn switch_on(cfg: &Config) -> bool {
    sync_value(cfg)["enabled"].as_bool().unwrap_or(false)
}

/// Whether `storage_sync` holds the fingerprint of its own fields, which only a passed Test
/// connection writes.
pub fn is_tested(sync: &Value) -> bool {
    let Some(tested) = sync.get("tested").and_then(Value::as_str) else {
        return false;
    };
    SyncConfig::from_json(sync).is_ok_and(|cfg| cfg.fingerprint() == tested)
}

/// The bucket to sync with: the switch is on and the fields passed Test connection. Settings
/// never turn it on otherwise; this also covers a hand-edited `config.json`.
pub fn sync_config(cfg: &Config) -> Option<SyncConfig> {
    let sync = sync_value(cfg);
    if !sync["enabled"].as_bool().unwrap_or(false) || !is_tested(&sync) {
        return None;
    }
    SyncConfig::from_json(&sync).ok()
}

/// Tells bucket sync what Settings say now.
pub(crate) fn configure_sync(core: &Core, cfg: &Config) {
    let Some(sync) = &core.sync else {
        return;
    };
    let bucket = sync_config(cfg);
    if bucket.is_some() {
        // The key exists before the worker connects, so the worker and Show key never both make
        // one: voice-sync's load-or-create isn't atomic, and two keys made at once would leave
        // uploads under a key that isn't saved. Nothing uploads here, so importing a key stays
        // allowed.
        let made = {
            let _key = lock(&core.sync_rt.key);
            sync.show_key()
        };
        if let Err(e) = made {
            log::warn!("sync: no encryption key yet: {}", e.message);
        }
    }
    sync.configure(bucket);
}

/// Moves a bucket secret that `settings_set` could write into `config.json` before Voice kept
/// it with DPAPI into `<data>/sync/bucket-secret.bin` (spec 2.3). Returns whether `cfg` changed;
/// the caller saves it. When the secret can't be stored it stays where it is, so it isn't lost.
pub fn migrate_secret(
    cfg: &mut Config,
    sync: Option<&BucketSync>,
    data_dir: &std::path::Path,
    protector: std::sync::Arc<dyn Protector>,
) -> bool {
    let Some(object) = cfg.extra.get_mut(SYNC_KEY).and_then(Value::as_object_mut) else {
        return false;
    };
    let Some(secret) = object.get("secret").and_then(Value::as_str) else {
        return false;
    };
    if secret.is_empty() {
        object.remove("secret");
        return true;
    }
    let secret = voice_sync::Zeroizing::new(secret.to_string());
    let saved = match sync {
        Some(sync) => sync.save_secret(&secret),
        None => KeyStore::new(&data_dir.join(voice_sync::SYNC_DIR), protector).save_secret(&secret),
    };
    match saved {
        Ok(()) => {
            object.remove("secret");
            log::info!("sync: moved the bucket secret out of config.json");
            true
        }
        Err(e) => {
            log::warn!(
                "sync: the bucket secret stays in config.json: {}",
                e.message
            );
            false
        }
    }
}

/// Settings' `sync` (voice/ui/API.md): voice-sync's status plus `screenRecording`, and `paused`
/// for a switch that is on while the connection waits for a new Test connection. Without the
/// sync store, `lastError` says why it didn't open.
pub(crate) fn sync_view(core: &Core) -> Value {
    let status = match &core.sync {
        Some(sync) => sync.status(),
        None => SyncStatus {
            available: false,
            last_error: lock(&core.sync_rt.open_error).clone(),
            ..SyncStatus::default()
        },
    };
    let paused = status.state == SyncState::Off && switch_on(&core.config());
    let mut view = serde_json::to_value(&status).unwrap_or_else(|_| json!({}));
    if paused {
        view["state"] = json!("paused");
    }
    // A platform whose protector can't keep the keys from other users offers no bucket sync, whatever
    // store opened here: Settings show the sync card only where this is true (voice/ui/API.md,
    // `sync.available` and `features.sync`).
    if !voice_sync::protect::SUPPORTED {
        view["available"] = json!(false);
    }
    // API.md shows the key's first 8 hex characters, enough to tell keys apart.
    if let Some(id) = view["key"]["id"].as_str() {
        view["key"]["id"] = json!(short_id(id));
    }
    view["screenRecording"] = json!(core.screen_recording());
    view
}

/// A key id as Settings show it: 8 hex characters.
pub fn short_id(id: &str) -> String {
    id.chars().take(8).collect()
}

/// An error of voice-sync as the page gets it: its code and English text.
pub fn api_error(error: &SyncError) -> crate::api::ApiError {
    crate::api::ApiError::new(error.code(), error.message.clone())
}

/// Retention's hold for one meeting (spec 2.10): voice-sync's answer while sync is open, which is
/// `true` on any error; with the switch on but the queue unreadable, every meeting with media.
///
/// The hold follows the switch, not whether uploads run: a changed connection field or secret
/// pauses uploads until Test connection passes again, and repairing a failing bucket that way must
/// never cost the media it was holding. Only turning the switch off lets that media go (D8).
pub fn holds(sync: Option<&BucketSync>, switch_on: bool, meeting: &str) -> bool {
    match sync {
        Some(sync) => sync.holds_media(meeting, switch_on),
        // The sweep asks only about meetings with media.
        None => switch_on,
    }
}

/// Below [`HOLD_FLOOR`] free, releases media retention held for the bucket: oldest meeting
/// first, one part at a time, checking free space after each. Media that is in the bucket goes
/// first (held only for the 24 hours after a restore used it), then video, frames and audio of
/// meetings with files that never reached the bucket, whose rows become `lost`. Returns whether
/// anything went.
pub(crate) fn release_below_floor(core: &Core, held: &[String]) -> bool {
    let lib = core.meetings.library();
    let free = || core.svc.system.free_bytes(lib.root());
    if held.is_empty() || free().is_none_or(|free| free >= HOLD_FLOOR) {
        return false;
    }
    let sync = core.sync.as_deref();
    let reached = |id: &str, path: &str| sync.is_some_and(|s| s.in_bucket(id, path));
    let (in_bucket, waiting): (Vec<&String>, Vec<&String>) = held.iter().partition(|id| {
        lib.dir(id)
            .is_some_and(|dir| all_in_bucket(sync, id, dir.path()))
    });
    let mut released = false;
    // Rows marked lost, and files removed that weren't in the bucket.
    let mut lost = 0;
    let mut unsynced = 0;
    'meetings: for id in in_bucket.into_iter().chain(waiting) {
        let Some(dir) = lib.dir(id) else {
            continue;
        };
        for part in MediaPart::ORDER {
            if free().is_none_or(|free| free >= HOLD_FLOOR) {
                break 'meetings;
            }
            let missing = part
                .files(dir.path())
                .iter()
                .filter(|p| !reached(id, p))
                .count();
            match release_held(lib, id, part) {
                Ok(0) => continue,
                Ok(bytes) => {
                    released = true;
                    unsynced += missing;
                    log::warn!(
                        "retention: the disk is almost full; removed {} of {id} ({bytes} bytes, {missing} files not in the bucket)",
                        part.kind()
                    );
                    if let Some(sync) = sync {
                        lost += sync.mark_lost(id, &[part.kind()]);
                    }
                }
                Err(e) => log::warn!("retention: {} of {id} stays: {e}", part.kind()),
            }
        }
    }
    if lost > 0 || unsynced > 0 {
        // Without rows to count, Settings can't say how much: sync's store didn't open or its
        // queue can't be read.
        let body = if lost > 0 { LOST_BODY } else { UNREADABLE_BODY };
        notify_daily(core, &core.sync_rt.lost_note, body);
    }
    released
}

/// Whether every media file the meeting still has reached the bucket, so releasing it loses
/// nothing. `has_waiting` can't say: it also counts the transcript and the manifest, and answers
/// `true` when the queue errors. So each media file is checked. Without sync nothing counts as
/// in the bucket.
fn all_in_bucket(sync: Option<&BucketSync>, id: &str, meeting_dir: &std::path::Path) -> bool {
    MediaPart::ORDER.iter().all(|part| {
        part.files(meeting_dir)
            .iter()
            .all(|path| sync.is_some_and(|s| s.in_bucket(id, path)))
    })
}

/// After a day of failing uploads, one notification a day (spec 2.15).
pub(crate) fn check_failing(core: &Core) {
    let Some(sync) = &core.sync else {
        return;
    };
    let status = sync.status();
    let now = now_ms();
    let failing_a_day = status.state == SyncState::Failing
        && status
            .failing_since
            .is_some_and(|since| now.saturating_sub(since) >= DAY_MS);
    if failing_a_day {
        notify_daily(core, &core.sync_rt.failing_note, FAILING_BODY);
    }
}

fn notify_daily(core: &Core, last: &Mutex<Option<Instant>>, body: &str) {
    {
        let mut last = lock(last);
        if last.is_some_and(|at| at.elapsed() < DAY) {
            return;
        }
        *last = Some(Instant::now());
    }
    core.notify(NOTIFY_SYNC, body.to_string());
}

/// Remembers whether the last sweep held media, so a sync change can sweep again.
pub(crate) fn note_held(core: &Core, held: bool) {
    core.sync_rt.held.store(held, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_that_was_never_saved_isnt_carried_over() {
        let mut cfg = Config::defaults(true);
        assert!(!migrate_secret(
            &mut cfg,
            None,
            std::path::Path::new("unused"),
            std::sync::Arc::new(voice_sync::protect::TestProtector),
        ));
        cfg.extra
            .insert(SYNC_KEY.into(), json!({"bucket": "b", "secret": ""}));
        assert!(migrate_secret(
            &mut cfg,
            None,
            std::path::Path::new("unused"),
            std::sync::Arc::new(voice_sync::protect::TestProtector),
        ));
        assert_eq!(sync_value(&cfg), json!({"bucket": "b"}));
    }

    #[test]
    fn untested_or_switched_off_settings_never_reach_sync() {
        let mut cfg = Config::defaults(true);
        assert!(!switch_on(&cfg));
        assert_eq!(sync_config(&cfg), None);
        cfg.extra
            .insert(SYNC_KEY.into(), json!({"enabled": true, "bucket": "b"}));
        assert!(switch_on(&cfg));
        assert!(!is_tested(&sync_value(&cfg)), "no fingerprint");
        assert_eq!(sync_config(&cfg), None);
        assert_eq!(short_id("0f1e2d3c4b5a6978"), "0f1e2d3c");
    }

    #[test]
    fn without_sync_the_hold_follows_the_switch() {
        assert!(holds(None, true, "m-20261030-171203-4f2a"));
        assert!(!holds(None, false, "m-20261030-171203-4f2a"));
    }

    #[test]
    fn media_counts_as_in_the_bucket_file_by_file() {
        let data = std::env::temp_dir().join(format!("voice-engine-floor-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&data);
        let meetings = data.join("meetings");
        let id = "m-20261030-171203-4f2a";
        let dir = meetings.join(id);
        std::fs::create_dir_all(dir.join("audio").join("mic")).unwrap();
        let sync = BucketSync::open(voice_sync::SyncOptions {
            data_dir: data.clone(),
            meetings_dir: meetings.clone(),
            protector: std::sync::Arc::new(voice_sync::protect::TestProtector),
            screen: std::sync::Arc::new(voice_sync::NoScreen),
            now: std::sync::Arc::new(now_ms),
            reserve: std::sync::Arc::new(|_| true),
            timing: voice_sync::SyncTiming::default(),
            on_change: std::sync::Arc::new(|| {}),
        })
        .unwrap();

        let mic = dir.join("audio").join("mic");
        // Nothing to lose: no media files, or only a fetch that never finished.
        assert!(all_in_bucket(Some(&sync), id, &dir));
        std::fs::write(mic.join("000001.wav.restore"), b"RI").unwrap();
        assert!(all_in_bucket(Some(&sync), id, &dir));
        // A segment without a done row isn't in the bucket.
        std::fs::write(mic.join("000001.wav"), b"RIFF").unwrap();
        assert!(!all_in_bucket(Some(&sync), id, &dir));
        // Without sync nothing can be checked, so nothing counts as in the bucket.
        assert!(!all_in_bucket(None, id, &dir));

        sync.shutdown();
        drop(sync);
        let _ = std::fs::remove_dir_all(&data);
    }
}
