//! Boundary conditions and forcing against analytical solutions.

mod common;

use cfd_core::examples::Canvas;
use cfd_core::scene::{CellType, InletVelocity, OutletBc};
use cfd_core::solver::Solver;
use cfd_lbm::{Collision, LbmConfig};
use common::*;

fn bgk() -> LbmConfig {
    LbmConfig {
        collision: Collision::Bgk,
    }
}

/// Body-force-driven Poiseuille flow between the implicit walls at the bottom and top edges,
/// periodic in x. Walls sit half-way, at y = 0 and y = H. Returns the relative L2 error.
fn poiseuille_error(h: u32, tau: f64, config: LbmConfig) -> f64 {
    let nu = (tau - 0.5) / 3.0;
    let u_max = 0.02;
    let hf = h as f64;
    let force = 8.0 * nu * u_max / (hf * hf);
    let c = Canvas::new(3, h, CellType::Fluid);
    let mut s = solver(lattice_domain(&c, periodic_x(), vec![], nu), config);
    s.set_body_force([force, 0.0]);
    let f = run_to_steady(&mut s, 500, 1e-6, 2_000_000);
    let sim: Vec<f64> = (0..h as usize).map(|y| f.ux[y * 3 + 1] as f64).collect();
    let exact: Vec<f64> = (0..h as usize)
        .map(|y| {
            let y = y as f64 + 0.5;
            force / (2.0 * nu) * y * (hf - y)
        })
        .collect();
    rel_l2(&sim, &exact)
}

#[test]
fn poiseuille_is_exact_with_trt() {
    // With Λ = 3/16 the half-way wall location is exact for any τ: only round-off remains.
    for tau in [0.6, 1.0, 1.5] {
        let e = poiseuille_error(16, tau, LbmConfig::default());
        assert!(e < 2e-4, "τ = {tau}: error {e}");
    }
}

#[test]
fn poiseuille_with_bgk_is_second_order() {
    // BGK bounce-back has a τ-dependent wall slip that vanishes as O(1/H²).
    let e: Vec<f64> = [8, 16, 32]
        .iter()
        .map(|&h| poiseuille_error(h, 1.2, bgk()))
        .collect();
    let p = order(e[1], e[2]);
    println!("BGK Poiseuille errors {e:?}, order {p:.2}");
    assert!(e[1] < 0.01, "error at H = 16: {}", e[1]);
    assert!((1.8..2.3).contains(&p), "order {p}");
}

#[test]
fn couette_profile_is_linear() {
    let (h, u_wall) = (16u32, 0.05);
    // Fluid rows 0..h, moving wall row at y = h, implicit wall below y = 0.
    let mut c = Canvas::new(3, h + 1, CellType::Fluid);
    c.rect(0, h, 3, h + 1, CellType::Solid, 1);
    let d = lattice_domain(&c, periodic_x(), vec![moving_wall(1, [u_wall, 0.0])], 0.1);
    let mut s = solver(d, LbmConfig::default());
    let f = run_to_steady(&mut s, 500, 1e-6, 1_000_000);
    let sim: Vec<f64> = (0..h as usize).map(|y| f.ux[y * 3] as f64).collect();
    let exact: Vec<f64> = (0..h as usize)
        .map(|y| u_wall * (y as f64 + 0.5) / h as f64)
        .collect();
    let e = rel_l2(&sim, &exact);
    assert!(e < 1e-4, "error {e}");
    assert!(f.uy.iter().all(|v| v.abs() < 1e-7));
}

/// Channel with walls drawn at the bottom/top rows, a parabolic inlet on the left and an outlet
/// on the right. Returns the steady velocity field and the fluid height.
fn channel(outlet_bc: OutletBc) -> (cfd_core::solver::LatticeFields, usize, usize, f64, f64) {
    let (h, l) = (18u32, 60u32);
    let u_max = 0.04;
    let nu = 0.08;
    let mut c = Canvas::new(l, h, CellType::Fluid);
    c.rect(0, 0, l, 1, CellType::Solid, 1);
    c.rect(0, h - 1, l, h, CellType::Solid, 1);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, 2);
    c.rect(l - 1, 1, l, h - 1, CellType::Outlet, 3);
    let elements = vec![
        wall(1),
        inlet(2, InletVelocity::Parabolic { peak: [u_max, 0.0] }),
        outlet(3, outlet_bc),
    ];
    let mut s = solver(
        lattice_domain(&c, Default::default(), elements, nu),
        LbmConfig::default(),
    );
    let f = run_to_steady(&mut s, 1000, 1e-6, 1_000_000);
    (f, l as usize, h as usize, u_max, nu)
}

