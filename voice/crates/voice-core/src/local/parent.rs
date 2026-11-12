//! Ends a worker when the app that started it dies, on Unix.
//!
//! Windows keeps this problem away: the local speech worker and the speaker label worker run in a
//! kill-on-close job object (`local/job.rs`), so Windows ends them with Voice. Unix has nothing
//! like it, and a worker whose app was hard-killed keeps a 1 GB model in a process nobody talks
//! to. So the spawning code writes its own pid in `PARENT_PID` and the worker watches it:
//!
//! - The worker reads that pid as its first act. `getppid()` alone won't do: Voice may already be
//!   gone and the worker reparented to launchd or init before `main` runs, and `getppid()` would
//!   then name a process that isn't Voice at all. The pid Voice wrote down stays right.
//! - macOS and the BSDs get a `kqueue` queue with `EVFILT_PROC` and `NOTE_EXIT` on that pid.
//! - Linux gets `pidfd_open` and `poll`, or a second of `kill(pid, 0)` and `getppid()` where the
//!   kernel is older than 5.3.
//!
//! When the parent is gone the worker logs one line and exits with `PARENT_GONE`. That is safe
//! because a worker holds nothing that needs saving: the local model is memory, and the speaker
//! worker writes nothing to disk. Only workers set the variable. The meeting recorder must outlive
//! the app, so the detached spawn in `voice-engine` never names a parent.

