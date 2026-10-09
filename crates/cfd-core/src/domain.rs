//! The solver-side domain: the scene converted to lattice units, with per-cell flags
//! precomputed so the kernels need no geometry logic (see `docs/03-data-model.md` §5).
//!
//! The same arrays are uploaded unchanged to every backend (CPU, wgpu, CUDA):
//!
//! - `flags[cell]` (`u16`): what the cell is and, for fluid cells, which links cross a boundary;
//! - `bc_slot[cell]` (`u32`): index into `bc_params` for non-fluid cells (0 = implicit wall);
//! - `bc_params[slot]` ([`BcParams`]): boundary values in lattice units.
//!
//! Cells are indexed `cell = y * width + x`, origin at the bottom-left corner (as the scene layers).
//!
//! All boundaries are *link-wise* and sit half-way between a fluid cell and its non-fluid
//! neighbour; only fluid cells are updated by the solver. The domain edges behave as no-slip,
//! adiabatic walls (slot 0) unless they are periodic.

use crate::scene::{
    Buoyancy, CellType, EdgeKind, ElementKind, InletVelocity, OutletBc, Precision, Scene,
    ThermalBc, WallVelocity,
};
use crate::units::{
    check_stability, relaxation_time, StabilityInput, UnitSystem, CS2, DEFAULT_LATTICE_VELOCITY,
};
use crate::validate::{validate, Issue, Report};

/// Lattice link directions `c_i`, shared by every backend. D2Q9 uses all nine; D2Q5 uses the
/// first five. Order (Krüger et al. 2017): rest, E, N, W, S, NE, NW, SW, SE.
pub const LINKS: [[i32; 2]; 9] = [
    [0, 0],
    [1, 0],
    [0, 1],
    [-1, 0],
    [0, -1],
    [1, 1],
    [-1, 1],
    [-1, -1],
    [1, -1],
];

/// Index of the opposite direction: `LINKS[OPPOSITE[i]] == -LINKS[i]`.
pub const OPPOSITE: [usize; 9] = [0, 3, 4, 1, 2, 7, 8, 5, 6];

/// Bit layout of the per-cell `flags`.
pub mod flags {
    /// Bits 0–2: how links pointing into this cell behave ([`FLUID`], [`BOUNCE_BACK`], …).
    pub const FLOW_MASK: u16 = 0b111;
    /// Regular fluid cell (updated by the solver).
    pub const FLUID: u16 = 0;
    /// Bounce-back with the slot velocity: walls, moving walls and velocity inlets.
    pub const BOUNCE_BACK: u16 = 1;
    /// Pressure outlet: anti-bounce-back with the slot density.
    pub const PRESSURE: u16 = 2;
    /// Zero-gradient outlet: the outlet state equals that of the neighbouring fluid cell.
    pub const ZERO_GRADIENT: u16 = 3;

    /// Bits 3–4: thermal behaviour of links pointing into this cell.
    pub const THERMAL_SHIFT: u16 = 3;
    pub const THERMAL_MASK: u16 = 0b11 << THERMAL_SHIFT;
    /// Zero flux (bounce-back of `g`).
    pub const ADIABATIC: u16 = 0;
    /// Fixed temperature (anti-bounce-back of `g`).
    pub const FIXED_TEMPERATURE: u16 = 1;
    /// Prescribed heat flux into the fluid.
    pub const HEAT_FLUX: u16 = 2;
    /// Zero temperature gradient (outlets, inlets without a fixed temperature).
    pub const THERMAL_ZERO_GRADIENT: u16 = 3;

    /// Bits 5–7 (outlet cells only): axis direction `d` (1…4) such that the cell at
    /// `x + c_d` is fluid; zero-gradient conditions copy the state of that interior cell.
    /// 0 when there is none.
    pub const INWARD_SHIFT: u16 = 5;
    pub const INWARD_MASK: u16 = 0b111 << INWARD_SHIFT;

    /// Bits 8–15 (fluid cells only): bit `LINK_SHIFT + i - 1` is set when population `i`
    /// (i = 1…8) is pulled from a non-fluid neighbour or from outside a non-periodic edge.
    pub const LINK_SHIFT: u16 = 8;
    pub const LINK_MASK: u16 = 0xff << LINK_SHIFT;

    pub const fn flow(flags: u16) -> u16 {
        flags & FLOW_MASK
    }

    pub const fn thermal(flags: u16) -> u16 {
        (flags & THERMAL_MASK) >> THERMAL_SHIFT
    }

    pub const fn inward(flags: u16) -> usize {
        ((flags & INWARD_MASK) >> INWARD_SHIFT) as usize
    }

    /// Whether population `i` (1…8) of a fluid cell comes through a boundary link.
    pub const fn is_boundary_link(flags: u16, i: usize) -> bool {
        flags & (1 << (LINK_SHIFT as usize + i - 1)) != 0
    }
}

