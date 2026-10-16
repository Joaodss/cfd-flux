//! Parity between a backend and the CPU reference (docs/04-numerical-methods.md §7): the same
//! domain on both, N steps, then the macroscopic fields must agree to < 1e-5 — ρ in absolute terms
//! (ρ ≈ 1), u and θ relative to the field's maximum. Diagnostics and forces are compared as well.
//!
//! Shared by the GPU backends: a test file provides a factory and expands [`parity_tests!`](crate::parity_tests)
//! into one `#[test]` per case.
//!
//! ```ignore
//! fn factory() -> Option<Box<cfd_verify::parity::Factory>> { /* None: skip */ }
//! cfd_verify::parity_tests!(factory);
//! ```

use std::f64::consts::PI;

use cfd_core::examples::Canvas;
use cfd_core::sample::{f16_bits, Precision, Sample, SampleData, SampleSpec};
use cfd_core::scene::OutputField;
use cfd_core::scene::{
    CellType, Element, ElementKind, InletVelocity, OutletBc, ThermalBc, WallVelocity,
};
use cfd_core::solver::{BackendKind, LatticeFields, Solver, SolverError};
use cfd_core::{Domain, DomainOptions, Scene};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};

use crate::scenes;

/// Field tolerance of 04 §7; also used for the kinetic energy and max |u| (relative).
pub const TOLERANCE: f64 = 1e-5;

/// Total mass, relative (f64 sums of f32 values, conserved by construction).
pub const MASS_TOLERANCE: f64 = 1e-7;

/// Force per element, relative to the largest force component of the reference.
pub const FORCE_TOLERANCE: f64 = 1e-4;

/// Builds the solver under test (in C#: a `Func<Domain, LbmConfig, ISolver>`).
pub type Factory = dyn Fn(Domain, LbmConfig) -> Box<dyn Solver>;

/// Expands into one `#[test]` per parity case. `$factory` is a function returning
/// `Option<Box<Factory>>`; `None` skips the tests (no device).
#[macro_export]
macro_rules! parity_tests {
    ($factory:path) => {
        $crate::parity_tests!(@cases $factory;
            taylor_green_vortex,
            taylor_green_with_bgk,
            poiseuille_body_force,
            couette_moving_wall,
            lid_driven_cavity,
            cylinder_pressure_outlet,
            channel_with_every_boundary_kind,
            isothermal_zero_gradient_outlet,
            heated_cavity,
            rayleigh_benard,
            divergence_is_reported,
            asynchronous_readback_matches_synchronous,
            sampling_matches_the_cpu_conversion,
            thermal_sampling_matches_the_cpu_conversion,
            skipping_macro_stores_changes_nothing,
        );
    };
    (@cases $factory:path; $($case:ident),* $(,)?) => {
        $(
            #[test]
            fn $case() {
                if let Some(make) = $factory() {
                    $crate::parity::$case(&*make);
                }
            }
        )*
    };
}

fn domain(scene: &Scene, lattice_velocity: f64) -> Domain {
    let opts = DomainOptions {
        lattice_velocity,
        characteristic_velocity: None,
    };
    Domain::from_scene(scene, &opts).unwrap_or_else(|r| panic!("{:#?}", r.issues))
}

/// `max |a − b|`.
pub fn max_diff(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs() as f64)
        .fold(0.0, f64::max)
}

/// `max |b|` (at least 1e-30).
pub fn max_abs(b: &[f32]) -> f64 {
    b.iter().map(|v| v.abs() as f64).fold(1e-30, f64::max)
}

/// Field differences, as defined in 04 §7.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Differences {
    pub rho: f64,
    pub u: f64,
    pub theta: f64,
}

/// ρ in absolute terms; both velocity components relative to the largest component of the
/// reference (a component that is zero up to round-off, e.g. uy in a channel, must not set the
/// scale); θ relative to its largest value.
pub fn differences(test: &LatticeFields, reference: &LatticeFields) -> Differences {
    let speed = max_abs(&reference.ux).max(max_abs(&reference.uy));
    Differences {
        rho: max_diff(&test.density, &reference.density),
        u: max_diff(&test.ux, &reference.ux).max(max_diff(&test.uy, &reference.uy)) / speed,
        theta: match (&test.theta, &reference.theta) {
            (Some(a), Some(b)) => max_diff(a, b) / max_abs(b),
            (None, None) => 0.0,
            _ => panic!("one backend has the thermal lattice and the other not"),
        },
    }
}

