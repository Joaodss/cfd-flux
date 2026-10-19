//! `meta.json`: solver, grid, units, dimensionless numbers, warnings, run status, timings and
//! diagnostics of a run.

use cfd_core::domain::Domain;
use cfd_core::solver::Diagnostics;
use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Meta {
    pub generator: String,
    pub scene: String,
    pub solver: SolverMeta,
    pub grid: GridMeta,
    pub units: UnitsMeta,
    pub lattice: LatticeMeta,
    pub dimensionless: DimensionlessMeta,
    pub warnings: Vec<WarningMeta>,
    pub run: RunMeta,
    pub timings: TimingsMeta,
    pub diagnostics: DiagnosticsMeta,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SolverMeta {
    pub backend: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    pub method: &'static str,
    pub lattices: Vec<&'static str>,
    pub collision: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trt_magic: Option<f64>,
    pub precision: &'static str,
    pub threads: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GridMeta {
    pub width: u32,
    pub height: u32,
    pub cells: usize,
    pub fluid_cells: usize,
    pub periodic: [bool; 2],
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnitsMeta {
    /// m
    pub dx: f64,
    /// s
    pub dt: f64,
    /// kg/m³
    pub rho0: f64,
    /// K
    pub t_ref: f64,
    /// K
    pub delta_t: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LatticeMeta {
    pub lattice_velocity: f64,
    pub nu: f64,
    pub tau: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tau_thermal: Option<f64>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DimensionlessMeta {
    pub characteristic_velocity: f64,
    pub characteristic_length: f64,
    pub reynolds: f64,
    pub mach: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prandtl: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rayleigh: Option<f64>,
    /// Scales of reported numbers (the scene's `reference`, defaults filled in).
    pub reference: ReferenceMeta,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReferenceMeta {
    pub length: f64,
    pub velocity: f64,
    pub temperature_difference: f64,
    pub reynolds: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rayleigh: Option<f64>,
}

#[derive(Serialize)]
pub(crate) struct WarningMeta {
    pub code: &'static str,
    pub message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RunMeta {
    /// `running`, `completed` or `diverged`.
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub steps_requested: u64,
    pub steps_done: u64,
    pub end_time: f64,
    pub sim_time: f64,
    pub output_every_steps: u64,
    pub sample_every_steps: u64,
    /// Output events (frames sampled), whether stored or not.
    pub frames: u32,
    /// Frame files written (fewer than `frames` with `--no-frames` or past the size limit).
    pub frames_stored: u32,
    pub frame_dtype: &'static str,
    pub frame_fields: Vec<&'static str>,
}

#[derive(Serialize, Default, Clone, Copy, Debug)]
#[serde(rename_all = "camelCase")]
pub struct TimingsMeta {
    pub wall_seconds: f64,
    pub solver_seconds: f64,
    pub output_seconds: f64,
    /// Million fluid-lattice updates per second of solver time.
    pub mlups: f64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagMeta {
    pub step: u64,
    pub time: f64,
    /// Lattice units: `Σ ρ` over fluid cells.
    pub mass: f64,
    /// Lattice units.
    pub kinetic_energy: f64,
    pub max_velocity: f64,
    pub max_mach: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_energy: Option<f64>,
    pub finite: bool,
}

impl DiagMeta {
    pub fn new(d: &Diagnostics, domain: &Domain) -> Self {
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
pub(crate) struct DiagnosticsMeta {
    pub initial: DiagMeta,
    #[serde(rename = "final")]
    pub last: DiagMeta,
    /// `(mass_final − mass_initial) / mass_initial`.
    pub relative_mass_drift: f64,
    /// Largest lattice Mach number seen in any sample.
    pub peak_mach: f64,
}