/// Boundary values of one slot, in lattice units. `#[repr(C)]` with explicit padding to 32 bytes
/// so the same struct can be uploaded to WGSL/CUDA buffers.
#[derive(Debug, Clone, Copy, PartialEq)]
#[repr(C)]
pub struct BcParams {
    /// Wall/inlet velocity.
    pub velocity: [f32; 2],
    /// Outlet density (pressure outlets).
    pub density: f32,
    /// Dimensionless temperature θ (fixed-temperature boundaries).
    pub theta: f32,
    /// Heat flux into the fluid, in lattice θ-flux units.
    pub heat_flux: f32,
    _pad: [f32; 3],
}

impl Default for BcParams {
    fn default() -> Self {
        Self {
            velocity: [0.0; 2],
            density: 1.0,
            theta: 0.0,
            heat_flux: 0.0,
            _pad: [0.0; 3],
        }
    }
}

/// Fluid properties and forces, in lattice units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LatticePhysics {
    /// Kinematic viscosity.
    pub nu: f64,
    /// Thermal diffusivity; `Some` when the thermal lattice is solved.
    pub alpha: Option<f64>,
    /// Constant body acceleration (not set from scenes; used by validation cases).
    pub body_force: [f64; 2],
    /// Boussinesq buoyancy per unit θ: the force density is `buoyancy · θ` (`ρ₀ = 1`).
    pub buoyancy: [f64; 2],
}

impl LatticePhysics {
    pub fn tau(&self) -> f64 {
        relaxation_time(self.nu)
    }

    pub fn tau_thermal(&self) -> Option<f64> {
        self.alpha.map(relaxation_time)
    }

    pub fn thermal(&self) -> bool {
        self.alpha.is_some()
    }
}

/// Uniform initial state, in lattice units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InitialState {
    pub density: f64,
    pub velocity: [f64; 2],
    pub theta: f64,
}

/// Characteristic scales and dimensionless numbers, for reports.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimensionless {
    /// Characteristic velocity U (m/s); maps to the target lattice velocity.
    pub velocity: f64,
    /// Characteristic length L (m): the shorter side of the fluid bounding box.
    pub length: f64,
    /// `U·L/ν`.
    pub reynolds: f64,
    /// Lattice Mach number of U.
    pub mach: f64,
    pub prandtl: Option<f64>,
    /// `g·β·ΔT·L³/(ν·α)`, with ΔT the temperature scale.
    pub rayleigh: Option<f64>,
}

/// Hydrostatic pressure added to the output pressure field: the solver only resolves the
/// dynamic part, since a uniform gravity in a single-phase flow is balanced by `∇p = ρ₀ g`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hydrostatic {
    /// Gravity (m/s²).
    pub gravity: [f64; 2],
    /// Point (m) where the hydrostatic contribution is zero: the centroid of the pressure
    /// outlets, or of the fluid when there are none.
    pub reference: [f64; 2],
}

impl Hydrostatic {
    /// Hydrostatic gauge pressure (Pa) at the centre of cell `(x, y)`.
    pub fn pressure(&self, rho0: f64, dx: f64, x: usize, y: usize) -> f64 {
        let rx = (x as f64 + 0.5) * dx - self.reference[0];
        let ry = (y as f64 + 0.5) * dx - self.reference[1];
        rho0 * (self.gravity[0] * rx + self.gravity[1] * ry)
    }
}

/// Run length in steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RunSteps {
    pub total: u64,
    pub output_every: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProbeCell {
    pub name: String,
    pub x: u32,
    pub y: u32,
}

/// Options of the `Scene → Domain` conversion.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DomainOptions {
    /// Lattice velocity the characteristic velocity maps to.
    pub lattice_velocity: f64,
    /// Overrides the characteristic velocity (m/s) derived from the scene.
    pub characteristic_velocity: Option<f64>,
}

impl Default for DomainOptions {
    fn default() -> Self {
        Self {
            lattice_velocity: DEFAULT_LATTICE_VELOCITY,
            characteristic_velocity: None,
        }
    }
}

/// The scene in lattice units, ready for a solver backend.
#[derive(Debug, Clone)]
pub struct Domain {
    pub width: u32,
    pub height: u32,
    /// Periodic along x and along y.
    pub periodic: [bool; 2],
    pub flags: Vec<u16>,
    pub bc_slot: Vec<u32>,
    pub bc_params: Vec<BcParams>,
    /// Element id of each slot (0 = the implicit wall at the domain edges).
    pub slot_element: Vec<u16>,
    pub physics: LatticePhysics,
    pub initial: InitialState,
    pub units: UnitSystem,
    pub numbers: Dimensionless,
    pub hydrostatic: Hydrostatic,
    pub run: RunSteps,
    pub probes: Vec<ProbeCell>,
    /// Warnings from validation, support and stability checks.
    pub issues: Vec<Issue>,
}