/// Runs the backend and the CPU reference for `steps` (in `chunks` calls) from `initial` (or
/// the domain's initial state) and checks fields, diagnostics and forces.
pub fn check(
    name: &str,
    make: &Factory,
    d: Domain,
    initial: Option<LatticeFields>,
    steps: u32,
    chunks: u32,
    config: LbmConfig,
) -> Differences {
    let mut cpu = CpuLbm::new(d.clone(), config);
    let mut gpu = make(d, config);
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
    let identical = fc == fg;
    eprintln!(
        "{name} ({:?}): {} steps, max |u| {:.2e}, max diff ρ {:.2e}, u {:.2e}, θ {:.2e}{}",
        gpu.backend(),
        cpu.steps_done(),
        max_abs(&fc.ux).max(max_abs(&fc.uy)),
        e.rho,
        e.u,
        e.theta,
        if identical { " (bit-identical)" } else { "" },
    );
    assert!(e.rho < TOLERANCE, "{name}: density differs by {:e}", e.rho);
    assert!(e.u < TOLERANCE, "{name}: velocity differs by {:e}", e.u);
    assert!(e.theta < TOLERANCE, "{name}: θ differs by {:e}", e.theta);

    let (dc, dg) = (cpu.diagnostics(), gpu.diagnostics());
    assert_eq!(dg.fluid_cells, dc.fluid_cells, "{name}: fluid cells");
    assert!(dg.finite && dc.finite);
    let rel = |a: f64, b: f64| (a - b).abs() / b.abs().max(1e-30);
    assert!(
        rel(dg.mass, dc.mass) < MASS_TOLERANCE,
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
                diff < FORCE_TOLERANCE,
                "{name}: force on element {} differs by {diff:e}",
                a.element
            );
        }
    }
    e
}

// ---- Bulk kernel ----

