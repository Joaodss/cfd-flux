//! The interface every solver backend implements, and the backend-independent types it returns.
//!
//! Backends only expose raw lattice fields ([`LatticeFields`]); the conversion to physical
//! output fields ([`FieldSet::from_lattice`]) is shared, so every backend reports identical units.

use crate::domain::{flags, Domain};
use crate::scene::OutputField;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendKind {
    Cpu,
    Wgpu,
    Cuda,
}

#[derive(Debug, thiserror::Error)]
pub enum SolverError {
    #[error("the solution diverged (NaN/Inf) by step {step}")]
    NonFinite { step: u64 },
    #[error("field length {found} does not match the domain ({expected} cells)")]
    FieldLength { expected: usize, found: usize },
    #[error("backend error: {0}")]
    Backend(String),
}

/// Macroscopic fields in lattice units, one value per cell (`cell = y * width + x`).
/// Non-fluid cells hold `ρ = 1`, `u = 0`.
#[derive(Debug, Clone, PartialEq)]
pub struct LatticeFields {
    pub density: Vec<f32>,
    pub ux: Vec<f32>,
    pub uy: Vec<f32>,
    /// Dimensionless temperature θ; `None` without the thermal lattice.
    pub theta: Option<Vec<f32>>,
}

impl LatticeFields {
    /// Uniform fields from the domain's initial state.
    pub fn initial(domain: &Domain) -> Self {
        let n = domain.cell_count();
        let s = &domain.initial;
        Self {
            density: vec![s.density as f32; n],
            ux: vec![s.velocity[0] as f32; n],
            uy: vec![s.velocity[1] as f32; n],
            theta: domain.physics.thermal().then(|| vec![s.theta as f32; n]),
        }
    }

    pub fn check_len(&self, cells: usize) -> Result<(), SolverError> {
        let lens = [self.density.len(), self.ux.len(), self.uy.len()];
        let theta = self.theta.as_ref().map(Vec::len);
        match lens.into_iter().chain(theta).find(|&l| l != cells) {
            Some(found) => Err(SolverError::FieldLength {
                expected: cells,
                found,
            }),
            None => Ok(()),
        }
    }
}

/// Global quantities, in lattice units, over the fluid cells.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Diagnostics {
    pub step: u64,
    pub fluid_cells: usize,
    /// `Σ ρ`.
    pub mass: f64,
    /// `½ Σ ρ |u|²`.
    pub kinetic_energy: f64,
    /// `max |u|`.
    pub max_velocity: f64,
    /// `Σ θ` (thermal lattice only).
    pub thermal_energy: Option<f64>,
    /// `false` if any NaN/Inf was found.
    pub finite: bool,
}

impl Diagnostics {
    pub fn max_mach(&self) -> f64 {
        self.max_velocity * 3f64.sqrt()
    }
}

/// Force exerted by the fluid on one element (momentum exchange).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ElementForce {
    /// Element id; 0 = the implicit walls at the domain edges.
    pub element: u16,
    /// Lattice units (per unit depth).
    pub lattice: [f64; 2],
}

/// Which output fields to sample.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleRequest {
    pub fields: Vec<OutputField>,
}

