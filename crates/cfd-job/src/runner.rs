//! The run loop (ADR-011): a **command loop** that steps the solver in batches of bounded
//! duration, samples the time series, hands frames to the output thread and writes `meta.json`.
//!
//! ```text
//! loop {
//!     1. drain commands without blocking; while paused, block until Resume/Cancel
//!     2. due work: diagnostics + CSV rows every sample interval, a frame every output interval
//!     3. step one batch: up to the next due event, and at most ~50 ms of measured work
//! }
//! ```
//!
//! The batch cap bounds how long a Pause or Cancel waits. The loop runs on whatever thread calls
//! [`run`] (the CLI's main thread, a dedicated worker thread in the server) and owns the solver:
//! GPU contexts stay on that thread, and only messages cross threads.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::Result;
use cfd_core::domain::Domain;
use cfd_core::sample::{PendingSamples, Sample, SampleSpec};
use cfd_core::scene::OutputField;
use cfd_core::solver::{Diagnostics, Solver, SolverError};
use cfd_core::Scene;
use cfd_engines::SolverChoice;
use cfd_io::frame::Dtype;

use crate::meta::*;
use crate::output::{OutputConfig, OutputJob, OutputThread, TrySend};
use crate::series::{Series, SeriesColumns, SeriesRow};
use crate::store::ResultDir;
use crate::{output_field_name, Prepared, RunOptions};

/// Default upper bound on the measured duration of one batch of steps.
pub const DEFAULT_BATCH_TIME: Duration = Duration::from_millis(50);

/// Who runs the job and how.
#[derive(Debug, Clone)]
pub struct JobContext {
    /// Written into every frame header (the CLI uses the output directory's name).
    pub id: String,
    /// `meta.json`'s `generator`, e.g. "cfd-cli 0.1.0".
    pub generator: String,
    /// Start of the wall-clock time reported in `meta.json`.
    pub wall_start: Instant,
    /// Upper bound on the duration of one batch of steps (the latency of commands).
    pub batch_time: Duration,
}

impl JobContext {
    pub fn new(id: impl Into<String>, generator: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            generator: generator.into(),
            wall_start: Instant::now(),
            batch_time: DEFAULT_BATCH_TIME,
        }
    }
}

/// A rectangle of cells to change while the simulation runs (interactive mode, Phase 5b).
#[derive(Debug, Clone, PartialEq)]
pub struct CellPatch {
    /// `[x, y, width, height]` in cells.
    pub rect: [u32; 4],
    /// New cell types, row-major inside the rectangle.
    pub cell_type: Vec<u8>,
}

/// A parameter to change while the simulation runs (interactive mode, Phase 5b).
#[derive(Debug, Clone, PartialEq)]
pub struct ParamChange {
    pub name: String,
    pub value: f64,
}

/// Messages to a running job, applied between batches (ADR-011).
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Pause,
    Resume,
    /// Stop as soon as possible; the results written so far stay valid.
    Cancel,
    /// Sample a frame now for the observers (e.g. a viewer that joined while paused). Not stored.
    Snapshot,
    /// Phase 5b; answered with a log message until then.
    PatchCells(CellPatch),
    /// Phase 5b; answered with a log message until then.
    SetParam(ParamChange),
}

/// The run has set everything up and is about to step.
pub struct Started<'a> {
    pub scene: &'a str,
    pub domain: &'a Domain,
    pub choice: &'a SolverChoice,
    /// The columns of `forces.csv`, `probes.csv` and `diagnostics.csv`.
    pub series: &'a SeriesColumns,
}

/// Where the run is, after a batch of steps.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Progress {
    pub step: u64,
    pub total: u64,
    /// Simulated time (s).
    pub sim_time: f64,
    /// Lattice Mach number of the last sample.
    pub max_mach: f64,
    /// `(mass − mass₀) / mass₀` at the last sample.
    pub mass_drift: f64,
    /// Million fluid-lattice updates per second of solver time so far.
    pub mlups: f64,
}