impl Domain {
    pub fn cell_count(&self) -> usize {
        self.width as usize * self.height as usize
    }

    pub fn fluid_cell_count(&self) -> usize {
        self.flags
            .iter()
            .filter(|&&f| flags::flow(f) == flags::FLUID)
            .count()
    }

    /// Converts a scene; on failure returns the report with every issue found.
    pub fn from_scene(scene: &Scene, opts: &DomainOptions) -> Result<Domain, Report> {
        let mut report = validate(scene);
        if !report.is_ok() {
            return Err(report);
        }
        let grid = &scene.grid;
        let (w, h) = (grid.width as usize, grid.height as usize);
        let cells = grid.cell_count();
        // Validation already decoded the layers successfully.
        let cell_type: Vec<CellType> = scene
            .layers
            .cell_type
            .decode_u8(cells)
            .expect("validated layer")
            .into_iter()
            .map(|v| CellType::try_from(v).expect("validated cell type"))
            .collect();
        let element_id = scene
            .layers
            .element_id
            .decode_u16(cells)
            .expect("validated layer");

        check_support(scene, &cell_type, &mut report);

        let fluid = scene
            .fluids
            .iter()
            .find(|f| f.id == scene.initial.fluid)
            .expect("validated initial fluid");
        let thermal = scene.physics.thermal;
        let boussinesq = thermal && scene.physics.buoyancy == Buoyancy::Boussinesq;
        let conductivity = fluid.thermal_conductivity.unwrap_or(0.0);
        let specific_heat = fluid.specific_heat.unwrap_or(1.0);
        let alpha = thermal.then(|| conductivity / (fluid.density * specific_heat));
        let beta = fluid.thermal_expansion.unwrap_or(0.0);
        let dx = grid.cell_size;

        // Characteristic length: shorter side of the fluid bounding box.
        let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
        for (i, ct) in cell_type.iter().enumerate() {
            if *ct == CellType::Fluid {
                let (x, y) = (i % w, i / w);
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
        let length = (x1 - x0).min(y1 - y0) as f64 * dx;

        // Temperature scale: spread of the prescribed temperatures (and of flux-driven ΔT).
        let t_ref = scene.initial.temperature;
        let (mut t_min, mut t_max, mut flux_dt) = (t_ref, t_ref, 0.0f64);
        for el in &scene.elements {
            match thermal_bc(&el.kind) {
                Some(ThermalBc::Fixed { value }) => {
                    t_min = t_min.min(*value);
                    t_max = t_max.max(*value);
                }
                Some(ThermalBc::Flux { value }) if conductivity > 0.0 => {
                    flux_dt = flux_dt.max(value.abs() * length / conductivity);
                }
                _ => {}
            }
        }
        let delta_t = match (t_max - t_min).max(flux_dt) {
            d if d > 0.0 => d,
            _ => 1.0,
        };

        // Characteristic velocity: the fastest prescribed velocity or the buoyancy velocity.
        let g = scene.physics.gravity;
        let g_mag = g[0].hypot(g[1]);
        let mut velocities = vec![norm(scene.initial.velocity)];
        for el in &scene.elements {
            match &el.kind {
                ElementKind::Inlet { velocity, .. } => velocities.push(match velocity {
                    InletVelocity::Uniform { value } => norm(*value),
                    InletVelocity::Parabolic { peak } => norm(*peak),
                }),
                ElementKind::Wall {
                    velocity: WallVelocity::Moving { velocity },
                    ..
                } => velocities.push(norm(*velocity)),
                _ => {}
            }
        }
        let u_buoyancy = if boussinesq {
            (g_mag * beta * delta_t * length).sqrt()
        } else {
            0.0
        };
        velocities.push(u_buoyancy);
        let u_scene = velocities.iter().copied().fold(0.0, f64::max);
        let u_char = opts.characteristic_velocity.unwrap_or(u_scene);

        let dt = if u_char > 0.0 {
            opts.lattice_velocity * dx / u_char
        } else {
            // Nothing moves: pick dt so the largest diffusivity has τ = 1.
            let d = fluid.kinematic_viscosity.max(alpha.unwrap_or(0.0));
            dx * dx / (6.0 * d)
        };
        let units = UnitSystem {
            dx,
            dt,
            rho0: fluid.density,
            t_ref,
            delta_t,
        };

        let nu = units.diffusivity_to_lattice(fluid.kinematic_viscosity);
        let alpha_lb = alpha.map(|a| units.diffusivity_to_lattice(a));
        check_stability(
            &StabilityInput {
                nu,
                alpha: alpha_lb,
                u_max: units.velocity_to_lattice(u_scene.max(u_char)),
            },
            &mut report,
        );
        if !report.is_ok() {
            return Err(report);
        }

        let g_lb = g.map(|c| units.acceleration_to_lattice(c));
        let buoyancy = if boussinesq {
            g_lb.map(|c| -beta * delta_t * c)
        } else {
            [0.0; 2]
        };
        let physics = LatticePhysics {
            nu,
            alpha: alpha_lb,
            body_force: [0.0; 2],
            buoyancy,
        };

        // Boundary slots and flags.
        let mut bc_params = vec![BcParams::default()];
        let mut slot_element = vec![0u16];
        let mut flags = vec![flags::FLUID; cells];
        let mut bc_slot = vec![0u32; cells];
        for el in &scene.elements {
            let ids: Vec<usize> = (0..cells).filter(|&i| element_id[i] == el.id).collect();
            if ids.is_empty() {
                continue;
            }
            let (flow, thermal_kind, base) = element_bc(&el.kind, &units, fluid, thermal);
            let cell_flags = flow | (thermal_kind << flags::THERMAL_SHIFT);
            match &el.kind {
                ElementKind::Inlet {
                    velocity: InletVelocity::Parabolic { peak },
                    ..
                } => {
                    let peak_lb = peak.map(|c| units.velocity_to_lattice(c));
                    for (i, profile) in parabolic_profile(&ids, w, *peak) {
                        flags[i] = cell_flags;
                        bc_slot[i] = bc_params.len() as u32;
                        bc_params.push(BcParams {
                            velocity: peak_lb.map(|c| (c * profile) as f32),
                            ..base
                        });
                        slot_element.push(el.id);
                    }
                }
                _ => {
                    let slot = bc_params.len() as u32;
                    bc_params.push(base);
                    slot_element.push(el.id);
                    for i in ids {
                        flags[i] = cell_flags;
                        bc_slot[i] = slot;
                    }
                }
            }
        }

        // Link masks of fluid cells.
        let periodic = [
            grid.edges.left == EdgeKind::Periodic,
            grid.edges.bottom == EdgeKind::Periodic,
        ];
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if cell_type[i] != CellType::Fluid {
                    continue;
                }
                for (dir, c) in LINKS.iter().enumerate().skip(1) {
                    let source = neighbour(x, y, -c[0], -c[1], w, h, periodic);
                    let boundary = match source {
                        None => true,
                        Some(j) => cell_type[j] != CellType::Fluid,
                    };
                    if boundary {
                        flags[i] |= 1 << (flags::LINK_SHIFT as usize + dir - 1);
                    }
                }
            }
        }

        // Inward axis direction of outlet cells: prefer a fluid neighbour whose opposite
        // neighbour is not fluid (the outlet is a one-cell layer), else any fluid neighbour.
        let is_fluid = |c: Option<usize>| c.is_some_and(|j| cell_type[j] == CellType::Fluid);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if cell_type[i] != CellType::Outlet {
                    continue;
                }
                let candidates = (1..5).filter(|&d| {
                    let [cx, cy] = LINKS[d];
                    is_fluid(neighbour(x, y, cx, cy, w, h, periodic))
                });
                let preferred = candidates.clone().find(|&d| {
                    let [cx, cy] = LINKS[d];
                    !is_fluid(neighbour(x, y, -cx, -cy, w, h, periodic))
                });
                if let Some(d) = preferred.or_else(|| candidates.clone().next()) {
                    flags[i] |= (d as u16) << flags::INWARD_SHIFT;
                }
            }
        }

        // Hydrostatic reference: centroid of the pressure outlets, else of the fluid.
        let centroid = |pick: &dyn Fn(usize) -> bool| -> Option<[f64; 2]> {
            let (mut sx, mut sy, mut n) = (0.0, 0.0, 0usize);
            for i in (0..cells).filter(|&i| pick(i)) {
                sx += (i % w) as f64 + 0.5;
                sy += (i / w) as f64 + 0.5;
                n += 1;
            }
            (n > 0).then(|| [sx / n as f64 * dx, sy / n as f64 * dx])
        };
        let reference = centroid(&|i| flags::flow(flags[i]) == flags::PRESSURE)
            .or_else(|| centroid(&|i| cell_type[i] == CellType::Fluid))
            .unwrap_or([0.0; 2]);

        let numbers = Dimensionless {
            velocity: u_char,
            length,
            reynolds: u_char * length / fluid.kinematic_viscosity,
            mach: units.velocity_to_lattice(u_char) / CS2.sqrt(),
            prandtl: alpha.map(|a| fluid.kinematic_viscosity / a),
            rayleigh: alpha
                .filter(|_| boussinesq)
                .map(|a| g_mag * beta * delta_t * length.powi(3) / (fluid.kinematic_viscosity * a)),
        };

        let initial = InitialState {
            density: units.gauge_pressure_to_density(scene.initial.pressure),
            velocity: scene.initial.velocity.map(|c| units.velocity_to_lattice(c)),
            theta: units.temperature_to_lattice(scene.initial.temperature),
        };

        Ok(Domain {
            width: grid.width,
            height: grid.height,
            periodic,
            flags,
            bc_slot,
            bc_params,
            slot_element,
            physics,
            initial,
            units,
            numbers,
            hydrostatic: Hydrostatic {
                gravity: g,
                reference,
            },
            run: RunSteps {
                total: units.time_to_steps(scene.run.end_time),
                output_every: units.time_to_steps(scene.run.output_interval),
            },
            probes: scene
                .probes
                .iter()
                .map(|p| ProbeCell {
                    name: p.name.clone(),
                    x: p.position[0],
                    y: p.position[1],
                })
                .collect(),
            issues: report.issues,
        })
    }
}

