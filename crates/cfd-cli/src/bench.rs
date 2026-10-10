//! `cfd-cli bench`: solver throughput in MLUPS (million fluid-lattice updates per second).
//!
//! Scenes: lid-driven cavity (isothermal, D2Q9) and differentially heated cavity (thermal,
//! D2Q9 + D2Q5), N² fluid cells, so walls and boundary links are included. Each measurement
//! runs a fixed number of steps after a warm-up; the median of the repeats is reported.

use std::time::Instant;

use anyhow::{Context, Result};
use cfd_core::{Domain, DomainOptions, Scene};
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
    #[command(flatten)]
    pub solver: SolverArgs,
}

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
    collision: &'static str,
    optimised_build: bool,
    results: Vec<BenchResult>,
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

pub fn bench(args: BenchArgs) -> Result<()> {
    let choice = args.solver.resolve(None)?;
    let optimised = !cfg!(debug_assertions);
    if !optimised {
        eprintln!("warning: unoptimised build; run `cargo run --release -p cfd-cli -- bench`");
    }
    let mut report = BenchReport {
        generator: format!("cfd-cli {}", env!("CARGO_PKG_VERSION")),
        cpu: cpu_name(),
        logical_cores: std::thread::available_parallelism().map_or(0, |n| n.get()),
        threads: choice.threads,
        backend: choice.backend,
        collision: choice.collision,
        optimised_build: optimised,
        results: Vec::new(),
    };
    eprintln!(
        "{} ({} logical cores), {} threads, {} backend, {}",
        report.cpu, report.logical_cores, report.threads, report.backend, report.collision
    );

    for &model in &args.model {
        for &n in &args.sizes {
            let opts = DomainOptions {
                lattice_velocity: 0.05,
                characteristic_velocity: None,
            };
            let domain = Domain::from_scene(&scene(model, n), &opts).map_err(|r| {
                anyhow::anyhow!("{model:?} {n}²: {:?}", r.errors().collect::<Vec<_>>())
            })?;
            let fluid = domain.fluid_cell_count();
            let mut solver = choice.make(domain)?;

            // Warm-up (caches, page faults, thread start-up), and a first rate estimate.
            let t = Instant::now();
            let mut warm = 0u64;
            while t.elapsed().as_secs_f64() < 0.3 || warm < 5 {
                solver.step(5).context("benchmark diverged")?;
                warm += 5;
            }
            let rate = warm as f64 / t.elapsed().as_secs_f64();
            let steps = ((rate * args.min_seconds).ceil() as u64).max(10);

            let mut mlups: Vec<f64> = (0..args.repeat.max(1))
                .map(|_| -> Result<f64> {
                    let t = Instant::now();
                    let mut left = steps;
                    while left > 0 {
                        let chunk = left.min(1000);
                        solver.step(chunk as u32).context("benchmark diverged")?;
                        left -= chunk;
                    }
                    Ok(fluid as f64 * steps as f64 / t.elapsed().as_secs_f64() / 1e6)
                })
                .collect::<Result<_>>()?;
            mlups.sort_by(f64::total_cmp);
            let r = BenchResult {
                model,
                size: n,
                fluid_cells: fluid,
                steps,
                mlups_median: mlups[mlups.len() / 2],
                mlups_min: mlups[0],
                mlups_max: mlups[mlups.len() - 1],
            };
            eprintln!(
                "  {model:?} {n}²: {:.1} MLUPS (min {:.1}, max {:.1}, {} steps × {})",
                r.mlups_median,
                r.mlups_min,
                r.mlups_max,
                steps,
                mlups.len()
            );
            report.results.push(r);
        }
    }

    println!(
        "CPU: {} ({} logical cores), {} threads, backend {}, collision {}, {}\n",
        report.cpu,
        report.logical_cores,
        report.threads,
        report.backend,
        report.collision,
        report.generator
    );
    println!("| Model | Grid (fluid) | Steps per run | MLUPS (median) | min | max |");
    println!("|-------|--------------|---------------|----------------|-----|-----|");
    for r in &report.results {
        let model = match r.model {
            Model::Iso => "isothermal (D2Q9)",
            Model::Thermal => "thermal (D2Q9 + D2Q5)",
        };
        println!(
            "| {model} | {n}² | {} | {:.1} | {:.1} | {:.1} |",
            r.steps,
            r.mlups_median,
            r.mlups_min,
            r.mlups_max,
            n = r.size
        );
    }
    if let Some(path) = &args.json {
        std::fs::write(path, serde_json::to_string_pretty(&report)? + "\n")
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    Ok(())
}
