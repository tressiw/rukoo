//! One Voice per data folder (decision 5): a second launch asks the running one to open Settings and
//! exits. The names include the data folder, so a development copy runs next to the installed one.
//!
//! Windows names a mutex and an event after the folder. Unix holds `flock(LOCK_EX | LOCK_NB)` on
//! `<data folder>/instance.lock` for the life of the process, which the kernel drops however the
//! process dies, so a lock file left by a crashed copy never blocks the next launch. The first copy
//! also listens on a Unix socket; a second copy connects, sends a short message and exits.
//!
//! The socket is `<data folder>/instance.sock` while that path fits in `sun_path` (104 bytes on macOS,
//! 108 on Linux, the terminator included). Data folders under a checkout's `tmp/` are longer than
//! that, so the fallback is `rukoo-voice-<hash>/instance.sock` in the per-user runtime folder
//! (`$XDG_RUNTIME_DIR` on Linux when it's set) or else the per-user temp folder (`$TMPDIR` on macOS),
//! named from the FNV-1a hash of the data folder's path without its trailing separator. The fallback
//! folder must be a real folder of this user's, made 0700, before a socket goes in it: a symlink or
//! another user's folder leaves this copy running without a listener, and a second launch then exits
//! after its wait, as it does when binding fails. Both copies work the path out from
//! `RUKOO_VOICE_DATA_DIR` alone, and neither depends on the current directory.

/// The FNV-1a hash of some bytes: stable across builds and platforms, unlike std's hasher. Used by
/// `names` and by the Unix fallback's folder; a platform with neither has no use for it.
#[cfg(any(windows, unix, test))]
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// The mutex and event names for a data folder. Windows paths ignore case and both slashes.
///
/// Windows only, plus the test that pins the names on every platform.
#[cfg(any(windows, test))]
pub fn names(data_dir: &std::path::Path) -> (String, String) {
    let path = data_dir.to_string_lossy().replace('/', "\\").to_lowercase();
    let path = path.trim_end_matches('\\');
    let hash = hash(path.as_bytes());
    (
        format!("Local\\RukooVoice-{hash:016x}"),
        format!("Local\\RukooVoice-{hash:016x}-settings"),
    )
}

pub enum Instance {
    First(Guard),
    /// Another Voice already runs on this data folder: Windows found its mutex, Unix found its lock
    /// file taken. `main.rs` signals it and exits.
    Running,
}

/// The first instance's lock. Windows keeps a mutex and an event; Unix keeps the locked file's
/// descriptor and, until `on_signal` takes it, the listening socket. The handles are never closed, so
/// the lock stays held until the process ends, however the guard itself goes.
pub struct Guard {
    #[cfg(windows)]
    event: isize,
    #[cfg(windows)]
    _mutex: isize,
    #[cfg(unix)]
    _lock: Option<std::os::unix::io::RawFd>,
    #[cfg(unix)]
    listener: std::sync::Mutex<Option<std::os::unix::net::UnixListener>>,
}

#[cfg(windows)]
mod imp {
    use std::path::Path;
    use std::time::{Duration, Instant};

    use windows::core::HSTRING;
    use windows::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
    use windows::Win32::System::Threading::{
        CreateEventW, CreateMutexW, OpenEventW, SetEvent, WaitForSingleObject, EVENT_MODIFY_STATE,
        INFINITE,
    };
    use windows::Win32::UI::WindowsAndMessaging::{AllowSetForegroundWindow, ASFW_ANY};

    use super::*;

    pub fn acquire(data_dir: &Path) -> Instance {
        let (mutex_name, event_name) = names(data_dir);
        unsafe {
            let mutex = match CreateMutexW(None, true, &HSTRING::from(mutex_name)) {
                Ok(mutex) => mutex,
                Err(error) => {
                    // Without the mutex there's no telling; run rather than refuse to start.
                    log::warn!("single instance: no mutex: {error}");
                    return Instance::First(Guard {
                        event: 0,
                        _mutex: 0,
                    });
                }
            };
            if GetLastError() == ERROR_ALREADY_EXISTS {
                return Instance::Running;
            }
            let event = CreateEventW(None, false, false, &HSTRING::from(event_name))
                .map(|h| h.0 as isize)
                .unwrap_or(0);
            Instance::First(Guard {
                event,
                _mutex: mutex.0 as isize,
            })
        }
    }

