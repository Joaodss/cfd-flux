//! Momentum-exchange forces and probes.

mod common;

use cfd_core::examples::Canvas;
use cfd_core::scene::CellType;
use cfd_core::solver::{ElementForce, Solver};
use cfd_lbm::LbmConfig;
use common::*;

fn force_on(forces: &[ElementForce], element: u16) -> [f64; 2] {
    forces
        .iter()
        .find(|f| f.element == element)
        .unwrap_or_else(|| panic!("no force on element {element}: {forces:?}"))
        .lattice
}

#[test]
fn wall_friction_balances_the_body_force() {
    let (w, h) = (4u32, 16u32);
    let mut c = Canvas::new(w, h + 2, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h + 1, w, h + 2, CellType::Solid, 2);
    let d = lattice_domain(&c, periodic_x(), vec![wall(1), wall(2)], 0.1);
    let mut s = solver(d, LbmConfig::default());
    let g = 1e-6;
    s.set_body_force([g, 0.0]);
    run_to_steady(&mut s, 500, 1e-6, 1_000_000);
    let diag = s.diagnostics();
    let forces = s.forces();
    let (bottom, top) = (force_on(&forces, 1), force_on(&forces, 2));
    // Total body force on the fluid is transmitted to the walls, half each.
    let total = g * diag.mass;
    assert!(
        ((bottom[0] + top[0]) / total - 1.0).abs() < 1e-3,
        "{bottom:?} {top:?} vs {total}"
    );
    assert!((bottom[0] / top[0] - 1.0).abs() < 1e-3);
    // Forces are gauge: the uniform reference pressure ρ₀ c_s² (= w/3 per wall) is not
    // included, and the density stays uniform, so there is no normal force.
    let p_ref = w as f64 / 3.0;
    assert!(bottom[1].abs() < 1e-5 * p_ref, "{bottom:?}");
    assert!(top[1].abs() < 1e-5 * p_ref, "{top:?}");
}

#[test]
fn couette_shear_stress_on_both_walls() {
    let (w, h, u_wall, nu) = (4u32, 16u32, 0.05, 0.1);
    let mut c = Canvas::new(w, h + 2, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h + 1, w, h + 2, CellType::Solid, 2);
    let elements = vec![wall(1), moving_wall(2, [u_wall, 0.0])];
    let d = lattice_domain(&c, periodic_x(), elements, nu);
    let mut s = solver(d, LbmConfig::default());
    run_to_steady(&mut s, 500, 1e-6, 1_000_000);
    let forces = s.forces();
    // τ = ρ ν U / H on each wall, over the wall length w: drags the bottom wall forward and
    // holds the moving wall back.
    let shear = nu * u_wall / h as f64 * w as f64;
    let (bottom, top) = (force_on(&forces, 1), force_on(&forces, 2));
    assert!(
        (bottom[0] / shear - 1.0).abs() < 2e-3,
        "{bottom:?} vs {shear}"
    );
    assert!((top[0] / -shear - 1.0).abs() < 2e-3, "{top:?} vs {shear}");
}

#[test]
fn probes_read_the_local_state() {
    let (w, h) = (4u32, 16u32);
    let c = Canvas::new(w, h, CellType::Fluid);
    let mut scene = lattice_scene(&c, periodic_x(), vec![], 0.1);
    scene.probes = vec![cfd_core::scene::Probe {
        name: "middle".into(),
        position: [1, h / 2],
    }];
    let d = cfd_core::Domain::from_scene(&scene, &LATTICE_UNITS).unwrap();
    let mut s = solver(d, LbmConfig::default());
    s.set_body_force([1e-6, 0.0]);
    let lf = run_to_steady(&mut s, 500, 1e-6, 1_000_000);
    let probes = s.probes();
    assert_eq!(probes.len(), 1);
    let i = (h / 2 * w + 1) as usize;
    // Lattice and physical units coincide in these scenes (velocities in m/s).
    assert!((probes[0].velocity[0] - lf.ux[i] as f64).abs() < 1e-12);
    assert!(probes[0].velocity[0] > 0.0);
}
