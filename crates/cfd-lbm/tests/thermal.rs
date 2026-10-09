//! Thermal D2Q5 lattice, thermal boundaries and Boussinesq coupling.

mod common;

use std::f64::consts::PI;

use cfd_core::examples::Canvas;
use cfd_core::scene::{CellType, ThermalBc};
use cfd_core::solver::{LatticeFields, Solver};
use cfd_lbm::{Collision, LbmConfig};
use common::*;

/// Column of `h` fluid rows between a bottom wall (row 0) and a top wall (row h + 1),
/// periodic in x.
fn column(h: u32, bottom: ThermalBc, top: ThermalBc, t0: f64, alpha: f64) -> cfd_lbm::CpuLbm {
    let mut c = Canvas::new(3, h + 2, CellType::Fluid);
    c.rect(0, 0, 3, 1, CellType::Solid, 1);
    c.rect(0, h + 1, 3, h + 2, CellType::Solid, 2);
    let elements = vec![thermal_wall(1, bottom), thermal_wall(2, top)];
    let d = thermal_domain(&c, periodic_x(), elements, 0.1, alpha, t0, None);
    solver(d, LbmConfig::default())
}

#[test]
fn conduction_between_fixed_temperature_walls_is_linear() {
    let h = 16;
    let mut s = column(
        h,
        ThermalBc::Fixed { value: 1.0 },
        ThermalBc::Fixed { value: 0.0 },
        0.5,
        0.1,
    );
    let f = run_to_steady_theta(&mut s, 500, 1e-7, 1_000_000);
    let theta = f.theta.unwrap();
    // Walls half-way: θ = +½ at y = 1, −½ at y = h + 1.
    for y in 1..=h as usize {
        let exact = 0.5 - (y as f64 + 0.5 - 1.0) / h as f64;
        let sim = theta[y * 3 + 1] as f64;
        assert!((sim - exact).abs() < 1e-5, "y = {y}: {sim} vs {exact}");
    }
    assert!(f.ux.iter().chain(&f.uy).all(|&u| u == 0.0));
}

#[test]
fn prescribed_heat_flux_gives_the_fourier_gradient() {
    let (h, alpha) = (16u32, 0.08);
    let mut s = column(
        h,
        ThermalBc::Flux { value: 0.002 },
        ThermalBc::Fixed { value: 0.0 },
        0.0,
        alpha,
    );
    let q = s.domain().bc_params[1].heat_flux as f64;
    let theta_top = s.domain().bc_params[2].theta as f64;
    assert!(q > 0.0);
    let f = run_to_steady_theta(&mut s, 500, 1e-7, 1_000_000);
    let theta = f.theta.unwrap();
    let top_face = h as f64 + 1.0;
    for y in 1..=h as usize {
        let exact = theta_top + q / alpha * (top_face - (y as f64 + 0.5));
        let sim = theta[y * 3 + 1] as f64;
        let range = q / alpha * h as f64;
        assert!(
            (sim - exact).abs() < 1e-4 * range,
            "y = {y}: {sim} vs {exact}"
        );
    }
}

/// A sine wave advected by a uniform flow and diffusing: `θ = A e^{−αk²t} sin(k(x − u t))`.
/// Returns the relative L2 error after one diffusive time `1/(αk²)` (diffusive scaling).
fn advected_sine_error(n: usize, config: LbmConfig) -> f64 {
    let alpha = 0.05;
    let u0 = 0.04 * 16.0 / n as f64;
    let k = 2.0 * PI / n as f64;
    let steps = (1.0 / (alpha * k * k)).round() as u32;
    let c = Canvas::new(n as u32, 4, CellType::Fluid);
    let d = thermal_domain(&c, periodic(), vec![], 0.05, alpha, 0.0, None);
    let mut s = solver(d, config);
    let field = |t: f64| -> Vec<f64> {
        (0..4 * n)
            .map(|i| {
                let x = (i % n) as f64 + 0.5;
                (-alpha * k * k * t).exp() * (k * (x - u0 * t)).sin()
            })
            .collect()
    };
    s.set_equilibrium(&LatticeFields {
        density: vec![1.0; 4 * n],
        ux: vec![u0 as f32; 4 * n],
        uy: vec![0.0; 4 * n],
        theta: Some(field(0.0).iter().map(|&v| v as f32).collect()),
    })
    .unwrap();
    s.step(steps).unwrap();
    let sim: Vec<f64> = s
        .lattice_fields()
        .theta
        .unwrap()
        .iter()
        .map(|&v| v as f64)
        .collect();
    rel_l2(&sim, &field(steps as f64))
}

