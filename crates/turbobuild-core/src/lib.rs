//! Engine-agnostic data model for turbobuild.
//!
//! This crate deliberately has no dependency on the turbo engine so that it
//! builds on stable-compatible code and stays easy to unit test. The engine
//! crate wraps these types in incremental, cached computations.

mod config;
mod graph;
mod turbo;

pub use config::{CONFIG_FILE_NAME, ProjectConfig, TargetConfig};
pub use graph::TargetGraph;
pub use turbo::{PACKAGE_JSON_FILE_NAME, TURBO_CONFIG_FILE_NAME};
