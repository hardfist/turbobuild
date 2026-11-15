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
│   ├── turbobuild-core/    # engine-agnostic model: turbo.json parsing, target graph
│   ├── turbobuild-engine/  # turbo tasks: file hashing, target fingerprints, sessions
│   └── turbobuild-cli/     # the `turbobuild` binary
└── examples/basic/         # a small sample project
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
```

The persistent cache is stored in `<project>/.turbobuild/cache`. Pass
`--no-cache` to run fully in memory.

## Project configuration (`turbo.json`)

A project is a directory with a `turbo.json`. Currently the `tasks` section is
supported in Turborepo's single-package mode: the project root is one package
and every task becomes a target.

```jsonc
{
  "$schema": "https://turborepo.com/schema.json",
  "tasks": {
    "build": { "dependsOn": ["^build", "codegen"], "inputs": ["src/**"] },
    "codegen": { "inputs": ["schema/*.graphql"] }
  }
}
```

A target's fingerprint hashes its name, its input files, its command, and the
fingerprints of its dependencies. So changing `schema/` changes the
fingerprints of `codegen` and `build`, and leaves unrelated targets alone.

| `turbo.json`                    | Behavior                                                      |
| ------------------------------- | ------------------------------------------------------------- |
| task name                       | target name                                                   |
| `dependsOn: ["task"]`           | dependency on `task` (must exist; cycles are rejected)        |
| `dependsOn: ["^task"]`          | ignored: a single package has no upstream packages            |
| `inputs`                        | globs relative to root; omitted or `$TURBO_DEFAULT$` means `**` |
| `package.json` `scripts.<task>` | target command; a task without a script has no command        |

Not supported yet, and rejected with an error: package tasks (`pkg#task`,
`//#task`), negated input globs (`!pattern`), and `$ENV` entries in
`dependsOn`. Other keys (`outputs`, `cache`, `env`, `persistent`,
`globalDependencies`, ...) are accepted and ignored for now. `node_modules` is
never an input. Comments and trailing commas are allowed, as in Turborepo.

## Task graph

```text
project_fingerprints_operation(root_dir)         operation root
├── project_root(root_dir)                       disk file system (+ watcher)
├── load_project(root)                           parses turbo.json and package.json
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
