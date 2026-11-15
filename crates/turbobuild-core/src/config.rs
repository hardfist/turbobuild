//! Project configuration parsed from `turbo.json` (and optional `package.json`).
//!
//! Only the `tasks` section is supported, in Turborepo's *single-package*
//! mode: the project root is one package, every task becomes a target, and a
//! task's command is the `package.json` script of the same name.
//!
//! ```jsonc
//! {
//!   "$schema": "https://turborepo.com/schema.json",
//!   "tasks": {
//!     "build": { "dependsOn": ["^build", "codegen"], "inputs": ["src/**"] },
//!     "codegen": { "inputs": ["schema/*.graphql"] }
//!   }
//! }
//! ```
//!
//! Mapping rules:
//!
//! * `dependsOn: ["task"]` becomes a dependency on target `task`.
//! * `dependsOn: ["^task"]` refers to `task` in *upstream packages*. A single package has none, so
//!   it adds no dependency.
//! * `inputs` are globs relative to the project root. When omitted, or when it contains
//!   `$TURBO_DEFAULT$`, all project files (`**`) are inputs. Negated globs (`!pattern`) are not
//!   supported yet.
//! * Package-qualified tasks (`pkg#task`, `//#task`) need a monorepo and are rejected.
//! * Every other key (`outputs`, `cache`, `env`, `persistent`, `globalDependencies`, ...) is
//!   accepted and ignored for now.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, de::DeserializeOwned};

/// Name of the project configuration file, expected at the project root.
pub const CONFIG_FILE_NAME: &str = "turbo.json";

/// Name of the package manifest that holds task commands (`scripts`).
pub const PACKAGE_JSON_FILE_NAME: &str = "package.json";

/// Input token that expands to Turborepo's default inputs.
const TURBO_DEFAULT_INPUTS: &str = "$TURBO_DEFAULT$";

/// Glob used for Turborepo's default inputs: every file in the project.
const DEFAULT_INPUTS_GLOB: &str = "**";

/// Top-level project configuration, built from [`CONFIG_FILE_NAME`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectConfig {
    /// Build targets keyed by name. A `BTreeMap` keeps iteration deterministic.
    pub targets: BTreeMap<String, TargetConfig>,
}

/// A single build target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetConfig {
    /// Glob patterns (relative to the project root) whose contents feed into
    /// this target's fingerprint.
    pub inputs: Vec<String>,
    /// Names of targets that must be built before this one.
    pub deps: Vec<String>,
    /// Shell command that produces the target's outputs.
    pub command: Option<String>,
}

/// The subset of `turbo.json` that turbobuild understands.
#[derive(Debug, Deserialize)]
struct TurboJson {
    #[serde(default)]
    tasks: BTreeMap<String, TurboTask>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TurboTask {
    #[serde(default)]
    depends_on: Vec<String>,
    #[serde(default)]
    inputs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize)]
struct PackageJson {
    #[serde(default)]
    scripts: BTreeMap<String, String>,
}

impl ProjectConfig {
    /// Builds a configuration from the `tasks` of a `turbo.json` file.
    ///
    /// `package_json` is the source of the root `package.json`, if any; its
    /// `scripts` provide the task commands. A task without a script is a
    /// no-op, as in Turborepo, and gets no command.
    pub fn from_turbo_json(turbo_json: &str, package_json: Option<&str>) -> Result<Self> {
        let turbo: TurboJson = parse_jsonc(turbo_json, CONFIG_FILE_NAME)?;
        let scripts = match package_json {
            Some(source) => parse_jsonc::<PackageJson>(source, PACKAGE_JSON_FILE_NAME)?.scripts,
            None => BTreeMap::new(),
        };

        let targets = turbo
            .tasks
            .into_iter()
            .map(|(name, task)| {
                let target = convert_task(&name, task, &scripts)
                    .with_context(|| format!("invalid task `{name}` in {CONFIG_FILE_NAME}"))?;
                Ok((name, target))
            })
            .collect::<Result<_>>()?;
        Ok(Self { targets })
    }
}

