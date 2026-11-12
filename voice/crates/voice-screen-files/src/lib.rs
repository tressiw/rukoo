//! The screen folder of a meeting, as `voice/RECORDING.md` "The screen folder" describes it.
//!
//! `rukoo-voice-screen` writes `screen/` inside a meeting folder; the app, the pipeline and bucket
//! sync read it. Everything here is platform-neutral: paths, the journal and its readers, the
//! mapping from the recorder's clock to meeting time, when a frame is saved, which frames go to
//! the video encoder, which part to drop
//! when the disk fills, the monitor choice, the fragmented MP4 box walker and writer, the control
//! port, and the answers to `frame.get` and `clip.get`.

pub mod caps;
pub mod clock;
pub mod control;
pub mod fmp4;
mod follow;
pub mod frames;
pub mod gate;
pub mod journal;
pub mod media;
pub mod monitors;
pub mod mp4;
pub mod paths;
pub mod policy;
pub mod screenfile;
pub mod timeline;

#[cfg(test)]
mod testdir;
