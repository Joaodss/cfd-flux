//! `cfd-cli bench`: solver throughput in MLUPS (million fluid-lattice updates per second).
//!
//! Scenes: lid-driven cavity (isothermal, D2Q9) and differentially heated cavity (thermal,
//! D2Q9 + D2Q5), N² fluid cells, so walls and boundary links are included. Each measurement
//! runs a fixed number of steps after a warm-up; the median of the repeats is reported.
//!
//! `--backends a,b` measures several backends in one interleaved run (a, b, a, b, …), so that
//! anything else happening on the machine affects every column alike.

use std::fmt::Write as _;
use std::time::Instant;

use anyhow::{Context, Result};
use cfd_core::{Domain, DomainOptions, Scene, Solver};
use cfd_engines::{EngineKind, SolverChoice};
use cfd_verify::scenes;
use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::backend::SolverArgs;

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Model {
    /// Lid-driven cavity, Re = 100.
    Iso,
    /// Heated cavity, Ra = 1e5, Pr = 0.71.
    Thermal,
}

impl Model {
    fn title(self) -> &'static str {
        match self {
            Model::Iso => "isothermal (D2Q9)",
            Model::Thermal => "thermal (D2Q9 + D2Q5)",
        }
    }
}

#[derive(Debug, Args)]
pub struct BenchArgs {
    /// Fluid cells per side.
    #[arg(long, value_delimiter = ',', default_value = "256,512,1024,2048")]
    pub sizes: Vec<u32>,
    /// Models to measure.
    #[arg(long, value_enum, value_delimiter = ',', default_value = "iso,thermal")]
    pub model: Vec<Model>,
    /// Timed repetitions per case (the median is reported).
    #[arg(long, default_value_t = 3)]
    pub repeat: usize,
    /// Minimum duration of each timed repetition, in seconds.
    #[arg(long, default_value_t = 2.0)]
    pub min_seconds: f64,
    /// Also write the results as JSON.
    #[arg(long, value_name = "FILE")]
    pub json: Option<std::path::PathBuf>,
    /// Several backends in one interleaved run (one table, a column per backend), instead of
    /// --backend. The same backend may appear twice (labelled `cuda-2`, …).
    #[arg(long, value_enum, value_delimiter = ',', conflicts_with = "backend")]
    pub backends: Option<Vec<EngineKind>>,
    #[command(flatten)]
    pub solver: SolverArgs,
}

/// One backend's measurement of one case.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
struct Timing {
    steps: u64,
    mlups_median: f64,
    mlups_min: f64,
    mlups_max: f64,
}

/// One case, with a timing per backend.
#[derive(Debug, Clone, PartialEq)]
struct Row {
    model: Model,
    size: u32,
    fluid_cells: usize,
    timings: Vec<Timing>,
}

/// JSON of a single-backend run (format unchanged since Phase 1).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchResult {
    model: Model,
    size: u32,
    fluid_cells: usize,
    steps: u64,
    mlups_median: f64,
    mlups_min: f64,
    mlups_max: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BenchReport {
    generator: String,
    cpu: String,
    logical_cores: usize,
    threads: usize,
    backend: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    workgroup: Option<u32>,
    collision: &'static str,
    optimised_build: bool,
    results: Vec<BenchResult>,
}

