# turbobuild

An incremental build system built on the **turbo engine**
([`turbo-tasks`](https://github.com/vercel/next.js/tree/canary/turbopack/crates/turbo-tasks)),
the memoization and invalidation engine behind Turbopack.

Every unit of work (reading the config, hashing a file, fingerprinting a
target) is a memoized turbo task. The engine records what each task reads, so
after a file change only the tasks that depend on it re-run, both within a
watching process and across runs via the persistent on-disk cache.

> Status: early scaffold. Targets can be declared and fingerprinted;
> executing target commands is not implemented yet.

## Repository layout

```text
.
├── Cargo.toml              # workspace: shared deps, lints, profiles, engine pin
├── rust-toolchain.toml     # nightly toolchain required by turbo-tasks
├── .cargo/config.toml      # rustflags (tokio_unstable) and `cargo tb` alias
├── crates/
│   ├── turbobuild-core/    # engine-agnostic model: config parsing, target graph
│   ├── turbobuild-engine/  # turbo tasks: file hashing, target fingerprints, sessions
│   └── turbobuild-cli/     # the `turbobuild` binary
└── examples/
    ├── basic/              # a small sample project (turbobuild.toml)
    └── turbo/              # the same idea configured with turbo.json
```

`turbobuild-core` has no engine dependency and is plain, easily testable Rust.
`turbobuild-engine` wraps it in turbo tasks. Put new engine-facing logic there,
and keep pure data structures and algorithms in `core`.

## Getting started

The turbo engine requires a nightly toolchain, which `rust-toolchain.toml`
pins. `rustup` installs it automatically on first build.

```bash
cargo build
cargo test --workspace
cargo tb -C examples/basic fingerprint          # one-shot
cargo tb -C examples/basic fingerprint --watch  # recompute on file changes
cargo tb -C examples/turbo fingerprint          # a turbo.json project
```

The persistent cache is stored in `<project>/.turbobuild/cache`. Pass
`--no-cache` to run fully in memory.

## Project configuration

A project is a directory with a `turbobuild.toml`:

```toml
[targets.shared]
inputs = ["shared/**/*.txt"]   # globs, relative to the project root

[targets.app]
inputs = ["app/src/**"]
deps = ["shared"]              # must exist; cycles are rejected
command = "cargo build -p app" # part of the fingerprint
```

A target's fingerprint hashes its name, its input files, its command, and the
fingerprints of its dependencies. So changing `shared/` changes the
fingerprints of `shared` and `app`, and leaves unrelated targets alone.

### Turborepo compatibility (`turbo.json`)

When there is no `turbobuild.toml`, turbobuild reads Turborepo's `turbo.json`
instead (having both is an error). Only the `tasks` section is supported, in
Turborepo's single-package mode: the project root is one package and every
task becomes a target.

```jsonc
{
  "tasks": {
    "build": { "dependsOn": ["^build", "codegen"], "inputs": ["src/**"] },
    "codegen": { "inputs": ["schema/*.graphql"] }
  }
}
```

| `turbo.json`                  | turbobuild                                                   |
| ----------------------------- | ------------------------------------------------------------ |
| task name                     | target name                                                  |
| `dependsOn: ["task"]`         | `deps`                                                       |
| `dependsOn: ["^task"]`        | ignored: a single package has no upstream packages           |
| `inputs`                      | `inputs`; omitted or `$TURBO_DEFAULT$` means all files (`**`) |
| `package.json` `scripts.<task>` | `command`; a task without a script has no command           |

Not supported yet, and rejected with an error: package tasks (`pkg#task`,
`//#task`), negated input globs (`!pattern`), and `$ENV` entries in
`dependsOn`. Other keys (`outputs`, `cache`, `env`, `persistent`,
`globalDependencies`, ...) are accepted and ignored for now. `node_modules` is
never an input. Comments and trailing commas are allowed, as in Turborepo.

## Task graph

```text
project_fingerprints_operation(root_dir)         operation root
├── project_root(root_dir)                       disk file system (+ watcher)
├── load_project(root)                           parses turbobuild.toml or turbo.json
└── target_fingerprint(project, name)            one task per target
    ├── target_spec(project, name)               isolates one target's config
    ├── inputs_hash(root, globs)
    │   └── glob_hash(read_glob(..))             one task per directory
    │       └── file_hash(path)                  one task per file
    └── target_fingerprint(project, dep)         recursion into deps
```

## Upgrading the turbo engine

The engine crates are not published on crates.io, so they are pulled from a
pinned `vercel/next.js` commit. To upgrade:

1. Change `rev` for every `turbo-*` entry in the root `Cargo.toml`.
2. Copy `channel` from that commit's `rust-toolchain.toml`.
3. Mirror any relevant `[patch.crates-io]` entries from that commit's
   `Cargo.toml` (currently `bincode`, `virtue`, `include_dir`).
4. Seed versions from upstream: copy that commit's `Cargo.lock` over ours, then
   run `cargo build`. The engine is only tested against upstream's exact
   dependency versions, and newer semver-compatible releases have broken it
   before (for example `notify`).

## License

MIT
