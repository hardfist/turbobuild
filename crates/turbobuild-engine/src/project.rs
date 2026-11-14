use anyhow::{Context, Result};
use turbo_rcstr::{RcStr, rcstr};
use turbo_tasks::{ResolvedVc, Vc};
use turbo_tasks_fs::{
    DiskFileSystem, DiskFileSystemMap, DiskWatcherConfig, FileContent, FileSystem, FileSystemPath,
};
use turbobuild_core::{CONFIG_FILE_NAME, ProjectConfig, TargetGraph};

/// Directories that are never treated as build inputs.
const DENIED_PATHS: &[&str] = &[".git", ".turbobuild"];

/// A loaded and validated project.
///
/// Turbo values must be serializable for the persistent cache, so the parsed
/// configuration is copied into plain [`TargetSpec`] values instead of holding
/// `turbobuild_core` types directly.
#[turbo_tasks::value]
pub struct Project {
    /// The directory containing `turbobuild.toml`.
    pub root: FileSystemPath,
    /// Targets in dependency order (dependencies first).
    pub targets: Vec<TargetSpec>,
}

/// A single target as seen by the engine.
#[turbo_tasks::value(shared)]
#[derive(Clone, Debug)]
pub struct TargetSpec {
    pub name: RcStr,
    pub inputs: Vec<RcStr>,
    pub deps: Vec<RcStr>,
    pub command: Option<RcStr>,
}

/// Returns the root of a disk file system for `root_dir`.
///
/// When `watch` is set the file system is watched, so on-disk changes
/// invalidate every task that read the changed files. Marked
/// `session_dependent` because watchers are process-local state: the task must
/// re-run in every session instead of being restored from the persistent cache.
#[turbo_tasks::function(session_dependent)]
pub async fn project_root(root_dir: RcStr, watch: bool) -> Result<Vc<FileSystemPath>> {
    let disk_fs = DiskFileSystem::new_with_options(
        rcstr!("project"),
        Vc::cell(root_dir),
        DENIED_PATHS.iter().map(|p| RcStr::from(*p)).collect(),
        DiskWatcherConfig::default(),
        DiskFileSystemMap::empty(),
    );
    if watch {
        disk_fs.await?.start_watching().await?;
    }
    let fs: Vc<Box<dyn FileSystem>> = Vc::upcast(disk_fs);
    Ok(fs.root())
}

/// Reads and validates `turbobuild.toml`. Re-runs only when that file changes.
#[turbo_tasks::function]
pub async fn load_project(root: FileSystemPath) -> Result<Vc<Project>> {
    let config_path = root.join(CONFIG_FILE_NAME)?;
    let content = config_path.read().await?;
    let source = match &*content {
        FileContent::Content(file) => file.content().to_str()?.into_owned(),
        FileContent::NotFound => anyhow::bail!("{CONFIG_FILE_NAME} not found in project root"),
    };

    let config = ProjectConfig::from_toml(&source)?;
    let graph = TargetGraph::new(&config)?;
    let targets = graph
        .topological_order()?
        .into_iter()
        .map(|name| {
            let target = &config.targets[name];
            TargetSpec {
                name: name.into(),
                inputs: target.inputs.iter().map(|s| s.as_str().into()).collect(),
                deps: target.deps.iter().map(|s| s.as_str().into()).collect(),
                command: target.command.as_deref().map(RcStr::from),
            }
        })
        .collect();

    Ok(Project { root, targets }.cell())
}

/// Extracts a single target from the project.
///
/// This is its own task so that downstream tasks depend on *one* target's
/// spec rather than the whole config: editing target `a` in `turbobuild.toml`
/// re-runs this task for every target, but because cells are compared by
/// value, only tasks reading target `a` are invalidated.
#[turbo_tasks::function]
pub async fn target_spec(project: ResolvedVc<Project>, name: RcStr) -> Result<Vc<TargetSpec>> {
    let project = project.await?;
    let spec = project
        .targets
        .iter()
        .find(|t| t.name == name)
        .cloned()
        .with_context(|| format!("unknown target `{name}`"))?;
    Ok(spec.cell())
}
