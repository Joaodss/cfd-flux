mod backend;
mod bench;
mod render;
mod run;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{bail, Context, Result};
use cfd_core::examples;
use cfd_core::validate::{validate, Severity};
use cfd_core::Scene;
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "cfd-cli", version, about = "live-fluids command-line tool")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print (or write) the JSON Schema of the scene format.
    Schema {
        /// Output file; prints to stdout when omitted.
        out: Option<PathBuf>,
    },
    /// Write built-in example scenes as JSON.
    Example {
        /// Example name (see `--list`).
        name: Option<String>,
        /// Write every example into this directory.
        #[arg(long, value_name = "DIR", conflicts_with = "name")]
        all: Option<PathBuf>,
        /// List the available examples.
        #[arg(long)]
        list: bool,
        /// Output file for a single example; prints to stdout when omitted.
        #[arg(long, short)]
        out: Option<PathBuf>,
    },
    /// Check scene files for format and consistency errors.
    Validate {
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Run a scene headless: frames, PNGs, probe/force/diagnostic CSVs and metadata.
    Run(run::RunArgs),
    /// Measure solver throughput (MLUPS) at several resolutions.
    Bench(bench::BenchArgs),
    /// Render the frames of a run directory as PNG sequences.
    Render {
        /// Directory written by `cfd-cli run`.
        dir: PathBuf,
        #[command(flatten)]
        opts: render::RenderOpts,
    },
}

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<ExitCode> {
    match cli.command {
        Command::Schema { out } => {
            let json = serde_json::to_string_pretty(&Scene::json_schema())? + "\n";
            emit(out.as_deref(), &json)?;
        }
        Command::Example {
            name,
            all,
            list,
            out,
        } => {
            if list {
                for n in examples::NAMES {
                    println!("{n}");
                }
            } else if let Some(dir) = all {
                std::fs::create_dir_all(&dir)?;
                for n in examples::NAMES {
                    let path = dir.join(format!("{n}.json"));
                    emit(
                        Some(&path),
                        &(examples::by_name(n).unwrap().to_json_pretty() + "\n"),
                    )?;
                }
            } else if let Some(n) = name {
                let Some(scene) = examples::by_name(&n) else {
                    bail!(
                        "unknown example '{n}'; available: {}",
                        examples::NAMES.join(", ")
                    );
                };
                emit(out.as_deref(), &(scene.to_json_pretty() + "\n"))?;
            } else {
                bail!("give an example name, --all <DIR> or --list");
            }
        }
        Command::Run(args) => return run::run(args),
        Command::Bench(args) => bench::bench(args)?,
        Command::Render { dir, opts } => {
            render::render_dir(&dir, &opts)?;
        }
        Command::Validate { files } => {
            let mut failed = false;
            for file in files {
                let json = std::fs::read_to_string(&file)
                    .with_context(|| format!("cannot read {}", file.display()))?;
                let report = match Scene::from_json(&json) {
                    Ok(scene) => validate(&scene),
                    Err(e) => {
                        println!("{}: invalid JSON/format: {e}", file.display());
                        failed = true;
                        continue;
                    }
                };
                for issue in &report.issues {
                    let level = match issue.severity {
                        Severity::Error => "error",
                        Severity::Warning => "warning",
                    };
                    println!(
                        "{}: {level} [{}] {}",
                        file.display(),
                        issue.code,
                        issue.message
                    );
                }
                if report.is_ok() {
                    println!("{}: ok", file.display());
                } else {
                    failed = true;
                }
            }
            if failed {
                return Ok(ExitCode::FAILURE);
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

/// Writes to `path` (with LF line endings) or to stdout.
fn emit(path: Option<&Path>, contents: &str) -> Result<()> {
    match path {
        Some(p) => {
            std::fs::write(p, contents).with_context(|| format!("cannot write {}", p.display()))?;
            eprintln!("wrote {}", p.display());
        }
        None => print!("{contents}"),
    }
    Ok(())
}