fn convert_task(
    name: &str,
    task: TurboTask,
    scripts: &BTreeMap<String, String>,
) -> Result<TargetConfig> {
    if name.contains('#') {
        bail!("package tasks (`package#task`) require a monorepo, which is not supported yet");
    }

    let mut deps = Vec::new();
    for dep in task.depends_on {
        if dep.starts_with('^') {
            // Upstream package dependencies: none in single-package mode.
            continue;
        }
        if dep.starts_with('$') {
            bail!("environment variable dependencies (`{dep}`) are not supported; use `env`");
        }
        if dep.contains('#') {
            bail!(
                "package task dependency `{dep}` requires a monorepo, which is not supported yet"
            );
        }
        if !deps.contains(&dep) {
            deps.push(dep);
        }
    }

    let mut inputs = Vec::new();
    for input in task
        .inputs
        .unwrap_or_else(|| vec![TURBO_DEFAULT_INPUTS.to_owned()])
    {
        if input.starts_with('!') {
            bail!("negated input globs (`{input}`) are not supported yet");
        }
        let input = if input == TURBO_DEFAULT_INPUTS {
            DEFAULT_INPUTS_GLOB.to_owned()
        } else {
            input
        };
        if !inputs.contains(&input) {
            inputs.push(input);
        }
    }

    Ok(TargetConfig {
        inputs,
        deps,
        command: scripts.get(name).cloned(),
    })
}

/// Parses JSON that may contain comments and trailing commas, as Turborepo
/// accepts in `turbo.json`.
fn parse_jsonc<T: DeserializeOwned>(source: &str, file_name: &str) -> Result<T> {
    let value = jsonc_parser::parse_to_serde_value(source, &Default::default())
        .map_err(|err| anyhow::anyhow!("{err}"))
        .with_context(|| format!("failed to parse {file_name}"))?
        .with_context(|| format!("{file_name} is empty"))?;
    serde_json::from_value(value).with_context(|| format!("invalid {file_name}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TargetGraph;

    #[test]
    fn maps_tasks_to_targets() {
        let config = ProjectConfig::from_turbo_json(
            r#"{
              // comments and trailing commas are allowed
              "$schema": "https://turborepo.com/schema.json",
              "globalDependencies": [".env"],
              "tasks": {
                "build": {
                  "dependsOn": ["^build", "codegen"],
                  "inputs": ["src/**", "codegen/**"],
                  "outputs": ["dist/**"],
                },
                "codegen": { "inputs": ["schema/*.graphql"], "cache": true },
                "lint": {},
                "dev": { "cache": false, "persistent": true },
              },
            }"#,
            Some(
                r#"{ "name": "app", "scripts": { "build": "tsc", "codegen": "gen", "dev": "vite" } }"#,
            ),
        )
        .unwrap();

        let build = &config.targets["build"];
        assert_eq!(build.deps, ["codegen"]);
        assert_eq!(build.inputs, ["src/**", "codegen/**"]);
        assert_eq!(build.command.as_deref(), Some("tsc"));

        // No `inputs` means all files; no script means no command.
        let lint = &config.targets["lint"];
        assert_eq!(lint.inputs, ["**"]);
        assert_eq!(lint.command, None);

        let graph = TargetGraph::new(&config).unwrap();
        let order = graph.topological_order().unwrap();
        let pos = |n| order.iter().position(|t| *t == n).unwrap();
        assert!(pos("codegen") < pos("build"));
    }

    #[test]
    fn expands_turbo_default_inputs() {
        let config = ProjectConfig::from_turbo_json(
            r#"{ "tasks": { "test": { "inputs": ["$TURBO_DEFAULT$", "fixtures/**"] } } }"#,
            None,
        )
        .unwrap();
        assert_eq!(config.targets["test"].inputs, ["**", "fixtures/**"]);
    }

    #[test]
    fn rejects_unsupported_features() {
        for (turbo_json, expected) in [
            (r#"{ "tasks": { "web#build": {} } }"#, "package tasks"),
            (
                r#"{ "tasks": { "build": { "dependsOn": ["web#build"] } } }"#,
                "package task dependency",
            ),
            (
                r#"{ "tasks": { "build": { "dependsOn": ["$NODE_ENV"] } } }"#,
                "environment variable dependencies",
            ),
            (
                r#"{ "tasks": { "build": { "inputs": ["!README.md"] } } }"#,
                "negated input globs",
            ),
        ] {
            let err = ProjectConfig::from_turbo_json(turbo_json, None).unwrap_err();
            assert!(format!("{err:#}").contains(expected), "{err:#}");
        }
    }

    #[test]
    fn unknown_dependency_is_reported_by_the_graph() {
        let config = ProjectConfig::from_turbo_json(
            r#"{ "tasks": { "build": { "dependsOn": ["missing"] } } }"#,
            None,
        )
        .unwrap();
        let err = TargetGraph::new(&config).unwrap_err();
        assert!(err.to_string().contains("unknown target `missing`"));
    }

    #[test]
    fn reports_invalid_json() {
        let err = ProjectConfig::from_turbo_json("{ tasks: [", None).unwrap_err();
        assert!(
            format!("{err:#}").contains("failed to parse turbo.json"),
            "{err:#}"
        );
    }
}
