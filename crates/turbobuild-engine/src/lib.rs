//! Incremental build computations for turbobuild.
//!
//! Every function annotated with `#[turbo_tasks::function]` is a memoized
//! task. The turbo engine records which files and other tasks each task reads,
//! so when a file changes only the tasks that (transitively) depend on it are
//! recomputed.
//!
//! The computation graph is:
//!
//! ```text
//! project_fingerprints
//!   └── load_project (reads turbobuild.toml)
//!   └── target_fingerprint(target)       one task per target
//!         ├── inputs_hash(target)        hashes files matched by the target's globs
//!         │     └── file_hash(path)      one task per file
//!         └── target_fingerprint(dep)    recursion into dependencies
//! ```

#![feature(arbitrary_self_types)]
#![feature(arbitrary_self_types_pointers)]

mod fingerprint;
mod project;
mod session;

pub use fingerprint::{TargetFingerprint, TargetFingerprints};
pub use project::Project;
pub use session::{BuildSession, FingerprintReport};
