//! `cfd-cli compare`: runs the same scene on several backends and reports their differences
//! against the first one (`cfd_verify::compare`; method in `docs/04-numerical-methods.md` §7).
//!
//! Output layout:
//!
//! ```text
//! DIR/README.md                  report (Markdown)
//! DIR/compare.json               the same, machine-readable
//! DIR/scene.json                 copy of the input (scene mode)
//! DIR/reference-<field>.png      reference speed (and temperature)
//! DIR/<backend>-<field>-diff.png difference maps of fields that are not bit-identical
//! DIR/kinetic-energy.svg, forces.svg  time series, one line per backend
//! ```
//!
//! With `--validation`, every validation geometry goes into one report (default directory
//! `validation/compare/`), with image names prefixed by the case id.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::{bail, Context, Result};
use cfd_core::units::DEFAULT_LATTICE_VELOCITY;
use cfd_core::validate::Severity;
use cfd_core::{Domain, DomainOptions, Scene, Solver};
use cfd_verify::compare::{self, Engine, Maps, Mode, Plan, ReportInfo, Setup};
use cfd_verify::parity;
use clap::{Args, ValueEnum};

use crate::backend::{self, EngineArg, EngineArgs, SolverChoice};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum MapsArg {
    /// No images.
    None,
    /// Difference maps only for fields that are not bit-identical.
    Differing,
    /// Difference maps for every field.
    All,
}

#[derive(Debug, Args)]
pub struct CompareArgs {
    /// Scene file (JSON).
    #[arg(required_unless_present = "validation")]
    pub scene: Option<PathBuf>,
    /// Compare the validation geometries (`cfd_verify::scenes`) instead of a scene.
    #[arg(long, conflicts_with_all = [
        "scene", "steps", "end_time", "lattice_velocity", "statistics", "stats_from",
        "ref_length", "ref_velocity", "sample_every",
    ])]
    pub validation: bool,
    /// Output directory [required for a scene; default with --validation: validation/compare].
    #[arg(long, short)]
    pub out: Option<PathBuf>,
    /// Overwrite a previous report in a non-empty directory.
    #[arg(long)]
    pub force: bool,
    /// Backends to compare; the first is the reference [default: every available backend:
    /// cpu, cuda, wgpu]. The same backend may appear twice (labelled `cpu-2`, …).
    #[arg(long, value_enum, value_delimiter = ',')]
    pub backends: Option<Vec<EngineArg>>,
    /// Number of steps.
    #[arg(long, default_value_t = 1000, conflicts_with = "end_time")]
    pub steps: u64,
    /// Simulated time in seconds, instead of --steps.
    #[arg(long, value_name = "SECONDS")]
    pub end_time: Option<f64>,
    /// Lattice velocity the characteristic velocity maps to (sets the time step).
    #[arg(long, default_value_t = DEFAULT_LATTICE_VELOCITY)]
    pub lattice_velocity: f64,
    /// Limit of the 04 §7 field metric (0 requires bit-identical fields).
    #[arg(long, default_value_t = parity::TOLERANCE)]
    pub tolerance: f64,
    /// Judge the force statistics instead of the instantaneous fields (chaotic flows).
    #[arg(long)]
    pub statistics: bool,
    /// Start of the statistics window in seconds [default: half the run].
    #[arg(long, value_name = "SECONDS")]
    pub stats_from: Option<f64>,
    /// Relative limit of the statistics (frequency; mean/min/max force vs the largest force).
    #[arg(long, default_value_t = compare::STATS_TOLERANCE)]
    pub stats_tolerance: f64,
    /// Reference length (m) for St and force coefficients [default: the scene's characteristic length].
    #[arg(long, value_name = "METRES")]
    pub ref_length: Option<f64>,
    /// Reference velocity (m/s) for St and force coefficients [default: the scene's characteristic velocity].
    #[arg(long, value_name = "M_PER_S")]
    pub ref_velocity: Option<f64>,
    /// Steps between samples of the diagnostics and forces [default: steps / 500].
    #[arg(long)]
    pub sample_every: Option<u64>,
    /// Which difference maps to write.
    #[arg(long, value_enum, default_value = "differing")]
    pub maps: MapsArg,
    /// Long side of the images in pixels [default: 480; 240 with --validation].
    #[arg(long)]
    pub map_size: Option<u32>,
    #[command(flatten)]
    pub engine: EngineArgs,
}

/// Opens the requested backends (an unavailable one is an error), or every available one.
fn open_backends(args: &CompareArgs) -> Result<Vec<(String, SolverChoice)>> {
    if let Some(list) = &args.backends {
        if list.len() < 2 {
            bail!("--backends needs at least two backends (the same one twice is allowed)");
        }
        return args.engine.open_all(list);
    }
    let mut opened = Vec::new();
    for b in [EngineArg::Cpu, EngineArg::Cuda, EngineArg::Wgpu] {
        match args.engine.open(b) {
            Ok(choice) => opened.push((b, choice)),
            Err(e) => eprintln!("note: {} unavailable, not compared: {e:#}", b.name()),
        }
    }
    if opened.len() < 2 {
        bail!(
            "only the CPU backend is available; compare it with itself with `--backends cpu,cpu`"
        );
    }
    Ok(backend::label(opened))
}

/// Refuses a non-empty directory without `--force`; with it, removes what a report writes.
fn prepare_out_dir(dir: &Path, force: bool) -> Result<()> {
    if dir.exists() && std::fs::read_dir(dir)?.next().is_some() && !force {
        bail!(
            "{} is not empty; use --force to overwrite a previous report",
            dir.display()
        );
    }
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))
}

