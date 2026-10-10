//! CPU ↔ wgpu parity (docs/04-numerical-methods.md §7): the same domain on both backends, N
//! steps, then the macroscopic fields must agree to < 1e-5 — ρ in absolute terms (ρ ≈ 1), u and
//! θ relative to the field's maximum. Diagnostics and forces are compared as well.
//!
//! Tests are skipped (with a message) when no wgpu adapter exists; with `CFD_REQUIRE_GPU=1` they
//! fail instead, so CI notices a missing software adapter.

use std::f64::consts::PI;
use std::sync::{Arc, OnceLock};

use cfd_core::examples::Canvas;
use cfd_core::scene::{
    CellType, Element, ElementKind, InletVelocity, OutletBc, ThermalBc, WallVelocity,
};
use cfd_core::solver::{LatticeFields, Solver};
use cfd_core::{Domain, DomainOptions, Scene};
use cfd_gpu::{GpuContext, GpuOptions, WgpuLbm};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};
use cfd_verify::scenes;

const TOLERANCE: f64 = 1e-5;

fn context() -> Option<Arc<GpuContext>> {
    static CTX: OnceLock<Option<Arc<GpuContext>>> = OnceLock::new();
    CTX.get_or_init(|| {
        let opts = GpuOptions {
            allow_software: true,
            ..Default::default()
        };
        match GpuContext::new(&opts) {
            Ok(c) => {
                eprintln!("wgpu adapter: {}", c.description());
                Some(c)
            }
            Err(e) if std::env::var_os("CFD_REQUIRE_GPU").is_some() => {
                panic!("CFD_REQUIRE_GPU is set but there is no wgpu adapter: {e}")
            }
            Err(e) => {
                eprintln!("skipping wgpu parity tests: {e}");
                None
            }
        }
    })
    .clone()
}

fn domain(scene: &Scene, lattice_velocity: f64) -> Domain {
    let opts = DomainOptions {
        lattice_velocity,
        characteristic_velocity: None,
    };
    Domain::from_scene(scene, &opts).unwrap_or_else(|r| panic!("{:#?}", r.issues))
}

/// `max |a − b|`.
fn max_diff(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs() as f64)
        .fold(0.0, f64::max)
}

/// `max |b|` (at least 1e-30).
fn max_abs(b: &[f32]) -> f64 {
    b.iter().map(|v| v.abs() as f64).fold(1e-30, f64::max)
}

struct Differences {
    rho: f64,
    u: f64,
    theta: f64,
}

/// ρ in absolute terms; both velocity components relative to the largest component of the
/// reference (a component that is zero up to round-off, e.g. uy in a channel, must not set the
/// scale); θ relative to its largest value.
fn differences(gpu: &LatticeFields, cpu: &LatticeFields) -> Differences {
    let speed = max_abs(&cpu.ux).max(max_abs(&cpu.uy));
    Differences {
        rho: max_diff(&gpu.density, &cpu.density),
        u: max_diff(&gpu.ux, &cpu.ux).max(max_diff(&gpu.uy, &cpu.uy)) / speed,
        theta: match (&gpu.theta, &cpu.theta) {
            (Some(a), Some(b)) => max_diff(a, b) / max_abs(b),
            (None, None) => 0.0,
            _ => panic!("one backend has the thermal lattice and the other not"),
        },
    }
}

/// Runs both backends for `steps` (in `chunks` calls) from `initial` (or the domain's initial
/// state) and checks fields, diagnostics and forces.
fn check(name: &str, d: Domain, initial: Option<LatticeFields>, steps: u32, chunks: u32) {
    let Some(ctx) = context() else { return };
    check_with(name, &ctx, d, initial, steps, chunks, LbmConfig::default());
}