/// Taylor-Green vortex fields on an `n × n` periodic box.
pub fn taylor_green(n: u32, u0: f32) -> LatticeFields {
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

pub fn taylor_green_vortex(make: &Factory) {
    let d = domain(&scenes::periodic_box(64, 1e-3), 0.05);
    let tg = taylor_green(64, 0.05);
    check(
        "taylor-green",
        make,
        d,
        Some(tg),
        500,
        3,
        LbmConfig::default(),
    );
}

pub fn taylor_green_with_bgk(make: &Factory) {
    let d = domain(&scenes::periodic_box(48, 1e-3), 0.05);
    let config = LbmConfig {
        collision: Collision::Bgk,
        ..Default::default()
    };
    check(
        "taylor-green bgk",
        make,
        d,
        Some(taylor_green(48, 0.05)),
        300,
        1,
        config,
    );
}

pub fn poiseuille_body_force(make: &Factory) {
    let mut d = domain(&scenes::plane_channel(32, 0.1, 0.0), 0.05);
    d.physics.body_force = [1e-5, 0.0];
    check("poiseuille", make, d, None, 3000, 4, LbmConfig::default());
}

// ---- Boundaries ----

pub fn couette_moving_wall(make: &Factory) {
    let d = domain(&scenes::plane_channel(32, 0.1, 0.05), 0.05);
    check("couette", make, d, None, 2000, 2, LbmConfig::default());
}

pub fn lid_driven_cavity(make: &Factory) {
    let d = domain(&scenes::lid_driven_cavity(64, 400.0), 0.1);
    check("cavity", make, d, None, 2000, 2, LbmConfig::default());
}

/// Velocity inlet, pressure outlet, walls and the staircase cylinder (Re 100).
pub fn cylinder_pressure_outlet(make: &Factory) {
    let d = domain(&scenes::schafer_turek(20, 1.5), 0.1);
    check("cylinder", make, d, None, 1500, 3, LbmConfig::default());
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

pub fn channel_with_every_boundary_kind(make: &Factory) {
    let d = thermal_channel();
    check(
        "thermal channel",
        make,
        d,
        None,
        2000,
        2,
        LbmConfig::default(),
    );
}

pub fn isothermal_zero_gradient_outlet(make: &Factory) {
    let mut d = thermal_channel();
    d.physics.alpha = None;
    d.physics.buoyancy = [0.0; 2];
    assert!(!d.physics.thermal());
    check(
        "zero-gradient channel",
        make,
        d,
        None,
        2000,
        1,
        LbmConfig::default(),
    );
}

// ---- Thermal lattice ----

pub fn heated_cavity(make: &Factory) {
    let d = domain(&scenes::heated_cavity(64, 1e5, 0.71), 0.08);
    check(
        "heated cavity",
        make,
        d,
        None,
        3000,
        3,
        LbmConfig::default(),
    );
}

pub fn rayleigh_benard(make: &Factory) {
    let d = domain(&scenes::rayleigh_benard(24, 2e4, 0.71), 0.08);
    // A perturbed conduction profile, so the flow is not trivially at rest.
    let mut init = cfd_lbm::shared::initial_fields(&d);
    let theta = init.theta.as_mut().unwrap();
    let w = d.width as usize;
    for (i, t) in theta.iter_mut().enumerate() {
        let (x, y) = (i % w, i / w);
        *t += 0.1 * ((x as f32 * 0.3).sin() + (y as f32 * 0.2).cos());
    }
    check(
        "rayleigh-benard",
        make,
        d,
        Some(init),
        6000,
        2,
        LbmConfig::default(),
    );
}

// ---- Solver behaviour ----

/// A NaN in the initial state is reported as a divergence, by `step` or at the latest `finish`.
pub fn divergence_is_reported(make: &Factory) {
    let d = domain(&scenes::periodic_box(16, 1e-3), 0.05);
    let mut gpu = make(d, LbmConfig::default());
    let mut init = cfd_lbm::shared::initial_fields(gpu.domain());
    init.density[37] = f32::NAN;
    gpu.set_equilibrium(&init).unwrap();
    let r = gpu.step(10).and_then(|_| gpu.finish());
    assert!(matches!(r, Err(SolverError::NonFinite { .. })), "{r:?}");
    assert!(!gpu.diagnostics().finite);
}

/// A copy requested with `request_fields` keeps the state of the step it was requested at, while
/// stepping continues.
pub fn asynchronous_readback_matches_synchronous(make: &Factory) {
    let d = domain(&scenes::lid_driven_cavity(32, 100.0), 0.1);
    let mut gpu = make(d, LbmConfig::default());
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

// ---- On-device sampling (design proposal §6) ----

/// The specs every sampling case requests at once: full resolution and two box filters (one that
/// leaves partial blocks at the edges), f32 and f16, fields in different orders.
fn sample_specs() -> Vec<SampleSpec> {
    use OutputField::*;
    let all = vec![Velocity, Pressure, Density, Temperature, Vorticity];
    vec![
        SampleSpec::full(all.clone(), Precision::F32),
        SampleSpec::full(all.clone(), Precision::F16),
        SampleSpec {
            fields: vec![Vorticity, Velocity, Temperature],
            factor: 3,
            precision: Precision::F32,
        },
        SampleSpec {
            fields: vec![Vorticity, Velocity, Temperature],
            factor: 3,
            precision: Precision::F16,
        },
        SampleSpec {
            fields: all,
            factor: 4,
            precision: Precision::F16,
        },
    ]
}

/// Distance in units in the last place between two f16 bit patterns (on the ordered line of
/// values, so +0 and −0 are one apart).
fn f16_ulps(a: u16, b: u16) -> u32 {
    let ordered = |v: u16| -> i32 {
        if v & 0x8000 != 0 {
            -((v & 0x7fff) as i32) - 1
        } else {
            v as i32
        }
    };
    (ordered(a) - ordered(b)).unsigned_abs()
}

/// Compares one backend sample with the reference: bit-identical when `exact`; otherwise f32
/// values within [`TOLERANCE`] of the field's largest value, and f16 values within one ulp (or
/// within that tolerance, for values near zero, where an f16 ulp is far below it).
fn compare_sample(name: &str, got: &Sample, want: &Sample, exact: bool) {
    assert_eq!(got.spec, want.spec, "{name}: spec");
    assert_eq!(
        (got.step, got.width, got.height),
        (want.step, want.width, want.height),
        "{name}: step and size"
    );
    assert!(
        (got.time - want.time).abs() <= 1e-12 * want.time.abs(),
        "{name}: time"
    );
    let kinds = |s: &Sample| {
        s.fields
            .iter()
            .map(|f| (f.field, f.components))
            .collect::<Vec<_>>()
    };
    assert_eq!(kinds(got), kinds(want), "{name}: fields");
    if exact {
        for (g, w) in got.fields.iter().zip(&want.fields) {
            let (gv, wv) = (g.data.to_f32(), w.data.to_f32());
            let first = (0..gv.len()).find(|&i| gv[i].to_bits() != wv[i].to_bits());
            if let Some(i) = first {
                panic!(
                    "{name} ({:?}): {:?}[{i}] = {:e} vs {:e}: not bit-identical",
                    got.spec, g.field, gv[i], wv[i]
                );
            }
        }
        assert!(got == want, "{name}: not bit-identical");
        return;
    }
    for (g, w) in got.fields.iter().zip(&want.fields) {
        let (gv, wv) = (g.data.to_f32(), w.data.to_f32());
        let scale = max_abs(&wv);
        match (&g.data, &w.data) {
            (SampleData::F32(_), SampleData::F32(_)) => {
                let d = max_diff(&gv, &wv) / scale;
                assert!(d < TOLERANCE, "{name}: {:?} differs by {d:e}", g.field);
            }
            (SampleData::F16(a), SampleData::F16(b)) => {
                for (i, (&x, &y)) in a.iter().zip(b).enumerate() {
                    let close = ((gv[i] - wv[i]).abs() as f64) < TOLERANCE * scale;
                    assert!(
                        f16_ulps(x, y) <= 1 || close,
                        "{name}: {:?}[{i}] = {} vs {} ({} f16 ulps)",
                        g.field,
                        gv[i],
                        wv[i],
                        f16_ulps(x, y)
                    );
                }
            }
            _ => panic!("{name}: precision differs"),
        }
    }
}

/// Requests every spec of [`sample_specs`] after `steps` steps, keeps stepping while the copy is
/// in flight, and compares each sample with [`Sample::from_lattice`] applied to the backend's own
/// lattice fields of that step: this isolates the sampling kernel from the stream-collide kernel
/// (whose parity the other cases check). CUDA must be bit-identical (f64 conversions, no FMA,
/// round-to-nearest-even f16); wgpu (f32 arithmetic, the driver's FMA contraction) is held to
/// [`TOLERANCE`] in f32 and one f16 ulp, and its f16 output must be its own f32 output rounded
/// to nearest-even within one ulp (`pack2x16float`'s rounding; the number of values one ulp off
/// is printed).
pub fn check_sampling(
    name: &str,
    make: &Factory,
    d: Domain,
    initial: Option<LatticeFields>,
    steps: u32,
) {
    let mut s = make(d, LbmConfig::default());
    if let Some(f) = &initial {
        s.set_equilibrium(f).unwrap();
    }
    s.step(steps).unwrap();
    let specs = sample_specs();
    let pending = s.request_samples(&specs);
    assert_eq!(pending.step(), steps as u64);
    let lf = s.lattice_fields();
    // Stepping goes on while the samples are copied; they keep the state they were taken at.
    s.step(10).unwrap();
    let got = pending.wait().into_samples(s.domain());
    s.finish().unwrap();
    assert_eq!(got.len(), specs.len());
    let exact = s.backend() != BackendKind::Wgpu;
    for (spec, g) in specs.iter().zip(&got) {
        let want = Sample::from_lattice(s.domain(), &lf, steps as u64, spec);
        eprintln!(
            "{name} ({:?}): k = {}, {:?}, {} fields{}",
            s.backend(),
            spec.factor,
            spec.precision,
            g.fields.len(),
            if *g == want { " (bit-identical)" } else { "" }
        );
        compare_sample(name, g, &want, exact);
    }
    // f16 = RNE(f32) of the same backend, at full resolution and with the box filter.
    for (i32_, i16_) in [(0, 1), (2, 3)] {
        let (a, b) = (&got[i32_], &got[i16_]);
        let mut off = 0usize;
        let mut total = 0usize;
        for (fa, fb) in a.fields.iter().zip(&b.fields) {
            let SampleData::F16(bits) = &fb.data else {
                panic!("{name}: f16 expected")
            };
            let rounded = f16_bits(&fa.data.to_f32());
            for (&x, &y) in bits.iter().zip(&rounded) {
                assert!(
                    f16_ulps(x, y) <= 1,
                    "{name}: f16 conversion off by more than one ulp"
                );
                off += (x != y) as usize;
                total += 1;
            }
        }
        eprintln!(
            "{name} ({:?}): f16 vs RNE(f32), k = {}: {off} of {total} values one ulp off",
            s.backend(),
            a.spec.factor
        );
        assert!(
            !exact || off == 0,
            "{name}: f16 is not round-to-nearest-even"
        );
    }
}

/// Velocity inlet, pressure outlet and the cylinder, with gravity for the hydrostatic pressure;
/// then a periodic box (the vorticity's differences wrap around).
pub fn sampling_matches_the_cpu_conversion(make: &Factory) {
    let mut d = domain(&scenes::schafer_turek(20, 1.5), 0.1);
    d.hydrostatic.gravity = [1.0, -cfd_core::units::STANDARD_GRAVITY];
    check_sampling("sampling cylinder", make, d, None, 600);
    let d = domain(&scenes::periodic_box(30, 1e-3), 0.05);
    check_sampling(
        "sampling periodic",
        make,
        d,
        Some(taylor_green(30, 0.05)),
        50,
    );
}

/// The thermal lattice: temperature.
pub fn thermal_sampling_matches_the_cpu_conversion(make: &Factory) {
    let d = domain(&scenes::heated_cavity(40, 1e5, 0.71), 0.08);
    check_sampling("sampling heated cavity", make, d, None, 800);
}

// ---- Macroscopic stores only when needed (design proposal §7) ----

/// `skip_macro` stores ρ, u and θ only on the last step of each `step` call (and u on cells with
/// a boundary link, which pressure outlets read on the next step). Only stores are skipped, so on
/// the CPU and CUDA a solver with it must give **exactly** (`==`) the fields, diagnostics and forces
/// of one that stores them every step, after calls of every length — including the odd ones that
/// end outside a CUDA graph. wgpu compiles the two variants as two pipelines, and a driver may
/// round them differently: DX12's FXC changes a few values by ~1e-8 (Vulkan on the RTX 4060 is
/// exact), so wgpu is held to exact or [`TOLERANCE`] (velocity relative to the run's peak
/// speed) — a missing store gives errors many orders larger (the solution diverges). Runs on the cylinder (velocity inlet, pressure outlet), the
/// thermal channel (every boundary kind, θ) and a periodic box.
pub fn skipping_macro_stores_changes_nothing(make: &Factory) {
    let cases = [
        (
            "cylinder",
            domain(&scenes::schafer_turek(20, 1.5), 0.1),
            None,
        ),
        ("thermal channel", thermal_channel(), None),
        (
            "periodic box",
            domain(&scenes::periodic_box(32, 1e-3), 0.05),
            Some(taylor_green(32, 0.05)),
        ),
    ];
    for (name, d, initial) in cases {
        let write_all = LbmConfig {
            skip_macro: false,
            ..Default::default()
        };
        let mut skip = make(d.clone(), LbmConfig::default());
        let mut all = make(d, write_all);
        if let Some(f) = &initial {
            skip.set_equilibrium(f).unwrap();
            all.set_equilibrium(f).unwrap();
        }
        let exact = skip.backend() != BackendKind::Wgpu;
        // Velocity scale: the largest speed of the run (initial state included).
        let speed_of = |f: &LatticeFields| max_abs(&f.ux).max(max_abs(&f.uy));
        let mut peak = speed_of(&skip.lattice_fields());
        let mut worst = 0.0f64;
        for n in [1, 2, 7, 51, 100, 1203, 3] {
            skip.step(n).unwrap();
            all.step(n).unwrap();
            let (a, b) = (skip.lattice_fields(), all.lattice_fields());
            peak = peak.max(speed_of(&b));
            if a == b {
                assert_eq!(skip.diagnostics(), all.diagnostics(), "{name}: diagnostics");
                assert_eq!(skip.forces(), all.forces(), "{name}: forces");
                continue;
            }
            assert!(
                !exact,
                "{name} ({:?}): fields differ after a {n}-step call",
                skip.backend()
            );
            // Velocity relative to the run's peak speed: the box's vortex decays to ~1e-8, where
            // round-off alone is a large share of the field's own maximum.
            let e = differences(&a, &b);
            let du = max_diff(&a.ux, &b.ux).max(max_diff(&a.uy, &b.uy)) / peak;
            let d = e.rho.max(du).max(e.theta);
            assert!(
                d < TOLERANCE,
                "{name} ({:?}): fields differ by {d:e} after a {n}-step call",
                skip.backend()
            );
            worst = worst.max(d);
        }
        eprintln!(
            "{name} ({:?}): skip_macro {} storing every step over {} steps",
            skip.backend(),
            if worst == 0.0 {
                "identical to".to_string()
            } else {
                format!("within {worst:.1e} of")
            },
            skip.steps_done()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The CPU reference follows the same rule (no GPU needed).
    #[test]
    fn cpu_skip_macro_changes_nothing() {
        skipping_macro_stores_changes_nothing(&|d, c| Box::new(CpuLbm::new(d, c)));
    }
}