/// One sampled field in physical units: values row-major, components interleaved.
#[derive(Debug, Clone, PartialEq)]
pub struct SampledField {
    pub field: OutputField,
    pub components: u8,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FieldSet {
    pub step: u64,
    /// Physical time (s).
    pub time: f64,
    pub width: u32,
    pub height: u32,
    pub fields: Vec<SampledField>,
}

impl FieldSet {
    /// Converts lattice fields to physical output fields. Non-fluid cells are 0, except the
    /// temperature, which is the reference temperature there. Pressure is the gauge pressure
    /// including the hydrostatic part; vorticity uses central differences (one-sided at boundaries).
    /// Fields that cannot be produced (e.g. temperature without the thermal lattice) are skipped.
    pub fn from_lattice(
        domain: &Domain,
        lf: &LatticeFields,
        step: u64,
        req: &SampleRequest,
    ) -> FieldSet {
        let (w, h) = (domain.width as usize, domain.height as usize);
        let u = &domain.units;
        let fluid = |i: usize| flags::flow(domain.flags[i]) == flags::FLUID;
        let vel = |v: f32| u.velocity_to_physical(v as f64) as f32;
        let mut fields = Vec::new();
        for &field in &req.fields {
            let (components, values): (u8, Vec<f32>) = match field {
                OutputField::Velocity => (
                    2,
                    (0..w * h)
                        .flat_map(|i| {
                            if fluid(i) {
                                [vel(lf.ux[i]), vel(lf.uy[i])]
                            } else {
                                [0.0; 2]
                            }
                        })
                        .collect(),
                ),
                OutputField::Pressure => (
                    1,
                    (0..w * h)
                        .map(|i| {
                            if !fluid(i) {
                                return 0.0;
                            }
                            let p = u.density_to_gauge_pressure(lf.density[i] as f64)
                                + domain.hydrostatic.pressure(u.rho0, u.dx, i % w, i / w);
                            p as f32
                        })
                        .collect(),
                ),
                OutputField::Density => (
                    1,
                    (0..w * h)
                        .map(|i| {
                            if fluid(i) {
                                (lf.density[i] as f64 * u.rho0) as f32
                            } else {
                                0.0
                            }
                        })
                        .collect(),
                ),
                OutputField::Temperature => {
                    let Some(theta) = &lf.theta else { continue };
                    (
                        1,
                        (0..w * h)
                            .map(|i| {
                                let t = if fluid(i) { theta[i] as f64 } else { 0.0 };
                                u.temperature_to_physical(t) as f32
                            })
                            .collect(),
                    )
                }
                OutputField::Vorticity => (1, vorticity(domain, lf)),
                OutputField::FillFraction => continue,
            };
            fields.push(SampledField {
                field,
                components,
                values,
            });
        }
        FieldSet {
            step,
            time: u.steps_to_time(step),
            width: domain.width,
            height: domain.height,
            fields,
        }
    }
}

/// `ω = ∂v/∂x − ∂u/∂y` in 1/s on fluid cells (0 elsewhere).
fn vorticity(domain: &Domain, lf: &LatticeFields) -> Vec<f32> {
    let (w, h) = (domain.width as usize, domain.height as usize);
    let fluid = |x: usize, y: usize| flags::flow(domain.flags[y * w + x]) == flags::FLUID;
    // Derivative of `f` along one axis at (x, y), using fluid neighbours only.
    let deriv = |f: &[f32], x: usize, y: usize, along_x: bool| -> f64 {
        let (n, p, periodic) = if along_x {
            (w, x, domain.periodic[0])
        } else {
            (h, y, domain.periodic[1])
        };
        let at = |q: usize| if along_x { (q, y) } else { (x, q) };
        let step = |d: i64| -> Option<usize> {
            let q = p as i64 + d;
            let q: i64 = if periodic {
                q.rem_euclid(n as i64)
            } else if (0..n as i64).contains(&q) {
                q
            } else {
                return None;
            };
            let q = q as usize;
            let (qx, qy) = at(q);
            fluid(qx, qy).then_some(q)
        };
        let val = |q: usize| {
            let (qx, qy) = at(q);
            f[qy * w + qx] as f64
        };
        match (step(-1), step(1)) {
            (Some(a), Some(b)) => (val(b) - val(a)) / 2.0,
            (None, Some(b)) => val(b) - val(p),
            (Some(a), None) => val(p) - val(a),
            (None, None) => 0.0,
        }
    };
    // Lattice vorticity (per step) → 1/s.
    let scale = 1.0 / domain.units.dt;
    let mut out = vec![0.0f32; w * h];
    for y in 0..h {
        for x in 0..w {
            if fluid(x, y) {
                let omega = deriv(&lf.uy, x, y, true) - deriv(&lf.ux, x, y, false);
                out[y * w + x] = (omega * scale) as f32;
            }
        }
    }
    out
}

/// A simulation running on one backend.
///
/// Construction is backend-specific (e.g. `CpuLbm::new(domain, config)`); code that picks a
/// backend at runtime holds a `Box<dyn Solver>`.
pub trait Solver: Send {
    fn backend(&self) -> BackendKind;

    fn domain(&self) -> &Domain;

    /// Number of steps done so far.
    fn steps_done(&self) -> u64;

    /// Advances `n` steps. Fails if the solution stops being finite.
    fn step(&mut self, n: u32) -> Result<(), SolverError>;

    /// Re-initialises every cell to the equilibrium of the given lattice fields
    /// (used by validation cases with non-uniform initial conditions).
    fn set_equilibrium(&mut self, fields: &LatticeFields) -> Result<(), SolverError>;

    /// Copies the macroscopic fields (lattice units) to the CPU.
    fn lattice_fields(&mut self) -> LatticeFields;

    /// Mass, energy, maximum velocity and NaN check.
    fn diagnostics(&mut self) -> Diagnostics;

    /// Force on each element touching the fluid, by momentum exchange, for the last step.
    fn forces(&mut self) -> Vec<ElementForce>;

    /// Output fields in physical units.
    fn sample(&mut self, req: &SampleRequest) -> FieldSet {
        let lf = self.lattice_fields();
        FieldSet::from_lattice(self.domain(), &lf, self.steps_done(), req)
    }
}