/// A frame in physical units: the samples taken for it (the stored frame, the observer's stream
/// formats).
pub struct FrameEvent<'a> {
    /// Output index; `None` for a snapshot.
    pub index: Option<u32>,
    /// Whether it was written to `frames/` (before this event).
    pub stored: bool,
    /// One per spec requested: when frames are stored, first the stored frame's (full resolution,
    /// the frame dtype), then those of [`JobObserver::stream_specs`] that differ from it.
    pub samples: &'a [Sample],
    /// The row of `heat.csv` of this frame, `[step, time, q₁, length₁, …]`, already on disk.
    pub heat: Option<&'a [f64]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Completed,
    /// The solution stopped being finite; the frames written before stay valid.
    Diverged,
    /// Stopped by a `Cancel` command (or by the controller going away while paused).
    Cancelled,
}

impl Status {
    pub fn name(self) -> &'static str {
        match self {
            Status::Completed => "completed",
            Status::Diverged => "diverged",
            Status::Cancelled => "cancelled",
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
    /// Output events (frames sampled), stored or not.
    pub frames: u32,
    /// Frames written to `frames/`.
    pub frames_stored: u32,
    pub timings: TimingsMeta,
    pub dir: PathBuf,
}

/// Receives the events of a run. Every method has an empty default, so an observer implements
/// only what it needs (in C# terms, an interface with default methods, or a set of events).
///
/// `Send + Sync` because one observer is shared (`Arc`) by the job's threads: [`frame`] is called
/// from the output thread, everything else from the thread running the loop.
///
/// Order on the loop's thread: the [`sample`]s taken before a batch, then (the series files
/// flushed) its [`progress`] — so every row an observer was given before a `progress` is on disk.
///
/// [`frame`]: JobObserver::frame
/// [`sample`]: JobObserver::sample
/// [`progress`]: JobObserver::progress
pub trait JobObserver: Send + Sync {
    fn started(&self, _info: &Started) {}
    fn progress(&self, _progress: &Progress) {}
    /// One row of every series, as written to the CSV files (every sample interval).
    fn sample(&self, _row: &SeriesRow) {}
    fn paused(&self, _paused: bool) {}
    /// Whether frames should be sampled even when they are not stored (`--no-frames`).
    fn wants_frames(&self) -> bool {
        false
    }
    /// Extra samples to take with every frame (e.g. downsampled f16 for viewers), asked again for
    /// each frame; `frame_fields` are the run's frame fields, `width × height` the grid.
    fn stream_specs(
        &self,
        _frame_fields: &[OutputField],
        _width: u32,
        _height: u32,
    ) -> Vec<SampleSpec> {
        Vec::new()
    }
    fn frame(&self, _frame: &FrameEvent) {}
    /// Something worth telling a user (e.g. an unsupported command).
    fn log(&self, _message: &str) {}
    fn finished(&self, _outcome: &Outcome) {}
}

/// An observer that ignores everything.
pub struct NoObserver;

impl JobObserver for NoObserver {}

/// Chooses the number of steps of each batch from the measured step rate, so a batch takes at most
/// about `target` (the latency of commands).
#[derive(Debug, Clone)]
pub(crate) struct Batcher {
    target: Duration,
    /// Steps per second, smoothed; `None` before the first measurement.
    rate: Option<f64>,
}

impl Batcher {
    pub fn new(target: Duration) -> Self {
        Self { target, rate: None }
    }

    /// Steps for the next batch, at most `remaining` (the distance to the next due event).
    pub fn next(&self, remaining: u64) -> u32 {
        let want = match self.rate {
            // The first batch is one step: its duration is unknown (and includes warm-up).
            None => 1.0,
            Some(rate) => (rate * self.target.as_secs_f64()).floor().max(1.0),
        };
        (want as u64).min(remaining).min(u32::MAX as u64) as u32
    }