/// Windows ends workers through the job object in `local/job.rs`, so there is nothing to watch.
#[cfg(not(unix))]
pub fn watch(_log: impl Fn(&str) + Send + Sync + 'static) {}

/// Everything below is Unix-only; the glob keeps the constants out of the import list a binary
/// crate doesn't use.
#[cfg(unix)]
pub use watched::*;

#[cfg(unix)]
mod watched {
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    /// The environment variable in which the spawning app names its own process id. The workers
    /// read it from the environment they inherit.
    pub const PARENT_PID: &str = "RUKOO_VOICE_PARENT_PID";

    /// What a worker exits with when the app that started it is gone: not 0, which means it was
    /// asked to quit, and not 1, which means it failed.
    pub const PARENT_GONE: i32 = 70;

    /// Starts the watch, if the app that started this process named itself. `log` gets one line
    /// when the parent is gone.
    ///
    /// A worker started by hand has no name to watch and keeps running until its stdin closes,
    /// which is what the protocol tests and a development run expect.
    pub fn watch(log: impl Fn(&str) + Send + Sync + 'static) {
        let Some(parent) = named_parent() else {
            return;
        };
        let log = Arc::new(log);
        if !alive(parent) {
            end_after_log(&*log, &gone(parent));
        }
        // Whether the named pid is this process's own parent decides if `getppid()` may be read
        // as "my parent is gone". A launcher or a test may name another process instead.
        // SAFETY: getppid takes no arguments and can't fail.
        let own_parent = unsafe { libc::getppid() as u32 } == parent;
        let logger = Arc::clone(&log);
        let spawned = thread::Builder::new()
            .name("parent-watch".into())
            .spawn(move || {
                wait_for_exit(parent, own_parent);
                end_after_log(&*logger, &gone(parent));
            });
        if let Err(error) = spawned {
            // No thread, no watch: the app still stops the worker it started.
            log(&format!("could not watch Voice ({parent}): {error}"));
        }
    }

    /// Logs why and ends the process. The app held this process's stderr pipe, so the line can
    /// hit a broken pipe and `eprintln!` panics; a log that fails must not keep the worker alive.
    fn end_after_log(log: &dyn Fn(&str), line: &str) -> ! {
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| log(line)));
        end()
    }

    /// Ends the process at once. `_exit` on purpose: a worker has nothing that needs saving, and
    /// an orderly exit waits for locks another thread may hold — one busy in a model, say —
    /// instead of ending when its app does.
    fn end() -> ! {
        // SAFETY: no handler runs and nothing needs flushing; the log line has been written.
        unsafe { libc::_exit(PARENT_GONE) }
    }

    fn named_parent() -> Option<u32> {
        let value = std::env::var(PARENT_PID).ok()?;
        let pid = value.trim().parse::<u32>().ok()?;
        // 0 and 1 are never Voice: an odd value means no watch.
        (pid > 1).then_some(pid)
    }

    fn gone(pid: u32) -> String {
        format!("Voice ({pid}) is gone; stopping.")
    }

    fn alive(pid: u32) -> bool {
        // SAFETY: signal 0 only checks that the process exists.
        let result = unsafe { libc::kill(pid as libc::pid_t, 0) };
        result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
    }

    /// Whether this process has been handed to another parent, which only happens when its own
    /// parent died. Only meaningful when the watched pid was this process's parent.
    fn reparented(pid: u32, own_parent: bool) -> bool {
        // SAFETY: getppid takes no arguments and can't fail.
        own_parent && unsafe { libc::getppid() as u32 } != pid
    }

    /// The slow path: once a second, cheaper than a queue and enough for the two seconds a worker
    /// is allowed to outlive its app.
    fn poll_until_gone(pid: u32, own_parent: bool) {
        loop {
            thread::sleep(Duration::from_secs(1));
            if reparented(pid, own_parent) || !alive(pid) {
                return;
            }
        }
    }

    #[cfg(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    ))]
    fn wait_for_exit(pid: u32, own_parent: bool) {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

        // SAFETY: the queue is owned from here on, and the kevent structs are zeroed before use.
        unsafe {
            let queue = libc::kqueue();
            if queue < 0 {
                return poll_until_gone(pid, own_parent);
            }
            let queue = OwnedFd::from_raw_fd(queue);
            let mut change: libc::kevent = std::mem::zeroed();
            change.ident = pid as libc::uintptr_t;
            change.filter = libc::EVFILT_PROC;
            change.flags = libc::EV_ADD | libc::EV_ENABLE | libc::EV_ONESHOT;
            change.fflags = libc::NOTE_EXIT;
            let added = libc::kevent(
                queue.as_raw_fd(),
                &change,
                1,
                std::ptr::null_mut(),
                0,
                std::ptr::null(),
            );
            if added < 0 {
                // ESRCH means the process ended before the watch was in place; anything else
                // means this queue can't watch it.
                if std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH) {
                    poll_until_gone(pid, own_parent);
                }
                return;
            }
            let mut event: libc::kevent = std::mem::zeroed();
            // No timeout: the queue wakes this thread when the process exits.
            let fired = libc::kevent(
                queue.as_raw_fd(),
                std::ptr::null(),
                0,
                &mut event,
                1,
                std::ptr::null(),
            );
            if fired <= 0 {
                poll_until_gone(pid, own_parent);
            }
        }
    }

    #[cfg(target_os = "linux")]
    fn wait_for_exit(pid: u32, own_parent: bool) {
        use std::os::fd::AsRawFd;

        let Some(fd) = pidfd(pid) else {
            return poll_until_gone(pid, own_parent);
        };
        let mut pending = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        loop {
            // SAFETY: one valid pollfd and no timeout.
            let ready = unsafe { libc::poll(&mut pending, 1, -1) };
            if ready > 0 {
                return;
            }
            if ready == 0 {
                break;
            }
            // A signal interrupted the wait; the process may still be running.
            if std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        poll_until_gone(pid, own_parent);
    }

    /// A descriptor that turns readable when the process exits, where the kernel has
    /// `pidfd_open` (Linux 5.3 and later).
    #[cfg(target_os = "linux")]
    fn pidfd(pid: u32) -> Option<std::os::fd::OwnedFd> {
        use std::os::fd::FromRawFd;

        // SAFETY: pidfd_open takes a pid and flags, and the descriptor is owned from here on.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid as libc::c_long, 0) };
        if fd < 0 {
            None
        } else {
            Some(unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as libc::c_int) })
        }
    }

    /// Every other Unix: a second of polling.
    #[cfg(not(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "ios",
        target_os = "freebsd",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "dragonfly"
    )))]
    fn wait_for_exit(pid: u32, own_parent: bool) {
        poll_until_gone(pid, own_parent);
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The variable is this process's alone: nothing else in `voice-core` reads it, and the
        /// test binary's other tests spawn no workers.
        #[test]
        fn only_a_real_pid_is_watched() {
            std::env::set_var(PARENT_PID, "1");
            assert_eq!(named_parent(), None);
            std::env::set_var(PARENT_PID, "0");
            assert_eq!(named_parent(), None);
            std::env::set_var(PARENT_PID, "not a pid");
            assert_eq!(named_parent(), None);
            std::env::set_var(PARENT_PID, format!(" {}", std::process::id()));
            assert_eq!(named_parent(), Some(std::process::id()));
            std::env::remove_var(PARENT_PID);
            assert_eq!(named_parent(), None);
        }
    }
}