/// Checks the steady channel flow. Velocity bounce-back imposes the momentum `j = ρ₀u`, and weak
/// compressibility makes `u = j/ρ` vary by `Δρ/ρ` along the channel (~1.7% here), so the mass-flux
/// profiles `j(y)` are compared by **shape** (normalised by their mean); the
/// inlet must impose the mean flux and the flux must be the same at every section.
fn check_channel(outlet_bc: OutletBc, tol_interior: f64, tol_outlet: f64) {
    let (f, l, h, u_max, nu) = channel(outlet_bc.clone());
    let hf = (h - 2) as f64; // walls at y = 1 and y = h − 1
    let profile = |x: usize| -> Vec<f64> {
        (1..h - 1)
            .map(|y| (f.density[y * l + x] * f.ux[y * l + x]) as f64)
            .collect()
    };
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len() as f64;
    let shape = |v: Vec<f64>| -> Vec<f64> {
        let m = mean(&v);
        v.into_iter().map(|u| u / m).collect()
    };
    let exact: Vec<f64> = (1..h - 1)
        .map(|y| {
            let s = y as f64 + 0.5 - 1.0;
            4.0 * u_max * s * (hf - s) / (hf * hf)
        })
        .collect();
    let u_mean = 2.0 * u_max / 3.0;
    let inlet_mean = mean(&profile(1));
    assert!(
        (inlet_mean / u_mean - 1.0).abs() < 2e-3,
        "{outlet_bc:?}: inlet mean flux {inlet_mean} vs {u_mean}"
    );
    for (x, tol) in [
        (l / 4, tol_interior),
        (l / 2, tol_interior),
        (l - 2, tol_outlet),
    ] {
        let e = rel_l2(&shape(profile(x)), &shape(exact.clone()));
        assert!(e < tol, "{outlet_bc:?}: profile shape error {e} at x = {x}");
    }
    // Mass flux is the same at every section.
    let flux = |x: usize| -> f64 {
        (1..h - 1)
            .map(|y| (f.density[y * l + x] * f.ux[y * l + x]) as f64)
            .sum()
    };
    let (q_in, q_out) = (flux(1), flux(l - 2));
    assert!(
        ((q_out - q_in) / q_in).abs() < 1e-3,
        "flux {q_in} → {q_out}"
    );
    // Pressure gradient of Poiseuille flow: dp/dx = −8 ν ρ u_max / H².
    let p = |x: usize| f.density[(h / 2) * l + x] as f64 / 3.0;
    let (x0, x1) = (l / 4, 3 * l / 4);
    let grad = (p(x1) - p(x0)) / (x1 - x0) as f64;
    let exact_grad = -8.0 * nu * u_max / (hf * hf);
    assert!(
        (grad / exact_grad - 1.0).abs() < 0.03,
        "{outlet_bc:?}: dp/dx {grad} vs {exact_grad}"
    );
}

#[test]
fn channel_with_pressure_outlet() {
    check_channel(OutletBc::Pressure { value: 0.0 }, 2e-3, 0.03);
}

/// Uniform flow through a velocity inlet and a zero-gradient outlet, periodic across: the
/// outlet must let the flow out undisturbed. (With walls, a zero-gradient outlet cannot carry the
/// Poiseuille pressure gradient and needs a pressure reference: see `domain.noPressureReference`.)
#[test]
fn zero_gradient_outlet_passes_uniform_flow() {
    let (h, l, u_in) = (8u32, 40u32, 0.03);
    let mut c = Canvas::new(l, h, CellType::Fluid);
    c.rect(0, 0, 1, h, CellType::Inlet, 1);
    c.rect(l - 1, 0, l, h, CellType::Outlet, 2);
    let edges = cfd_core::scene::DomainEdges {
        bottom: cfd_core::scene::EdgeKind::Periodic,
        top: cfd_core::scene::EdgeKind::Periodic,
        ..Default::default()
    };
    let elements = vec![
        inlet(1, InletVelocity::Uniform { value: [u_in, 0.0] }),
        outlet(2, OutletBc::ZeroGradient),
    ];
    let mut s = solver(
        lattice_domain(&c, edges, elements, 0.05),
        LbmConfig::default(),
    );
    let f = run_to_steady(&mut s, 1000, 1e-6, 1_000_000);
    for x in 1..l as usize - 1 {
        for y in 0..h as usize {
            let i = y * l as usize + x;
            let j = (f.density[i] * f.ux[i]) as f64;
            assert!((j / u_in - 1.0).abs() < 1e-4, "j = {j} at ({x}, {y})");
            assert!(f.uy[i].abs() < 1e-7);
        }
    }
}

#[test]
fn lid_driven_cavity_is_stable_and_conserves_mass() {
    let n = 32u32;
    let u_lid = 0.1;
    let nu = u_lid * n as f64 / 100.0; // Re = 100
                                       // Fluid n × n, lid row drawn on top, implicit walls elsewhere.
    let mut c = Canvas::new(n, n + 1, CellType::Fluid);
    c.rect(0, n, n, n + 1, CellType::Solid, 1);
    let d = lattice_domain(
        &c,
        Default::default(),
        vec![moving_wall(1, [u_lid, 0.0])],
        nu,
    );
    let mut s = solver(d, LbmConfig::default());
    let m0 = s.diagnostics().mass;
    s.step(20_000).unwrap();
    let diag = s.diagnostics();
    assert!(diag.finite);
    assert!(
        ((diag.mass - m0) / m0).abs() < 1e-5,
        "mass {m0} → {}",
        diag.mass
    );
    // Primary vortex: forward flow under the lid, return flow near the bottom.
    let f = s.lattice_fields();
    let mid = (n / 2) as usize;
    let at = |y: usize| f.ux[y * n as usize + mid];
    assert!(at(n as usize - 2) > 0.0 && at(4) < 0.0);
    assert!(diag.max_velocity < u_lid * 1.01);
}