fn check_with(
    name: &str,
    ctx: &Arc<GpuContext>,
    d: Domain,
    initial: Option<LatticeFields>,
    steps: u32,
    chunks: u32,
    config: LbmConfig,
) {
    let mut cpu = CpuLbm::new(d.clone(), config);
    let mut gpu = WgpuLbm::new(ctx.clone(), d, config).expect("wgpu solver");
    if let Some(f) = &initial {
        cpu.set_equilibrium(f).unwrap();
        gpu.set_equilibrium(f).unwrap();
    }
    // Initial state: identical by construction (same code builds the populations).
    let e = differences(&gpu.lattice_fields(), &cpu.lattice_fields());
    assert_eq!(
        (e.rho, e.u, e.theta),
        (0.0, 0.0, 0.0),
        "{name}: initial state"
    );
    for _ in 0..chunks {
        cpu.step(steps / chunks).unwrap();
        gpu.step(steps / chunks).unwrap();
    }
    gpu.finish().unwrap();
    assert_eq!(gpu.steps_done(), cpu.steps_done());
    let (fc, fg) = (cpu.lattice_fields(), gpu.lattice_fields());
    let e = differences(&fg, &fc);
    eprintln!(
        "{name}: {} steps, max |u| {:.2e}, max diff ρ {:.2e}, u {:.2e}, θ {:.2e}",
        cpu.steps_done(),
        max_abs(&fc.ux).max(max_abs(&fc.uy)),
        e.rho,
        e.u,
        e.theta
    );
    assert!(e.rho < TOLERANCE, "{name}: density differs by {:e}", e.rho);
    assert!(e.u < TOLERANCE, "{name}: velocity differs by {:e}", e.u);
    assert!(e.theta < TOLERANCE, "{name}: θ differs by {:e}", e.theta);

    let (dc, dg) = (cpu.diagnostics(), gpu.diagnostics());
    assert_eq!(dg.fluid_cells, dc.fluid_cells, "{name}: fluid cells");
    assert!(dg.finite && dc.finite);
    let rel = |a: f64, b: f64| (a - b).abs() / b.abs().max(1e-30);
    assert!(
        rel(dg.mass, dc.mass) < 1e-7,
        "{name}: mass {} vs {}",
        dg.mass,
        dc.mass
    );
    assert!(
        rel(dg.kinetic_energy, dc.kinetic_energy) < TOLERANCE,
        "{name}: energy {} vs {}",
        dg.kinetic_energy,
        dc.kinetic_energy
    );
    assert!(
        rel(dg.max_velocity, dc.max_velocity) < TOLERANCE,
        "{name}: max |u|"
    );
    match (dg.thermal_energy, dc.thermal_energy) {
        (Some(a), Some(b)) => assert!((a - b).abs() < TOLERANCE * dc.fluid_cells as f64),
        (None, None) => {}
        _ => panic!("{name}: thermal energy reported by one backend only"),
    }

    let (fc, fg) = (cpu.forces(), gpu.forces());
    assert_eq!(fc.len(), fg.len(), "{name}: elements with forces");
    let scale = fc
        .iter()
        .flat_map(|f| f.lattice)
        .map(f64::abs)
        .fold(1e-30, f64::max);
    for (a, b) in fg.iter().zip(&fc) {
        assert_eq!(a.element, b.element);
        for c in 0..2 {
            let diff = (a.lattice[c] - b.lattice[c]).abs() / scale;
            assert!(
                diff < 1e-4,
                "{name}: force on element {} differs by {diff:e}",
                a.element
            );
        }
    }
}

// ---- Step 2: bulk kernel ----

fn taylor_green(n: u32, u0: f32) -> LatticeFields {
    let n_us = n as usize;
    let mut f = LatticeFields {
        density: vec![1.0; n_us * n_us],
        ux: vec![0.0; n_us * n_us],
        uy: vec![0.0; n_us * n_us],
        theta: None,
    };
    let k = 2.0 * PI / n as f64;
    for y in 0..n_us {
        for x in 0..n_us {
            let (px, py) = (k * (x as f64 + 0.5), k * (y as f64 + 0.5));
            let i = y * n_us + x;
            f.ux[i] = -u0 * (px.cos() * py.sin()) as f32;
            f.uy[i] = u0 * (px.sin() * py.cos()) as f32;
            f.density[i] =
                1.0 - (0.75 * (u0 as f64).powi(2) * ((2.0 * px).cos() + (2.0 * py).cos())) as f32;
        }
    }
    f
}

#[test]
fn taylor_green_vortex() {
    let d = domain(&scenes::periodic_box(64, 1e-3), 0.05);
    let tg = taylor_green(64, 0.05);
    check("taylor-green", d, Some(tg), 500, 3);
}

#[test]
fn taylor_green_with_bgk() {
    let Some(ctx) = context() else { return };
    let d = domain(&scenes::periodic_box(48, 1e-3), 0.05);
    let config = LbmConfig {
        collision: Collision::Bgk,
    };
    check_with(
        "taylor-green bgk",
        &ctx,
        d,
        Some(taylor_green(48, 0.05)),
        300,
        1,
        config,
    );
}

#[test]
fn poiseuille_body_force() {
    let mut d = domain(&scenes::plane_channel(32, 0.1, 0.0), 0.05);
    d.physics.body_force = [1e-5, 0.0];
    check("poiseuille", d, None, 3000, 4);
}

// ---- Step 3: boundaries ----

#[test]
fn couette_moving_wall() {
    let d = domain(&scenes::plane_channel(32, 0.1, 0.05), 0.05);
    check("couette", d, None, 2000, 2);
}

#[test]
fn lid_driven_cavity() {
    let d = domain(&scenes::lid_driven_cavity(64, 400.0), 0.1);
    check("cavity", d, None, 2000, 2);
}

#[test]
fn cylinder_pressure_outlet() {
    // Velocity inlet, pressure outlet, walls and the staircase cylinder (Re 100).
    let d = domain(&scenes::schafer_turek(20, 1.5), 0.1);
    check("cylinder", d, None, 1500, 3);
}

