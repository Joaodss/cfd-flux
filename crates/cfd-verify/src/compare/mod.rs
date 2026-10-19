//! Engine comparison (`cfd-cli compare`, `docs/04-numerical-methods.md` §7): the same domain is
//! run on several backends from the same initial state, and every backend is compared with the
//! first one, the reference (the CPU backend when present).
//!
//! What is compared, for each backend against the reference:
//! - the lattice fields ρ, ux, uy, θ: max |a − b|, relative L2 and the metric of 04 §7
//!   ([`parity::differences`]), which decides pass/fail; bit-identity is reported separately;
//! - the physical output fields (speed, pressure, vorticity, temperature), for information;
//! - diagnostics (mass, kinetic energy, max |u|) and the force on each element, for information,
//!   except the mass of a closed domain, which is conserved by construction;
//! - in [`Mode::Statistics`], for chaotic flows whose instantaneous fields diverge, the
//!   statistics of the force series (mean, extremes, oscillation frequency) instead.
//!
//! The runs only see [`SolverFactory`]s, so this module never names a backend. Backends run one
//! after another (not concurrently), so a GPU backend has the device to itself.

mod report;
mod validation;

use std::time::Instant;

use cfd_core::domain::flags;
use cfd_core::scene::{OutputField, Scene};
use cfd_core::solver::{Diagnostics, FieldSet, LatticeFields, SampleRequest, Solver};
use cfd_core::Domain;
use cfd_io::image::{self, Colormap, ColourScale};
use serde::Serialize;

use crate::harness::upward_crossings;
use crate::parity::{self, MASS_TOLERANCE, TOLERANCE};
use crate::{Metric, Plot, Series, Snapshot, SolverFactory};

pub use report::{verdict, write, ReportInfo};
pub use validation::validation_set;

/// Default relative tolerance of the force statistics in [`Mode::Statistics`].
pub const STATS_TOLERANCE: f64 = 1e-3;

/// What decides pass/fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// The instantaneous fields after the last step (04 §7).
    Fields,
    /// Statistics of the force series: for chaotic flows, where round-off differences grow until
    /// the instantaneous fields are unrelated while the statistics stay the same.
    Statistics,
}

/// Which difference maps to draw.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Maps {
    None,
    /// Only for fields that are not bit-identical.
    Differing,
    All,
}

/// How a comparison is run and judged.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    pub steps: u64,
    /// Steps between samples of the diagnostics and forces (time series).
    pub sample_every: u64,
    pub mode: Mode,
    /// First step of the statistics window.
    pub stats_from: u64,
    /// Limit of the 04 §7 field metric (0 = bit-identical required).
    pub tolerance: f64,
    /// Limit of the statistics, relative to the reference frequency and the largest force.
    pub stats_tolerance: f64,
    /// Length (m) and velocity (m/s) of the Strouhal number and the force coefficients.
    pub ref_length: f64,
    pub ref_velocity: f64,
    #[serde(skip)]
    pub maps: Maps,
    /// Long side of the field images, in pixels.
    #[serde(skip)]
    pub map_size: u32,
}

impl Plan {
    /// Defaults: about 500 samples, fields mode, statistics over the second half, the domain's
    /// reference scales (the scene's `reference`, else the characteristic ones), maps of
    /// differing fields at 480 px.
    pub fn new(domain: &Domain, steps: u64) -> Self {
        Self {
            steps,
            sample_every: (steps / 500).max(1),
            mode: Mode::Fields,
            stats_from: steps / 2,
            tolerance: TOLERANCE,
            stats_tolerance: STATS_TOLERANCE,
            ref_length: domain.numbers.reference.length,
            ref_velocity: domain.numbers.reference.velocity,
            maps: Maps::Differing,
            map_size: 480,
        }
    }
}

/// One comparison input: a domain, its initial state and how to run it.
pub struct Setup {
    pub id: String,
    pub title: String,
    /// What was set up (Markdown).
    pub description: String,
    pub domain: Domain,
    /// Initial fields (`set_equilibrium`); `None` = the domain's initial state.
    pub initial: Option<LatticeFields>,
    pub plan: Plan,
    /// Element names for the force tables; elements not listed are called `element<id>`.
    pub element_names: Vec<(u16, String)>,
}

/// Element names of a scene (id 0 = the implicit walls at the domain edges).
pub fn element_names(scene: &Scene) -> Vec<(u16, String)> {
    scene
        .elements
        .iter()
        .map(|e| {
            let name = e.name.clone().unwrap_or_else(|| format!("element{}", e.id));
            (e.id, name)
        })
        .chain([(0, "domain edges".to_string())])
        .collect()
}

/// A backend taking part in a comparison.
pub struct Engine<'a> {
    /// Short name, unique within the comparison (e.g. `cuda`, `cpu#2`).
    pub label: String,
    /// The backend with its device, e.g. `wgpu (NVIDIA GeForce RTX 4060 (Vulkan, DiscreteGpu))`.
    pub description: String,
    pub factory: SolverFactory<'a>,
}

/// One sample of the time series; forces in N/m, in the order of [`CaseComparison::elements`].
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Sample {
    pub step: u64,
    pub time: f64,
    /// Lattice units.
    pub kinetic_energy: f64,
    /// m/s
    pub max_velocity: f64,
    pub forces: Vec<[f64; 2]>,
}

/// Global quantities after the last step.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagSummary {
    /// `Σ ρ` (lattice units).
    pub mass: f64,
    /// `½ Σ ρ |u|²` (lattice units).
    pub kinetic_energy: f64,
    /// m/s
    pub max_velocity: f64,
    /// `Σ θ` (lattice units).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thermal_energy: Option<f64>,
}

impl DiagSummary {
    fn new(d: &Diagnostics, domain: &Domain) -> Self {
        Self {
            mass: d.mass,
            kinetic_energy: d.kinetic_energy,
            max_velocity: domain.units.velocity_to_physical(d.max_velocity),
            thermal_energy: d.thermal_energy,
        }
    }
}