    /// Records that `steps` took `elapsed`.
    pub fn record(&mut self, steps: u32, elapsed: Duration) {
        let secs = elapsed.as_secs_f64().max(1e-6);
        let measured = steps as f64 / secs;
        // Halfway between the old estimate and the new measurement: quick to follow, not jumpy.
        self.rate = Some(match self.rate {
            None => measured,
            Some(r) => 0.5 * (r + measured),
        });
    }
}

/// What a frame request is for.
#[derive(Debug, Clone, Copy)]
struct FrameRequest {
    index: Option<u32>,
    store: bool,
}

/// Whether the loop goes on after the commands were handled.
enum Flow {
    Continue,
    Cancel,
}

/// The mutable state of one run, so the command handling and the loop body can share it.
struct Runner<'a> {
    solver: Box<dyn Solver>,
    series: Series,
    output: OutputThread,
    observer: Arc<dyn JobObserver>,
    commands: &'a Receiver<Command>,
    /// Commands received while waiting for the output queue, to apply at the next poll.
    deferred: VecDeque<Command>,
    cancelled: bool,
    /// A snapshot was asked for and is still to be taken.
    snapshot: bool,
    solver_time: Duration,
    output_time: Duration,
    /// The stored frames' spec (full resolution, the frame dtype).
    stored_spec: SampleSpec,
    frame_fields: Vec<OutputField>,
}

impl Runner<'_> {
    /// Handles the commands received since the last batch; while paused, blocks for the next one.
    fn poll_commands(&mut self) -> Result<Flow> {
        loop {
            if self.cancelled {
                return Ok(Flow::Cancel);
            }
            let cmd = match self.deferred.pop_front() {
                Some(c) => c,
                None => match self.commands.try_recv() {
                    Ok(c) => c,
                    // Nobody can send commands any more (e.g. the CLI): keep running.
                    Err(TryRecvError::Empty | TryRecvError::Disconnected) => {
                        return Ok(Flow::Continue)
                    }
                },
            };
            if let Flow::Cancel = self.apply(cmd)? {
                return Ok(Flow::Cancel);
            }
        }
    }

    fn apply(&mut self, cmd: Command) -> Result<Flow> {
        match cmd {
            Command::Pause => self.pause(),
            // Already running; a Resume that arrives late is harmless.
            Command::Resume => Ok(Flow::Continue),
            Command::Cancel => Ok(self.cancel()),
            Command::Snapshot => {
                self.snapshot = true;
                Ok(Flow::Continue)
            }
            Command::PatchCells(_) | Command::SetParam(_) => {
                self.observer
                    .log("changing the scene while it runs is not supported yet (Phase 5b)");
                Ok(Flow::Continue)
            }
        }
    }

    /// Stops as soon as possible: frames still waiting for the output thread are dropped.
    fn cancel(&mut self) -> Flow {
        self.cancelled = true;
        self.output.abandon();
        Flow::Cancel
    }

    /// Blocks until Resume or Cancel. Snapshots are taken at once (the solver is idle).
    fn pause(&mut self) -> Result<Flow> {
        self.observer.paused(true);
        loop {
            let cmd = match self.deferred.pop_front() {
                Some(c) => c,
                // `recv` blocks this thread (not an async task): it is the job's own thread.
                None => match self.commands.recv() {
                    Ok(c) => c,
                    // The controller is gone: nobody can resume, so stop.
                    Err(_) => return Ok(self.cancel()),
                },
            };
            match cmd {
                Command::Resume => break,
                Command::Cancel => return Ok(self.cancel()),
                Command::Pause => {}
                Command::Snapshot => {
                    let req = FrameRequest {
                        index: None,
                        store: false,
                    };
                    let pending = self.request(req);
                    self.collect(req, pending)?;
                    if self.cancelled {
                        return Ok(Flow::Cancel);
                    }
                }
                other => {
                    self.apply(other)?;
                }
            }
        }
        self.observer.paused(false);
        Ok(Flow::Continue)
    }

    /// Starts sampling a frame: the stored frame (if `req.store`), then every stream format the
    /// observer wants that differs from it. GPU backends compute the samples on the device.
    fn request(&mut self, req: FrameRequest) -> Box<dyn PendingSamples> {
        let d = self.solver.domain();
        let mut specs = Vec::new();
        if req.store {
            specs.push(self.stored_spec.clone());
        }
        for s in self
            .observer
            .stream_specs(&self.frame_fields, d.width, d.height)
        {
            if !specs.contains(&s) {
                specs.push(s);
            }
        }
        self.solver.request_samples(&specs)
    }

    /// Waits for a requested copy and queues it for the output thread. While the queue is full
    /// (the disk or the encoder is slower than the solver), commands are still read: a Cancel
    /// drops the frame and returns at once, other commands are kept for the next poll.
    fn collect(&mut self, req: FrameRequest, pending: Box<dyn PendingSamples>) -> Result<()> {
        let t = Instant::now();
        let mut job = OutputJob {
            index: req.index,
            store: req.store,
            samples: pending.wait(),
        };
        loop {
            match self.output.try_send(job)? {
                TrySend::Queued => break,
                TrySend::Full(back) => job = back,
            }
            while let Ok(cmd) = self.commands.try_recv() {
                if cmd == Command::Cancel {
                    self.cancel();
                    self.output_time += t.elapsed();
                    return Ok(());
                }
                self.deferred.push_back(cmd);
            }
            std::thread::sleep(Duration::from_millis(1));
        }
        self.output_time += t.elapsed();
        Ok(())
    }
}

