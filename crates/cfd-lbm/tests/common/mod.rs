//! Helpers to build domains directly in lattice units.
#![allow(dead_code)]

use cfd_core::domain::{Domain, DomainOptions};
use cfd_core::examples::{self, Canvas};
use cfd_core::scene::{
    DomainEdges, EdgeKind, Element, ElementKind, InletVelocity, OutletBc, Scene, ThermalBc,
    WallVelocity,
};
use cfd_core::solver::{LatticeFields, Solver};
use cfd_lbm::{CpuLbm, LbmConfig};

/// Options under which physical and lattice units coincide when `cellSize = 1` and
/// `density = 1`: `dt = u_lb · dx / U = dx`.
pub const LATTICE_UNITS: DomainOptions = DomainOptions {
    lattice_velocity: 0.01,
    characteristic_velocity: Some(0.01),
};

pub fn periodic() -> DomainEdges {
    DomainEdges {
        left: EdgeKind::Periodic,
        right: EdgeKind::Periodic,
        bottom: EdgeKind::Periodic,
        top: EdgeKind::Periodic,
    }
}

pub fn periodic_x() -> DomainEdges {
    DomainEdges {
        left: EdgeKind::Periodic,
        right: EdgeKind::Periodic,
        ..DomainEdges::default()
    }
}

/// A scene in lattice units: `cellSize = 1`, `density = 1`, kinematic viscosity `nu`.
pub fn lattice_scene(
    canvas: &Canvas,
    edges: DomainEdges,
    elements: Vec<Element>,
    nu: f64,
) -> Scene {
    let mut scene = examples::channel();
    scene.grid.width = canvas.width;
    scene.grid.height = canvas.height;
    scene.grid.cell_size = 1.0;
    scene.grid.edges = edges;
    scene.fluids[0].density = 1.0;
    scene.fluids[0].kinematic_viscosity = nu;
    scene.elements = elements;
    scene.probes.clear();
    scene.layers = canvas.layers();
    scene.run.end_time = 1.0e6;
    scene.run.output_interval = 1.0e3;
    scene
}

pub fn lattice_domain(
    canvas: &Canvas,
    edges: DomainEdges,
    elements: Vec<Element>,
    nu: f64,
) -> Domain {
    let scene = lattice_scene(canvas, edges, elements, nu);
    let d =
        Domain::from_scene(&scene, &LATTICE_UNITS).unwrap_or_else(|r| panic!("{:#?}", r.issues));
    assert!((d.units.dt - 1.0).abs() < 1e-12 && (d.physics.nu - nu).abs() < 1e-12);
    d
}

pub fn wall(id: u16) -> Element {
    moving_wall(id, [0.0, 0.0])
}

pub fn moving_wall(id: u16, velocity: [f64; 2]) -> Element {
    Element {
        id,
        name: None,
        kind: ElementKind::Wall {
            velocity: if velocity == [0.0, 0.0] {
                WallVelocity::NoSlip
            } else {
                WallVelocity::Moving { velocity }
            },
            thermal: ThermalBc::Adiabatic,
            material: None,
        },
    }
}

pub fn inlet(id: u16, velocity: InletVelocity) -> Element {
    Element {
        id,
        name: None,
        kind: ElementKind::Inlet {
            velocity,
            thermal: ThermalBc::Adiabatic,
            fluid: 1,
        },
    }
}

pub fn outlet(id: u16, pressure: OutletBc) -> Element {
    Element {
        id,
        name: None,
        kind: ElementKind::Outlet { pressure },
    }
}

pub fn solver(domain: Domain, config: LbmConfig) -> CpuLbm {
    CpuLbm::new(domain, config)
}

/// Relative L2 error `‖s − r‖ / ‖r‖`.
pub fn rel_l2(sim: &[f64], reference: &[f64]) -> f64 {
    let num: f64 = sim
        .iter()
        .zip(reference)
        .map(|(s, r)| (s - r).powi(2))
        .sum();
    let den: f64 = reference.iter().map(|r| r * r).sum();
    (num / den).sqrt()
}

