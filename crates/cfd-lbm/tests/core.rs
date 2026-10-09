//! Solver core without boundaries: equilibrium, conservation, Taylor-Green decay, determinism.

mod common;

use std::f64::consts::PI;

use cfd_core::examples::Canvas;
use cfd_core::scene::CellType;
use cfd_core::solver::{LatticeFields, Solver};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};
use common::*;

fn bgk() -> LbmConfig {
    LbmConfig {
        collision: Collision::Bgk,
    }
}

#[test]
fn rest_state_is_preserved_exactly() {
    let mut c = Canvas::new(16, 12, CellType::Fluid);
    c.rect(5, 4, 8, 7, CellType::Solid, 1);
    let mut s = solver(
        lattice_domain(&c, Default::default(), vec![wall(1)], 0.1),
        LbmConfig::default(),
    );
    s.step(200).unwrap();
    let f = s.lattice_fields();
    assert!(f.density.iter().all(|&r| r == 1.0));
    assert!(f.ux.iter().chain(&f.uy).all(|&u| u == 0.0));
}

/// Taylor-Green vortex on an N×N periodic box: `u = −U cos(kx) sin(ky)`, `v = U sin(kx) cos(ky)`,
/// `p = −ρU²/4 (cos 2kx + cos 2ky)`, decaying as `exp(−2νk²t)`.
fn taylor_green_fields(n: usize, u0: f64, nu: f64, t: f64) -> LatticeFields {
    let k = 2.0 * PI / n as f64;
    let decay = (-2.0 * nu * k * k * t).exp();
    let mut f = LatticeFields {
        density: vec![0.0; n * n],
        ux: vec![0.0; n * n],
        uy: vec![0.0; n * n],
        theta: None,
    };
    for y in 0..n {
        for x in 0..n {
            let (kx, ky) = (k * (x as f64 + 0.5), k * (y as f64 + 0.5));
            let u = u0 * decay;
            let p = -u * u / 4.0 * ((2.0 * kx).cos() + (2.0 * ky).cos());
            let i = y * n + x;
            f.density[i] = (1.0 + 3.0 * p) as f32;
            f.ux[i] = (-u * kx.cos() * ky.sin()) as f32;
            f.uy[i] = (u * kx.sin() * ky.cos()) as f32;
        }
    }
    f
}

fn taylor_green_solver(n: usize, u0: f64, nu: f64, config: LbmConfig) -> CpuLbm {
    let c = Canvas::new(n as u32, n as u32, CellType::Fluid);
    let mut s = solver(lattice_domain(&c, periodic(), vec![], nu), config);
    s.set_equilibrium(&taylor_green_fields(n, u0, nu, 0.0))
        .unwrap();
    s
}

/// L2 velocity error after one viscous time `1/(2νk²)`, diffusive scaling: `u₀ ∝ 1/N`, `ν` fixed.
fn taylor_green_error(n: usize, config: LbmConfig) -> f64 {
    let nu = 0.05;
    let u0 = 0.04 * 16.0 / n as f64;
    let k = 2.0 * PI / n as f64;
    let steps = (1.0 / (2.0 * nu * k * k)).round() as u32;
    let mut s = taylor_green_solver(n, u0, nu, config);
    s.step(steps).unwrap();
    let sim = s.lattice_fields();
    let exact = taylor_green_fields(n, u0, nu, steps as f64);
    let cat =
        |f: &LatticeFields| -> Vec<f64> { f.ux.iter().chain(&f.uy).map(|&v| v as f64).collect() };
    rel_l2(&cat(&sim), &cat(&exact))
}

#[test]
fn taylor_green_is_second_order() {
    for config in [LbmConfig::default(), bgk()] {
        let e: Vec<f64> = [16, 32, 64]
            .iter()
            .map(|&n| taylor_green_error(n, config))
            .collect();
        let (p1, p2) = (order(e[0], e[1]), order(e[1], e[2]));
        println!("{config:?}: errors {e:?}, orders {p1:.2} {p2:.2}");
        assert!(e[1] < 0.01, "{config:?}: error at N=32 is {}", e[1]);
        assert!((1.8..2.3).contains(&p2), "{config:?}: order {p2}");
    }
}

#[test]
fn taylor_green_energy_decays_at_the_viscous_rate() {
    // The decay rate carries an O((k·dx)²) dispersion error (~0.6% at N = 32).
    let (n, nu, u0) = (64, 0.02, 0.015);
    let mut s = taylor_green_solver(n, u0, nu, LbmConfig::default());
    let e0 = s.diagnostics().kinetic_energy;
    s.step(2000).unwrap();
    let e1 = s.diagnostics().kinetic_energy;
    let k = 2.0 * PI / n as f64;
    let expected = (-4.0 * nu * k * k * 2000.0).exp();
    assert!(
        ((e1 / e0) / expected - 1.0).abs() < 3e-3,
        "{} vs {expected}",
        e1 / e0
    );
}

#[test]
fn periodic_box_conserves_mass_and_momentum() {
    let n = 32;
    let mut s = taylor_green_solver(n, 0.05, 0.01, LbmConfig::default());
    // Add a uniform drift so the total momentum is not zero.
    let mut f = taylor_green_fields(n, 0.05, 0.01, 0.0);
    f.ux.iter_mut().for_each(|u| *u += 0.02);
    s.set_equilibrium(&f).unwrap();
    let momentum = |s: &mut CpuLbm| -> [f64; 2] {
        let f = s.lattice_fields();
        let mut m = [0.0; 2];
        for i in 0..f.density.len() {
            m[0] += (f.density[i] * f.ux[i]) as f64;
            m[1] += (f.density[i] * f.uy[i]) as f64;
        }
        m
    };
    let (m0, p0) = (s.diagnostics().mass, momentum(&mut s));
    s.step(1000).unwrap();
    let (m1, p1) = (s.diagnostics().mass, momentum(&mut s));
    assert!(((m1 - m0) / m0).abs() < 1e-6, "mass {m0} → {m1}");
    assert!(((p1[0] - p0[0]) / p0[0]).abs() < 1e-4, "px {p0:?} → {p1:?}");
    assert!(p1[1].abs() < 1e-3, "py {p0:?} → {p1:?}");
}

#[test]
fn results_do_not_depend_on_the_thread_count() {
    let run = |threads: usize| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
            .install(|| {
                let mut s = taylor_green_solver(48, 0.05, 0.02, LbmConfig::default());
                s.step(100).unwrap();
                s.lattice_fields()
            })
    };
    assert!(run(1) == run(4));
}

#[test]
fn divergence_is_reported() {
    let n = 16;
    let mut s = taylor_green_solver(n, 0.05, 0.01, LbmConfig::default());
    let mut f = taylor_green_fields(n, 0.05, 0.01, 0.0);
    f.density[7] = f32::NAN;
    s.set_equilibrium(&f).unwrap();
    assert!(!s.diagnostics().finite);
    assert!(s.step(5).is_err());
}
