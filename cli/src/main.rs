mod error;
pub mod manifest;
mod switching;

use clap::{Parser, Subcommand};
use error::AppError;
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
    let result = (|| {
        let cli = Cli::parse();
        let manifest_path = cli
            .manifest
            .or_else(|| env::var_os("SUMI_MANIFEST").map(PathBuf::from))
            .ok_or(AppError::ManifestNotConfigured)?;
        let manifest_path = fs::canonicalize(&manifest_path)
            .map_err(|source| AppError::fs("resolve manifest", &manifest_path, source))?;
        let manifest_file = fs::File::open(&manifest_path)
            .map_err(|source| AppError::fs("open manifest", &manifest_path, source))?;
        let config = Config::parse(manifest_file).map_err(|source| AppError::ParseManifest {
            path: manifest_path.clone(),
            source,
        })?;

        let state_dir = cli
            .state_dir
            .or_else(|| env::var_os("SUMI_STATE_DIR").map(PathBuf::from))
            .unwrap_or_else(|| config.home().join(".local/state/sumi"));
        let state_dir = if state_dir.is_absolute() {
            state_dir
        } else {
            env::current_dir()
                .map_err(|source| AppError::fs("resolve current directory", ".", source))?
                .join(state_dir)
        };

        match cli.command {
            Command::Switch { set } => {
                switching::switch(&config, &manifest_path, &state_dir, &set)?;
                println!("switched selection");
            }
        }

        Ok::<_, AppError>(())
    })();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sumi: {error}");
            ExitCode::FAILURE
        }
    }
}