/// Statistics of one force component over the statistics window (N/m).
#[derive(Debug, Clone, Copy, Serialize)]
pub struct ComponentStats {
    pub mean: f64,
    pub min: f64,
    pub max: f64,
    /// Standard deviation.
    pub rms: f64,
}

/// Statistics of the force on one element.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForceStats {
    pub element: u16,
    pub x: ComponentStats,
    pub y: ComponentStats,
    /// Frequency (Hz) of the component that fluctuates most, from its upward zero crossings
    /// (at least two whole periods).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frequency: Option<f64>,
}

/// What one backend produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendRun {
    pub label: String,
    pub description: String,
    pub steps: u64,
    /// Time spent stepping (`step` + `finish`), excluding samples and copies.
    pub seconds: f64,
    pub mlups: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<DiagSummary>,
    /// Force on each element after the last step (N/m).
    pub forces: Vec<[f64; 2]>,
    /// Over the statistics window (empty when it has fewer than two samples).
    pub statistics: Vec<ForceStats>,
    /// Mean kinetic energy over the statistics window (lattice units).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mean_kinetic_energy: Option<f64>,
    /// Set when the run could not complete (setup error, divergence).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    #[serde(skip)]
    pub series: Vec<Sample>,
    #[serde(skip)]
    initial: Option<LatticeFields>,
    #[serde(skip)]
    fields: Option<LatticeFields>,
}

/// Differences of one field.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDiff {
    pub field: String,
    pub unit: String,
    pub max_abs: f64,
    /// `‖a − b‖ / ‖b‖`; `None` when the reference is zero. For ρ it is taken on ρ − 1 and for the
    /// temperature on T − T_ref, the parts that carry the flow.
    pub rel_l2: Option<f64>,
    /// The 04 §7 metric (lattice fields only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metric: Option<f64>,
    pub identical: bool,
}

/// One backend against the reference.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Comparison {
    pub backend: String,
    /// The initial fields are bit-identical to the reference's (they must be).
    pub initial_identical: bool,
    /// Every lattice field is bit-identical to the reference.
    pub identical: bool,
    /// ρ, ux, uy, θ (lattice units).
    pub fields: Vec<FieldDiff>,
    /// Physical output fields, for information.
    pub physical: Vec<FieldDiff>,
    /// Field verdicts (fields mode), diagnostics, forces and statistics.
    pub checks: Vec<Metric>,
    pub passed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
    /// Difference maps.
    #[serde(skip)]
    pub maps: Vec<Snapshot>,
}

impl Comparison {
    /// The largest 04 §7 field metric.
    pub fn max_metric(&self) -> f64 {
        self.fields
            .iter()
            .filter_map(|f| f.metric)
            .fold(0.0, f64::max)
    }
}

/// Everything a comparison of one setup produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseComparison {
    pub id: String,
    pub title: String,
    pub description: String,
    pub width: u32,
    pub height: u32,
    pub fluid_cells: usize,
    /// s
    pub dt: f64,
    pub thermal: bool,
    pub plan: Plan,
    /// Elements with a force, in the order of the force arrays.
    pub elements: Vec<ElementInfo>,
    /// `2 / (ρ₀ U² L)`: force (N/m) → coefficient.
    pub coefficient_scale: f64,
    /// The reference first.
    pub runs: Vec<BackendRun>,
    /// One per backend after the reference.
    pub comparisons: Vec<Comparison>,
    #[serde(skip)]
    pub plots: Vec<Plot>,
    #[serde(skip)]
    pub snapshots: Vec<Snapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ElementInfo {
    pub id: u16,
    pub name: String,
    /// Some force acts on it in the reference run (outlets and unused edges carry none and are
    /// left out of the tables and checks).
    pub loaded: bool,
}

impl CaseComparison {
    pub fn passed(&self) -> bool {
        self.runs.first().is_some_and(|r| r.failure.is_none())
            && self.comparisons.iter().all(|c| c.passed)
    }
}

/// Runs `setup` on every engine (the first is the reference) and compares the results.
/// Failures of a backend (setup errors, divergence) end up in the result, not as errors.
pub fn compare(setup: &Setup, engines: &[Engine], log: &dyn Fn(&str)) -> CaseComparison {
    let d = &setup.domain;
    let mut elements: Vec<u16> = d.slot_element.clone();
    elements.sort_unstable();
    elements.dedup();
    let elements: Vec<ElementInfo> = elements
        .into_iter()
        .map(|id| ElementInfo {
            id,
            loaded: false,
            name: setup
                .element_names
                .iter()
                .find(|e| e.0 == id)
                .map_or_else(|| format!("element{id}"), |e| e.1.clone()),
        })
        .collect();
    let ids: Vec<u16> = elements.iter().map(|e| e.id).collect();
    let plan = &setup.plan;
    let mut out = CaseComparison {
        id: setup.id.clone(),
        title: setup.title.clone(),
        description: setup.description.clone(),
        width: d.width,
        height: d.height,
        fluid_cells: d.fluid_cell_count(),
        dt: d.units.dt,
        thermal: d.physics.thermal(),
        plan: plan.clone(),
        elements,
        coefficient_scale: 2.0 / (d.units.rho0 * plan.ref_velocity.powi(2) * plan.ref_length),
        runs: Vec::new(),
        comparisons: Vec::new(),
        plots: Vec::new(),
        snapshots: Vec::new(),
    };

    let mut reference: Option<BackendRun> = None;
    let mut reference_physical: Option<FieldSet> = None;
    for engine in engines {
        log(&format!(
            "  {}: {} steps on {}",
            setup.id, plan.steps, engine.description
        ));
        let mut run = run_backend(setup, engine, &ids);
        if let Some(f) = &run.failure {
            log(&format!("    {}: FAILED: {f}", engine.label));
        } else {
            log(&format!(
                "    {}: {:.2} s, {:.0} MLUPS",
                engine.label, run.seconds, run.mlups
            ));
        }
        match &reference {
            None => {
                for (k, el) in out.elements.iter_mut().enumerate() {
                    el.loaded = run.series.iter().any(|s| s.forces[k] != [0.0; 2]);
                }
                if let Some(f) = &run.fields {
                    reference_physical = Some(physical_fields(d, f));
                    out.snapshots.extend(reference_maps(setup, f));
                }
                reference = Some(run);
                continue;
            }
            Some(r) => {
                let c = compare_runs(setup, &out, r, reference_physical.as_ref(), &run);
                log(&format!(
                    "    {} vs {}: {}{}",
                    run.label,
                    r.label,
                    if c.identical {
                        "bit-identical".to_string()
                    } else {
                        format!("max 04 §7 difference {:.2e}", c.max_metric())
                    },
                    if c.passed { "" } else { " — FAIL" }
                ));
                let mut c = c;
                if let (Some(a), Some(b)) = (&run.fields, &r.fields) {
                    c.maps = difference_maps(setup, &c, a, b);
                }
                out.comparisons.push(c);
            }
        }
        run.fields = None; // only the reference's fields are kept
        run.initial = None;
        out.runs.push(run);
    }
    if let Some(mut r) = reference {
        r.fields = None;
        r.initial = None;
        out.runs.insert(0, r);
    }
    out.plots = series_plots(&out);
    out
}

