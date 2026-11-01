//! Free surface (ADR-032) on the CPU reference: the invariants of the four passes, exact mass
//! accounting, water at rest, a free-falling block and a collapsing column.

mod common;

use cfd_core::domain::{phase, Domain, LINKS};
use cfd_core::examples::Canvas;
use cfd_core::scene::{CellType, DomainEdges, Turbulence};
use cfd_core::solver::Solver;
use cfd_lbm::shared::initial_fields;
use cfd_lbm::surface::state;
use cfd_lbm::{CpuLbm, LbmConfig};
use common::{lattice_scene, periodic_x, LATTICE_UNITS};

/// A free-surface domain in lattice units (dx = dt = 1, ρ₀ = 1), gravity `g` downwards, with
/// liquid where `liquid(x, y)` holds and gas elsewhere; the domain edges are walls.
fn tank(
    w: u32,
    h: u32,
    edges: DomainEdges,
    nu: f64,
    g: f64,
    liquid: impl Fn(u32, u32) -> bool,
) -> Domain {
    let mut c = Canvas::new(w, h, CellType::Empty);
    for y in 0..h {
        for x in 0..w {
            if liquid(x, y) {
                c.set(x, y, CellType::Fluid, 0);
            }
        }
    }
    let mut scene = lattice_scene(&c, edges, vec![], nu);
    scene.physics.free_surface = true;
    scene.physics.gravity = [0.0, -g];
    let d =
        Domain::from_scene(&scene, &LATTICE_UNITS).unwrap_or_else(|r| panic!("{:#?}", r.issues));
    assert!((d.units.dt - 1.0).abs() < 1e-12);
    assert!((d.physics.body_force[1] + g).abs() < 1e-15);
    d
}

/// Liquid mass `Σ (m + m_ex)` in f64.
fn liquid_mass(s: &CpuLbm) -> f64 {
    let p = s.surface().expect("free surface");
    p.mass
        .iter()
        .zip(&p.massex)
        .map(|(&m, &e)| m as f64 + e as f64)
        .sum()
}

/// Checks the invariants of the end of a step (design proposal §4): no liquid cell next to gas,
/// interface masses in [0, ρ], no pending marks, φ consistent with the phase.
fn check_invariants(s: &mut CpuLbm) {
    let d = s.domain().clone();
    let (w, h) = (d.width as i64, d.height as i64);
    let lf = s.lattice_fields();
    let p = s.surface().expect("free surface");
    for k in 0..d.cell_count() {
        let st = p.state[k];
        let ph = state::phase(st);
        assert_eq!(state::mark(st), state::MARK_NONE, "cell {k}: mark left");
        match ph {
            phase::LIQUID => {
                assert_eq!(p.phi[k], 1.0);
                let (x, y) = (k as i64 % w, k as i64 / w);
                for c in &LINKS[1..] {
                    let (nx, ny) = (x + c[0] as i64, y + c[1] as i64);
                    let wrap_x = if d.periodic[0] { nx.rem_euclid(w) } else { nx };
                    let wrap_y = if d.periodic[1] { ny.rem_euclid(h) } else { ny };
                    if (0..w).contains(&wrap_x) && (0..h).contains(&wrap_y) {
                        let j = (wrap_y * w + wrap_x) as usize;
                        assert_ne!(
                            state::phase(p.state[j]),
                            phase::GAS,
                            "liquid cell ({x}, {y}) next to gas"
                        );
                    }
                }
            }
            phase::INTERFACE => {
                let r = lf.density[k];
                // Kept excess (no receivers) may leave m outside [0, ρ]; otherwise it is clamped.
                if state::receivers(st) > 0 {
                    assert!(
                        p.mass[k] >= 0.0 && p.mass[k] <= r,
                        "cell {k}: m = {}",
                        p.mass[k]
                    );
                }
                assert!(
                    (0.0..=1.0).contains(&p.phi[k]),
                    "cell {k}: φ = {}",
                    p.phi[k]
                );
            }
            phase::GAS => assert_eq!(p.phi[k], 0.0),
            _ => {}
        }
    }
}

