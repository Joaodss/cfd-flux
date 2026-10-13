//! The run loop: steps the solver, samples the time series, writes frames and `meta.json`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use cfd_core::domain::Domain;
use cfd_core::solver::{FieldSet, PendingFields, SampleRequest, SolverError};
use cfd_engines::SolverChoice;
use cfd_io::frame::{Dtype, Field, FieldDesc, Frame};

use crate::meta::*;
use crate::series::Series;
use crate::store::ResultDir;
use crate::{output_field_name, Prepared};

/// Who runs the job and since when.
#[derive(Debug, Clone)]
pub struct JobContext {
    /// Written into every frame header (the CLI uses the output directory's name).
    pub id: String,
    /// `meta.json`'s `generator`, e.g. "cfd-cli 0.1.0".
    pub generator: String,
    /// Start of the wall-clock time reported in `meta.json`.
    pub wall_start: Instant,
}

/// The run has set everything up and is about to step.
pub struct Started<'a> {
    pub scene: &'a str,
    pub domain: &'a Domain,
    pub choice: &'a SolverChoice,
}

/// Where the run is, after a batch of steps.
#[derive(Debug, Clone, Copy)]
pub struct Progress {
    pub step: u64,
    pub total: u64,
    /// Simulated time (s).
    pub sim_time: f64,
    /// Lattice Mach number of the last sample.
    pub max_mach: f64,
    /// Million fluid-lattice updates per second of solver time so far.
    pub mlups: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Completed,
    /// The solution stopped being finite; the frames written before stay valid.
    Diverged,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Completed => "completed",
            Status::Diverged => "diverged",
        }
    }
}

/// How a run ended.
#[derive(Debug, Clone)]
pub struct Outcome {
    pub status: Status,
    /// Why it stopped early, with a hint (divergence).
    pub message: Option<String>,
    pub steps_done: u64,
    pub frames: u32,
    pub timings: TimingsMeta,
    pub dir: PathBuf,
}

/// Receives the events of a run. Every method has an empty default, so an observer implements
/// only what it needs (in C# terms, an interface with default methods, or a set of events).
/// `Send + Sync` so one observer can be shared by the threads of a job.
pub trait JobObserver: Send + Sync {
    fn started(&self, _info: &Started) {}
    fn progress(&self, _progress: &Progress) {}
    fn finished(&self, _outcome: &Outcome) {}
}

/// An observer that ignores everything.
pub struct NoObserver;

impl JobObserver for NoObserver {}

/// Converts copied lattice fields into a frame file.
struct FrameWriter {
    dir: ResultDir,
    job_id: String,
    req: SampleRequest,
    dtype: Dtype,
}

impl FrameWriter {
    fn write(&self, domain: &Domain, index: u32, pending: Box<dyn PendingFields>) -> Result<()> {
        let step = pending.step();
        let fs = FieldSet::from_lattice(domain, &pending.wait(), step, &self.req);
        let frame = Frame {
            job_id: self.job_id.clone(),
            frame_index: index,
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
                        dtype: self.dtype,
                    },
                    values: f.values,
                })
                .collect(),
        };
        let path = self.dir.frame_path(index);
        std::fs::write(&path, frame.encode()?)
            .with_context(|| format!("cannot write {}", path.display()))
    }
}