// ---- running one backend ----------------------------------------------------------------

fn failed(engine: &Engine, msg: String) -> BackendRun {
    BackendRun {
        failure: Some(msg),
        ..empty(engine)
    }
}

fn empty(engine: &Engine) -> BackendRun {
    BackendRun {
        label: engine.label.clone(),
        description: engine.description.clone(),
        steps: 0,
        seconds: 0.0,
        mlups: 0.0,
        diagnostics: None,
        forces: Vec::new(),
        statistics: Vec::new(),
        mean_kinetic_energy: None,
        failure: None,
        series: Vec::new(),
        initial: None,
        fields: None,
    }
}

fn forces_by_element(solver: &mut dyn Solver, ids: &[u16]) -> Vec<[f64; 2]> {
    let units = solver.domain().units;
    let forces = solver.forces();
    ids.iter()
        .map(|id| {
            forces
                .iter()
                .find(|f| f.element == *id)
                .map_or([0.0; 2], |f| f.physical(&units))
        })
        .collect()
}

fn sample(solver: &mut dyn Solver, ids: &[u16]) -> (Sample, Diagnostics) {
    let diag = solver.diagnostics();
    let units = solver.domain().units;
    let s = Sample {
        step: solver.steps_done(),
        time: units.steps_to_time(solver.steps_done()),
        kinetic_energy: diag.kinetic_energy,
        max_velocity: units.velocity_to_physical(diag.max_velocity),
        forces: forces_by_element(solver, ids),
    };
    (s, diag)
}

fn run_backend(setup: &Setup, engine: &Engine, ids: &[u16]) -> BackendRun {
    let mut solver = match (engine.factory)(setup.domain.clone()) {
        Ok(s) => s,
        Err(e) => return failed(engine, format!("{e:#}")),
    };
    let mut run = empty(engine);
    if let Some(f) = &setup.initial {
        if let Err(e) = solver.set_equilibrium(f) {
            return failed(engine, e.to_string());
        }
    }
    run.initial = Some(solver.lattice_fields());
    let plan = &setup.plan;
    let fluid = setup.domain.fluid_cell_count() as f64;
    run.series.push(sample(solver.as_mut(), ids).0);
    let mut last_diag = None;
    while solver.steps_done() < plan.steps {
        let n = plan.sample_every.min(plan.steps - solver.steps_done());
        let t = Instant::now();
        let mut left = n;
        let mut result = Ok(());
        while left > 0 && result.is_ok() {
            let chunk = left.min(1 << 20);
            result = solver.step(chunk as u32);
            left -= chunk;
        }
        // Asynchronous backends: the clock is read once the queued steps are done.
        result = result.and_then(|_| solver.finish());
        run.seconds += t.elapsed().as_secs_f64();
        if let Err(e) = result {
            run.failure = Some(e.to_string());
            break;
        }
        let (s, diag) = sample(solver.as_mut(), ids);
        if !diag.finite {
            run.failure = Some(format!(
                "the solution diverged (NaN/Inf) by step {}",
                diag.step
            ));
            break;
        }
        run.series.push(s);
        last_diag = Some(diag);
    }
    run.steps = solver.steps_done();
    run.mlups = if run.seconds > 0.0 {
        fluid * run.steps as f64 / run.seconds / 1e6
    } else {
        0.0
    };
    if run.failure.is_some() {
        return run;
    }
    let diag = last_diag.unwrap_or_else(|| solver.diagnostics());
    run.diagnostics = Some(DiagSummary::new(&diag, solver.domain()));
    run.forces = run
        .series
        .last()
        .map(|s| s.forces.clone())
        .unwrap_or_default();
    run.statistics = statistics(&run.series, plan.stats_from, ids);
    let window: Vec<f64> = run
        .series
        .iter()
        .filter(|s| s.step >= plan.stats_from)
        .map(|s| s.kinetic_energy)
        .collect();
    run.mean_kinetic_energy =
        (window.len() >= 2).then(|| window.iter().sum::<f64>() / window.len() as f64);
    run.fields = Some(solver.lattice_fields());
    run
}

