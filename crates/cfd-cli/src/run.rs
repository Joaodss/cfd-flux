//! `cfd-cli run`: runs a scene headless and writes frames, CSV time series, PNGs and metadata.
//!
//! Output layout (`docs/03-data-model.md` §6.2):
//!
//! ```text
//! DIR/scene.json         exact copy of the input
//! DIR/meta.json          solver, units, dimensionless numbers, warnings, status, timings, diagnostics
//! DIR/frames/NNNNNN.bin  cfd-io frames every output interval
//! DIR/png/<field>/NNNNNN.png, png/ranges.json (colour ranges)
//! DIR/probes.csv, forces.csv, diagnostics.csv   time series every sample interval
//! ```

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cfd_core::domain::Domain;
use cfd_core::scene::OutputField;
use cfd_core::solver::{Diagnostics, SampleRequest, Solver, SolverError};
use cfd_core::units::DEFAULT_LATTICE_VELOCITY;
use cfd_core::validate::Severity;
use cfd_core::{DomainOptions, Scene};
use cfd_io::frame::{Dtype, Field, FieldDesc, Frame};
use clap::{Args, ValueEnum};
use serde::Serialize;

use crate::backend::{SolverArgs, SolverChoice};
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

pub fn output_field_name(f: OutputField) -> &'static str {
    match f {
        OutputField::Velocity => "velocity",
        OutputField::Pressure => "pressure",
        OutputField::Density => "density",
        OutputField::Temperature => "temperature",
        OutputField::Vorticity => "vorticity",
        OutputField::FillFraction => "fillFraction",
    }
}

fn png_source(f: PngField) -> OutputField {
    match f {
        PngField::Velocity => OutputField::Velocity,
        PngField::Vorticity => OutputField::Vorticity,
        PngField::Temperature => OutputField::Temperature,
        PngField::Pressure => OutputField::Pressure,
    }
}

/// Column-safe version of a name: letters, digits, `-` and `_`.
fn column_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