/// JSON of a `--backends` run.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MultiReport {
    generator: String,
    cpu: String,
    logical_cores: usize,
    threads: usize,
    collision: &'static str,
    optimised_build: bool,
    backends: Vec<BackendInfo>,
    results: Vec<MultiResult>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct BackendInfo {
    label: String,
    backend: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    workgroup: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MultiResult {
    model: Model,
    size: u32,
    fluid_cells: usize,
    /// In the order of `backends`.
    timings: Vec<LabelledTiming>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct LabelledTiming {
    backend: String,
    #[serde(flatten)]
    timing: Timing,
}

/// CPU model name, best effort (no extra dependencies).
pub fn cpu_name() -> String {
    if let Ok(info) = std::fs::read_to_string("/proc/cpuinfo") {
        if let Some(line) = info.lines().find(|l| l.starts_with("model name")) {
            if let Some((_, name)) = line.split_once(':') {
                return name.trim().to_string();
            }
        }
    }
    let query = |cmd: &str, args: &[&str]| -> Option<String> {
        let out = std::process::Command::new(cmd).args(args).output().ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    if cfg!(windows) {
        let key = r"HKLM\HARDWARE\DESCRIPTION\System\CentralProcessor\0";
        if let Some(out) = query("reg", &["query", key, "/v", "ProcessorNameString"]) {
            if let Some(name) = out
                .lines()
                .find_map(|l| l.split_once("REG_SZ").map(|(_, n)| n.trim().to_string()))
            {
                return name;
            }
        }
    } else if cfg!(target_os = "macos") {
        if let Some(out) = query("sysctl", &["-n", "machdep.cpu.brand_string"]) {
            return out.trim().to_string();
        }
    }
    std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown CPU".into())
}

fn scene(model: Model, n: u32) -> Scene {
    match model {
        Model::Iso => scenes::lid_driven_cavity(n, 100.0),
        Model::Thermal => scenes::heated_cavity(n, 1e5, 0.71),
    }
}

/// Steps until the warm-up has lasted 0.3 s (caches, page faults, thread start-up, GPU clocks),
/// then the number of steps one timed repetition needs to last `min_seconds`.
fn warm_up(solver: &mut dyn Solver, min_seconds: f64) -> Result<u64> {
    let t = Instant::now();
    let mut warm = 0u64;
    while t.elapsed().as_secs_f64() < 0.3 || warm < 5 {
        solver.step(5).context("benchmark diverged")?;
        warm += 5;
    }
    solver.finish().context("benchmark diverged")?;
    let rate = warm as f64 / t.elapsed().as_secs_f64();
    Ok(((rate * min_seconds).ceil() as u64).max(10))
}

/// One timed repetition, in MLUPS.
fn timed(solver: &mut dyn Solver, steps: u64, fluid: usize) -> Result<f64> {
    let t = Instant::now();
    let mut left = steps;
    while left > 0 {
        let chunk = left.min(1000);
        solver.step(chunk as u32).context("benchmark diverged")?;
        left -= chunk;
    }
    // Asynchronous backends: wait for the queued steps before reading the clock.
    solver.finish().context("benchmark diverged")?;
    Ok(fluid as f64 * steps as f64 / t.elapsed().as_secs_f64() / 1e6)
}

fn summary(steps: u64, mut mlups: Vec<f64>) -> Timing {
    mlups.sort_by(f64::total_cmp);
    Timing {
        steps,
        mlups_median: mlups[mlups.len() / 2],
        mlups_min: mlups[0],
        mlups_max: mlups[mlups.len() - 1],
    }
}

/// Measures one case on every backend. The solvers all exist at once; after each one's warm-up,
/// the repetitions are interleaved: backend 1, backend 2, …, then again.
fn measure(
    backends: &[(String, SolverChoice)],
    domain: &Domain,
    args: &BenchArgs,
) -> Result<Vec<Timing>> {
    let fluid = domain.fluid_cell_count();
    let mut solvers = backends
        .iter()
        .map(|(_, c)| c.make(domain.clone()))
        .collect::<Result<Vec<_>>>()?;
    let steps = solvers
        .iter_mut()
        .map(|s| warm_up(s.as_mut(), args.min_seconds))
        .collect::<Result<Vec<_>>>()?;
    let mut mlups = vec![Vec::new(); solvers.len()];
    for _ in 0..args.repeat.max(1) {
        for (k, s) in solvers.iter_mut().enumerate() {
            mlups[k].push(timed(s.as_mut(), steps[k], fluid)?);
        }
    }
    Ok(steps
        .into_iter()
        .zip(mlups)
        .map(|(s, m)| summary(s, m))
        .collect())
}

/// The table of a single-backend run (format unchanged since Phase 1).
fn single_table(rows: &[Row]) -> String {
    let mut md = String::new();
    let _ = writeln!(
        md,
        "| Model | Grid (fluid) | Steps per run | MLUPS (median) | min | max |"
    );
    let _ = writeln!(
        md,
        "|-------|--------------|---------------|----------------|-----|-----|"
    );
    for r in rows {
        let t = r.timings[0];
        let _ = writeln!(
            md,
            "| {} | {}² | {} | {:.1} | {:.1} | {:.1} |",
            r.model.title(),
            r.size,
            t.steps,
            t.mlups_median,
            t.mlups_min,
            t.mlups_max
        );
    }
    md
}

/// One column per backend (median, with min–max); with exactly two backends, their ratio.
fn multi_table(labels: &[String], rows: &[Row]) -> String {
    let mut md = String::new();
    let ratio = labels.len() == 2;
    let cols: Vec<String> = labels
        .iter()
        .map(|l| format!("{l} MLUPS (min–max)"))
        .collect();
    let _ = write!(md, "| Model | Grid (fluid) | {} |", cols.join(" | "));
    if ratio {
        let _ = write!(md, " {} / {} |", labels[1], labels[0]);
    }
    let _ = writeln!(md);
    let _ = write!(
        md,
        "|-------|--------------|{}",
        "---:|".repeat(labels.len())
    );
    if ratio {
        md.push_str("---:|");
    }
    let _ = writeln!(md);
    for r in rows {
        let cells: Vec<String> = r
            .timings
            .iter()
            .map(|t| {
                format!(
                    "{:.0} ({:.0}–{:.0})",
                    t.mlups_median, t.mlups_min, t.mlups_max
                )
            })
            .collect();
        let _ = write!(
            md,
            "| {} | {}² | {} |",
            r.model.title(),
            r.size,
            cells.join(" | ")
        );
        if ratio {
            let _ = write!(
                md,
                " {:.3} |",
                r.timings[1].mlups_median / r.timings[0].mlups_median
            );
        }
        let _ = writeln!(md);
    }
    md
}

pub fn bench(args: BenchArgs) -> Result<()> {
    let backends: Vec<(String, SolverChoice)> = match &args.backends {
        Some(list) => {
            args.solver.engine.configure_threads()?;
            args.solver.engine.open_all(list)?
        }
        None => {
            let choice = args.solver.resolve(None)?;
            vec![(choice.backend.to_string(), choice)]
        }
    };
    let optimised = !cfg!(debug_assertions);
    if !optimised {
        eprintln!("warning: unoptimised build; run `cargo run --release -p cfd-cli -- bench`");
    }
    let first = &backends[0].1;
    let (cpu, cores) = (
        cpu_name(),
        std::thread::available_parallelism().map_or(0, |n| n.get()),
    );
    eprintln!(
        "{cpu} ({cores} logical cores), {} threads, {}",
        first.threads, first.collision
    );
    for (label, c) in &backends {
        eprintln!("  {label}: {}", c.describe());
    }

    let mut rows = Vec::new();
    for &model in &args.model {
        for &n in &args.sizes {
            let opts = DomainOptions {
                lattice_velocity: 0.05,
                characteristic_velocity: None,
            };
            let domain = Domain::from_scene(&scene(model, n), &opts).map_err(|r| {
                anyhow::anyhow!("{model:?} {n}²: {:?}", r.errors().collect::<Vec<_>>())
            })?;
            let timings = measure(&backends, &domain, &args)?;
            for ((label, _), t) in backends.iter().zip(&timings) {
                eprintln!(
                    "  {model:?} {n}² {label}: {:.1} MLUPS (min {:.1}, max {:.1}, {} steps × {})",
                    t.mlups_median,
                    t.mlups_min,
                    t.mlups_max,
                    t.steps,
                    args.repeat.max(1)
                );
            }
            rows.push(Row {
                model,
                size: n,
                fluid_cells: domain.fluid_cell_count(),
                timings,
            });
        }
    }

    let generator = format!("cfd-cli {}", env!("CARGO_PKG_VERSION"));
    let json = if backends.len() == 1 {
        println!(
            "CPU: {cpu} ({cores} logical cores), {} threads, backend {}{}, collision {}, {generator}\n",
            first.threads,
            first.describe(),
            first
                .workgroup()
                .map_or(String::new(), |w| format!(", workgroup {w}")),
            first.collision,
        );
        print!("{}", single_table(&rows));
        serde_json::to_string_pretty(&BenchReport {
            generator,
            cpu,
            logical_cores: cores,
            threads: first.threads,
            backend: first.backend,
            device: first.device.clone(),
            workgroup: first.workgroup(),
            collision: first.collision,
            optimised_build: optimised,
            results: rows
                .iter()
                .map(|r| BenchResult {
                    model: r.model,
                    size: r.size,
                    fluid_cells: r.fluid_cells,
                    steps: r.timings[0].steps,
                    mlups_median: r.timings[0].mlups_median,
                    mlups_min: r.timings[0].mlups_min,
                    mlups_max: r.timings[0].mlups_max,
                })
                .collect(),
        })?
    } else {
        println!(
            "CPU: {cpu} ({cores} logical cores), {} threads, collision {}, {generator}; \
             interleaved runs, {} repetitions of ≥ {} s each",
            first.threads,
            first.collision,
            args.repeat.max(1),
            args.min_seconds
        );
        for (label, c) in &backends {
            println!(
                "- {label}: {}{}",
                c.describe(),
                c.workgroup()
                    .map_or(String::new(), |w| format!(", workgroup/block {w}"))
            );
        }
        println!();
        let labels: Vec<String> = backends.iter().map(|(l, _)| l.clone()).collect();
        print!("{}", multi_table(&labels, &rows));
        serde_json::to_string_pretty(&MultiReport {
            generator,
            cpu,
            logical_cores: cores,
            threads: first.threads,
            collision: first.collision,
            optimised_build: optimised,
            backends: backends
                .iter()
                .map(|(label, c)| BackendInfo {
                    label: label.clone(),
                    backend: c.backend,
                    device: c.device.clone(),
                    workgroup: c.workgroup(),
                })
                .collect(),
            results: rows
                .iter()
                .map(|r| MultiResult {
                    model: r.model,
                    size: r.size,
                    fluid_cells: r.fluid_cells,
                    timings: labels
                        .iter()
                        .zip(&r.timings)
                        .map(|(l, t)| LabelledTiming {
                            backend: l.clone(),
                            timing: *t,
                        })
                        .collect(),
                })
                .collect(),
        })?
    };
    if let Some(path) = &args.json {
        std::fs::write(path, json + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        bench: BenchArgs,
    }

    fn parse(args: &[&str]) -> Result<BenchArgs, clap::Error> {
        Cli::try_parse_from(std::iter::once("bench").chain(args.iter().copied())).map(|c| c.bench)
    }

    #[test]
    fn backends_parse_as_a_list_of_engines() {
        let a = parse(&["--backends", "wgpu,cuda", "--sizes", "1024"]).unwrap();
        assert_eq!(a.backends, Some(vec![EngineKind::Wgpu, EngineKind::Cuda]));
        assert_eq!(a.sizes, [1024]);
        assert!(a.solver.backend.is_none());
        // Engine options still apply to every backend.
        let a = parse(&["--backends", "cuda,cuda", "--workgroup", "256"]).unwrap();
        assert_eq!(a.solver.engine.workgroup, 256);
        // A single backend keeps working as before.
        assert!(parse(&["--backend", "cpu"]).unwrap().backends.is_none());
    }

    #[test]
    fn backends_reject_auto_and_conflict_with_backend() {
        assert!(parse(&["--backends", "cpu,auto"]).is_err());
        let e = parse(&["--backends", "wgpu,cuda", "--backend", "cpu"]).unwrap_err();
        assert_eq!(e.kind(), clap::error::ErrorKind::ArgumentConflict);
    }

    #[test]
    fn tables() {
        let t = |m: f64| Timing {
            steps: 100,
            mlups_median: m,
            mlups_min: m - 10.0,
            mlups_max: m + 10.0,
        };
        let rows = [Row {
            model: Model::Iso,
            size: 1024,
            fluid_cells: 1 << 20,
            timings: vec![t(2873.0), t(2969.0)],
        }];
        let md = multi_table(&["cuda".into(), "wgpu".into()], &rows);
        let lines: Vec<&str> = md.lines().collect();
        assert_eq!(
            lines[0],
            "| Model | Grid (fluid) | cuda MLUPS (min–max) | wgpu MLUPS (min–max) | wgpu / cuda |"
        );
        assert_eq!(lines[1], "|-------|--------------|---:|---:|---:|");
        assert_eq!(
            lines[2],
            "| isothermal (D2Q9) | 1024² | 2873 (2863–2883) | 2969 (2959–2979) | 1.033 |"
        );
        // Three backends: no ratio column.
        let three = multi_table(&["a".into(), "b".into(), "c".into()], &[]);
        assert!(!three.contains(" / "));
        assert!(single_table(&rows).contains("| isothermal (D2Q9) | 1024² | 100 | 2873.0 |"));
    }
}
