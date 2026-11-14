use std::{fs, path::Path};

use turbobuild_engine::{BuildSession, FingerprintReport};

fn write(root: &Path, path: &str, content: &str) {
    let path = root.join(path);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

fn fingerprint(report: &FingerprintReport, name: &str) -> u64 {
    report
        .targets
        .iter()
        .find(|t| t.name == name)
        .unwrap_or_else(|| panic!("missing target {name}"))
        .fingerprint
}

fn setup() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "turbobuild.toml",
        r#"
        [targets.util]
        inputs = ["util/**/*.txt"]

        [targets.lib]
        inputs = ["lib/**"]
        deps = ["util"]

        [targets.docs]
        inputs = ["docs/*.md"]
        "#,
    );
    write(root, "util/a.txt", "a");
    write(root, "util/nested/b.txt", "b");
    write(root, "lib/main.rs", "fn main() {}");
    write(root, "docs/readme.md", "# docs");
    dir
}

async fn run(root: &Path, cache: Option<&Path>) -> FingerprintReport {
    let session = BuildSession::new(root, cache, false).unwrap();
    let report = session.fingerprints().await.unwrap();
    session.shutdown().await;
    report
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fingerprints_follow_dependency_order_and_inputs() {
    let dir = setup();
    let report = run(dir.path(), None).await;

    let names: Vec<_> = report.targets.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["docs", "util", "lib"]);
    let util = report.targets.iter().find(|t| t.name == "util").unwrap();
    assert_eq!(util.input_files, 2);

    // Same inputs => same fingerprints.
    let again = run(dir.path(), None).await;
    for name in ["docs", "util", "lib"] {
        assert_eq!(fingerprint(&report, name), fingerprint(&again, name));
    }

    // Changing a dependency's input changes dependents but not unrelated targets.
    write(dir.path(), "util/nested/b.txt", "changed");
    let changed = run(dir.path(), None).await;
    assert_ne!(fingerprint(&report, "util"), fingerprint(&changed, "util"));
    assert_ne!(fingerprint(&report, "lib"), fingerprint(&changed, "lib"));
    assert_eq!(fingerprint(&report, "docs"), fingerprint(&changed, "docs"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_cache_sees_changes_between_sessions() {
    let dir = setup();
    let cache = tempfile::tempdir().unwrap();

    let first = run(dir.path(), Some(cache.path())).await;
    let cached = run(dir.path(), Some(cache.path())).await;
    assert_eq!(fingerprint(&first, "lib"), fingerprint(&cached, "lib"));

    write(dir.path(), "lib/main.rs", "fn main() { println!(); }");
    let changed = run(dir.path(), Some(cache.path())).await;
    assert_ne!(fingerprint(&first, "lib"), fingerprint(&changed, "lib"));
    assert_eq!(fingerprint(&first, "util"), fingerprint(&changed, "util"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_invalid_config() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "turbobuild.toml",
        "[targets.a]\ndeps = [\"missing\"]",
    );
    let session = BuildSession::new(dir.path(), None, false).unwrap();
    let err = session.fingerprints().await.unwrap_err();
    assert!(
        format!("{err:?}").contains("unknown target `missing`"),
        "{err:?}"
    );
    session.shutdown().await;
}