/// Runs a prepared scene on `choice` into `dir` (created with [`ResultDir::create`]).
///
/// A divergence is an [`Outcome`] (status `diverged`), not an error; errors are I/O and backend
/// failures.
pub fn run(
    prepared: Prepared,
    choice: &SolverChoice,
    dir: &ResultDir,
    job: &JobContext,
    observer: &dyn JobObserver,
) -> Result<Outcome> {
    let Prepared {
        scene,
        scene_json,
        domain,
        sample_every,
        frame_fields,
        options,
    } = prepared;
    let units = domain.units;
    let total = domain.run.total;
    let output_every = domain.run.output_every;
    let thermal = domain.physics.thermal();
    let dtype = options.frame_dtype;

    dir.write("scene.json", &scene_json)?;
    if options.write_frames {
        std::fs::create_dir_all(dir.frames_dir())?;
    }

    let mut series = Series::create(dir, &scene, &domain)?;
    // From here on the solver owns the domain (`make` takes it by value: a move, not a copy);
    // `solver.domain()` lends it back read-only.
    let mut solver = choice.make(domain)?;
    let initial = solver.diagnostics();
    let domain = solver.domain();
    let n = domain.numbers;
    observer.started(&Started {
        scene: &scene.name,
        domain,
        choice,
    });

    let mut meta = Meta {
        generator: job.generator.clone(),
        scene: scene.name.clone(),
        solver: SolverMeta {
            backend: choice.backend,
            device: choice.device.clone(),
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
            lattice_velocity: options.lattice_velocity,
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
        dir.write("meta.json", serde_json::to_string_pretty(meta)? + "\n")
    };
    write_meta(&meta)?;

    let req = SampleRequest {
        fields: frame_fields,
    };

    let mut last = initial;
    let mut peak_mach: f64 = initial.max_mach();
    let (mut next_sample, mut next_output) = (0u64, 0u64);
    let mut frame_index = 0u32;
    let (mut solver_time, mut output_time) = (Duration::ZERO, Duration::ZERO);
    let mut failure: Option<String> = None;
    let writer = FrameWriter {
        dir: dir.clone(),
        job_id: job.id.clone(),
        req,
        dtype,
    };
    let mut pending: Option<(u32, Box<dyn PendingFields>)> = None;

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
            if options.write_frames {
                // Only start the copy here: the frame is converted and written while the next
                // steps run (on asynchronous backends; the CPU backend copies immediately).
                pending = Some((frame_index, solver.request_fields()));
            }
            frame_index += 1;
            next_output += output_every;
        }
        output_time += t0.elapsed();
        if step >= total {
            if let Some((index, p)) = pending.take() {
                writer.write(solver.domain(), index, p)?;
            }
            break;
        }

        let target = next_sample.min(next_output).min(total);
        let chunk = (target - step).min(u32::MAX as u64) as u32;
        let t1 = Instant::now();
        let mut result = solver.step(chunk);
        solver_time += t1.elapsed();
        if let Some((index, p)) = pending.take() {
            let t = Instant::now();
            writer.write(solver.domain(), index, p)?;
            output_time += t.elapsed();
        }
        // GPU time hidden behind the frame output above is not counted as solver time.
        let t2 = Instant::now();
        result = result.and_then(|_| solver.finish());
        solver_time += t2.elapsed();
        match result {
            Ok(()) => {}
            Err(SolverError::NonFinite { step }) => {
                failure = Some(format!("the solution diverged (NaN/Inf) by step {step}"));
                break;
            }
            Err(e) => return Err(e.into()),
        }
        let done = solver.steps_done();
        observer.progress(&Progress {
            step: done,
            total,
            sim_time: solver.domain().units.steps_to_time(done),
            max_mach: last.max_mach(),
            mlups: solver.domain().fluid_cell_count() as f64 * done as f64
                / solver_time.as_secs_f64()
                / 1e6,
        });
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
        wall_seconds: job.wall_start.elapsed().as_secs_f64(),
        solver_seconds: solver_s,
        output_seconds: output_time.as_secs_f64(),
        mlups: if solver_s > 0.0 {
            fluid * done as f64 / solver_s / 1e6
        } else {
            0.0
        },
    };
    let status = match &failure {
        None => Status::Completed,
        Some(msg) => {
            let hints: Vec<&str> = meta.warnings.iter().map(|w| w.code).collect();
            let hint = if hints.is_empty() {
                "lower the lattice velocity (--lattice-velocity) or refine the grid".to_string()
            } else {
                format!("check the warnings: {}", hints.join(", "))
            };
            meta.run.message = Some(format!("{msg}; {hint}"));
            Status::Diverged
        }
    };
    meta.run.status = status.name();
    write_meta(&meta)?;
    let outcome = Outcome {
        status,
        message: meta.run.message.clone(),
        steps_done: done,
        frames: frame_index,
        timings: meta.timings,
        dir: dir.path().to_path_buf(),
    };
    observer.finished(&outcome);
    Ok(outcome)
}
