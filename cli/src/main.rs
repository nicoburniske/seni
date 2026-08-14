mod error;
pub type Result<T> = std::result::Result<T, error::Error>;

pub mod manifest;
mod switching;

use clap::{Parser, Subcommand};
use error::Context;
use manifest::Config;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "sumi", version, about = "facet-based runtime config switching")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    #[arg(long, global = true)]
    manifest: Option<PathBuf>,

    #[arg(long, global = true)]
    state_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    Switch {
        #[arg(value_name = "FACET=VALUE")]
        set: Vec<String>,
    },
}

fn main() -> ExitCode {
    let result: crate::Result<()> = (|| {
        let cli = Cli::parse();
        let manifest_path = cli
            .manifest
            .or_else(|| env::var_os("SUMI_MANIFEST").map(PathBuf::from))
            .context("manifest not configured; pass --manifest or set SUMI_MANIFEST")?;
        let manifest_path = fs::canonicalize(&manifest_path).context(format_args!(
            "could not resolve manifest '{}'",
            manifest_path.display()
        ))?;
        let manifest_file = fs::File::open(&manifest_path).context(format_args!(
            "could not open manifest '{}'",
            manifest_path.display()
        ))?;
        let config = Config::parse(manifest_file)?;

        let state_dir = cli
            .state_dir
            .or_else(|| env::var_os("SUMI_STATE_DIR").map(PathBuf::from))
            .unwrap_or_else(|| config.home().join(".local/state/sumi"));
        let state_dir = if state_dir.is_absolute() {
            state_dir
        } else {
            env::current_dir()
                .context("could not resolve current directory '.'")?
                .join(state_dir)
        };

        match cli.command {
            Command::Switch { set } => {
                switching::switch(&config, &manifest_path, &state_dir, &set)?;
                println!("switched selection");
            }
        }

        Ok(())
    })();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sumi: {error}");
            ExitCode::FAILURE
        }
    }
}