/// Mean, extremes, standard deviation and frequency of each element's force from `from` on.
pub fn statistics(series: &[Sample], from: u64, ids: &[u16]) -> Vec<ForceStats> {
    let window: Vec<&Sample> = series.iter().filter(|s| s.step >= from).collect();
    if window.len() < 2 {
        return Vec::new();
    }
    let t: Vec<f64> = window.iter().map(|s| s.time).collect();
    ids.iter()
        .enumerate()
        .map(|(k, &element)| {
            let comp = |c: usize| -> (Vec<f64>, ComponentStats) {
                let y: Vec<f64> = window.iter().map(|s| s.forces[k][c]).collect();
                let n = y.len() as f64;
                let mean = y.iter().sum::<f64>() / n;
                let var = y.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
                let stats = ComponentStats {
                    mean,
                    min: y.iter().copied().fold(f64::INFINITY, f64::min),
                    max: y.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                    rms: var.sqrt(),
                };
                (y, stats)
            };
            let ((yx, x), (yy, y)) = (comp(0), comp(1));
            let (signal, s) = if y.rms > x.rms { (&yy, y) } else { (&yx, x) };
            let crossings = upward_crossings(&t, signal);
            // Fluctuations below 1e-3 of the force are round-off noise, not an oscillation.
            let oscillates = s.rms > 1e-3 * s.min.abs().max(s.max.abs());
            let frequency = (crossings.len() >= 3 && oscillates).then(|| {
                (crossings.len() - 1) as f64 / (crossings[crossings.len() - 1] - crossings[0])
            });
            ForceStats {
                element,
                x,
                y,
                frequency,
            }
        })
        .collect()
}

// ---- metrics ----------------------------------------------------------------------------

/// Bitwise equality (so −0.0 ≠ 0.0 and equal NaNs are equal).
pub fn bit_identical(a: &[f32], b: &[f32]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x.to_bits() == y.to_bits())
}

/// `‖(a − o) − (b − o)‖ / ‖b − o‖` for an offset `o`; `None` when `‖b − o‖` is zero.
pub fn rel_l2(test: &[f32], reference: &[f32], offset: f64) -> Option<f64> {
    let (mut num, mut den) = (0.0, 0.0);
    for (a, b) in test.iter().zip(reference) {
        num += (*a as f64 - *b as f64).powi(2);
        den += (*b as f64 - offset).powi(2);
    }
    (den > 0.0).then(|| (num / den).sqrt())
}

fn field_diff(name: &str, unit: &str, a: &[f32], b: &[f32], offset: f64) -> FieldDiff {
    FieldDiff {
        field: name.into(),
        unit: unit.into(),
        max_abs: parity::max_diff(a, b),
        rel_l2: rel_l2(a, b, offset),
        metric: None,
        identical: bit_identical(a, b),
    }
}

/// ρ, ux, uy (and θ) with the 04 §7 metric of each: ρ absolute, both velocity components
/// divided by the same scale (the largest reference component), θ relative to its maximum.
pub fn lattice_diffs(test: &LatticeFields, reference: &LatticeFields) -> Vec<FieldDiff> {
    let speed = parity::max_abs(&reference.ux).max(parity::max_abs(&reference.uy));
    let mut rho = field_diff("ρ", "lattice", &test.density, &reference.density, 1.0);
    rho.metric = Some(rho.max_abs);
    let mut ux = field_diff("ux", "lattice", &test.ux, &reference.ux, 0.0);
    ux.metric = Some(ux.max_abs / speed);
    let mut uy = field_diff("uy", "lattice", &test.uy, &reference.uy, 0.0);
    uy.metric = Some(uy.max_abs / speed);
    let mut out = vec![rho, ux, uy];
    if let (Some(a), Some(b)) = (&test.theta, &reference.theta) {
        let mut theta = field_diff("θ", "lattice", a, b, 0.0);
        theta.metric = Some(theta.max_abs / parity::max_abs(b));
        out.push(theta);
    }
    out
}

const PHYSICAL: [OutputField; 4] = [
    OutputField::Velocity,
    OutputField::Pressure,
    OutputField::Vorticity,
    OutputField::Temperature,
];

fn physical_fields(domain: &Domain, f: &LatticeFields) -> FieldSet {
    FieldSet::from_lattice(
        domain,
        f,
        0,
        &SampleRequest {
            fields: PHYSICAL.to_vec(),
        },
    )
}

/// Scalar values of a sampled field (velocity → speed).
fn scalar(fs: &FieldSet, field: OutputField) -> Option<Vec<f32>> {
    let f = fs.fields.iter().find(|f| f.field == field)?;
    Some(match f.components {
        1 => f.values.clone(),
        n => f
            .values
            .chunks_exact(n as usize)
            .map(|c| c.iter().map(|v| v * v).sum::<f32>().sqrt())
            .collect(),
    })
}

fn physical_diffs(domain: &Domain, test: &FieldSet, reference: &FieldSet) -> Vec<FieldDiff> {
    let t_ref = domain.units.t_ref;
    PHYSICAL
        .iter()
        .filter_map(|&field| {
            let (a, b) = (scalar(test, field)?, scalar(reference, field)?);
            let (name, unit, offset) = match field {
                OutputField::Velocity => ("speed", "m/s", 0.0),
                OutputField::Pressure => ("pressure", "Pa", 0.0),
                OutputField::Vorticity => ("vorticity", "1/s", 0.0),
                _ => ("temperature", "K", t_ref),
            };
            Some(field_diff(name, unit, &a, &b, offset))
        })
        .collect()
}

fn rel(a: f64, b: f64) -> f64 {
    (a - b).abs() / b.abs().max(1e-30)
}

/// A difference shown for information (no limit).
fn info(name: &str, value: f64, reference: f64, error: f64) -> Metric {
    Metric {
        name: name.into(),
        value,
        reference: crate::fmt_num(reference),
        error: Some(error),
        limit: None,
    }
}