fn count(s: &CpuLbm, ph: u8) -> usize {
    let p = s.surface().expect("free surface");
    p.state.iter().filter(|&&st| state::phase(st) == ph).count()
}

#[test]
fn initial_state_is_hydrostatic_with_full_interface_cells() {
    let g = 1e-4;
    let d = tank(12, 20, DomainEdges::default(), 0.05, g, |_, y| y < 10);
    let f = initial_fields(&d);
    let s = CpuLbm::new(d, LbmConfig::default());
    // The top liquid row (y = 9) is the interface, full; ρ grows by 3g per row downwards.
    for x in 0..12 {
        assert_eq!(f.phase.as_ref().unwrap()[9 * 12 + x], phase::INTERFACE);
        assert_eq!(f.phase.as_ref().unwrap()[8 * 12 + x], phase::LIQUID);
        let rho = |y: usize| f.density[y * 12 + x] as f64;
        assert!((rho(9) - (1.0 + 1.5 * g)).abs() < 1e-7);
        assert!((rho(0) - (1.0 + 3.0 * g * 9.5)).abs() < 1e-7);
        assert_eq!(rho(15), 1.0);
    }
    let p = s.surface().unwrap();
    assert_eq!(p.phi[9 * 12 + 3], 1.0);
    assert_eq!(p.mass[9 * 12 + 3], f.density[9 * 12 + 3]);
    assert_eq!(p.mass[15 * 12 + 3], 0.0);
}

#[test]
fn mass_is_conserved_through_conversions() {
    // A column collapsing in a periodic channel: many conversions per step.
    let d = tank(48, 24, periodic_x(), 0.02, 2e-4, |x, y| x < 12 && y < 16);
    let mut s = CpuLbm::new(d, LbmConfig::default());
    let m0 = liquid_mass(&s);
    for _ in 0..30 {
        s.step(20).expect("finite");
        check_invariants(&mut s);
    }
    let drift = (liquid_mass(&s) - m0) / m0;
    assert!(drift.abs() < 1e-5, "mass drift {drift:e}");
    // The column has spread: the interface moved and conversions happened both ways.
    let wet = count(&s, phase::LIQUID) + count(&s, phase::INTERFACE);
    assert!(wet > 12 * 16, "{wet}");
    let fill = s.lattice_fields().fill.unwrap();
    assert!(fill[2 * 48 + 20] > 0.5, "the front has not reached x = 20");
    let diag = s.diagnostics();
    assert_eq!(diag.mass, liquid_mass(&s));
    assert!(diag.liquid.unwrap().interface_cells > 0);
}

#[test]
fn skip_macro_gives_the_same_free_surface_run() {
    let run = |skip_macro| {
        let d = tank(24, 16, DomainEdges::default(), 0.02, 2e-4, |x, y| {
            x < 8 && y < 12
        });
        let mut s = CpuLbm::new(
            d,
            LbmConfig {
                skip_macro,
                ..LbmConfig::default()
            },
        );
        s.step(150).expect("finite");
        (s.lattice_fields(), s.surface().unwrap().clone())
    };
    assert_eq!(run(true), run(false));
}

