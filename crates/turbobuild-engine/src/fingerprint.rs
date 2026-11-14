use anyhow::Result;
use turbo_rcstr::RcStr;
use turbo_tasks::{ResolvedVc, TryJoinIterExt, Vc};
use turbo_tasks_fs::{
    DirectoryEntry, FileContent, FileSystemPath, ReadGlobResult,
    glob::{Glob, GlobOptions},
};
use turbo_tasks_hash::{DeterministicHasher, Xxh3Hash64Hasher, hash_xxh3_hash64};

use crate::project::{Project, load_project, project_root, target_spec};

/// The content fingerprint of a target: a hash of its input files combined
/// with the fingerprints of all its dependencies. If the fingerprint is
/// unchanged, the target's outputs are up to date.
#[turbo_tasks::value(shared)]
#[derive(Clone, Debug)]
pub struct TargetFingerprint {
    pub name: RcStr,
    /// Number of input files matched by the target's globs.
    pub input_files: u64,
    /// Hash over the target's own inputs only.
    pub inputs_hash: u64,
    /// Hash over the inputs and all dependency fingerprints.
    pub fingerprint: u64,
}

/// Fingerprints for every target in the project, in dependency order.
#[turbo_tasks::value(transparent)]
pub struct TargetFingerprints(Vec<TargetFingerprint>);

/// Hash of the matched files of a glob: `(file count, hash)`.
#[turbo_tasks::value(shared)]
#[derive(Clone, Copy, Debug, Default)]
struct InputsHash {
    files: u64,
    hash: u64,
}

/// Entry point: computes fingerprints for all targets of the project in
/// `root_dir`. This is the root of the task graph; reading it again after a
/// file change only re-executes invalidated tasks.
#[turbo_tasks::function(operation, root)]
pub async fn project_fingerprints_operation(
    root_dir: RcStr,
    watch: bool,
) -> Result<Vc<TargetFingerprints>> {
    let root = project_root(root_dir, watch).owned().await?;
    let project = load_project(root).to_resolved().await?;
    let fingerprints = project
        .await?
        .targets
        .iter()
        .map(|t| target_fingerprint(*project, t.name.clone()).owned())
        .try_join()
        .await?;
    Ok(Vc::cell(fingerprints))
}

#[turbo_tasks::function]
async fn target_fingerprint(
    project: ResolvedVc<Project>,
    name: RcStr,
) -> Result<Vc<TargetFingerprint>> {
    let spec = target_spec(*project, name.clone()).await?;
    let root = project.await?.root.clone();
    let inputs = inputs_hash(root, spec.inputs.clone()).owned().await?;
    let dep_fingerprints = spec
        .deps
        .iter()
        .map(|dep| target_fingerprint(*project, dep.clone()).owned())
        .try_join()
        .await?;

    let mut hasher = Xxh3Hash64Hasher::new();
    hasher.write_bytes(name.as_bytes());
    hasher.write_u64(inputs.hash);
    if let Some(command) = &spec.command {
        hasher.write_bytes(command.as_bytes());
    }
    for dep in &dep_fingerprints {
        hasher.write_bytes(dep.name.as_bytes());
        hasher.write_u64(dep.fingerprint);
    }

    Ok(TargetFingerprint {
        name,
        input_files: inputs.files,
        inputs_hash: inputs.hash,
        fingerprint: hasher.finish(),
    }
    .cell())
}

/// Hashes all files matched by `patterns` (relative to `root`).
#[turbo_tasks::function]
async fn inputs_hash(root: FileSystemPath, patterns: Vec<RcStr>) -> Result<Vc<InputsHash>> {
    let hashes = patterns
        .into_iter()
        .map(|pattern| {
            let root = root.clone();
            async move {
                let glob = Glob::new(pattern.clone(), GlobOptions::default());
                let result = glob_hash(root.read_glob(glob)).owned().await?;
                anyhow::Ok((pattern, result))
            }
        })
        .try_join()
        .await?;

    let mut hasher = Xxh3Hash64Hasher::new();
    let mut files = 0;
    for (pattern, result) in hashes {
        hasher.write_bytes(pattern.as_bytes());
        hasher.write_u64(result.hash);
        files += result.files;
    }
    Ok(InputsHash {
        files,
        hash: hasher.finish(),
    }
    .cell())
}

/// Recursively hashes a glob result. Entries are sorted by name so the hash
/// does not depend on directory iteration order.
#[turbo_tasks::function]
async fn glob_hash(result: Vc<ReadGlobResult>) -> Result<Vc<InputsHash>> {
    let result = result.await?;

    let mut files: Vec<_> = result
        .results
        .iter()
        .filter_map(|(name, entry)| match entry {
            DirectoryEntry::File(path) => Some((name.clone(), path.clone())),
            _ => None,
        })
        .collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let file_hashes = files
        .into_iter()
        .map(|(name, path)| async move { anyhow::Ok((name, *file_hash(path).await?)) })
        .try_join()
        .await?;

    let mut dirs: Vec<_> = result.inner.iter().collect();
    dirs.sort_by(|a, b| a.0.cmp(b.0));
    let dir_hashes = dirs
        .into_iter()
        .map(|(name, inner)| async move { anyhow::Ok((name.clone(), *glob_hash(**inner).await?)) })
        .try_join()
        .await?;

    let mut hasher = Xxh3Hash64Hasher::new();
    let mut count = file_hashes.len() as u64;
    for (name, hash) in &file_hashes {
        hasher.write_bytes(name.as_bytes());
        hasher.write_u64(*hash);
    }
    for (name, inner) in &dir_hashes {
        hasher.write_bytes(name.as_bytes());
        hasher.write_u64(inner.hash);
        count += inner.files;
    }
    Ok(InputsHash {
        files: count,
        hash: hasher.finish(),
    }
    .cell())
}

/// Hashes a single file's content. One task per file, so an edit re-hashes
/// only the file that changed.
#[turbo_tasks::function]
async fn file_hash(path: FileSystemPath) -> Result<Vc<u64>> {
    let content = path.read().await?;
    let hash = match &*content {
        FileContent::Content(file) => hash_xxh3_hash64(file.content().content_hash()),
        FileContent::NotFound => 0,
    };
    Ok(Vc::cell(hash))
}