fn norm(v: [f64; 2]) -> f64 {
    v[0].hypot(v[1])
}

fn thermal_bc(kind: &ElementKind) -> Option<&ThermalBc> {
    match kind {
        ElementKind::Wall { thermal, .. }
        | ElementKind::Inlet { thermal, .. }
        | ElementKind::HeatSource { thermal, .. } => Some(thermal),
        ElementKind::Outlet { .. } => None,
    }
}

/// Index of the cell at `(x + dx, y + dy)`, wrapping periodic axes; `None` outside the domain.
fn neighbour(
    x: usize,
    y: usize,
    dx: i32,
    dy: i32,
    w: usize,
    h: usize,
    periodic: [bool; 2],
) -> Option<usize> {
    let wrap = |v: usize, d: i32, n: usize, periodic: bool| -> Option<usize> {
        let t = v as i64 + d as i64;
        if (0..n as i64).contains(&t) {
            Some(t as usize)
        } else if periodic {
            Some(t.rem_euclid(n as i64) as usize)
        } else {
            None
        }
    };
    Some(wrap(y, dy, h, periodic[1])? * w + wrap(x, dx, w, periodic[0])?)
}

/// Flow kind, thermal kind and slot values of an element.
fn element_bc(
    kind: &ElementKind,
    units: &UnitSystem,
    fluid: &crate::scene::Fluid,
    thermal: bool,
) -> (u16, u16, BcParams) {
    let mut p = BcParams::default();
    let thermal_kind = |bc: &ThermalBc, p: &mut BcParams, fallback: u16| -> u16 {
        if !thermal {
            return flags::ADIABATIC;
        }
        match *bc {
            ThermalBc::Adiabatic => fallback,
            ThermalBc::Fixed { value } => {
                p.theta = units.temperature_to_lattice(value) as f32;
                flags::FIXED_TEMPERATURE
            }
            ThermalBc::Flux { value } => {
                p.heat_flux = units.heat_flux_to_lattice(
                    value,
                    fluid.density,
                    fluid.specific_heat.unwrap_or(1.0),
                ) as f32;
                flags::HEAT_FLUX
            }
            // Rejected by `check_support`.
            ThermalBc::Convective { .. } => flags::ADIABATIC,
        }
    };
    match kind {
        ElementKind::Wall {
            velocity, thermal, ..
        } => {
            if let WallVelocity::Moving { velocity } = velocity {
                p.velocity = velocity.map(|c| units.velocity_to_lattice(c) as f32);
            }
            let t = thermal_kind(thermal, &mut p, flags::ADIABATIC);
            (flags::BOUNCE_BACK, t, p)
        }
        ElementKind::HeatSource { thermal, .. } => {
            let t = thermal_kind(thermal, &mut p, flags::ADIABATIC);
            (flags::BOUNCE_BACK, t, p)
        }
        ElementKind::Inlet {
            velocity, thermal, ..
        } => {
            if let InletVelocity::Uniform { value } = velocity {
                p.velocity = value.map(|c| units.velocity_to_lattice(c) as f32);
            }
            // An "adiabatic" inlet lets the fluid carry its own temperature in.
            let t = thermal_kind(thermal, &mut p, flags::THERMAL_ZERO_GRADIENT);
            (flags::BOUNCE_BACK, t, p)
        }
        ElementKind::Outlet { pressure } => {
            let t = if thermal {
                flags::THERMAL_ZERO_GRADIENT
            } else {
                flags::ADIABATIC
            };
            match pressure {
                OutletBc::Pressure { value } => {
                    p.density = units.gauge_pressure_to_density(*value) as f32;
                    (flags::PRESSURE, t, p)
                }
                OutletBc::ZeroGradient => (flags::ZERO_GRADIENT, t, p),
            }
        }
    }
}