/// Builds the initial `meta.json` (status `running`).
#[allow(clippy::too_many_arguments)]
fn initial_meta(
    scene: &Scene,
    domain: &Domain,
    choice: &SolverChoice,
    job: &JobContext,
    options: &RunOptions,
    sample_every: u64,
    frame_fields: &[cfd_core::scene::OutputField],
    initial: &Diagnostics,
) -> Meta {
    let (units, n) = (domain.units, domain.numbers);
    let thermal = domain.physics.thermal();
    let total = domain.run.total;
    Meta {
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
            reference: ReferenceMeta {
                length: n.reference.length,
                velocity: n.reference.velocity,
                temperature_difference: n.reference.temperature_difference,
                reynolds: n.reference.reynolds,
                rayleigh: n.reference.rayleigh,
            },
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
            output_every_steps: domain.run.output_every,
            sample_every_steps: sample_every,
            frames: 0,
            frames_stored: 0,
            frame_dtype: match options.frame_dtype {
                Dtype::F32 => "f32",
                Dtype::F16 => "f16",
            },
            frame_fields: frame_fields.iter().map(|f| output_field_name(*f)).collect(),
        },
        timings: TimingsMeta::default(),
        diagnostics: DiagnosticsMeta {
            initial: DiagMeta::new(initial, domain),
            last: DiagMeta::new(initial, domain),
            relative_mass_drift: 0.0,
            peak_mach: 0.0,
        },
    }
}

