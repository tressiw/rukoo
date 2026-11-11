//! The local speech model: the pinned catalog, where the files are, their download, and the
//! client for the `rukoo-voice-local` worker that runs them.

mod disk;
mod download;
mod engine;
#[cfg(windows)]
mod job;
// The Unix parent watch both workers run. It lives here, next to the code that spawns them, and
// not in a crate of its own: the workers include the file by path, because a worker that a model
// takes down shouldn't link the app's network and archive stacks.
pub mod parent;
mod registry;

pub use download::{extract, Progress, Stage};
pub use engine::{
    hotword_terms, hotwords_string, LocalEngine, LocalOptions, LocalState, LocalStatus, Timed,
};
// The speaker label worker goes in a kill-on-close job too, and its runtime check hashes files.
#[cfg(windows)]
pub(crate) use job::Job;
pub(crate) use registry::sha256_file;
pub use registry::{
    files_present, validate, Archive, Located, Location, ModelError, ModelErrorKind, ModelFile,
    ModelRegistry, ModelSpec, Verification,
};
