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
        "turbo.json",
        r#"{
          // Turborepo single-package config
          "tasks": {
            "build": { "dependsOn": ["^build", "codegen"], "inputs": ["src/**"], "outputs": ["dist/**"] },
            "codegen": { "inputs": ["schema/**/*.graphql"] },
            "docs": { "inputs": ["docs/*.md"] },
            "lint": {}
          }
        }"#,
    );
    write(
        root,
        "package.json",
        r#"{ "scripts": { "build": "tsc", "codegen": "gen" } }"#,
    );
    write(root, "schema/a.graphql", "type Query { a: Int }");
    write(root, "schema/nested/b.graphql", "type Mutation { b: Int }");
    write(root, "src/index.ts", "export {}");
    write(root, "docs/readme.md", "# docs");
    write(root, "node_modules/dep/index.js", "ignored");
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
    let root = dir.path();
    let report = run(root, None).await;

    let names: Vec<_> = report.targets.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["codegen", "build", "docs", "lint"]);
    let codegen = report.targets.iter().find(|t| t.name == "codegen").unwrap();
    assert_eq!(codegen.input_files, 2);
    // `lint` has no `inputs`: every project file except `node_modules`.
    let lint = report.targets.iter().find(|t| t.name == "lint").unwrap();
    assert_eq!(lint.input_files, 6);

    // Same inputs => same fingerprints.
    let again = run(root, None).await;
    for name in ["codegen", "build", "docs", "lint"] {
        assert_eq!(fingerprint(&report, name), fingerprint(&again, name));
    }

    // Changing a dependency's input changes dependents but not unrelated targets.
    write(root, "schema/nested/b.graphql", "type Mutation { c: Int }");
    let changed = run(root, None).await;
    assert_ne!(
        fingerprint(&report, "codegen"),
        fingerprint(&changed, "codegen")
    );
    assert_ne!(
        fingerprint(&report, "build"),
        fingerprint(&changed, "build")
    );
    assert_eq!(fingerprint(&report, "docs"), fingerprint(&changed, "docs"));

    // Scripts in package.json are the task commands.
    write(
        root,
        "package.json",
        r#"{ "scripts": { "build": "tsc -b", "codegen": "gen" } }"#,
    );
    let rescripted = run(root, None).await;
    assert_ne!(
        fingerprint(&changed, "build"),
        fingerprint(&rescripted, "build")
    );
    assert_eq!(
        fingerprint(&changed, "codegen"),
        fingerprint(&rescripted, "codegen")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn persistent_cache_sees_changes_between_sessions() {
    let dir = setup();
    let cache = tempfile::tempdir().unwrap();

    let first = run(dir.path(), Some(cache.path())).await;
    let cached = run(dir.path(), Some(cache.path())).await;
    assert_eq!(fingerprint(&first, "build"), fingerprint(&cached, "build"));

    write(dir.path(), "src/index.ts", "export const x = 1;");
    let changed = run(dir.path(), Some(cache.path())).await;
    assert_ne!(fingerprint(&first, "build"), fingerprint(&changed, "build"));
    assert_eq!(
        fingerprint(&first, "codegen"),
        fingerprint(&changed, "codegen")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reports_missing_or_invalid_config() {
    let empty = tempfile::tempdir().unwrap();
    let session = BuildSession::new(empty.path(), None, false).unwrap();
    let err = session.fingerprints().await.unwrap_err();
    assert!(
        format!("{err:?}").contains("turbo.json not found"),
        "{err:?}"
    );
    session.shutdown().await;

    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "turbo.json",
        r#"{ "tasks": { "a": { "dependsOn": ["missing"] } } }"#,
    );
    let session = BuildSession::new(dir.path(), None, false).unwrap();
    let err = session.fingerprints().await.unwrap_err();
    assert!(
        format!("{err:?}").contains("unknown target `missing`"),
        "{err:?}"
    );
    session.shutdown().await;
}
