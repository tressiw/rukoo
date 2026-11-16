//! The syncer: a thread of its own that puts the recorder's files on disk, so a slow disk never
//! holds up the recorder thread, its heartbeats or its audio.
//!
//! Jobs run in the order they were sent. A closed segment comes with its `segment_end` line,
//! which the syncer hands back only once the segment is on disk and closed. The recorder writes
//! the line then, so no sync of the timeline, not even one asked for before the segment closed,
//! puts a `segment_end` on disk ahead of the audio it describes.

use std::fs::File;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::disk;
use crate::record::log;
use crate::timeline::Line;

/// A sync slower than this is logged, so a recording that stalls can be told from a slow disk.
pub const SLOW: Duration = Duration::from_millis(500);

/// Files to sync, each with what to call it in the log.
type Files = Vec<(String, File)>;

/// Puts one file on disk; the name is what the log calls it.
type SyncFn = Box<dyn Fn(&str, &File) -> io::Result<()> + Send>;

struct Job {
    files: Files,
    periodic: bool,
    /// Handed back once the files are on disk and closed.
    then: Option<Line>,
}

pub struct Syncer {
    jobs: Option<Sender<Job>>,
    thread: Option<JoinHandle<()>>,
    /// A periodic sync waits or runs.
    periodic: Arc<AtomicBool>,
    /// Lines whose files are on disk, in the order the files were handed over.
    synced: Receiver<Line>,
    /// Hands a line back at once when the syncer can't take its file.
    returned: Sender<Line>,
}

impl Syncer {
    pub fn start() -> io::Result<Self> {
        Self::start_with(Box::new(|_, file| disk::sync(file)))
    }

    fn start_with(sync: SyncFn) -> io::Result<Self> {
        let (jobs, rx) = mpsc::channel::<Job>();
        let (returned, synced) = mpsc::channel::<Line>();
        let done = returned.clone();
        let periodic = Arc::new(AtomicBool::new(false));
        let running = periodic.clone();
        let thread = std::thread::Builder::new()
            .name("syncer".into())
            .spawn(move || {
                for job in rx {
                    for (what, file) in &job.files {
                        let started = Instant::now();
                        if let Err(e) = sync(what, file) {
                            log(format_args!("can't sync {what}: {e}"));
                        }
                        let took = started.elapsed();
                        if took > SLOW {
                            log(format_args!("syncing {what} took {} ms", took.as_millis()));
                        }
                    }
                    // Closes the files handed over.
                    drop(job.files);
                    if job.periodic {
                        running.store(false, Ordering::Release);
                    }
                    // A failed sync was logged; the recording goes on, and so does its timeline.
                    if let Some(line) = job.then {
                        let _ = done.send(line);
                    }
                }
            })?;
        Ok(Self {
            jobs: Some(jobs),
            thread: Some(thread),
            periodic,
            synced,
            returned,
        })
    }

    /// Puts a file that the recorder no longer writes on disk, then closes it. `then` comes back
    /// from [`Syncer::synced`] after that.
    pub fn close(&self, what: String, file: File, then: Line) {
        self.send(Job {
            files: vec![(what, file)],
            periodic: false,
            then: Some(then),
        });
    }

    /// Whether a periodic sync may be queued: the previous one is done. On a slow disk the
    /// periodic syncs never pile up, while every closed segment still gets its own.
    pub fn periodic_due(&self) -> bool {
        !self.periodic.load(Ordering::Acquire)
    }

    /// Puts files that stay open on disk, through handles of their own.
    pub fn periodic(&self, files: Files) {
        self.periodic.store(true, Ordering::Release);
        self.send(Job {
            files,
            periodic: true,
            then: None,
        });
    }

    /// The lines handed over with files that are on disk now, in the order they were handed
    /// over. Never waits.
    pub fn synced(&self) -> Vec<Line> {
        self.synced.try_iter().collect()
    }

    fn send(&self, job: Job) {
        let job = match &self.jobs {
            Some(jobs) => match jobs.send(job) {
                Ok(()) => return,
                Err(e) => e.0,
            },
            None => job,
        };
        log("the syncer stopped; a file wasn't synced");
        if let Some(line) = job.then {
            let _ = self.returned.send(line);
        }
    }