fn scene_setup(args: &CompareArgs, path: &Path) -> Result<(Setup, String)> {
    let json =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let scene =
        Scene::from_json(&json).with_context(|| format!("invalid scene {}", path.display()))?;
    let opts = DomainOptions {
        lattice_velocity: args.lattice_velocity,
        characteristic_velocity: None,
    };
    let domain = match Domain::from_scene(&scene, &opts) {
        Ok(d) => d,
        Err(report) => {
            for i in &report.issues {
                let level = match i.severity {
                    Severity::Error => "error",
                    Severity::Warning => "warning",
                };
                eprintln!("{level} [{}] {}", i.code, i.message);
            }
            bail!("the scene cannot be run");
        }
    };
    for i in &domain.issues {
        eprintln!("warning [{}] {}", i.code, i.message);
    }
    let units = domain.units;
    let steps = match args.end_time {
        Some(t) => units.time_to_steps(t),
        None => args.steps,
    };
    let mut plan = Plan::new(&domain, steps);
    if args.statistics {
        plan.mode = Mode::Statistics;
    }
    if let Some(t) = args.stats_from {
        plan.stats_from = units.time_to_steps(t);
    }
    if let Some(n) = args.sample_every {
        plan.sample_every = n.max(1);
    }
    if let Some(l) = args.ref_length {
        plan.ref_length = l;
    }
    if let Some(u) = args.ref_velocity {
        plan.ref_velocity = u;
    }
    let setup = Setup {
        id: path
            .file_stem()
            .map_or_else(|| "scene".into(), |s| s.to_string_lossy().into_owned()),
        title: scene.name.clone(),
        description: scene.description.clone().unwrap_or_default(),
        element_names: compare::element_names(&scene),
        domain,
        initial: None,
        plan,
    };
    Ok((setup, json))
}

pub fn compare(args: CompareArgs) -> Result<ExitCode> {
    let wall = Instant::now();
    // (setups, output directory, scene JSON to copy); nothing is written before the backends open.
    let (mut setups, out, scene_json) = match (&args.scene, args.validation) {
        (Some(path), false) => {
            let Some(out) = args.out.clone() else {
                bail!("--out DIR is required when comparing a scene");
            };
            let (setup, json) = scene_setup(&args, path)?;
            (vec![setup], out, Some(json))
        }
        // The committed report is regenerated in place, like `verify`'s.
        _ => (
            compare::validation_set()?,
            args.out
                .clone()
                .unwrap_or_else(|| PathBuf::from("validation/compare")),
            None,
        ),
    };
    let map_size = args
        .map_size
        .unwrap_or(if args.validation { 240 } else { 480 });
    for s in &mut setups {
        s.plan.tolerance = args.tolerance;
        s.plan.stats_tolerance = args.stats_tolerance;
        s.plan.maps = match args.maps {
            MapsArg::None => Maps::None,
            MapsArg::Differing => Maps::Differing,
            MapsArg::All => Maps::All,
        };
        s.plan.map_size = map_size;
    }

    args.engine.configure_threads()?;
    let backends = open_backends(&args)?;
    match &scene_json {
        Some(json) => {
            prepare_out_dir(&out, args.force)?;
            std::fs::write(out.join("scene.json"), json)?;
        }
        None => std::fs::create_dir_all(&out)?,
    }
    // One factory per backend: closures capturing each choice (`Fn(Domain) -> Box<dyn Solver>`).
    let factories: Vec<_> = backends
        .iter()
        .map(|(_, choice)| move |d: Domain| -> Result<Box<dyn Solver>> { choice.make(d) })
        .collect();
    let engines: Vec<Engine> = backends
        .iter()
        .zip(&factories)
        .map(|((label, choice), f)| Engine {
            label: label.clone(),
            description: choice.describe(),
            factory: f,
        })
        .collect();
    let (_, first) = &backends[0];
    eprintln!(
        "compare: {} case(s) on {} (reference: {}), {}, {} threads",
        setups.len(),
        engines
            .iter()
            .map(|e| e.label.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        engines[0].label,
        first.collision,
        first.threads
    );

    let log = |msg: &str| eprintln!("{msg}");
    let cases: Vec<_> = setups
        .iter()
        .map(|s| compare::compare(s, &engines, &log))
        .collect();

    println!(
        "\n| Case | {} |",
        engines
            .iter()
            .skip(1)
            .map(|e| format!("{} vs {}", e.label, engines[0].label))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    println!("|------|{}", "---|".repeat(engines.len() - 1));
    for c in &cases {
        let cols: Vec<String> = c
            .comparisons
            .iter()
            .map(|x| compare::verdict(x, c.plan.mode))
            .collect();
        println!("| {} | {} |", c.id, cols.join(" | "));
    }
    let failed = cases.iter().filter(|c| !c.passed()).count();
    println!(
        "\n{} of {} case(s) within tolerance",
        cases.len() - failed,
        cases.len()
    );

    let info = ReportInfo {
        command: std::iter::once("cfd-cli".to_string())
            .chain(std::env::args().skip(1))
            .collect::<Vec<_>>()
            .join(" "),
        generator: format!("cfd-cli {}", env!("CARGO_PKG_VERSION")),
        commit: crate::verify::git_commit(),
        cpu: crate::bench::cpu_name(),
        threads: first.threads,
        collision: first.collision.into(),
        seconds: wall.elapsed().as_secs_f64(),
        // The committed report lives two levels below the repository root.
        docs: args.validation.then(|| "../../docs".into()),
    };
    compare::write(&out, &info, &cases)?;
    eprintln!("report written to {}", out.join("README.md").display());
    Ok(if failed == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    })
}
