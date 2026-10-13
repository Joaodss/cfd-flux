//! `cfd-cli run`: runs a scene headless through `cfd-job` (frames, CSV time series, metadata in
//! the layout of `docs/03-data-model.md` §6.2), then renders PNGs from the frames.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cfd_core::scene::OutputField;
use cfd_core::units::DEFAULT_LATTICE_VELOCITY;
use cfd_core::validate::Severity;
use cfd_io::frame::Dtype;
use cfd_job::{
    JobContext, JobObserver, Outcome, PrepareError, Progress, RunOptions, Started, Status,
};
use clap::{Args, ValueEnum};

use crate::backend::SolverArgs;
use crate::render::{self, PngField, RenderOpts};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum DtypeArg {
    F32,
    F16,
}

#[derive(Debug, Args)]
pub struct RunArgs {
    /// Scene file (JSON).
    pub scene: PathBuf,
    /// Output directory.
    #[arg(long, short)]
    pub out: PathBuf,
    /// Overwrite the outputs of a previous run in a non-empty directory.
    #[arg(long)]
    pub force: bool,
    #[command(flatten)]
    pub solver: SolverArgs,
    /// Lattice velocity the characteristic velocity maps to (sets the time step).
    #[arg(long, default_value_t = DEFAULT_LATTICE_VELOCITY)]
    pub lattice_velocity: f64,
    /// Simulated time in seconds (overrides the scene's `run.endTime`).
    #[arg(long, value_name = "SECONDS")]
    pub end_time: Option<f64>,
    /// Time between frames in seconds (overrides `run.outputInterval`).
    #[arg(long, value_name = "SECONDS")]
    pub output_interval: Option<f64>,
    /// Time between probe/force/diagnostic samples in seconds (default: output interval / 10).
    #[arg(long, value_name = "SECONDS")]
    pub sample_interval: Option<f64>,
    /// Precision of the stored frames.
    #[arg(long, value_enum, default_value = "f32")]
    pub dtype: DtypeArg,
    /// Do not write frames (also disables PNGs, which are rendered from the frames).
    #[arg(long)]
    pub no_frames: bool,
    #[command(flatten)]
    pub render: RenderOpts,
}

fn png_source(f: PngField) -> OutputField {
    match f {
        PngField::Velocity => OutputField::Velocity,
        PngField::Vorticity => OutputField::Vorticity,
        PngField::Temperature => OutputField::Temperature,
        PngField::Pressure => OutputField::Pressure,
    }
}

/// Prints the run to stderr: a summary line, progress every 2 s, the outcome.
struct Console {
    last_report: Mutex<Instant>,
}

impl JobObserver for Console {
    fn started(&self, s: &Started) {
        let (d, c) = (s.domain, s.choice);
        let (units, n) = (d.units, d.numbers);
        eprintln!(
            "{}: {}×{} cells, dx = {:.4e} m, dt = {:.4e} s, {} steps, Re = {:.4}, Ma = {:.3}, τ = {:.4} ({} backend, {}, {} threads)",
            s.scene,
            d.width,
            d.height,
            units.dx,
            units.dt,
            d.run.total,
            n.reynolds,
            n.mach,
            d.physics.tau(),
            c.describe(),
            c.collision,
            c.threads
        );
    }

    fn progress(&self, p: &Progress) {
        let mut last = self.last_report.lock().unwrap();
        if last.elapsed() > Duration::from_secs(2) {
            *last = Instant::now();
            eprintln!(
                "  step {}/{} ({:.0}%), t = {:.4} s, max Ma = {:.3}, {:.0} MLUPS",
                p.step,
                p.total,
                100.0 * p.step as f64 / p.total as f64,
                p.sim_time,
                p.max_mach,
                p.mlups
            );
        }
    }

    fn finished(&self, o: &Outcome) {
        if let Some(msg) = &o.message {
            eprintln!("error: {msg}");
        }
        eprintln!(
            "{} steps in {:.1} s ({:.0} MLUPS), {} frames → {}",
            o.steps_done,
            o.timings.solver_seconds,
            o.timings.mlups,
            o.frames,
            o.dir.display()
        );
    }
}

pub fn run(args: RunArgs) -> Result<ExitCode> {
    let wall_start = Instant::now();
    let json = std::fs::read_to_string(&args.scene)
        .with_context(|| format!("cannot read {}", args.scene.display()))?;
    // Whether the scene is thermal is only known after `prepare`; asking for temperature here is
    // harmless, since `prepare` drops it from the frames without the thermal lattice.
    let png_fields = |thermal: bool| {
        if args.no_frames {
            Vec::new()
        } else {
            args.render.fields(thermal)
        }
    };
    let opts = RunOptions {
        lattice_velocity: args.lattice_velocity,
        end_time: args.end_time,
        output_interval: args.output_interval,
        sample_interval: args.sample_interval,
        frame_dtype: match args.dtype {
            DtypeArg::F32 => Dtype::F32,
            DtypeArg::F16 => Dtype::F16,
        },
        write_frames: !args.no_frames,
        extra_fields: png_fields(true).into_iter().map(png_source).collect(),
    };
    let prepared = match cfd_job::prepare(json, opts) {
        Ok(p) => p,
        Err(PrepareError::Json(e)) => {
            return Err(e).with_context(|| format!("invalid scene {}", args.scene.display()))
        }
        Err(PrepareError::Invalid(report)) => {
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
    for i in &prepared.domain.issues {
        eprintln!("warning [{}] {}", i.code, i.message);
    }
    let png_fields = png_fields(prepared.domain.physics.thermal());

    let choice = args.solver.resolve(Some(prepared.scene.run.backend))?;
    let out = &args.out;
    let dir = cfd_job::ResultDir::create(out, args.force)?;
    let job = JobContext {
        id: out
            .file_name()
            .map_or_else(|| "run".into(), |s| s.to_string_lossy().into_owned()),
        generator: format!("cfd-cli {}", env!("CARGO_PKG_VERSION")),
        wall_start,
    };
    let console = Console {
        last_report: Mutex::new(Instant::now()),
    };
    let outcome = cfd_job::run(prepared, &choice, &dir, &job, &console)?;

    if !png_fields.is_empty() && outcome.frames > 0 {
        let opts = RenderOpts {
            png: Some(png_fields),
            ..args.render
        };
        render::render_dir(out, &opts)?;
    }
    Ok(match outcome.status {
        Status::Completed => ExitCode::SUCCESS,
        Status::Diverged => ExitCode::from(2),
    })
}
