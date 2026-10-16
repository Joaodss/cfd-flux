//! The job runner shared by `cfd-cli run` and `cfd-server`: one run loop and one result layout
//! (`docs/03-data-model.md` §6.2), so both produce the same result directories.
//!
//! ```text
//! DIR/scene.json         exact copy of the input
//! DIR/meta.json          solver, units, dimensionless numbers, warnings, status, timings, diagnostics
//! DIR/frames/NNNNNN.bin  cfd-io frames every output interval
//! DIR/probes.csv, forces.csv, diagnostics.csv   time series every sample interval
//! ```
//!
//! (`cfd-cli run` adds `png/`, rendered from the frames afterwards.)
//!
//! - [`prepare`]: `Scene → Domain` with the run's overrides (end time, intervals, lattice velocity).
//! - [`run`]: the command loop (pause, resume, cancel between batches of at most ~50 ms), with
//!   frames converted and written on an output thread, reporting to a [`JobObserver`].

mod meta;
mod output;
mod runner;
mod series;
pub mod store;

use cfd_core::domain::Domain;
use cfd_core::sample::{Precision, Sample};
use cfd_core::scene::OutputField;
use cfd_core::units::DEFAULT_LATTICE_VELOCITY;
use cfd_core::validate::Report;
use cfd_core::{DomainOptions, Scene};
use cfd_io::frame::{encode_raw, Codec, Dtype, FieldDesc, FrameError, FrameInfo, RawField};

pub use meta::TimingsMeta as Timings;
pub use runner::{
    run, CellPatch, Command, FrameEvent, JobContext, JobObserver, NoObserver, Outcome, ParamChange,
    Progress, Started, Status, DEFAULT_BATCH_TIME,
};
pub use store::{LocalStore, ResultDir};

/// The name of an output field in frames and `meta.json`.
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

/// The sample precision of a frame dtype.
pub fn precision(dtype: Dtype) -> Precision {
    match dtype {
        Dtype::F32 => Precision::F32,
        Dtype::F16 => Precision::F16,
    }
}

/// Encodes a sample as an LFFR frame (its values go into the frame as they are, f16 bits
/// included: no conversion on the CPU).
pub fn encode_sample(job_id: &str, index: u32, s: &Sample) -> Result<Vec<u8>, FrameError> {
    let dtype = match s.spec.precision {
        Precision::F32 => Dtype::F32,
        Precision::F16 => Dtype::F16,
    };
    let descs: Vec<FieldDesc> = s
        .fields
        .iter()
        .map(|f| FieldDesc {
            name: output_field_name(f.field).into(),
            components: f.components,
            dtype,
        })
        .collect();
    let raw: Vec<RawField> = descs
        .iter()
        .zip(&s.fields)
        .map(|(desc, f)| RawField {
            desc,
            bytes: f.data.bytes(),
        })
        .collect();
    let info = FrameInfo {
        job_id: job_id.to_string(),
        frame_index: index,
        step: s.step,
        sim_time: s.time,
        width: s.width,
        height: s.height,
    };
    encode_raw(&info, &raw, Codec::default())
}

/// Run settings that are not part of the scene (the same overrides as `cfd-cli run`).
#[derive(Debug, Clone, PartialEq)]
pub struct RunOptions {
    /// Lattice velocity the characteristic velocity maps to (sets the time step).
    pub lattice_velocity: f64,
    /// Simulated time in seconds (overrides the scene's `run.endTime`).
    pub end_time: Option<f64>,
    /// Time between frames in seconds (overrides `run.outputInterval`).
    pub output_interval: Option<f64>,
    /// Time between probe/force/diagnostic samples in seconds (default: output interval / 10).
    pub sample_interval: Option<f64>,
    /// Precision of the stored frames.
    pub frame_dtype: Dtype,
    /// Whether frames are written at all.
    pub write_frames: bool,
    /// Fields stored in the frames besides the scene's `run.outputFields`.
    pub extra_fields: Vec<OutputField>,
    /// Stop storing frames (not the run) past this many bytes.
    pub max_stored_bytes: Option<u64>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            lattice_velocity: DEFAULT_LATTICE_VELOCITY,
            end_time: None,
            output_interval: None,
            sample_interval: None,
            frame_dtype: Dtype::F32,
            write_frames: true,
            extra_fields: Vec::new(),
            max_stored_bytes: None,
        }
    }
}

/// A scene ready to run: converted, with the overrides applied.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub scene: Scene,
    /// The scene exactly as received (written to `scene.json`).
    pub scene_json: String,
    /// `domain.run` holds the total steps and the output interval in steps.
    pub domain: Domain,
    pub sample_every: u64,
    /// The fields of every frame.
    pub frame_fields: Vec<OutputField>,
    pub options: RunOptions,
}

impl Prepared {
    pub fn total_steps(&self) -> u64 {
        self.domain.run.total
    }
}

#[derive(Debug, thiserror::Error)]
pub enum PrepareError {
    #[error("invalid scene: {0}")]
    Json(#[from] serde_json::Error),
    /// The scene cannot run; the report lists every issue (errors and warnings).
    #[error("the scene cannot be run")]
    Invalid(Report),
}

/// Parses and converts a scene, applying the overrides of `opts`.
pub fn prepare(scene_json: String, opts: RunOptions) -> Result<Prepared, PrepareError> {
    let scene = Scene::from_json(&scene_json)?;
    let dopts = DomainOptions {
        lattice_velocity: opts.lattice_velocity,
        characteristic_velocity: None,
    };
    let mut domain = Domain::from_scene(&scene, &dopts).map_err(PrepareError::Invalid)?;
    let units = domain.units;
    if let Some(t) = opts.end_time {
        domain.run.total = units.time_to_steps(t);
    }
    if let Some(t) = opts.output_interval {
        domain.run.output_every = units.time_to_steps(t);
    }
    let sample_every = match opts.sample_interval {
        Some(t) => units.time_to_steps(t),
        None => (domain.run.output_every / 10).max(1),
    };
    let thermal = domain.physics.thermal();
    let mut frame_fields: Vec<OutputField> = scene.run.output_fields.clone();
    for f in &opts.extra_fields {
        if !frame_fields.contains(f) {
            frame_fields.push(*f);
        }
    }
    frame_fields.retain(|f| match f {
        OutputField::Temperature => thermal,
        OutputField::FillFraction => false,
        _ => true,
    });
    Ok(Prepared {
        scene,
        scene_json,
        domain,
        sample_every,
        frame_fields,
        options: opts,
    })
}