/// Order of convergence from errors at resolutions N and 2N.
pub fn order(e_coarse: f64, e_fine: f64) -> f64 {
    (e_coarse / e_fine).log2()
}

/// Runs until the max change of `u` between checks (every `every` steps), relative to `max |u|`,
/// is below `tol`. f32 round-off limits `tol` to ~1e-6.
pub fn run_to_steady(s: &mut CpuLbm, every: u32, tol: f64, max_steps: u64) -> LatticeFields {
    let mut prev = s.lattice_fields();
    let mut change = f64::INFINITY;
    while s.steps_done() < max_steps {
        s.step(every).expect("finite");
        let now = s.lattice_fields();
        let umax = now
            .ux
            .iter()
            .chain(&now.uy)
            .map(|v| v.abs() as f64)
            .fold(1e-30, f64::max);
        change = now
            .ux
            .iter()
            .zip(&prev.ux)
            .chain(now.uy.iter().zip(&prev.uy))
            .map(|(a, b)| (a - b).abs() as f64)
            .fold(0.0, f64::max)
            / umax;
        prev = now;
        if change < tol {
            return prev;
        }
    }
    panic!("not steady after {max_steps} steps (relative change {change:e})");
}

/// A thermal scene in lattice units: `ρ = c_p = 1`, so the conductivity equals the diffusivity
/// `alpha`; the initial temperature is `t0` (= T_ref). With `boussinesq = Some(gβ)` gravity
/// points down and buoyancy is on.
pub fn thermal_domain(
    canvas: &Canvas,
    edges: DomainEdges,
    elements: Vec<Element>,
    nu: f64,
    alpha: f64,
    t0: f64,
    boussinesq: Option<f64>,
) -> Domain {
    let mut scene = lattice_scene(canvas, edges, elements, nu);
    scene.physics.thermal = true;
    scene.fluids[0].thermal_conductivity = Some(alpha);
    scene.fluids[0].specific_heat = Some(1.0);
    scene.initial.temperature = t0;
    if let Some(g_beta) = boussinesq {
        let g = cfd_core::units::STANDARD_GRAVITY;
        scene.physics.gravity = [0.0, -g];
        scene.physics.buoyancy = cfd_core::scene::Buoyancy::Boussinesq;
        scene.fluids[0].thermal_expansion = Some(g_beta / g);
    }
    let d =
        Domain::from_scene(&scene, &LATTICE_UNITS).unwrap_or_else(|r| panic!("{:#?}", r.issues));
    assert!((d.units.dt - 1.0).abs() < 1e-12);
    assert!((d.physics.alpha.unwrap() - alpha).abs() < 1e-12);
    d
}

pub fn thermal_wall(id: u16, thermal: ThermalBc) -> Element {
    Element {
        id,
        name: None,
        kind: ElementKind::Wall {
            velocity: WallVelocity::NoSlip,
            thermal,
            material: None,
        },
    }
}

/// Runs until the max change of θ between checks, relative to `max |θ|`, is below `tol`.
pub fn run_to_steady_theta(s: &mut CpuLbm, every: u32, tol: f64, max_steps: u64) -> LatticeFields {
    let mut prev = s.lattice_fields();
    let mut change = f64::INFINITY;
    while s.steps_done() < max_steps {
        s.step(every).expect("finite");
        let now = s.lattice_fields();
        let (a, b) = (now.theta.as_ref().unwrap(), prev.theta.as_ref().unwrap());
        let tmax = a.iter().map(|v| v.abs() as f64).fold(1e-30, f64::max);
        change = a
            .iter()
            .zip(b)
            .map(|(x, y)| (x - y).abs() as f64)
            .fold(0.0, f64::max)
            / tmax;
        prev = now;
        if change < tol {
            return prev;
        }
    }
    panic!("θ not steady after {max_steps} steps (relative change {change:e})");
}
