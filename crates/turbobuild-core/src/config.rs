use std::collections::BTreeMap;

use anyhow::{Context, Result};
use serde::Deserialize;

/// Name of the project configuration file, expected at the project root.
pub const CONFIG_FILE_NAME: &str = "turbobuild.toml";

/// Top-level project configuration, parsed from [`CONFIG_FILE_NAME`].
///
/// ```toml
/// [targets.lib]
/// inputs = ["src/**/*.rs"]
///
/// [targets.app]
/// inputs = ["app/**"]
/// deps = ["lib"]
/// command = "cargo build -p app"
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    /// Build targets keyed by name. A `BTreeMap` keeps iteration deterministic.
    #[serde(default)]
    pub targets: BTreeMap<String, TargetConfig>,
}

/// A single build target.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TargetConfig {
    /// Glob patterns (relative to the project root) whose contents feed into
    /// this target's fingerprint.
    #[serde(default)]
    pub inputs: Vec<String>,
    /// Names of targets that must be built before this one.
    #[serde(default)]
    pub deps: Vec<String>,
    /// Shell command that produces the target's outputs.
    #[serde(default)]
    pub command: Option<String>,
}

impl ProjectConfig {
    /// Parses a configuration from TOML source.
    pub fn from_toml(source: &str) -> Result<Self> {
        toml::from_str(source).with_context(|| format!("failed to parse {CONFIG_FILE_NAME}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_targets() {
        let config = ProjectConfig::from_toml(
            r#"
            [targets.lib]
            inputs = ["src/**/*.rs"]

            [targets.app]
            deps = ["lib"]
            command = "echo app"
            "#,
        )
        .unwrap();

        assert_eq!(config.targets.len(), 2);
        assert_eq!(config.targets["lib"].inputs, ["src/**/*.rs"]);
        assert_eq!(config.targets["app"].deps, ["lib"]);
        assert_eq!(config.targets["app"].command.as_deref(), Some("echo app"));
    }

    #[test]
    fn rejects_unknown_fields() {
        assert!(ProjectConfig::from_toml("[targets.a]\nunknown = 1").is_err());
    }
}