#[test]
fn still_water_stays_at_rest() {
    // A closed tank half full: the hydrostatic start is (nearly) the discrete equilibrium.
    let (h_liquid, g) = (20u32, 1e-4);
    let d = tank(24, 40, DomainEdges::default(), 0.05, g, |_, y| y < h_liquid);
    let mut s = CpuLbm::new(d, LbmConfig::default());
    let m0 = liquid_mass(&s);
    let wet0 = count(&s, phase::LIQUID) + count(&s, phase::INTERFACE);
    s.step(4000).expect("finite");
    check_invariants(&mut s);
    let lf = s.lattice_fields();
    let umax = lf
        .ux
        .iter()
        .zip(&lf.uy)
        .map(|(a, b)| a.hypot(*b) as f64)
        .fold(0.0, f64::max);
    let u_ref = (g * h_liquid as f64).sqrt();
    assert!(
        umax < 1e-3 * u_ref,
        "spurious currents {umax:e} (√(gh) = {u_ref:e})"
    );
    let drift = (liquid_mass(&s) - m0) / m0;
    assert!(drift.abs() < 1e-6, "mass drift {drift:e}");
    // The surface stays put: same wet cells, top row partially filled at most.
    let wet = count(&s, phase::LIQUID) + count(&s, phase::INTERFACE);
    assert!(wet.abs_diff(wet0) <= 24, "{wet} vs {wet0}");
    // Bottom pressure: ρ₀ g h (lattice: (ρ − 1)/3 = g h with h to the cell centre).
    let p_bottom = (lf.density[12] as f64 - 1.0) / 3.0;
    let expected = g * (h_liquid as f64 - 0.5);
    assert!(
        (p_bottom / expected - 1.0).abs() < 0.01,
        "{p_bottom:e} vs {expected:e}"
    );
}

/// Releases liquid from rest with a uniform density (free fall has no pressure gradient) and
/// returns the fall of its mass centroid after `steps`, relative to ½ g t²; checks the mass
/// and the uniform velocity −g t of the liquid (not interface) cells.
fn free_fall(d: Domain, g: f64, steps: u32) -> f64 {
    let w = d.width as usize;
    let mut f = initial_fields(&d);
    f.density.iter_mut().for_each(|r| *r = 1.0);
    let mut s = CpuLbm::new(d, LbmConfig::default());
    s.set_equilibrium(&f).unwrap();
    let centroid = |s: &CpuLbm| {
        let p = s.surface().unwrap();
        let (mut my, mut m) = (0.0, 0.0);
        for (k, (&mk, &ek)) in p.mass.iter().zip(&p.massex).enumerate() {
            let mk = mk as f64 + ek as f64;
            my += mk * ((k / w) as f64 + 0.5);
            m += mk;
        }
        my / m
    };
    let (c0, m0) = (centroid(&s), liquid_mass(&s));
    s.step(steps).expect("finite");
    check_invariants(&mut s);
    let t = steps as f64;
    let drift = (liquid_mass(&s) - m0) / m0;
    assert!(drift.abs() < 1e-6, "mass drift {drift:e}");
    let lf = s.lattice_fields();
    let p = s.surface().unwrap();
    let liquid: Vec<usize> = (0..lf.uy.len())
        .filter(|&k| state::phase(p.state[k]) == phase::LIQUID)
        .collect();
    assert!(!liquid.is_empty());
    for &k in &liquid {
        let (vx, vy) = (lf.ux[k] as f64, lf.uy[k] as f64);
        let spread = vx.hypot(vy + g * t);
        assert!(spread < 0.01 * g * t, "cell {k}: u = ({vx:e}, {vy:e})");
    }
    (c0 - centroid(&s)) / (0.5 * g * t * t)
}

#[test]
fn a_slab_falls_freely() {
    // A 16-cell slab across a periodic channel: only a leading and a trailing face.
    let g = 2e-4;
    let d = tank(40, 64, periodic_x(), 0.05, g, |_, y| (40..56).contains(&y));
    let ratio = free_fall(d, g, 300);
    assert!((ratio - 1.0).abs() < 0.01, "fell {ratio} × ½ g t²");
}

#[test]
fn a_block_falls_freely() {
    // A 16-cell square: its side faces slide along the gas, and interface cells there carry
    // mass at the weight ½(φ + φ_j) < 1, so the centroid trails ½ g t² by ≈ 2 % (measured
    // 2026-11-01: 0.978 at t = 300, 0.980 at t = 600, while the slab above is within 0.6 % and
    // the liquid's momentum within 0.3 %). The validation case (#45, 1 %) decides whether the
    // normal-based reconstruction of ADR-032's fallback is needed.
    let (g, size) = (2e-4, 16u32);
    let (x0, y0) = (12u32, 40u32);
    let d = tank(40, 64, DomainEdges::default(), 0.05, g, |x, y| {
        (x0..x0 + size).contains(&x) && (y0..y0 + size).contains(&y)
    });
    let ratio = free_fall(d, g, 300);
    assert!((ratio - 1.0).abs() < 0.03, "fell {ratio} × ½ g t²");
}

