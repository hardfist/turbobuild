use std::{path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use turbo_rcstr::RcStr;
use turbo_tasks::{TurboTasks, Vc};
use turbo_tasks_backend::{
    BackendOptions, BackingStorageOptions, GitVersionInfo, StorageMode, TurboTasksBackend,
    noop_backing_storage, turbo_backing_storage,
};

use crate::{TargetFingerprint, fingerprint::project_fingerprints_operation};

/// Result of one fingerprint pass.
#[derive(Debug, Clone)]
pub struct FingerprintReport {
    pub targets: Vec<TargetFingerprint>,
}

/// A running turbo engine instance bound to one project directory.
pub struct BuildSession {
    tt: Arc<TurboTasks<TurboTasksBackend>>,
    root_dir: RcStr,
    watch: bool,
}

impl BuildSession {
    /// Creates a session for the project at `root_dir`.
    ///
    /// * `cache_dir`: where to persist the task graph between runs. `None` keeps everything in
    ///   memory.
    /// * `watch`: watch the file system so changes invalidate cached tasks.
    pub fn new(root_dir: &Path, cache_dir: Option<&Path>, watch: bool) -> Result<Self> {
        // The disk file system requires a canonical root: symlinks in the root path
        // (e.g. `/var` -> `/private/var` on macOS) would otherwise fail path resolution.
        let root_dir = root_dir
            .canonicalize()
            .with_context(|| format!("invalid project root: {}", root_dir.display()))?;
        let root_dir = RcStr::from(
            root_dir
                .to_str()
                .context("project root must be valid UTF-8")?,
        );

        let backend = match cache_dir {
            Some(cache_dir) => {
                std::fs::create_dir_all(cache_dir).with_context(|| {
                    format!("failed to create cache dir {}", cache_dir.display())
                })?;
                let (storage, _cache_state) = turbo_backing_storage(
                    cache_dir,
                    &GitVersionInfo {
                        // The cache is invalidated whenever turbobuild's version changes.
                        describe: env!("CARGO_PKG_VERSION"),
                        dirty: false,
                    },
                    BackingStorageOptions {
                        is_short_session: !watch,
                        ..Default::default()
                    },
                )?;
                TurboTasksBackend::new(
                    BackendOptions {
                        storage_mode: Some(if watch {
                            StorageMode::ReadWrite
                        } else {
                            StorageMode::ReadWriteOnShutdown
                        }),
                        ..Default::default()
                    },
                    storage,
                )
            }
            None => TurboTasksBackend::new(
                BackendOptions {
                    storage_mode: None,
                    ..Default::default()
                },
                noop_backing_storage(),
            ),
        };

        Ok(Self {
            tt: TurboTasks::new(backend),
            root_dir,
            watch,
        })
    }

    /// Computes (or re-reads from cache) the fingerprints of all targets.
    pub async fn fingerprints(&self) -> Result<FingerprintReport> {
        let root_dir = self.root_dir.clone();
        let watch = self.watch;
        let targets = self
            .tt
            .run_once(async move {
                let fingerprints = project_fingerprints_operation(root_dir, watch)
                    .read_strongly_consistent()
                    .await?;
                Ok(fingerprints.iter().cloned().collect())
            })
            .await?;
        Ok(FingerprintReport { targets })
    }

    /// Recomputes fingerprints whenever inputs change, calling `on_change`
    /// with each new report, and never returns.
    ///
    /// This spawns a root task: the engine re-executes it automatically
    /// whenever anything it (transitively) read is invalidated, so there is
    /// no polling. The loop below only reports engine statistics.
    pub async fn watch<F>(&self, on_change: F) -> !
    where
        F: Fn(FingerprintReport) + Send + Sync + Clone + 'static,
    {
        let root_dir = self.root_dir.clone();
        let watch = self.watch;
        self.tt.spawn_root_task(move || {
            let root_dir = root_dir.clone();
            let on_change = on_change.clone();
            async move {
                let fingerprints = project_fingerprints_operation(root_dir, watch)
                    .read_strongly_consistent()
                    .await;
                match fingerprints {
                    Ok(fingerprints) => on_change(FingerprintReport {
                        targets: fingerprints.iter().cloned().collect(),
                    }),
                    // Keep watching: the next change may fix the error.
                    Err(err) => eprintln!("error: {err:?}"),
                }
                Ok(Vc::<()>::default())
            }
        });

        loop {
            let update = self
                .tt
                .get_or_wait_aggregated_update_info(Duration::from_millis(100))
                .await;
            tracing::info!(
                "{} task(s) re-executed in {:?}",
                update.tasks,
                update.duration
            );
        }
    }

    /// Flushes the persistent cache and stops the engine.
    pub async fn shutdown(self) {
        self.tt.stop_and_wait().await;
    }
}
