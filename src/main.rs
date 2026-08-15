mod effects;
mod engine;
mod error;
pub type Result<T> = std::result::Result<T, error::Error>;

pub mod manifest;

use clap::{Parser, Subcommand};
use error::{error, Context};
use manifest::{Config, Facet, NamedSelection, VariantId};
use serde::Serialize;
use serde_json::{json, Map, Value};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "seni", version, about = "runtime config switching for nix")]
struct Cli {
    #[command(subcommand)]
    command: Command,

    /// path to the manifest; overrides SENI_MANIFEST
    #[arg(long, global = true, value_name = "PATH")]
    manifest: Option<PathBuf>,

    /// state directory; overrides SENI_STATE_DIR
    #[arg(long, global = true, value_name = "PATH")]
    state_dir: Option<PathBuf>,
}

#[derive(Subcommand)]
enum Command {
    /// create or update managed links and run effects
    Activate,

    /// remove seni-managed links and clear the active state
    Deactivate,

    /// list facets or the values of one facet
    Facets {
        /// facet whose values to list
        #[arg(value_name = "FACET")]
        facet: Option<String>,

        /// print JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// show the current selection
    Selection {
        /// print JSON instead of text
        #[arg(long)]
        json: bool,
    },

    /// change selected facet values and run affected effects
    Switch {
        /// facet value to select
        #[arg(value_name = "FACET=VALUE")]
        set: Vec<String>,
    },
}

fn main() -> ExitCode {
    let result: crate::Result<()> = (|| {
        let Cli {
            command,
            manifest,
            state_dir,
        } = Cli::parse();
        let absolute = |path: PathBuf| -> crate::Result<PathBuf> {
            if path.is_absolute() {
                Ok(path)
            } else {
                Ok(env::current_dir().context("current directory")?.join(path))
            }
        };

        if let Command::Deactivate = &command {
            let state_dir = state_dir
                .or_else(|| env::var_os("SENI_STATE_DIR").map(PathBuf::from))
                .context("state directory not configured; use --state-dir or SENI_STATE_DIR")?;
            let state_dir = absolute(state_dir)?;
            let summary = engine::deactivate(&state_dir)?;
            println!(
                "removed={} missing={} changed={} failed={}",
                summary.removed, summary.missing, summary.changed, summary.failed
            );
            if summary.failed != 0 {
                return Err(error!(
                    "deactivation incomplete: {} managed links remain",
                    summary.failed
                ));
            }
            println!("deactivated configuration");
            return Ok(());
        }

        let manifest_path = manifest
            .or_else(|| env::var_os("SENI_MANIFEST").map(PathBuf::from))
            .context("manifest not configured; use --manifest or SENI_MANIFEST")?;
        let manifest_path = fs::canonicalize(&manifest_path)
            .context(format_args!("manifest '{}'", manifest_path.display()))?;
        let manifest_file = fs::File::open(&manifest_path)
            .context(format_args!("manifest '{}'", manifest_path.display()))?;
        let config = Config::parse(manifest_file)?;

        let state_dir = state_dir
            .or_else(|| env::var_os("SENI_STATE_DIR").map(PathBuf::from))
            .unwrap_or_else(|| config.home.join(".local/state/seni"));
        let state_dir = absolute(state_dir)?;

        match command {
            Command::Activate => {
                engine::activate(&config, &manifest_path, &state_dir)?;
                println!("activated configuration");
            }
            Command::Deactivate => unreachable!(),
            Command::Facets { facet, json } => {
                let selection = engine::current_selection(&config, &state_dir)?;
                if let Some(name) = facet {
                    let facet_id = config
                        .facet_id(&name)
                        .context(format_args!("unknown facet '{name}'"))?;
                    let facet = &config[facet_id];
                    if json {
                        print_json(&facet_json(facet, selection[facet_id]))?;
                    } else {
                        let current = facet.variant(selection[facet_id]).0;
                        for variant in facet.variants.keys() {
                            let prefix = if variant.as_ref() == current {
                                "*"
                            } else {
                                " "
                            };
                            println!("{prefix} {variant}");
                        }
                    }
                } else if json {
                    let facets = config
                        .facets()
                        .map(|(facet_id, name, facet)| {
                            (name.to_owned(), facet_json(facet, selection[facet_id]))
                        })
                        .collect::<Map<_, _>>();
                    print_json(&facets)?;
                } else {
                    for (facet_id, name, facet) in config.facets() {
                        println!(
                            "{name} current={} default={} variants={}",
                            facet.variant(selection[facet_id]).0,
                            facet.variant(facet.default).0,
                            facet.variants.len()
                        );
                    }
                }
            }
            Command::Selection { json } => {
                let selection = engine::current_selection(&config, &state_dir)?;
                if json {
                    print_json(&NamedSelection {
                        config: &config,
                        selection: &selection,
                    })?;
                } else {
                    for (facet_id, name, facet) in config.facets() {
                        println!("{name}={}", facet.variant(selection[facet_id]).0);
                    }
                }
            }
            Command::Switch { set } => {
                engine::switch(&config, &manifest_path, &state_dir, &set)?;
                println!("switched selection");
            }
        }

        Ok(())
    })();

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("seni: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_json(value: &impl Serialize) -> crate::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer_pretty(&mut stdout, value).context("JSON output")?;
    writeln!(stdout).context("stdout")?;
    Ok(())
}

fn facet_json(facet: &Facet, current: VariantId) -> Value {
    json!({
        "current": facet.variant(current).0,
        "default": facet.variant(facet.default).0,
        "variants": facet.variants.keys().collect::<Vec<_>>(),
    })
}