/// Thermal channel exercising every boundary branch: uniform inlet at a fixed temperature, a
/// heat-flux bottom wall, a moving adiabatic top wall, a zero-gradient outlet (flow and θ) and a
/// fixed-temperature obstacle.
fn thermal_channel() -> Domain {
    let mut s = scenes::heated_cavity(16, 1e3, 0.71);
    let (w, h) = (64, 18);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 10);
    c.rect(0, h - 1, w, h, CellType::Solid, 11);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, 12);
    c.rect(w - 1, 1, w, h - 1, CellType::Outlet, 13);
    c.disk(20.0, 9.0, 3.0, CellType::Solid, 14);
    s.grid.width = w;
    s.grid.height = h;
    s.layers = c.layers();
    let wall = |id, velocity, thermal| Element {
        id,
        name: None,
        kind: ElementKind::Wall {
            velocity,
            thermal,
            material: None,
        },
    };
    s.elements = vec![
        wall(10, WallVelocity::NoSlip, ThermalBc::Flux { value: 0.5 }),
        wall(
            11,
            WallVelocity::Moving {
                velocity: [0.1, 0.0],
            },
            ThermalBc::Adiabatic,
        ),
        Element {
            id: 12,
            name: None,
            kind: ElementKind::Inlet {
                velocity: InletVelocity::Uniform { value: [0.3, 0.0] },
                thermal: ThermalBc::Fixed {
                    value: scenes::T_COLD,
                },
                fluid: 1,
            },
        },
        Element {
            id: 13,
            name: None,
            kind: ElementKind::Outlet {
                pressure: OutletBc::ZeroGradient,
            },
        },
        wall(
            14,
            WallVelocity::NoSlip,
            ThermalBc::Fixed {
                value: scenes::T_HOT,
            },
        ),
    ];
    domain(&s, 0.08)
}

#[test]
fn channel_with_every_boundary_kind() {
    check("thermal channel", thermal_channel(), None, 2000, 2);
}

#[test]
fn isothermal_zero_gradient_outlet() {
    let mut d = thermal_channel();
    d.physics.alpha = None;
    d.physics.buoyancy = [0.0; 2];
    assert!(!d.physics.thermal());
    check("zero-gradient channel", d, None, 2000, 1);
}

// ---- Step 4: thermal lattice ----

#[test]
fn heated_cavity() {
    let d = domain(&scenes::heated_cavity(64, 1e5, 0.71), 0.08);
    check("heated cavity", d, None, 3000, 3);
}

#[test]
fn rayleigh_benard() {
    let d = domain(&scenes::rayleigh_benard(24, 2e4, 0.71), 0.08);
    // A perturbed conduction profile, so the flow is not trivially at rest.
    let mut init = cfd_lbm::shared::initial_fields(&d);
    let theta = init.theta.as_mut().unwrap();
    let w = d.width as usize;
    for (i, t) in theta.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        *t += 0.1 * ((x as f32 * 0.3).sin() + (y as f32 * 0.2).cos());
    }
    check("rayleigh-benard", d, Some(init), 6000, 2);
}

// ---- Solver behaviour ----

#[test]
fn divergence_is_reported() {
    let Some(ctx) = context() else { return };
    let d = domain(&scenes::periodic_box(16, 1e-3), 0.05);
    let mut gpu = WgpuLbm::new(ctx, d, LbmConfig::default()).unwrap();
    let mut init = cfd_lbm::shared::initial_fields(gpu.domain());
    init.density[37] = f32::NAN;
    gpu.set_equilibrium(&init).unwrap();
    let r = gpu.step(10).and_then(|_| gpu.finish());
    assert!(
        matches!(r, Err(cfd_core::solver::SolverError::NonFinite { .. })),
        "{r:?}"
    );
    assert!(!gpu.diagnostics().finite);
}

#[test]
fn asynchronous_readback_matches_synchronous() {
    let Some(ctx) = context() else { return };
    let d = domain(&scenes::lid_driven_cavity(32, 100.0), 0.1);
    let mut gpu = WgpuLbm::new(ctx, d, LbmConfig::default()).unwrap();
    gpu.step(100).unwrap();
    let mut pending = gpu.request_fields();
    assert_eq!(pending.step(), 100);
    let snapshot_sync = gpu.lattice_fields();
    // Stepping continues while the copy is in flight; the copy keeps the state at step 100.
    gpu.step(100).unwrap();
    let fields = loop {
        if let Some(f) = pending.try_take() {
            break f;
        }
        std::thread::yield_now();
    };
    assert_eq!(fields, snapshot_sync);
    assert!(pending.try_take().is_none());
    gpu.finish().unwrap();
    assert_ne!(gpu.lattice_fields(), snapshot_sync);
}