    /// Asks the running Voice to open Settings, and lets it come to the foreground.
    pub fn signal(data_dir: &Path) -> bool {
        let (_, event_name) = names(data_dir);
        let deadline = Instant::now() + Duration::from_secs(3);
        unsafe {
            let _ = AllowSetForegroundWindow(ASFW_ANY);
            loop {
                // The first instance may still be creating its event.
                if let Ok(event) = OpenEventW(
                    EVENT_MODIFY_STATE,
                    false,
                    &HSTRING::from(event_name.as_str()),
                ) {
                    let ok = SetEvent(event).is_ok();
                    let _ = windows::Win32::Foundation::CloseHandle(event);
                    return ok;
                }
                if Instant::now() >= deadline {
                    return false;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }

    impl Guard {
        /// Calls `on_signal` on its own thread each time another launch signals.
        pub fn on_signal(&self, on_signal: impl Fn() + Send + 'static) {
            let event = self.event;
            if event == 0 {
                return;
            }
            let _ = std::thread::Builder::new()
                .name("single-instance".into())
                .spawn(move || loop {
                    let result = unsafe { WaitForSingleObject(HANDLE(event as *mut _), INFINITE) };
                    if result.0 != 0 {
                        return;
                    }
                    on_signal();
                });
        }
    }
}

/// macOS and Linux: a lock file plus a socket, as `main.rs` and the module comment describe.
#[cfg(unix)]
mod imp {
    use std::fs::{self, DirBuilder, OpenOptions};
    use std::io::{Read, Write};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
    use std::os::unix::io::{IntoRawFd, RawFd};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, Instant};

    use super::*;

    const LOCK: &str = "instance.lock";
    const SOCKET: &str = "instance.sock";
    /// What a second launch sends. Anything else that finds the socket is ignored.
    const PING: &[u8] = b"settings\n";
    /// How long a second launch waits for the first one's socket, whose copy may still be starting or
    /// may have given up on listening. The whole second launch has to be gone within two seconds.
    const SIGNAL_WAIT: Duration = Duration::from_millis(1200);
    /// How long the listener waits for a connection's message before it takes it as a stray client.
    const PING_WAIT: Duration = Duration::from_millis(200);

    pub fn acquire(data_dir: &Path) -> Instance {
        let lock_path = data_dir.join(LOCK);
        let lock = match OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&lock_path)
        {
            Ok(file) => file,
            Err(error) => {
                // Without the lock file there's no telling; run rather than refuse to start.
                log::warn!("single instance: {}: {error}", lock_path.display());
                return Instance::First(Guard {
                    _lock: None,
                    listener: Mutex::new(None),
                });
            }
        };
        // The descriptor is never closed, so the lock outlives the guard, as the mutex handle does.
        let fd = lock.into_raw_fd();
        match try_lock(fd) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Instance::Running;
            }
            Err(error) => {
                log::warn!(
                    "single instance: no lock on {}: {error}",
                    lock_path.display()
                );
                return Instance::First(Guard {
                    _lock: Some(fd),
                    listener: Mutex::new(None),
                });
            }
        }
        Instance::First(Guard {
            _lock: Some(fd),
            listener: Mutex::new(listen(data_dir)),
        })
    }

    fn try_lock(fd: RawFd) -> std::io::Result<()> {
        // SAFETY: `fd` is open and stays open; flock only reads it.
        if unsafe { libc::flock(fd, libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    /// Binds the socket a second launch connects to. We hold the lock, so an existing socket file is
    /// stale and can go.
    fn listen(data_dir: &Path) -> Option<UnixListener> {
        let path = socket_path(data_dir);
        let dir = path.parent()?;
        if dir != data_dir && !private_dir(dir) {
            return None;
        }
        let _ = fs::remove_file(&path);
        match UnixListener::bind(&path) {
            Ok(listener) => {
                let _ = fs::set_permissions(&path, fs::Permissions::from_mode(0o600));
                Some(listener)
            }
            Err(error) => {
                log::warn!(
                    "single instance: can't listen on {}: {error}",
                    path.display()
                );
                None
            }
        }
    }

    /// The short fallback folder: made if it isn't there, then checked. Only a real folder of this
    /// user's, open to nobody else, may hold the socket; anything else gets no listener, and this copy
    /// still runs.
    fn private_dir(dir: &Path) -> bool {
        if let Err(error) = DirBuilder::new().recursive(true).mode(0o700).create(dir) {
            log::warn!("single instance: {}: {error}", dir.display());
            return false;
        }
        let meta = match fs::symlink_metadata(dir) {
            Ok(meta) => meta,
            Err(error) => {
                log::warn!("single instance: {}: {error}", dir.display());
                return false;
            }
        };
        if !meta.is_dir() {
            log::warn!(
                "single instance: {} isn't a folder; not listening there",
                dir.display()
            );
            return false;
        }
        // SAFETY: getuid only asks the kernel who this process belongs to.
        let ours = unsafe { libc::getuid() };
        if meta.uid() != ours {
            log::warn!(
                "single instance: {} belongs to user {}; not listening there",
                dir.display(),
                meta.uid()
            );
            return false;
        }
        if meta.mode() & 0o077 != 0 {
            if let Err(error) = fs::set_permissions(dir, fs::Permissions::from_mode(0o700)) {
                log::warn!(
                    "single instance: {} stays open to other users: {error}",
                    dir.display()
                );
                return false;
            }
        }
        true
    }

    /// Asks the running Voice to open Settings. The first copy may still be starting up or may have
    /// given up on its socket, so retry briefly; `main.rs` exits either way.
    pub fn signal(data_dir: &Path) -> bool {
        let path = socket_path(data_dir);
        let deadline = Instant::now() + SIGNAL_WAIT;
        loop {
            match UnixStream::connect(&path) {
                Ok(mut socket) => {
                    return socket.write_all(PING).is_ok() && socket.flush().is_ok();
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        log::warn!("single instance: {}: {error}", path.display());
                        return false;
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            }
        }
    }

    /// `<data folder>/instance.sock`, or the short fallback when that path doesn't fit in `sun_path`.
    pub(super) fn socket_path(data_dir: &Path) -> PathBuf {
        let in_data = data_dir.join(SOCKET);
        if fits(&in_data) {
            return in_data;
        }
        short_dir(data_dir).join(SOCKET)
    }

    /// `<short folder>/rukoo-voice-<hash>`, named from the data folder's path.
    fn short_dir(data_dir: &Path) -> PathBuf {
        short_base().join(format!("rukoo-voice-{:016x}", path_hash(data_dir)))
    }

    /// Where the fallback folder goes: Linux's per-user runtime folder when the session sets one, else
    /// the per-user temp folder. `/tmp` is shared, which is why the folder is checked before use.
    fn short_base() -> PathBuf {
        #[cfg(target_os = "linux")]
        if let Some(dir) = xdg_runtime_dir() {
            return dir;
        }
        std::env::temp_dir()
    }

    #[cfg(target_os = "linux")]
    fn xdg_runtime_dir() -> Option<PathBuf> {
        let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
        dir.is_absolute().then_some(dir)
    }

    /// The data folder's path without a trailing separator, hashed. Unix paths are case-sensitive,
    /// so this hash is too: folders that differ only in case get a folder each.
    fn path_hash(data_dir: &Path) -> u64 {
        let mut bytes = data_dir.as_os_str().as_bytes();
        while let [rest @ .., b'/'] = bytes {
            bytes = rest;
        }
        hash(bytes)
    }

    /// Whether `path` fits in `sun_path` with room for its terminating NUL.
    fn fits(path: &Path) -> bool {
        path.as_os_str().as_bytes().len() <= sun_path_capacity()
    }

    /// The longest path `sun_path` takes: its 104 bytes on macOS or 108 on Linux, less the NUL.
    fn sun_path_capacity() -> usize {
        std::mem::size_of::<libc::sockaddr_un>()
            - std::mem::offset_of!(libc::sockaddr_un, sun_path)
            - 1
    }

    impl Guard {
        /// Calls `on_signal` on its own thread each time another launch asks for Settings.
        pub fn on_signal(&self, on_signal: impl Fn() + Send + 'static) {
            let listener = match self.listener.lock() {
                Ok(mut slot) => slot.take(),
                Err(_) => None,
            };
            let Some(listener) = listener else {
                return;
            };
            let _ = thread::Builder::new()
                .name("single-instance".into())
                .spawn(move || {
                    for socket in listener.incoming() {
                        match socket {
                            Ok(socket) => {
                                if pinged(socket) {
                                    on_signal();
                                }
                            }
                            Err(error) => {
                                log::warn!("single instance: accept: {error}");
                                return;
                            }
                        }
                    }
                });
        }
    }

    /// Whether a client sent the message, rather than something else that found the socket.
    fn pinged(mut socket: UnixStream) -> bool {
        let _ = socket.set_read_timeout(Some(PING_WAIT));
        let mut buffer = [0u8; PING.len()];
        socket.read_exact(&mut buffer).is_ok() && buffer == *PING
    }
}

#[cfg(not(any(windows, unix)))]
mod imp {
    use std::path::Path;

    use super::*;

    pub fn acquire(_data_dir: &Path) -> Instance {
        Instance::First(Guard {})
    }

    pub fn signal(_data_dir: &Path) -> bool {
        false
    }

    impl Guard {
        pub fn on_signal(&self, _on_signal: impl Fn() + Send + 'static) {}
    }
}

pub use imp::{acquire, signal};

#[cfg(test)]
mod tests {
    use std::path::Path;
    #[cfg(any(windows, unix))]
    use std::path::PathBuf;

    use super::*;

    /// A scratch data folder of this test's own, so the tests don't find each other's lock.
    #[cfg(any(windows, unix))]
    fn temp(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("voice-app-single-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// A data folder deep enough that `<data folder>/instance.sock` is too long for `sun_path`, as a
    /// data folder under a worktree's `tmp/` can be. Such a folder gets the short fallback socket.
    #[cfg(unix)]
    fn long_data_folder(name: &str) -> PathBuf {
        let mut dir = temp(name);
        for _ in 0..3 {
            dir = dir.join("a-long-development-data-folder-name");
        }
        std::fs::create_dir_all(&dir).unwrap();
        assert!(dir.join("instance.sock").as_os_str().len() > 104);
        dir
    }

    #[test]
    fn names_ignore_case_slashes_and_a_trailing_separator() {
        let a = names(Path::new("C:\\Users\\Bart\\AppData\\Local\\Rukoo Voice"));
        assert_eq!(
            a,
            names(Path::new("c:/users/bart/appdata/local/rukoo voice/"))
        );
        assert!(a.0.starts_with("Local\\RukooVoice-") && a.1.ends_with("-settings"));
        assert_ne!(
            a,
            names(Path::new("E:\\Development\\rukoo\\tmp\\voice-dev"))
        );
    }

    /// The Windows check, run on Unix too: the second acquire finds the first, and the signal reaches
    /// the first copy's callback.
    #[cfg(any(windows, unix))]
    #[test]
    fn a_second_acquire_finds_the_first_and_signals_it() {
        let dir = temp("second-acquire");
        let Instance::First(guard) = acquire(&dir) else {
            panic!("the first acquire should win");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        guard.on_signal(move || {
            let _ = tx.send(());
        });
        assert!(matches!(acquire(&dir), Instance::Running));
        assert!(signal(&dir));
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first instance heard the signal");
        #[cfg(unix)]
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn another_folder_is_first() {
        use super::imp::socket_path;

        let one = temp("folder-one");
        let two = temp("folder-two");
        let Instance::First(_guard) = acquire(&one) else {
            panic!("the first acquire should win");
        };
        // The plain case: the socket sits in the data folder itself.
        assert_eq!(socket_path(&one), one.join("instance.sock"));
        assert!(matches!(acquire(&two), Instance::First(_)));
        let _ = std::fs::remove_dir_all(&one);
        let _ = std::fs::remove_dir_all(&two);
    }

    #[cfg(unix)]
    #[test]
    fn a_socket_and_lock_file_left_by_a_dead_process_do_not_block() {
        use std::os::unix::net::UnixListener;

        let dir = temp("stale");
        // What a killed copy leaves: its lock file, unlocked, and its socket file with no listener.
        std::fs::write(dir.join("instance.lock"), b"").unwrap();
        let socket = dir.join("instance.sock");
        drop(UnixListener::bind(&socket).unwrap());
        assert!(
            socket.exists(),
            "the stale socket file should still be there"
        );

        let Instance::First(guard) = acquire(&dir) else {
            panic!("a stale lock file and socket shouldn't block a new start");
        };
        let (tx, rx) = std::sync::mpsc::channel();
        guard.on_signal(move || {
            let _ = tx.send(());
        });
        assert!(matches!(acquire(&dir), Instance::Running));
        assert!(signal(&dir));
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the new instance replaced the stale socket");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_signal_without_a_first_instance_gives_up() {
        let dir = temp("no-first");
        let started = std::time::Instant::now();
        assert!(!signal(&dir), "nothing was there to take the signal");
        // A locked folder whose first copy is still starting gets this long, not longer.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_data_folder_still_starts() {
        use std::os::unix::fs::PermissionsExt;

        let dir = temp("unwritable");
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o500)).unwrap();
        let first = matches!(acquire(&dir), Instance::First(_));
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        // Without a lock file there's no telling; it runs rather than refuse to start. As root the
        // lock file gets written and it starts as first anyway.
        assert!(first);
    }

    #[cfg(unix)]
    #[test]
    fn a_long_data_folder_gets_a_short_private_socket() {
        use std::os::unix::fs::PermissionsExt;

        use super::imp::socket_path;

        let dir = long_data_folder("long");
        let socket = socket_path(&dir);
        assert!(!socket.starts_with(&dir), "{}", socket.display());
        assert!(
            socket.starts_with(std::env::temp_dir()),
            "{}",
            socket.display()
        );

        let Instance::First(guard) = acquire(&dir) else {
            panic!("the first acquire should win");
        };
        let folder = socket.parent().unwrap();
        let mode = std::fs::metadata(folder).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{folder:?} is not private: {mode:o}");
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{socket:?} is not private: {mode:o}");

        let (tx, rx) = std::sync::mpsc::channel();
        guard.on_signal(move || {
            let _ = tx.send(());
        });
        assert!(matches!(acquire(&dir), Instance::Running));
        assert!(signal(&dir));
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first instance heard the signal");

        let _ = std::fs::remove_dir_all(folder);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_fallback_folder_open_to_others_is_made_private() {
        use std::os::unix::fs::PermissionsExt;

        use super::imp::socket_path;

        let dir = long_data_folder("shared-fallback");
        let socket = socket_path(&dir);
        let folder = socket.parent().unwrap().to_path_buf();
        // What a previous run with a looser umask, or someone else, left behind.
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o755)).unwrap();

        let Instance::First(guard) = acquire(&dir) else {
            panic!("the first acquire should win");
        };
        let mode = std::fs::metadata(&folder).unwrap().permissions().mode();
        assert_eq!(mode & 0o077, 0, "{folder:?} stayed open: {mode:o}");

        let (tx, rx) = std::sync::mpsc::channel();
        guard.on_signal(move || {
            let _ = tx.send(());
        });
        assert!(matches!(acquire(&dir), Instance::Running));
        assert!(signal(&dir));
        rx.recv_timeout(std::time::Duration::from_secs(5))
            .expect("the first instance heard the signal");

        let _ = std::fs::remove_dir_all(&folder);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_fallback_folder_that_is_a_symlink_is_not_used() {
        use super::imp::socket_path;

        let dir = long_data_folder("symlinked-fallback");
        let socket = socket_path(&dir);
        let folder = socket.parent().unwrap().to_path_buf();
        let elsewhere = temp("symlink-target");
        std::os::unix::fs::symlink(&elsewhere, &folder).unwrap();

        let Instance::First(_guard) = acquire(&dir) else {
            panic!("a folder with no lock stays free, whatever its socket");
        };
        assert!(!socket.exists(), "a socket was made through the symlink");
        assert!(!elsewhere.join("instance.sock").exists());
        // No listener, so a second launch gives up after its wait and exits anyway.
        assert!(!signal(&dir));

        let _ = std::fs::remove_file(&folder);
        let _ = std::fs::remove_dir_all(&elsewhere);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