#[test]
fn advection_diffusion_is_second_order() {
    let bgk = LbmConfig {
        collision: Collision::Bgk,
    };
    for config in [LbmConfig::default(), bgk] {
        let e: Vec<f64> = [16, 32, 64]
            .iter()
            .map(|&n| advected_sine_error(n, config))
            .collect();
        let p = order(e[1], e[2]);
        println!("{config:?}: errors {e:?}, order {p:.2}");
        assert!(e[1] < 0.01, "{config:?}: error at N = 32 is {}", e[1]);
        assert!((1.8..2.3).contains(&p), "{config:?}: order {p}");
    }
}

#[test]
fn adiabatic_walls_conserve_heat() {
    let n = 16u32;
    let mut c = Canvas::new(n, n, CellType::Fluid);
    c.rect(6, 6, 9, 9, CellType::Solid, 1);
    let d = thermal_domain(
        &c,
        Default::default(),
        vec![thermal_wall(1, ThermalBc::Adiabatic)],
        0.1,
        0.1,
        0.0,
        None,
    );
    let mut s = solver(d, LbmConfig::default());
    let cells = (n * n) as usize;
    let theta: Vec<f32> = (0..cells)
        .map(|i| 0.1 + 0.5 * ((i % 16) as f32 * 0.7).sin())
        .collect();
    s.set_equilibrium(&LatticeFields {
        density: vec![1.0; cells],
        ux: vec![0.0; cells],
        uy: vec![0.0; cells],
        theta: Some(theta),
    })
    .unwrap();
    let h0 = s.diagnostics().thermal_energy.unwrap();
    s.step(3000).unwrap();
    let h1 = s.diagnostics().thermal_energy.unwrap();
    // θ is centred on 0 by construction (T_ref = initial temperature), which keeps f32 round-off
    // small; the drift is measured against Σ|θ|.
    let scale: f64 = s
        .lattice_fields()
        .theta
        .unwrap()
        .iter()
        .map(|t| t.abs() as f64)
        .sum();
    assert!(
        ((h1 - h0) / scale).abs() < 1e-5,
        "{h0} → {h1} (Σ|θ| = {scale})"
    );
}

/// de Vahl Davis (1983) cavity at Ra = 1e4, Pr = 0.71, on a coarse 32² grid.
/// The mean Nusselt number is `Nu = 1 + L ⟨u θ⟩ / α` (exact for the steady state, since the
/// x-averaged conduction term integrates to the wall temperature difference).
#[test]
fn heated_cavity_ra1e4_nusselt() {
    let n = 32u32;
    let (ra, pr, u_b): (f64, f64, f64) = (1.0e4, 0.71, 0.05);
    let l = n as f64;
    let g_beta = u_b * u_b / l; // ΔT = 1
    let nu = u_b * l * (pr / ra).sqrt();
    let alpha = nu / pr;
    let mut c = Canvas::new(n + 2, n, CellType::Fluid);
    c.rect(0, 0, 1, n, CellType::Solid, 1);
    c.rect(n + 1, 0, n + 2, n, CellType::Solid, 2);
    let elements = vec![
        thermal_wall(1, ThermalBc::Fixed { value: 1.0 }),
        thermal_wall(2, ThermalBc::Fixed { value: 0.0 }),
    ];
    let d = thermal_domain(
        &c,
        Default::default(),
        elements,
        nu,
        alpha,
        0.5,
        Some(g_beta),
    );
    assert!((d.numbers.rayleigh.unwrap() / ra - 1.0).abs() < 1e-9);
    let mut s = solver(d, LbmConfig::default());
    let f = run_to_steady_theta(&mut s, 1000, 1e-6, 2_000_000);
    let theta = f.theta.unwrap();
    let w = (n + 2) as usize;
    let mut flux = 0.0;
    for y in 0..n as usize {
        for x in 1..=n as usize {
            flux += (f.ux[y * w + x] * theta[y * w + x]) as f64;
        }
    }
    let nusselt = 1.0 + l * (flux / (l * l)) / alpha;
    println!("Nu = {nusselt:.4} after {} steps", s.steps_done());
    assert!((nusselt / 2.243 - 1.0).abs() < 0.03, "Nu = {nusselt}");
    // Hot fluid rises along the hot (left) wall.
    assert!(f.uy[(n as usize / 2) * w + 2] > 0.0);
}