fn compare_runs(
    setup: &Setup,
    case: &CaseComparison,
    reference: &BackendRun,
    reference_physical: Option<&FieldSet>,
    test: &BackendRun,
) -> Comparison {
    let plan = &setup.plan;
    let mut c = Comparison {
        backend: test.label.clone(),
        initial_identical: false,
        identical: false,
        fields: Vec::new(),
        physical: Vec::new(),
        checks: Vec::new(),
        passed: false,
        failure: None,
        maps: Vec::new(),
    };
    let (Some(rf), Some(tf), Some(rd), Some(td)) = (
        &reference.fields,
        &test.fields,
        &reference.diagnostics,
        &test.diagnostics,
    ) else {
        c.failure = Some(match (&reference.failure, &test.failure) {
            (Some(f), _) => format!("the reference ({}) failed: {f}", reference.label),
            (_, Some(f)) => f.clone(),
            _ => "no fields".into(),
        });
        return c;
    };
    c.initial_identical = match (&test.initial, &reference.initial) {
        (Some(a), Some(b)) => lattice_diffs(a, b).iter().all(|f| f.identical),
        _ => false,
    };
    c.fields = lattice_diffs(tf, rf);
    c.identical = c.fields.iter().all(|f| f.identical);
    // The per-field metrics split the parity metric of the parity tests by component.
    let e = parity::differences(tf, rf);
    debug_assert_eq!(Some(e.rho), c.fields[0].metric);
    debug_assert_eq!(
        c.fields[1]
            .metric
            .zip(c.fields[2].metric)
            .map(|(x, y)| x.max(y)),
        Some(e.u)
    );
    if let Some(rp) = reference_physical {
        c.physical = physical_diffs(&setup.domain, &physical_fields(&setup.domain, tf), rp);
    }

    let fields_mode = plan.mode == Mode::Fields;
    if fields_mode {
        for f in &c.fields {
            let m = f.metric.unwrap_or(f64::NAN);
            let kind = if f.field == "ρ" {
                "absolute"
            } else {
                "relative to the field maximum"
            };
            c.checks.push(Metric::check(
                format!("{} (04 §7, {kind})", f.field),
                m,
                reference.label.clone(),
                m,
                plan.tolerance,
            ));
        }
    }
    // The verdict is the 04 §7 field metric. In a closed domain mass is conserved by
    // construction, so it must also agree to round-off (in both modes; a sum is never held to
    // less, since backends reduce in different orders even with identical fields). Everything
    // else is a consequence of the fields with its own sensitivity — the kinetic energy is
    // quadratic in u, forces on pressure-loaded walls measure ρ − 1 relatively — and is shown as
    // information; the parity tests keep their limits on the reduction and force kernels.
    let open = setup
        .domain
        .flags
        .iter()
        .any(|&f| matches!(flags::flow(f), flags::PRESSURE | flags::ZERO_GRADIENT));
    let mass_error = rel(td.mass, rd.mass);
    if !open {
        c.checks.push(Metric::check(
            "mass Σρ (relative; closed domain)",
            td.mass,
            crate::fmt_num(rd.mass),
            mass_error,
            MASS_TOLERANCE,
        ));
    }
    if fields_mode {
        if open {
            c.checks.push(info(
                "mass Σρ (relative; open boundaries)",
                td.mass,
                rd.mass,
                mass_error,
            ));
        }
        c.checks.push(info(
            "kinetic energy (relative)",
            td.kinetic_energy,
            rd.kinetic_energy,
            rel(td.kinetic_energy, rd.kinetic_energy),
        ));
        c.checks.push(info(
            "max |u| (relative)",
            td.max_velocity,
            rd.max_velocity,
            rel(td.max_velocity, rd.max_velocity),
        ));
        if let (Some(a), Some(b)) = (td.thermal_energy, rd.thermal_energy) {
            c.checks.push(info(
                "thermal energy Σθ (per fluid cell)",
                a,
                b,
                (a - b).abs() / case.fluid_cells as f64,
            ));
        }
        let scale = reference
            .forces
            .iter()
            .flatten()
            .map(|v| v.abs())
            .fold(1e-30, f64::max);
        for (k, el) in case.elements.iter().enumerate().filter(|(_, e)| e.loaded) {
            let (a, b) = (test.forces[k], reference.forces[k]);
            c.checks.push(Metric {
                name: format!("force on {} (relative to the largest force)", el.name),
                value: a[0].hypot(a[1]),
                reference: format!("|F| = {}", crate::fmt_num(b[0].hypot(b[1]))),
                error: Some((a[0] - b[0]).abs().max((a[1] - b[1]).abs()) / scale),
                limit: None,
            });
        }
    } else {
        statistics_checks(&mut c, case, reference, test, plan.stats_tolerance);
    }
    c.passed = c.initial_identical
        && c.failure.is_none()
        && c.checks.iter().all(|m| m.passed() != Some(false));
    c
}

/// Mean, extremes and frequency of every element's force: frequencies relative to the
/// reference frequency, forces relative to the largest reference force in the window.
fn statistics_checks(
    c: &mut Comparison,
    case: &CaseComparison,
    reference: &BackendRun,
    test: &BackendRun,
    limit: f64,
) {
    let (Some(ka), Some(kb)) = (test.mean_kinetic_energy, reference.mean_kinetic_energy) else {
        c.failure = Some("the statistics window has fewer than two samples".into());
        return;
    };
    c.checks.push(Metric::check(
        "mean kinetic energy over the window (relative)",
        ka,
        crate::fmt_num(kb),
        rel(ka, kb),
        limit,
    ));
    let scale = reference
        .statistics
        .iter()
        .flat_map(|s| [s.x.min, s.x.max, s.y.min, s.y.max])
        .map(f64::abs)
        .fold(1e-30, f64::max);
    for ((r, t), el) in reference
        .statistics
        .iter()
        .zip(&test.statistics)
        .zip(&case.elements)
        .filter(|(_, el)| el.loaded)
    {
        match (t.frequency, r.frequency) {
            (Some(a), Some(b)) => c.checks.push(Metric::check(
                format!("{}: frequency (Hz, relative)", el.name),
                a,
                crate::fmt_num(b),
                rel(a, b),
                limit,
            )),
            (None, None) => {}
            (a, b) => c.checks.push(Metric::check(
                format!("{}: frequency (Hz)", el.name),
                a.unwrap_or(f64::NAN),
                b.map_or("none".into(), crate::fmt_num),
                f64::NAN,
                limit,
            )),
        }
        for (comp, a, b) in [("x", t.x, r.x), ("y", t.y, r.y)] {
            for (what, va, vb) in [
                ("mean", a.mean, b.mean),
                ("min", a.min, b.min),
                ("max", a.max, b.max),
            ] {
                c.checks.push(Metric::check(
                    format!(
                        "{}: {what} F{comp} (N/m, relative to the largest force)",
                        el.name
                    ),
                    va,
                    crate::fmt_num(vb),
                    (va - vb).abs() / scale,
                    limit,
                ));
            }
        }
    }
}

