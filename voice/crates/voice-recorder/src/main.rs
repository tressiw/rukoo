use std::io::Write;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match voice_recorder::run(&args) {
        Err(e) => {
            let _ = writeln!(std::io::stderr(), "rukoo-voice-recorder: {e}");
            std::process::exit(e.code);
        }
        Ok(()) => exit_now(),
    }
}

/// Ends the process without running exit handlers. A recording that stopped has synced its
/// `stop` line and answered, but a device open can still be blocked inside Core Audio, such as a
/// process tap waiting for the System Audio Recording prompt, and an exit handler that waits on
/// a lock that thread holds would leave the recorder, and its prompt, behind.
#[cfg(unix)]
fn exit_now() -> ! {
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    // SAFETY: `_exit` ends the process at once; nothing that follows needs to run.
    unsafe { libc::_exit(0) }
}

#[cfg(not(unix))]
fn exit_now() {}