#[test]
fn smagorinsky_keeps_a_low_viscosity_column_stable() {
    // τ₀ = 0.5003, accepted only with the model (a warning); C_s = 0.1.
    let mut c = Canvas::new(64, 32, CellType::Empty);
    c.rect(0, 0, 16, 24, CellType::Fluid, 0);
    let mut scene = lattice_scene(&c, DomainEdges::default(), vec![], 1e-4);
    scene.physics.free_surface = true;
    scene.physics.gravity = [0.0, -2e-4];
    scene.physics.turbulence = Some(Turbulence::Smagorinsky { constant: 0.1 });
    let d = Domain::from_scene(&scene, &LATTICE_UNITS).expect("a warning only");
    assert_eq!(d.physics.smagorinsky, Some(0.1));
    let mut s = CpuLbm::new(d, LbmConfig::default());
    let m0 = liquid_mass(&s);
    s.step(1500).expect("finite");
    check_invariants(&mut s);
    let drift = (liquid_mass(&s) - m0) / m0;
    assert!(drift.abs() < 1e-5, "mass drift {drift:e}");
}

/// Front of the dam break (design proposal §10.3): in the first fluid row, from the left wall
/// while φ ≥ ½, interpolated to φ = ½; in cells from the wall face.
fn dam_break_front(fill: &[f32], w: usize) -> f64 {
    let row = &fill[w..2 * w];
    let mut x = 1;
    while x + 1 < w - 1 && row[x + 1] >= 0.5 {
        x += 1;
    }
    let (a, b) = (row[x] as f64, row[x + 1] as f64);
    // Cell x's centre is x − ½ from the wall face (cell 1 starts at the face).
    (x as f64 - 0.5) + (a - 0.5) / (a - b)
}

/// Measures the front Z(T) of the dam-break example with the LES model (ADR-032 option A) and
/// with an effective viscosity of τ = 0.51 (option B), for the comparison of #43/#45. Prints a
/// table; `cargo test -p cfd-lbm --release --test free_surface dam_break_front_les -- --ignored
/// --nocapture`.
#[test]
#[ignore]
fn dam_break_front_les_and_effective_viscosity() {
    use cfd_core::domain::DomainOptions;
    let a_cells = 40.0;
    let mut runs = Vec::new();
    for (label, les) in [("A: LES C_s = 0.1", true), ("B: τ = 0.51", false)] {
        let mut scene = cfd_core::examples::dam_break();
        if !les {
            scene.physics.turbulence = None;
            // ν_eff for τ = 0.51 at the example's dx and dt: ν_lb = 0.01/3.
            let d = Domain::from_scene(&cfd_core::examples::dam_break(), &DomainOptions::default())
                .unwrap();
            scene.fluids[0].kinematic_viscosity = 0.01 / 3.0 * d.units.dx.powi(2) / d.units.dt;
        }
        let d = Domain::from_scene(&scene, &DomainOptions::default()).unwrap();
        let (w, dt) = (d.width as usize, d.units.dt);
        let t_scale = (2.0 * cfd_core::units::STANDARD_GRAVITY / 0.1f64).sqrt();
        let mut s = CpuLbm::new(d, LbmConfig::default());
        let mut rows = Vec::new();
        for target in [0.5, 1.0, 1.5, 2.0, 2.5, 3.0] {
            let steps = (target / t_scale / dt).round() as u64;
            s.step((steps - s.steps_done()) as u32).expect("finite");
            let fill = s.lattice_fields().fill.unwrap();
            let t = s.steps_done() as f64 * dt * t_scale;
            rows.push((t, dam_break_front(&fill, w) / a_cells));
        }
        runs.push((label, rows));
    }
    for (label, rows) in &runs {
        println!("{label}");
        for (t, z) in rows {
            println!("  T = {t:.3}  Z = {z:.3}");
        }
    }
}