// ---- images -----------------------------------------------------------------------------

fn fluid_mask(domain: &Domain) -> Vec<bool> {
    domain
        .flags
        .iter()
        .map(|&f| flags::flow(f) == flags::FLUID)
        .collect()
}

/// A field image with an explicit range, `size` pixels on the long side (at least 1 px/cell).
fn map(
    setup: &Setup,
    file: String,
    caption: String,
    values: &[f32],
    scale: ColourScale,
) -> Option<Snapshot> {
    let d = &setup.domain;
    let long = d.width.max(d.height).max(1);
    let px = setup.plan.map_size.div_ceil(long).clamp(1, 16);
    let img = image::render(values, d.width, d.height, &scale, Some(&fluid_mask(d)), px).ok()?;
    Some(Snapshot {
        file,
        caption,
        image: img,
    })
}

/// Range of the fluid values: `[min, max]`, or `[−m, m]` with `m = max |v|` when symmetric.
fn range(setup: &Setup, values: &[f32], symmetric: bool) -> (f32, f32) {
    let mask = fluid_mask(&setup.domain);
    let fluid = values
        .iter()
        .zip(&mask)
        .filter(|(_, m)| **m)
        .map(|(v, _)| *v);
    let (lo, hi) = if symmetric {
        let m = fluid.fold(0.0f32, |m, v| m.max(v.abs()));
        (-m, m)
    } else {
        fluid.fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        })
    };
    if hi > lo && hi.is_finite() && lo.is_finite() {
        (lo, hi)
    } else {
        (lo.min(0.0) - 0.5, lo.max(0.0) + 0.5)
    }
}

/// The reference speed (and temperature) with the colour maps of `cfd-cli render`.
fn reference_maps(setup: &Setup, f: &LatticeFields) -> Vec<Snapshot> {
    if setup.plan.maps == Maps::None {
        return Vec::new();
    }
    let fs = physical_fields(&setup.domain, f);
    let mut out = Vec::new();
    let mut add = |field, name: &str, unit: &str, colormap| {
        if let Some(v) = scalar(&fs, field) {
            let (min, max) = range(setup, &v, false);
            out.extend(map(
                setup,
                format!("reference-{name}.png"),
                format!("Reference {name} ({unit}), range [{min:.4e}, {max:.4e}]"),
                &v,
                ColourScale { colormap, min, max },
            ));
        }
    };
    add(OutputField::Velocity, "speed", "m/s", Colormap::Viridis);
    if setup.domain.physics.thermal() {
        add(
            OutputField::Temperature,
            "temperature",
            "K",
            Colormap::Inferno,
        );
    }
    out
}

/// Signed differences of ρ and θ (cool-warm, symmetric) and the magnitude of the velocity
/// difference |Δu| (inferno), for fields that differ (or all, with [`Maps::All`]).
fn difference_maps(
    setup: &Setup,
    c: &Comparison,
    test: &LatticeFields,
    reference: &LatticeFields,
) -> Vec<Snapshot> {
    let policy = setup.plan.maps;
    let draw = |identical: bool| match policy {
        Maps::None => false,
        Maps::Differing => !identical,
        Maps::All => true,
    };
    let identical = |name: &str| c.fields.iter().any(|f| f.field == name && f.identical);
    let diff = |a: &[f32], b: &[f32]| -> Vec<f32> { a.iter().zip(b).map(|(x, y)| x - y).collect() };
    let mut out = Vec::new();
    let mut signed = |name: &str, file: &str, values: Vec<f32>| {
        let (min, max) = range(setup, &values, true);
        out.extend(map(
            setup,
            format!("{}-{file}-diff.png", c.backend),
            format!(
                "{name}({}) − {name}(reference), range ±{max:.2e} (lattice units)",
                c.backend
            ),
            &values,
            ColourScale {
                colormap: Colormap::Coolwarm,
                min,
                max,
            },
        ));
    };
    if draw(identical("ρ")) {
        signed("ρ", "rho", diff(&test.density, &reference.density));
    }
    if let (Some(a), Some(b)) = (&test.theta, &reference.theta) {
        if draw(identical("θ")) {
            signed("θ", "theta", diff(a, b));
        }
    }
    if draw(identical("ux") && identical("uy")) {
        let du: Vec<f32> = (0..test.ux.len())
            .map(|i| (test.ux[i] - reference.ux[i]).hypot(test.uy[i] - reference.uy[i]))
            .collect();
        let (_, max) = range(setup, &du, false);
        out.extend(map(
            setup,
            format!("{}-u-diff.png", c.backend),
            format!(
                "|u({}) − u(reference)|, range [0, {max:.2e}] (lattice units)",
                c.backend
            ),
            &du,
            ColourScale {
                colormap: Colormap::Inferno,
                min: 0.0,
                max,
            },
        ));
    }
    out
}

const MAX_PLOT_POINTS: usize = 1000;