/// Runs a prepared scene on `choice` into `dir` (created with [`ResultDir::create`]), applying
/// the commands received on `commands` between batches. The solver is made here, so it lives on
/// the calling thread.
///
/// A divergence or a cancellation is an [`Outcome`], not an error; errors are I/O and backend
/// failures.
pub fn run(
    prepared: Prepared,
    choice: &SolverChoice,
    dir: &ResultDir,
    job: &JobContext,
    observer: Arc<dyn JobObserver>,
    commands: &Receiver<Command>,
) -> Result<Outcome> {
    let Prepared {
        scene,
        scene_json,
        domain,
        sample_every,
        frame_fields,
        options,
    } = prepared;
    let total = domain.run.total;
    let output_every = domain.run.output_every;
    let sample_frames = options.write_frames || observer.wants_frames();

    dir.write("scene.json", &scene_json)?;
    if options.write_frames {
        std::fs::create_dir_all(dir.frames_dir())?;
    }
    // `heat.csv` (heat through the thermal walls, per stored frame) needs the stored temperature.
    let heat = options.write_frames && frame_fields.contains(&OutputField::Temperature);
    let mut series = Series::create(dir, &scene, &domain, heat)?;
    // From here on the solver owns the domain (`make` takes it by value: a move, not a copy);
    // `solver.domain()` lends it back read-only.
    let mut solver = choice.make(domain)?;
    let initial = solver.diagnostics();
    let mut meta = initial_meta(
        &scene,
        solver.domain(),
        choice,
        job,
        &options,
        sample_every,
        &frame_fields,
        &initial,
    );
    let write_meta = |meta: &Meta| -> Result<()> {
        dir.write("meta.json", serde_json::to_string_pretty(meta)? + "\n")
    };
    write_meta(&meta)?;
    let output = OutputThread::spawn(
        solver.domain(),
        OutputConfig {
            dir: dir.clone(),
            job_id: job.id.clone(),
            max_stored_bytes: options.max_stored_bytes,
        },
        observer.clone(),
        series.take_heat(),
    )?;
    observer.started(&Started {
        scene: &scene.name,
        domain: solver.domain(),
        choice,
        series: series.columns(),
    });

    let mut r = Runner {
        solver,
        series,
        output,
        observer,
        commands,
        deferred: VecDeque::new(),
        cancelled: false,
        snapshot: false,
        solver_time: Duration::ZERO,
        output_time: Duration::ZERO,
        stored_spec: SampleSpec::full(frame_fields.clone(), crate::precision(options.frame_dtype)),
        frame_fields,
    };
    let mut batcher = Batcher::new(job.batch_time);
    let mut last = initial;
    let mut peak_mach: f64 = initial.max_mach();
    let (mut next_sample, mut next_output) = (0u64, 0u64);
    let mut frame_index = 0u32;
    let mut failure: Option<String> = None;
    let mut cancelled = false;
    let fluid = r.solver.domain().fluid_cell_count() as f64;
    let mass_drift = |d: &Diagnostics| (d.mass - initial.mass) / initial.mass;

    loop {
        // 1. Commands.
        if let Flow::Cancel = r.poll_commands()? {
            cancelled = true;
            break;
        }

        // 2. Due work: samples, frames.
        let step = r.solver.steps_done();
        let t0 = Instant::now();
        if step >= next_sample || step >= next_output {
            last = r.solver.diagnostics();
            peak_mach = peak_mach.max(last.max_mach());
            if !last.finite {
                failure = Some(format!("the solution diverged (NaN/Inf) by step {step}"));
                break;
            }
        }
        if step >= next_sample {
            let row = r.series.sample(r.solver.as_mut(), &last)?;
            r.observer.sample(&row);
            next_sample += sample_every;
        }
        let mut request = None;
        if step >= next_output {
            if sample_frames {
                request = Some(FrameRequest {
                    index: Some(frame_index),
                    store: options.write_frames,
                });
            }
            frame_index += 1;
            next_output += output_every;
        }
        if r.snapshot {
            r.snapshot = false;
            request.get_or_insert(FrameRequest {
                index: None,
                store: false,
            });
        }
        // Only start the copy here: it is collected after the next batch has been queued, so on
        // asynchronous backends the copy overlaps with the steps (the CPU copies immediately).
        let pending = request.map(|req| (req, r.request(req)));
        r.output_time += t0.elapsed();
        if step >= total {
            if let Some((req, p)) = pending {
                r.collect(req, p)?;
            }
            break;
        }

        // 3. One batch: up to the next due event, and at most `batch_time` of measured work.
        let target = next_sample.min(next_output).min(total);
        let n = batcher.next(target - step);
        let t1 = Instant::now();
        let mut result = r.solver.step(n);
        let queued = t1.elapsed();
        if let Some((req, p)) = pending {
            r.collect(req, p)?;
        }
        // GPU time hidden behind collecting the frame above is not counted as solver time.
        let t2 = Instant::now();
        result = result.and_then(|_| r.solver.finish());
        let stepped = queued + t2.elapsed();
        r.solver_time += stepped;
        batcher.record(n, stepped);
        match result {
            Ok(()) => {}
            Err(SolverError::NonFinite { step }) => {
                failure = Some(format!("the solution diverged (NaN/Inf) by step {step}"));
                break;
            }
            Err(e) => return Err(e.into()),
        }
        // Complete lines on disk before the progress that announces them (three small writes
        // per batch of at most ≈ 50 ms).
        r.series.flush()?;
        let done = r.solver.steps_done();
        r.observer.progress(&Progress {
            step: done,
            total,
            sim_time: r.solver.domain().units.steps_to_time(done),
            max_mach: last.max_mach(),
            mass_drift: mass_drift(&last),
            mlups: fluid * done as f64 / r.solver_time.as_secs_f64() / 1e6,
        });
    }
    r.series.flush()?;
    let Runner {
        solver,
        output,
        observer,
        solver_time,
        output_time,
        ..
    } = r;
    // Every queued frame is on disk before `meta.json` says the run is over.
    let stored = output.finish()?;

    let domain = solver.domain();
    let done = solver.steps_done();
    meta.run.steps_done = done;
    meta.run.sim_time = domain.units.steps_to_time(done);
    meta.run.frames = frame_index;
    meta.run.frames_stored = stored.frames;
    if stored.limit_reached {
        meta.warnings.push(WarningMeta {
            code: "storage.limit",
            message: format!(
                "frames stopped being stored after {} ({} MiB, the size limit)",
                stored.frames,
                stored.bytes >> 20
            ),
        });
    }
    meta.diagnostics.last = DiagMeta::new(&last, domain);
    meta.diagnostics.relative_mass_drift = mass_drift(&last);
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
    let status = match (&failure, cancelled) {
        (Some(msg), _) => {
            let hints: Vec<&str> = meta.warnings.iter().map(|w| w.code).collect();
            let hint = if hints.is_empty() {
                "lower the lattice velocity (--lattice-velocity) or refine the grid".to_string()
            } else {
                format!("check the warnings: {}", hints.join(", "))
            };
            meta.run.message = Some(format!("{msg}; {hint}"));
            Status::Diverged
        }
        (None, true) => {
            meta.run.message = Some(format!("cancelled at step {done}"));
            Status::Cancelled
        }
        (None, false) => Status::Completed,
    };
    meta.run.status = status.name();
    write_meta(&meta)?;
    let outcome = Outcome {
        status,
        message: meta.run.message.clone(),
        steps_done: done,
        frames: frame_index,
        frames_stored: stored.frames,
        timings: meta.timings,
        dir: dir.path().to_path_buf(),
    };
    observer.finished(&outcome);
    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_batch_is_one_step_then_the_rate_sets_the_size() {
        let mut b = Batcher::new(Duration::from_millis(50));
        assert_eq!(b.next(1000), 1);
        // 1 step in 1 ms → 1000 steps/s → 50 steps per 50 ms.
        b.record(1, Duration::from_millis(1));
        assert_eq!(b.next(1000), 50);
        // Never past the next due event.
        assert_eq!(b.next(7), 7);
        // 50 steps in 10 ms → 5000 steps/s; smoothed with 1000 → 3000 → 150 steps.
        b.record(50, Duration::from_millis(10));
        assert_eq!(b.next(1000), 150);
    }

    #[test]
    fn a_slow_engine_still_steps() {
        let mut b = Batcher::new(Duration::from_millis(50));
        b.record(1, Duration::from_secs(2));
        assert_eq!(b.next(u64::MAX), 1);
        // A zero duration counts as 1 µs; batches never exceed what `step(u32)` takes.
        let mut fast = Batcher::new(Duration::from_secs(10));
        fast.record(1, Duration::ZERO);
        assert_eq!(fast.next(u64::MAX), 10_000_000);
        fast.record(u32::MAX, Duration::from_millis(1));
        assert_eq!(fast.next(u64::MAX), u32::MAX);
    }
}