// ---- meta.json ---------------------------------------------------------------------------

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Meta {
    generator: String,
    scene: String,
    solver: SolverMeta,
    grid: GridMeta,
    units: UnitsMeta,
    lattice: LatticeMeta,
    dimensionless: DimensionlessMeta,
    warnings: Vec<WarningMeta>,
    run: RunMeta,
    timings: TimingsMeta,
    diagnostics: DiagnosticsMeta,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SolverMeta {
    backend: &'static str,
    method: &'static str,
    lattices: Vec<&'static str>,
    collision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    trt_magic: Option<f64>,
    precision: &'static str,
    threads: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GridMeta {
    width: u32,
    height: u32,
    cells: usize,
    fluid_cells: usize,
    periodic: [bool; 2],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct UnitsMeta {
    /// m
    dx: f64,
    /// s
    dt: f64,
    /// kg/m³
    rho0: f64,
    /// K
    t_ref: f64,
    /// K
    delta_t: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LatticeMeta {
    lattice_velocity: f64,
    nu: f64,
    tau: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    alpha: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tau_thermal: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DimensionlessMeta {
    characteristic_velocity: f64,
    characteristic_length: f64,
    reynolds: f64,
    mach: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    prandtl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rayleigh: Option<f64>,
}

#[derive(Serialize)]
struct WarningMeta {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunMeta {
    /// `running`, `completed` or `diverged`.
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    steps_requested: u64,
    steps_done: u64,
    end_time: f64,
    sim_time: f64,
    output_every_steps: u64,
    sample_every_steps: u64,
    frames: u32,
    frame_dtype: &'static str,
    frame_fields: Vec<&'static str>,
}

#[derive(Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct TimingsMeta {
    wall_seconds: f64,
    solver_seconds: f64,
    output_seconds: f64,
    /// Million fluid-lattice updates per second of solver time.
    mlups: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagMeta {
    step: u64,
    time: f64,
    /// Lattice units: `Σ ρ` over fluid cells.
    mass: f64,
    /// Lattice units.
    kinetic_energy: f64,
    max_velocity: f64,
    max_mach: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    thermal_energy: Option<f64>,
    finite: bool,
}

impl DiagMeta {
    fn new(d: &Diagnostics, domain: &Domain) -> Self {
        Self {
            step: d.step,
            time: domain.units.steps_to_time(d.step),
            mass: d.mass,
            kinetic_energy: d.kinetic_energy,
            max_velocity: domain.units.velocity_to_physical(d.max_velocity),
            max_mach: d.max_mach(),
            thermal_energy: d.thermal_energy,
            finite: d.finite,
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsMeta {
    initial: DiagMeta,
    #[serde(rename = "final")]
    last: DiagMeta,
    /// `(mass_final − mass_initial) / mass_initial`.
    relative_mass_drift: f64,
    /// Largest lattice Mach number seen in any sample.
    peak_mach: f64,
}

// ---- time series -------------------------------------------------------------------------

/// The three CSV time series.
struct Series {
    probes: BufWriter<File>,
    forces: BufWriter<File>,
    diagnostics: BufWriter<File>,
    /// Element id → column name, in column order.
    force_columns: Vec<(u16, String)>,
    thermal: bool,
}

impl Series {
    fn create(dir: &Path, scene: &Scene, domain: &Domain) -> Result<Self> {
        let open = |name: &str| -> Result<BufWriter<File>> {
            let p = dir.join(name);
            Ok(BufWriter::new(File::create(&p).with_context(|| {
                format!("cannot create {}", p.display())
            })?))
        };
        let thermal = domain.physics.thermal();
        let mut probes = open("probes.csv")?;
        write!(probes, "step,time_s")?;
        for p in &domain.probes {
            let n = column_name(&p.name);
            write!(probes, ",{n}.ux_m_s,{n}.uy_m_s,{n}.p_Pa")?;
            if thermal {
                write!(probes, ",{n}.T_K")?;
            }
        }
        writeln!(probes)?;

        // Every element that owns a bounce-back slot can receive a force; 0 = domain edges.
        let mut ids: Vec<u16> = domain.slot_element.clone();
        ids.sort_unstable();
        ids.dedup();
        let force_columns: Vec<(u16, String)> = ids
            .into_iter()
            .map(|id| {
                let name = match scene.elements.iter().find(|e| e.id == id) {
                    _ if id == 0 => "edges".to_string(),
                    Some(e) => e.name.clone().unwrap_or_else(|| format!("element{id}")),
                    None => format!("element{id}"),
                };
                (id, column_name(&name))
            })
            .collect();
        let mut forces = open("forces.csv")?;
        write!(forces, "step,time_s")?;
        for (_, n) in &force_columns {
            write!(forces, ",{n}.fx_N_m,{n}.fy_N_m")?;
        }
        writeln!(forces)?;

        let mut diagnostics = open("diagnostics.csv")?;
        write!(
            diagnostics,
            "step,time_s,mass_lu,kinetic_energy_lu,max_velocity_m_s,max_mach"
        )?;
        if thermal {
            write!(diagnostics, ",thermal_energy_lu")?;
        }
        writeln!(diagnostics)?;
        Ok(Self {
            probes,
            forces,
            diagnostics,
            force_columns,
            thermal,
        })
    }

    fn sample(&mut self, solver: &mut dyn Solver, diag: &Diagnostics) -> Result<()> {
        let domain = solver.domain();
        let (step, time) = (diag.step, domain.units.steps_to_time(diag.step));
        let units = domain.units;

        write!(
            self.diagnostics,
            "{step},{time},{},{},{},{}",
            diag.mass,
            diag.kinetic_energy,
            units.velocity_to_physical(diag.max_velocity),
            diag.max_mach()
        )?;
        if let Some(e) = diag.thermal_energy {
            write!(self.diagnostics, ",{e}")?;
        }
        writeln!(self.diagnostics)?;

        let forces = solver.forces();
        write!(self.forces, "{step},{time}")?;
        for (id, _) in &self.force_columns {
            let f = forces
                .iter()
                .find(|f| f.element == *id)
                .map_or([0.0; 2], |f| f.physical(&units));
            write!(self.forces, ",{},{}", f[0], f[1])?;
        }
        writeln!(self.forces)?;

        let probes = solver.probes();
        write!(self.probes, "{step},{time}")?;
        for p in &probes {
            write!(
                self.probes,
                ",{},{},{}",
                p.velocity[0], p.velocity[1], p.pressure
            )?;
            if self.thermal {
                write!(self.probes, ",{}", p.temperature.unwrap_or(f64::NAN))?;
            }
        }
        writeln!(self.probes)?;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        self.probes.flush()?;
        self.forces.flush()?;
        self.diagnostics.flush()?;
        Ok(())
    }
}

// ---- the run -----------------------------------------------------------------------------

fn prepare_out_dir(dir: &Path, force: bool) -> Result<()> {
    if dir.exists() {
        let non_empty = std::fs::read_dir(dir)?.next().is_some();
        if non_empty && !force {
            bail!(
                "{} is not empty; use --force to overwrite a previous run",
                dir.display()
            );
        }
        // Only remove what a run writes.
        for sub in ["frames", "png"] {
            let p = dir.join(sub);
            if p.exists() {
                std::fs::remove_dir_all(&p)
                    .with_context(|| format!("cannot remove {}", p.display()))?;
            }
        }
    }
    std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
    Ok(())
}

pub fn run(args: RunArgs) -> Result<ExitCode> {
    let wall = Instant::now();
    let json = std::fs::read_to_string(&args.scene)
        .with_context(|| format!("cannot read {}", args.scene.display()))?;
    let scene = Scene::from_json(&json)
        .with_context(|| format!("invalid scene {}", args.scene.display()))?;
    let opts = DomainOptions {
        lattice_velocity: args.lattice_velocity,
        characteristic_velocity: None,
    };
    let mut domain = match Domain::from_scene(&scene, &opts) {
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
    if let Some(t) = args.end_time {
        domain.run.total = units.time_to_steps(t);
    }
    if let Some(t) = args.output_interval {
        domain.run.output_every = units.time_to_steps(t);
    }
    let total = domain.run.total;
    let output_every = domain.run.output_every;
    let sample_every = match args.sample_interval {
        Some(t) => units.time_to_steps(t),
        None => (output_every / 10).max(1),
    };

    let choice: SolverChoice = args.solver.resolve(Some(scene.run.backend))?;
    let thermal = domain.physics.thermal();
    let png_fields = if args.no_frames {
        Vec::new()
    } else {
        args.render.fields(thermal)
    };
    let mut frame_fields: Vec<OutputField> = scene.run.output_fields.clone();
    for f in &png_fields {
        let src = png_source(*f);
        if !frame_fields.contains(&src) {
            frame_fields.push(src);
        }
    }
    frame_fields.retain(|f| match f {
        OutputField::Temperature => thermal,
        OutputField::FillFraction => false,
        _ => true,
    });
    let dtype = match args.dtype {
        DtypeArg::F32 => Dtype::F32,
        DtypeArg::F16 => Dtype::F16,
    };

    let out = &args.out;
    prepare_out_dir(out, args.force)?;
    std::fs::write(out.join("scene.json"), &json)?;
    if !args.no_frames {
        std::fs::create_dir_all(out.join("frames"))?;
    }

    let mut series = Series::create(out, &scene, &domain)?;
    // From here on the solver owns the domain (`make` takes it by value: a move, not a copy);
    // `solver.domain()` lends it back read-only.
    let mut solver = choice.make(domain)?;
    let initial = solver.diagnostics();
    let domain = solver.domain();
    let n = domain.numbers;
    eprintln!(
        "{}: {}×{} cells, dx = {:.4e} m, dt = {:.4e} s, {total} steps, Re = {:.4}, Ma = {:.3}, τ = {:.4} ({} backend, {}, {} threads)",
        scene.name,
        domain.width,
        domain.height,
        units.dx,
        units.dt,
        n.reynolds,
        n.mach,
        domain.physics.tau(),
        choice.backend,
        choice.collision,
        choice.threads
    );

    let mut meta = Meta {
        generator: format!("cfd-cli {}", env!("CARGO_PKG_VERSION")),
        scene: scene.name.clone(),
        solver: SolverMeta {
            backend: choice.backend,
            method: "lbm",
            lattices: if thermal {
                vec!["D2Q9", "D2Q5"]
            } else {
                vec!["D2Q9"]
            },
            collision: choice.collision,
            trt_magic: choice.trt_magic,
            precision: "f32",
            threads: choice.threads,
        },
        grid: GridMeta {
            width: domain.width,
            height: domain.height,
            cells: domain.cell_count(),
            fluid_cells: domain.fluid_cell_count(),
            periodic: domain.periodic,
        },
        units: UnitsMeta {
            dx: units.dx,
            dt: units.dt,
            rho0: units.rho0,
            t_ref: units.t_ref,
            delta_t: units.delta_t,
        },
        lattice: LatticeMeta {
            lattice_velocity: args.lattice_velocity,
            nu: domain.physics.nu,
            tau: domain.physics.tau(),
            alpha: domain.physics.alpha,
            tau_thermal: domain.physics.tau_thermal(),
        },
        dimensionless: DimensionlessMeta {
            characteristic_velocity: n.velocity,
            characteristic_length: n.length,
            reynolds: n.reynolds,
            mach: n.mach,
            prandtl: n.prandtl,
            rayleigh: n.rayleigh,
        },
        warnings: domain
            .issues
            .iter()
            .map(|i| WarningMeta {
                code: i.code,
                message: i.message.clone(),
            })
            .collect(),
        run: RunMeta {
            status: "running",
            message: None,
            steps_requested: total,
            steps_done: 0,
            end_time: units.steps_to_time(total),
            sim_time: 0.0,
            output_every_steps: output_every,
            sample_every_steps: sample_every,
            frames: 0,
            frame_dtype: match dtype {
                Dtype::F32 => "f32",
                Dtype::F16 => "f16",
            },
            frame_fields: frame_fields.iter().map(|f| output_field_name(*f)).collect(),
        },
        timings: TimingsMeta::default(),
        diagnostics: DiagnosticsMeta {
            initial: DiagMeta::new(&initial, domain),
            last: DiagMeta::new(&initial, domain),
            relative_mass_drift: 0.0,
            peak_mach: 0.0,
        },
    };
    let write_meta = |meta: &Meta| -> Result<()> {
        std::fs::write(
            out.join("meta.json"),
            serde_json::to_string_pretty(meta)? + "\n",
        )
        .context("cannot write meta.json")
    };
    write_meta(&meta)?;

    let req = SampleRequest {
        fields: frame_fields,
    };
    let job_id = out
        .file_name()
        .map_or_else(|| "run".into(), |s| s.to_string_lossy().into_owned());

    let mut last = initial;
    let mut peak_mach: f64 = initial.max_mach();
    let (mut next_sample, mut next_output) = (0u64, 0u64);
    let mut frame_index = 0u32;
    let (mut solver_time, mut output_time) = (Duration::ZERO, Duration::ZERO);
    let mut last_report = Instant::now();
    let mut failure: Option<String> = None;

    loop {
        let step = solver.steps_done();
        let t0 = Instant::now();
        if step >= next_sample || step >= next_output {
            last = solver.diagnostics();
            peak_mach = peak_mach.max(last.max_mach());
            if !last.finite {
                failure = Some(format!("the solution diverged (NaN/Inf) by step {step}"));
                break;
            }
        }
        if step >= next_sample {
            series.sample(solver.as_mut(), &last)?;
            next_sample += sample_every;
        }
        if step >= next_output {
            if !args.no_frames {
                let fs = solver.sample(&req);
                let frame = Frame {
                    job_id: job_id.clone(),
                    frame_index,
                    step: fs.step,
                    sim_time: fs.time,
                    width: fs.width,
                    height: fs.height,
                    fields: fs
                        .fields
                        .into_iter()
                        .map(|f| Field {
                            desc: FieldDesc {
                                name: output_field_name(f.field).into(),
                                components: f.components,
                                dtype,
                            },
                            values: f.values,
                        })
                        .collect(),
                };
                let path = out.join("frames").join(format!("{frame_index:06}.bin"));
                std::fs::write(&path, frame.encode()?)
                    .with_context(|| format!("cannot write {}", path.display()))?;
            }
            frame_index += 1;
            next_output += output_every;
        }
        output_time += t0.elapsed();
        if step >= total {
            break;
        }

        let target = next_sample.min(next_output).min(total);
        let chunk = (target - step).min(u32::MAX as u64) as u32;
        let t1 = Instant::now();
        let result = solver.step(chunk);
        solver_time += t1.elapsed();
        match result {
            Ok(()) => {}
            Err(SolverError::NonFinite { step }) => {
                failure = Some(format!("the solution diverged (NaN/Inf) by step {step}"));
                break;
            }
            Err(e) => return Err(e.into()),
        }
        if last_report.elapsed() > Duration::from_secs(2) {
            last_report = Instant::now();
            let done = solver.steps_done();
            let mlups = solver.domain().fluid_cell_count() as f64 * done as f64
                / solver_time.as_secs_f64()
                / 1e6;
            eprintln!(
                "  step {done}/{total} ({:.0}%), t = {:.4} s, max Ma = {:.3}, {mlups:.0} MLUPS",
                100.0 * done as f64 / total as f64,
                solver.domain().units.steps_to_time(done),
                last.max_mach()
            );
        }
    }
    series.flush()?;

    let domain = solver.domain();
    let done = solver.steps_done();
    let fluid = domain.fluid_cell_count() as f64;
    meta.run.steps_done = done;
    meta.run.sim_time = domain.units.steps_to_time(done);
    meta.run.frames = frame_index;
    meta.diagnostics.last = DiagMeta::new(&last, domain);
    meta.diagnostics.relative_mass_drift = (last.mass - initial.mass) / initial.mass;
    meta.diagnostics.peak_mach = peak_mach;
    let solver_s = solver_time.as_secs_f64();
    meta.timings = TimingsMeta {
        wall_seconds: wall.elapsed().as_secs_f64(),
        solver_seconds: solver_s,
        output_seconds: output_time.as_secs_f64(),
        mlups: if solver_s > 0.0 {
            fluid * done as f64 / solver_s / 1e6
        } else {
            0.0
        },
    };
    let code = match &failure {
        None => {
            meta.run.status = "completed";
            ExitCode::SUCCESS
        }
        Some(msg) => {
            meta.run.status = "diverged";
            let hints: Vec<&str> = meta.warnings.iter().map(|w| w.code).collect();
            let hint = if hints.is_empty() {
                "lower the lattice velocity (--lattice-velocity) or refine the grid".to_string()
            } else {
                format!("check the warnings: {}", hints.join(", "))
            };
            meta.run.message = Some(format!("{msg}; {hint}"));
            eprintln!("error: {msg}; {hint}");
            ExitCode::from(2)
        }
    };
    write_meta(&meta)?;
    eprintln!(
        "{} steps in {:.1} s ({:.0} MLUPS), {} frames → {}",
        done,
        solver_s,
        meta.timings.mlups,
        frame_index,
        out.display()
    );

    if !png_fields.is_empty() && frame_index > 0 {
        let opts = RenderOpts {
            png: Some(png_fields),
            ..args.render
        };
        render::render_dir(out, &opts)?;
    }
    Ok(code)
}
