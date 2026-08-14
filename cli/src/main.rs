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
    Activate,
    Deactivate,
    Facets {
        facet: Option<String>,

        #[arg(long)]
        json: bool,
    },
    Selection {
        #[arg(long)]
        json: bool,
    },
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
            .unwrap_or_else(|| config.home.join(".local/state/sumi"));
        let state_dir = if state_dir.is_absolute() {
            state_dir
        } else {
            env::current_dir()
                .context("could not resolve current directory '.'")?
                .join(state_dir)
        };

        match cli.command {
            Command::Activate => {
                engine::activate(&config, &manifest_path, &state_dir)?;
                println!("activated configuration");
            }
            Command::Deactivate => {
                let summary = engine::deactivate(&state_dir)?;
                println!(
                    "removed={} missing={} changed={} failed={}",
                    summary.removed, summary.missing, summary.changed, summary.failed
                );
                if summary.failed != 0 {
                    return Err(error!(
                        "could not deactivate configuration: {} managed links could not be removed",
                        summary.failed
                    ));
                }
                println!("deactivated configuration");
            }
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
            eprintln!("sumi: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_json(value: &impl Serialize) -> crate::Result<()> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer_pretty(&mut stdout, value).context("could not serialize JSON")?;
    writeln!(stdout).context("could not write JSON")?;
    Ok(())
}

fn facet_json(facet: &Facet, current: VariantId) -> Value {
    json!({
        "current": facet.variant(current).0,
        "default": facet.variant(facet.default).0,
        "variants": facet.variants.keys().collect::<Vec<_>>(),
    })
}