/// Parabolic profile factor `4ξ(1 − ξ)` of each cell, with ξ the position across the element
/// (perpendicular to the peak velocity) over the full extent of its cells.
fn parabolic_profile(cells: &[usize], w: usize, peak: [f64; 2]) -> Vec<(usize, f64)> {
    let n = norm(peak);
    if n == 0.0 {
        return cells.iter().map(|&i| (i, 0.0)).collect();
    }
    let t = [-peak[1] / n, peak[0] / n];
    let s = |i: usize| ((i % w) as f64 + 0.5) * t[0] + ((i / w) as f64 + 0.5) * t[1];
    let half = 0.5 * (t[0].abs() + t[1].abs());
    let lo = cells.iter().map(|&i| s(i)).fold(f64::INFINITY, f64::min) - half;
    let hi = cells
        .iter()
        .map(|&i| s(i))
        .fold(f64::NEG_INFINITY, f64::max)
        + half;
    cells
        .iter()
        .map(|&i| {
            let xi = (s(i) - lo) / (hi - lo);
            (i, 4.0 * xi * (1.0 - xi))
        })
        .collect()
}

/// Features that are valid in the format but not implemented by the Phase 1 solver.
fn check_support(scene: &Scene, cell_type: &[CellType], r: &mut Report) {
    let has_pressure_outlet = scene.elements.iter().any(|e| {
        matches!(
            e.kind,
            ElementKind::Outlet {
                pressure: OutletBc::Pressure { .. }
            }
        )
    });
    if cell_type.contains(&CellType::Inlet)
        && cell_type.contains(&CellType::Outlet)
        && !has_pressure_outlet
    {
        r.warning(
            "domain.noPressureReference",
            "velocity inlets with only zero-gradient outlets leave the pressure level undetermined: \
             mass may drift; use a pressure outlet",
        );
    }
    if scene.physics.free_surface || cell_type.contains(&CellType::Empty) {
        r.error(
            "support.freeSurface",
            "free-surface flows are not supported yet (planned for Phase 6a)",
        );
    }
    if let Some(layer) = &scene.layers.fluid_id {
        let v = layer.decode_u8(scene.grid.cell_count()).unwrap_or_default();
        if v.iter().any(|&f| f != 0 && f != scene.initial.fluid) {
            r.error(
                "support.multiFluid",
                "only one fluid per simulation is supported (the fluidId layer uses several)",
            );
        }
    }
    if scene.run.precision == Precision::F64 {
        r.warning(
            "support.precision",
            "the solver runs in f32 (f64 is not supported yet; WebGPU has no f64)",
        );
    }
    if scene.physics.buoyancy == Buoyancy::Boussinesq && !scene.physics.thermal {
        r.warning(
            "physics.buoyancy",
            "Boussinesq buoyancy needs the thermal solver; it is ignored",
        );
    }
    for el in &scene.elements {
        match &el.kind {
            ElementKind::Wall {
                velocity: WallVelocity::Slip,
                ..
            } => r.error(
                "support.slip",
                format!("element {}: slip walls are not supported yet", el.id),
            ),
            ElementKind::Inlet { fluid, .. } if *fluid != scene.initial.fluid => r.warning(
                "support.inletFluid",
                format!(
                    "element {}: inlet fluid {fluid} differs from the initial fluid; the initial fluid is used",
                    el.id
                ),
            ),
            _ => {}
        }
        if !scene.physics.thermal {
            continue;
        }
        match (thermal_bc(&el.kind), &el.kind) {
            (Some(ThermalBc::Convective { .. }), _) => r.error(
                "support.convective",
                format!(
                    "element {}: convective thermal boundaries are not supported yet",
                    el.id
                ),
            ),
            (Some(ThermalBc::Flux { .. }), ElementKind::Inlet { .. }) => r.error(
                "support.inletFlux",
                format!(
                    "element {}: inlets accept a fixed temperature or adiabatic",
                    el.id
                ),
            ),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{self, Canvas};
    use crate::scene::{DomainEdges, Element};

    fn domain(scene: &Scene) -> Domain {
        Domain::from_scene(scene, &DomainOptions::default())
            .unwrap_or_else(|r| panic!("{:#?}", r.issues))
    }

    fn error_codes(scene: &Scene) -> Vec<&'static str> {
        match Domain::from_scene(scene, &DomainOptions::default()) {
            Ok(_) => vec![],
            Err(r) => r.errors().map(|i| i.code).collect(),
        }
    }

    #[test]
    fn opposite_links_are_consistent() {
        for i in 0..9 {
            let (c, o) = (LINKS[i], LINKS[OPPOSITE[i]]);
            assert_eq!([c[0] + o[0], c[1] + o[1]], [0, 0]);
            assert_eq!(OPPOSITE[OPPOSITE[i]], i);
        }
    }

    #[test]
    fn bc_params_have_gpu_friendly_size() {
        assert_eq!(std::mem::size_of::<BcParams>(), 32);
    }

    /// 5 × 4 channel: walls at the bottom/top rows, inlet on the left, outlet on the right.
    fn tiny_channel(edges: DomainEdges) -> Scene {
        let mut scene = examples::channel();
        let mut c = Canvas::new(5, 4, CellType::Fluid);
        c.rect(0, 0, 5, 1, CellType::Solid, 1);
        c.rect(0, 3, 5, 4, CellType::Solid, 1);
        if edges.left != EdgeKind::Periodic {
            c.rect(0, 1, 1, 3, CellType::Inlet, 2);
            c.rect(4, 1, 5, 3, CellType::Outlet, 3);
        }
        scene.grid.width = 5;
        scene.grid.height = 4;
        scene.grid.edges = edges;
        scene.layers = c.layers();
        scene.probes.clear();
        scene
    }

    #[test]
    fn flags_and_link_masks_of_a_tiny_channel() {
        let d = domain(&tiny_channel(DomainEdges::default()));
        let at = |x: usize, y: usize| d.flags[y * 5 + x];
        assert_eq!(flags::flow(at(0, 0)), flags::BOUNCE_BACK);
        assert_eq!(flags::flow(at(0, 1)), flags::BOUNCE_BACK); // inlet
        assert_eq!(flags::flow(at(4, 2)), flags::PRESSURE);
        assert_eq!(flags::inward(at(4, 2)), 3); // interior is to the west
        assert_eq!(d.bc_slot[0], 1);
        assert_eq!(d.slot_element, vec![0, 1, 2, 3]);
        // Inlet slot carries the lattice inlet velocity (= the target lattice velocity).
        let inlet = d.bc_params[d.bc_slot[5] as usize];
        assert!((inlet.velocity[0] - 0.05).abs() < 1e-7, "{inlet:?}");

        // Fluid cell (1, 1): wall below, inlet on the left.
        let f = at(1, 1);
        assert_eq!(flags::flow(f), flags::FLUID);
        let boundary: Vec<usize> = (1..9).filter(|&i| flags::is_boundary_link(f, i)).collect();
        // Pulled from (x − c_i): E(1) ← inlet, N(2) ← wall, NE(5) ← corner wall,
        // NW(6) ← (2, 0) wall, SE(8) ← inlet (0, 2).
        assert_eq!(boundary, vec![1, 2, 5, 6, 8]);
        // Fluid cell (2, 2): only the top wall.
        let boundary: Vec<usize> = (1..9)
            .filter(|&i| flags::is_boundary_link(at(2, 2), i))
            .collect();
        assert_eq!(boundary, vec![4, 7, 8]);
    }

    #[test]
    fn periodic_edges_wrap_and_open_edges_are_walls() {
        let periodic = DomainEdges {
            left: EdgeKind::Periodic,
            right: EdgeKind::Periodic,
            ..DomainEdges::default()
        };
        let mut scene = tiny_channel(periodic);
        scene.elements.retain(|e| e.id == 1);
        let d = domain(&scene);
        assert_eq!(d.periodic, [true, false]);
        // (0, 1) sees the bottom wall only; the left neighbour wraps to (4, 1), a fluid cell.
        let boundary: Vec<usize> = (1..9)
            .filter(|&i| flags::is_boundary_link(d.flags[5], i))
            .collect();
        assert_eq!(boundary, vec![2, 5, 6]);

        // Without walls drawn, the non-periodic edges are walls.
        let mut c = Canvas::new(5, 4, CellType::Fluid);
        c.rect(0, 0, 0, 0, CellType::Fluid, 0);
        scene.layers = c.layers();
        let d = domain(&scene);
        let boundary: Vec<usize> = (1..9)
            .filter(|&i| flags::is_boundary_link(d.flags[0], i))
            .collect();
        assert_eq!(boundary, vec![2, 5, 6]);
        assert_eq!(d.fluid_cell_count(), 20);
    }

    #[test]
    fn cylinder_has_parabolic_inlet_and_expected_tau() {
        let scene = examples::cylinder_re100();
        let d = domain(&scene);
        // U = 1.5 m/s peak ↦ 0.05; ν_lb = 0.05 · 1e-3 / (1.5 · 0.005) ⇒ τ ≈ 0.52.
        assert!((d.physics.tau() - 0.52).abs() < 1e-9, "{}", d.physics.tau());
        assert!(d.issues.is_empty(), "{:#?}", d.issues);
        let w = scene.grid.width as usize;
        let inlet_u = |y: usize| d.bc_params[d.bc_slot[y * w] as usize].velocity[0] as f64;
        // 82 inlet cells; the profile is symmetric and peaks at 0.05 · (1 − 1/82²) at the centre.
        assert!((inlet_u(41) - inlet_u(42)).abs() < 1e-7);
        assert!((inlet_u(41) - 0.05 * (1.0 - 1.0 / 82f64.powi(2))).abs() < 1e-7);
        let xi = 0.5 / 82.0;
        assert!((inlet_u(1) - 0.05 * 4.0 * xi * (1.0 - xi)).abs() < 1e-8);
        // Mean over the inlet = 2/3 of the peak (the benchmark's U_mean = 1 m/s).
        let mean: f64 = (1..83).map(inlet_u).sum::<f64>() / 82.0;
        assert!((d.units.velocity_to_physical(mean) - 1.0).abs() < 1e-3);
        // 8 s of physical time.
        assert_eq!(d.run.total, d.units.time_to_steps(8.0));
    }

    #[test]
    fn heated_cavity_scales_temperature_and_buoyancy() {
        let d = domain(&examples::heated_cavity_ra1e5());
        let ra = d.numbers.rayleigh.unwrap();
        assert!((ra / 1e5 - 1.0).abs() < 1e-3, "Ra = {ra}");
        assert!((d.numbers.prandtl.unwrap() - 0.71).abs() < 0.01);
        // Hot and cold walls at θ = ±0.5.
        let thetas: Vec<f32> = d.bc_params.iter().map(|p| p.theta).collect();
        assert!(thetas.iter().any(|t| (t - 0.5).abs() < 1e-4), "{thetas:?}");
        assert!(thetas.iter().any(|t| (t + 0.5).abs() < 1e-4), "{thetas:?}");
        // Buoyancy points up for θ > 0, and its magnitude matches u_lb² / L_lb for u_b ↦ u_lb.
        assert!(d.physics.buoyancy[1] > 0.0 && d.physics.buoyancy[0] == 0.0);
        let l_lb = d.numbers.length / d.units.dx;
        assert!((d.physics.buoyancy[1] * l_lb / 0.05f64.powi(2) - 1.0).abs() < 1e-9);
        let (tau, tau_g) = (d.physics.tau(), d.physics.tau_thermal().unwrap());
        assert!(tau > 0.54 && tau < 0.56, "τ = {tau}");
        assert!(tau_g > 0.56 && tau_g < 0.58, "τ_g = {tau_g}");
        // Hot wall cells are fixed-temperature bounce-back cells.
        let hot = d.flags[130];
        assert_eq!(flags::flow(hot), flags::BOUNCE_BACK);
        assert_eq!(flags::thermal(hot), flags::FIXED_TEMPERATURE);
    }

    #[test]
    fn unsupported_features_are_rejected() {
        assert_eq!(
            error_codes(&examples::dam_break()),
            vec!["support.freeSurface"]
        );
        let mut scene = examples::channel();
        scene.elements[0] = Element {
            id: 1,
            name: None,
            kind: ElementKind::Wall {
                velocity: WallVelocity::Slip,
                thermal: ThermalBc::Adiabatic,
                material: None,
            },
        };
        assert_eq!(error_codes(&scene), vec!["support.slip"]);
    }

    #[test]
    fn zero_gradient_outlet_without_pressure_reference_warns() {
        let mut scene = examples::channel();
        scene.elements[2].kind = ElementKind::Outlet {
            pressure: OutletBc::ZeroGradient,
        };
        let codes: Vec<_> = domain(&scene).issues.iter().map(|i| i.code).collect();
        assert_eq!(codes, vec!["domain.noPressureReference"]);
    }

    #[test]
    fn f64_precision_is_a_warning() {
        let mut scene = examples::channel();
        scene.run.precision = Precision::F64;
        let d = domain(&scene);
        let codes: Vec<_> = d.issues.iter().map(|i| i.code).collect();
        assert_eq!(codes, vec!["support.precision"]);
    }

    #[test]
    fn unstable_scene_is_rejected() {
        let mut scene = examples::channel();
        // A much faster inlet with the same viscosity and grid ⇒ τ → 0.5.
        scene.grid.cell_size = 0.01;
        scene.fluids[0].kinematic_viscosity = 1e-7;
        assert!(error_codes(&scene).contains(&"stability.tau"));
    }

    #[test]
    fn pressure_outlet_density_and_hydrostatic_reference() {
        let mut scene = examples::channel();
        scene.elements[2].kind = ElementKind::Outlet {
            pressure: OutletBc::Pressure { value: 0.1 },
        };
        scene.physics.gravity = [0.0, -crate::units::STANDARD_GRAVITY];
        let d = domain(&scene);
        let slot = d.bc_params[3];
        let p = d.units.density_to_gauge_pressure(slot.density as f64);
        assert!((p - 0.1).abs() < 1e-4, "p = {p}");
        // Reference at the outlet centroid (x = 399.5 cells, y = 50 cells).
        let r = d.hydrostatic.reference;
        assert!(
            (r[0] - 0.3995).abs() < 1e-9 && (r[1] - 0.05).abs() < 1e-9,
            "{r:?}"
        );
        let rho0 = d.units.rho0;
        // One cell below the reference row: positive hydrostatic pressure.
        let ph = d.hydrostatic.pressure(rho0, d.units.dx, 399, 49);
        assert!((ph - rho0 * crate::units::STANDARD_GRAVITY * 0.0005).abs() < 1e-9);
    }
}