/// Kinetic energy and the force on the most loaded element against time, one line per backend.
fn series_plots(case: &CaseComparison) -> Vec<Plot> {
    let runs: Vec<&BackendRun> = case.runs.iter().filter(|r| r.series.len() > 1).collect();
    if runs.is_empty() {
        return Vec::new();
    }
    let line = |k: usize, label: String, points: Vec<(f64, f64)>| {
        if k == 0 {
            Series::line(label, points)
        } else {
            Series::dashed(label, points)
        }
    };
    let mut plots = vec![Plot {
        file: "kinetic-energy.svg".into(),
        title: format!("{}: kinetic energy", case.title),
        x_label: "t (s)".into(),
        y_label: "½ Σ ρ|u|² (lattice units)".into(),
        log_x: false,
        log_y: false,
        series: runs
            .iter()
            .enumerate()
            .map(|(k, r)| {
                line(
                    k,
                    r.label.clone(),
                    // At most ~1000 points per line (the SVG stays small).
                    r.series
                        .iter()
                        .step_by(r.series.len().div_ceil(MAX_PLOT_POINTS))
                        .map(|s| (s.time, s.kinetic_energy))
                        .collect(),
                )
            })
            .collect(),
    }];
    // The element with the largest force in the reference (the domain edges only if alone).
    let peak = |k: usize| {
        runs[0]
            .series
            .iter()
            .map(|s| s.forces[k][0].hypot(s.forces[k][1]))
            .fold(0.0, f64::max)
    };
    let main = (0..case.elements.len())
        .filter(|&k| case.elements[k].id != 0 || case.elements.len() == 1)
        .max_by(|&a, &b| peak(a).total_cmp(&peak(b)));
    if let Some(k) = main.filter(|&k| peak(k) > 0.0) {
        // The last samples at full resolution, so oscillations stay visible in long runs.
        let tail = |r: &BackendRun| r.series.len().saturating_sub(MAX_PLOT_POINTS);
        let mut series = Vec::new();
        for (j, r) in runs.iter().enumerate() {
            for (c, comp) in ["Fx", "Fy"].iter().enumerate() {
                series.push(line(
                    j,
                    format!("{comp} {}", r.label),
                    r.series[tail(r)..]
                        .iter()
                        .map(|s| (s.time, s.forces[k][c]))
                        .collect(),
                ));
            }
        }
        let partial = if tail(runs[0]) > 0 {
            ", last samples"
        } else {
            ""
        };
        plots.push(Plot {
            file: "forces.svg".into(),
            title: format!(
                "{}: force on {}{partial}",
                case.title, case.elements[k].name
            ),
            x_label: "t (s)".into(),
            y_label: "F (N/m)".into(),
            log_x: false,
            log_y: false,
            series,
        });
    }
    plots
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields(v: f32) -> LatticeFields {
        LatticeFields {
            density: vec![1.0, 1.0 + v, 1.0],
            ux: vec![0.1, 0.05 + v, 0.0],
            uy: vec![0.0, 0.01, -0.02],
            theta: None,
        }
    }

    #[test]
    fn lattice_metrics_follow_04_section_7() {
        let (a, b) = (fields(1e-6), fields(0.0));
        let d = lattice_diffs(&a, &b);
        assert_eq!(d.len(), 3);
        // ρ absolute; u relative to the largest reference component (0.1).
        assert!((d[0].metric.unwrap() / 1e-6 - 1.0).abs() < 0.1); // f32 spacing at 1 is 1.2e-7
        assert!((d[1].metric.unwrap() / 1e-5 - 1.0).abs() < 1e-2);
        assert_eq!(d[2].metric, Some(0.0));
        assert!(d[2].identical && !d[1].identical);
        // Same numbers as the parity tests' metric.
        let e = parity::differences(&a, &b);
        assert_eq!(e.rho, d[0].metric.unwrap());
        assert_eq!(e.u, d[1].metric.unwrap().max(d[2].metric.unwrap()));
        // Relative L2 of ρ is taken on ρ − 1; a zero reference has none.
        assert!(d[0].rel_l2.is_none());
        assert!(d[1].rel_l2.unwrap() > 0.0);
    }

    #[test]
    fn bit_identity_is_bitwise() {
        assert!(bit_identical(&[1.0, f32::NAN], &[1.0, f32::NAN]));
        assert!(!bit_identical(&[0.0], &[-0.0]));
        assert!(!bit_identical(&[1.0], &[1.0, 2.0]));
        assert_eq!(rel_l2(&[1.0, 2.0], &[1.0, 2.0], 0.0), Some(0.0));
        assert_eq!(rel_l2(&[3.0], &[1.0], 1.0), None);
    }

    #[test]
    fn statistics_of_an_oscillating_force() {
        let f = 4.0;
        let series: Vec<Sample> = (0..4000)
            .map(|i| {
                let t = i as f64 * 1e-3;
                let s = (2.0 * std::f64::consts::PI * f * t).sin();
                Sample {
                    step: i,
                    time: t,
                    kinetic_energy: 0.0,
                    max_velocity: 0.0,
                    forces: vec![[3.0 + 0.1 * s, 0.5 * s]],
                }
            })
            .collect();
        let s = &statistics(&series, 1000, &[4])[0];
        assert_eq!(s.element, 4);
        assert!((s.frequency.unwrap() / f - 1.0).abs() < 1e-3);
        assert!((s.x.mean - 3.0).abs() < 1e-3 && (s.y.max - 0.5).abs() < 1e-3);
        assert!((s.y.rms - 0.5 / 2f64.sqrt()).abs() < 1e-3);
        // A steady force has no frequency.
        let steady: Vec<Sample> = series
            .iter()
            .map(|s| Sample {
                forces: vec![[1.0, 0.0]],
                ..s.clone()
            })
            .collect();
        assert_eq!(statistics(&steady, 0, &[4])[0].frequency, None);
        assert!(statistics(&series[..1], 0, &[4]).is_empty());
    }

    use cfd_lbm::{Collision, CpuLbm, LbmConfig};

    /// The cylinder at D = 10 cells (peak inlet velocity in m/s: 0.3 → Re 20, 1.5 → Re 100).
    fn small_cylinder(u_max: f64, steps: u64) -> Setup {
        let s = crate::scenes::schafer_turek(10, u_max);
        let opts = cfd_core::DomainOptions {
            lattice_velocity: 0.08,
            characteristic_velocity: None,
        };
        let d = Domain::from_scene(&s, &opts).unwrap();
        let plan = Plan::new(&d, steps);
        Setup {
            id: "cyl".into(),
            title: "Small cylinder".into(),
            description: String::new(),
            domain: d,
            initial: None,
            plan,
            element_names: element_names(&s),
        }
    }

    fn cpu(collision: Collision) -> impl Fn(Domain) -> anyhow::Result<Box<dyn Solver>> + Sync {
        move |d| {
            let config = LbmConfig {
                collision,
                ..Default::default()
            };
            Ok(Box::new(CpuLbm::new(d, config)) as Box<dyn Solver>)
        }
    }

    fn engine<'a>(label: &str, factory: SolverFactory<'a>) -> Engine<'a> {
        Engine {
            label: label.into(),
            description: format!("{label} (test)"),
            factory,
        }
    }

    #[test]
    fn cpu_against_cpu_is_bit_identical() {
        let setup = small_cylinder(0.3, 300);
        let trt = cpu(Collision::default());
        let engines = [engine("cpu", &trt), engine("cpu-2", &trt)];
        let c = compare(&setup, &engines, &|_| {});
        assert!(c.passed(), "{:#?}", c.comparisons);
        let x = &c.comparisons[0];
        assert!(x.identical && x.initial_identical && x.max_metric() == 0.0);
        assert!(x.maps.is_empty(), "no difference maps for identical fields");
        assert_eq!(c.snapshots.len(), 1, "reference speed only");
        assert_eq!(c.runs.len(), 2);
        assert_eq!(c.runs[1].steps, 300);
        assert!(c.runs[1].series.len() > 100);
        // The cylinder (id 4) and the channel walls carry forces.
        assert!(c.elements.iter().any(|e| e.id == 4 && e.name == "Cylinder"));
        assert!(x.checks.iter().all(|m| m.passed() != Some(false)));
        // The verdict is the three field metrics only (the channel is open: mass is information).
        assert_eq!(x.checks.iter().filter(|m| m.passed().is_some()).count(), 3);
        assert!(x
            .checks
            .iter()
            .any(|m| m.name.starts_with("force on Cylinder")));
        assert!(x.physical.iter().all(|f| f.identical));
        assert_eq!(c.plots.len(), 2);

        // A tolerance of 0 asks for bit-identical fields, which they are.
        let mut strict = small_cylinder(0.3, 50);
        strict.plan.tolerance = 0.0;
        assert!(compare(&strict, &engines, &|_| {}).passed());
    }

    #[test]
    fn a_backend_that_differs_fails_and_gets_maps() {
        let setup = small_cylinder(0.3, 300);
        let (trt, bgk) = (cpu(Collision::default()), cpu(Collision::Bgk));
        let engines = [engine("cpu", &trt), engine("bgk", &bgk)];
        let c = compare(&setup, &engines, &|_| {});
        assert!(!c.passed());
        let x = &c.comparisons[0];
        assert!(!x.passed && !x.identical && x.initial_identical);
        assert!(x.max_metric() > TOLERANCE);
        assert_eq!(x.maps.len(), 2, "ρ and |Δu|");

        // Written as a report, with the failure visible.
        let dir = std::env::temp_dir().join(format!("cfd-compare-test-{}", std::process::id()));
        let info = ReportInfo {
            command: "test".into(),
            generator: "test".into(),
            commit: None,
            cpu: "cpu".into(),
            threads: 1,
            collision: "trt".into(),
            seconds: 0.0,
            docs: None,
        };
        write(&dir, &info, std::slice::from_ref(&c)).unwrap();
        let md = std::fs::read_to_string(dir.join("README.md")).unwrap();
        assert!(md.contains("❌"), "{md}");
        assert!(dir.join("bgk-rho-diff.png").exists() && dir.join("forces.svg").exists());
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("compare.json")).unwrap())
                .unwrap();
        assert_eq!(json["cases"][0]["comparisons"][0]["passed"], false);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_diverging_reference_fails_the_comparison() {
        // Re 100 at D = 10 cells: grid Reynolds number 15, τ ≈ 0.516 — diverges.
        let setup = small_cylinder(1.5, 2000);
        let trt = cpu(Collision::default());
        let engines = [engine("cpu", &trt), engine("cpu-2", &trt)];
        let c = compare(&setup, &engines, &|_| {});
        assert!(!c.passed());
        assert!(c.runs[0].failure.as_ref().unwrap().contains("diverged"));
        let failure = c.comparisons[0].failure.as_ref().unwrap();
        assert!(failure.contains("the reference (cpu) failed"), "{failure}");
    }

    #[test]
    fn statistics_mode_judges_the_force_series() {
        let mut setup = small_cylinder(0.3, 2000);
        setup.plan.mode = Mode::Statistics;
        setup.plan.sample_every = 5;
        let trt = cpu(Collision::default());
        let engines = [engine("cpu", &trt), engine("cpu-2", &trt)];
        let c = compare(&setup, &engines, &|_| {});
        assert!(
            c.passed(),
            "{:#?} {:?}",
            c.comparisons[0],
            c.runs[0].failure
        );
        let checks = &c.comparisons[0].checks;
        // No field verdicts, but mean/min/max per force component of each loaded element and the
        // mean kinetic energy; no instantaneous mass check, the channel being open.
        assert!(!checks.iter().any(|m| m.name.contains("04 §7")));
        assert!(checks
            .iter()
            .any(|m| m.name == "Cylinder: mean Fx (N/m, relative to the largest force)"));
        assert!(checks
            .iter()
            .any(|m| m.name.starts_with("mean kinetic energy")));
        assert!(!checks.iter().any(|m| m.name.starts_with("mass")));
        assert!(!checks.iter().any(|m| m.name.starts_with("Outlet")));
    }

    #[test]
    fn a_closed_domain_conserves_mass_to_round_off() {
        let s = crate::scenes::lid_driven_cavity(24, 100.0);
        let d = Domain::from_scene(&s, &cfd_core::DomainOptions::default()).unwrap();
        let setup = Setup {
            id: "cavity".into(),
            title: "Cavity".into(),
            description: String::new(),
            plan: Plan::new(&d, 200),
            domain: d,
            initial: None,
            element_names: element_names(&s),
        };
        let trt = cpu(Collision::default());
        let engines = [engine("cpu", &trt), engine("cpu-2", &trt)];
        let c = compare(&setup, &engines, &|_| {});
        let mass = c.comparisons[0]
            .checks
            .iter()
            .find(|m| m.name.starts_with("mass"))
            .unwrap();
        assert_eq!(mass.limit, Some(MASS_TOLERANCE));
        assert!(mass.name.contains("closed domain"));
    }
}
