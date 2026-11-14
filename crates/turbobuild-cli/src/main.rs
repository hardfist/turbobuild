//! `turbobuild` command-line entry point.

use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;
use turbobuild_engine::{BuildSession, FingerprintReport};

#[derive(Parser)]
#[command(
    name = "turbobuild",
    version,
    about = "An incremental build system powered by the turbo engine"
)]
struct Cli {
    /// Project root (the directory containing `turbobuild.toml` or `turbo.json`).
    #[arg(long, short = 'C', global = true, default_value = ".")]
    dir: PathBuf,

    /// Disable the persistent cache in `<dir>/.turbobuild/cache`.
    #[arg(long, global = true)]
    no_cache: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print the content fingerprint of every target.
    Fingerprint {
        /// Keep running and recompute fingerprints whenever inputs change.
        #[arg(long)]
        watch: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let cli = Cli::parse();
    let cache_dir = (!cli.no_cache).then(|| cli.dir.join(".turbobuild").join("cache"));

    match cli.command {
        Command::Fingerprint { watch } => {
            let session = BuildSession::new(&cli.dir, cache_dir.as_deref(), watch)?;
            if watch {
                session.watch(|report| print_report(&report)).await;
            }
            print_report(&session.fingerprints().await?);
            session.shutdown().await;
        }
    }
    Ok(())
}

fn print_report(report: &FingerprintReport) {
    println!("---");
    for target in &report.targets {
        println!(
            "{:<24} {:016x}  ({} input file(s))",
            target.name, target.fingerprint, target.input_files
        );
    }
}
