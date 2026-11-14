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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn supports_turbo_json_tasks() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(
        root,
        "turbo.json",
        r#"{
          // Turborepo single-package config
          "tasks": {
            "build": { "dependsOn": ["^build", "codegen"], "inputs": ["src/**"], "outputs": ["dist/**"] },
            "codegen": { "inputs": ["schema/*.graphql"] },
            "lint": {}
          }
        }"#,
    );
    write(
        root,
        "package.json",
        r#"{ "scripts": { "build": "tsc", "codegen": "gen" } }"#,
    );
    write(root, "src/index.ts", "export {}");
    write(root, "schema/api.graphql", "type Query { a: Int }");
    write(root, "node_modules/dep/index.js", "ignored");

    let report = run(root, None).await;
    let names: Vec<_> = report.targets.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(names, ["codegen", "build", "lint"]);
    // `lint` has no `inputs`: every project file except `node_modules`.
    let lint = report.targets.iter().find(|t| t.name == "lint").unwrap();
    assert_eq!(lint.input_files, 4);

    // A dependency task's input invalidates its dependents.
    write(root, "schema/api.graphql", "type Query { b: Int }");
    let changed = run(root, None).await;
    assert_ne!(
        fingerprint(&report, "codegen"),
        fingerprint(&changed, "codegen")
    );
    assert_ne!(
        fingerprint(&report, "build"),
        fingerprint(&changed, "build")
    );

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
async fn rejects_both_config_files() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "turbobuild.toml", "[targets.a]");
    write(dir.path(), "turbo.json", r#"{ "tasks": {} }"#);
    let session = BuildSession::new(dir.path(), None, false).unwrap();
    let err = session.fingerprints().await.unwrap_err();
    assert!(format!("{err:?}").contains("found both"), "{err:?}");
    session.shutdown().await;
}