    /// Waits until every file handed over is on disk, and returns the lines that came with the
    /// files not yet returned by [`Syncer::synced`].
    pub fn finish(&mut self) -> Vec<Line> {
        drop(self.jobs.take());
        if let Some(thread) = self.thread.take() {
            if thread.join().is_err() {
                log("the syncer failed");
            }
        }
        self.synced()
    }
}

impl Drop for Syncer {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_file(name: &str) -> (std::path::PathBuf, File) {
        let path = std::env::temp_dir().join(format!(
            "voice-recorder-syncer-{name}-{}-{}",
            std::process::id(),
            crate::clock::wall_ms()
        ));
        let file = File::create(&path).unwrap();
        (path, file)
    }

    fn end(seq: u64) -> Line {
        Line::SegmentEnd {
            t: seq * 1_000,
            track: "mic".into(),
            seq,
            samples: 16_000,
        }
    }

    /// A disk that holds every sync until the test lets it go, and says which file it holds.
    struct Gate {
        entered: Receiver<String>,
        go: Sender<io::Result<()>>,
    }

    fn gated() -> (Syncer, Gate) {
        let (entered_tx, entered) = mpsc::channel();
        let (go, go_rx) = mpsc::channel::<io::Result<()>>();
        let syncer = Syncer::start_with(Box::new(move |what, _| {
            entered_tx.send(what.to_string()).unwrap();
            go_rx.recv().unwrap()
        }))
        .unwrap();
        (syncer, Gate { entered, go })
    }

    impl Gate {
        fn holds(&self, what: &str) {
            let held = self.entered.recv_timeout(Duration::from_secs(10)).unwrap();
            assert_eq!(held, what);
        }
    }

    #[test]
    fn finish_waits_for_every_file_it_was_handed() {
        let mut syncer = Syncer::start().unwrap();
        let (open, file_open) = temp_file("open");
        let (closed, file_closed) = temp_file("closed");
        assert!(syncer.periodic_due());
        syncer.periodic(vec![("open".into(), file_open.try_clone().unwrap())]);
        syncer.close("closed".into(), file_closed, end(1));
        assert_eq!(syncer.finish(), vec![end(1)]);
        assert!(syncer.periodic_due(), "the periodic sync ran");
        // Closed: the file can go.
        std::fs::remove_file(&closed).unwrap();
        // After finish nothing is synced any more, and the line still comes back.
        syncer.close("late".into(), file_open.try_clone().unwrap(), end(2));
        assert_eq!(syncer.synced(), vec![end(2)]);
        drop(file_open);
        std::fs::remove_file(&open).unwrap();
    }

    #[test]
    fn a_segment_end_comes_back_only_after_its_own_sync() {
        let (mut syncer, gate) = gated();
        let (timeline_path, timeline) = temp_file("timeline");
        let (paths, files): (Vec<_>, Vec<_>) =
            (1..=2).map(|n| temp_file(&format!("segment-{n}"))).unzip();
        // A periodic sync of the timeline is running when two segments close.
        syncer.periodic(vec![("the timeline".into(), timeline.try_clone().unwrap())]);
        gate.holds("the timeline");
        for (n, file) in files.into_iter().enumerate() {
            syncer.close(format!("segment {}", n + 1), file, end(n as u64 + 1));
        }
        assert_eq!(syncer.synced(), vec![], "nothing is on disk yet");
        // The timeline is on disk, the first segment isn't yet: a recorder that wrote the
        // segment_end lines now would have had them synced ahead of their audio.
        gate.go.send(Ok(())).unwrap();
        gate.holds("segment 1");
        assert_eq!(syncer.synced(), vec![]);
        assert!(syncer.periodic_due(), "the periodic sync is done");
        // A failed sync is logged and its line still comes back: the recording goes on. The
        // syncer hands it back before it starts on the next file.
        gate.go.send(Err(io::Error::other("disk full"))).unwrap();
        gate.holds("segment 2");
        assert_eq!(syncer.synced(), vec![end(1)]);
        gate.go.send(Ok(())).unwrap();
        assert_eq!(syncer.finish(), vec![end(2)]);
        drop(timeline);
        for path in paths.into_iter().chain([timeline_path]) {
            std::fs::remove_file(path).unwrap();
        }
    }
}
