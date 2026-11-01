//! Backend-independent pieces every LBM backend uses unchanged: the initial state, equilibrium
//! populations in the SoA layout, and the momentum-exchange bookkeeping. Keeping them here (and
//! not re-implemented per backend) means a parity difference can only come from a kernel.

use std::collections::BTreeMap;

use cfd_core::domain::{flags, phase, Domain, LINKS, OPPOSITE};
use cfd_core::solver::{Diagnostics, ElementForce, LatticeFields};

use crate::config::{flow_rates, thermal_rates, Collision, LbmConfig};
use crate::kernel::{equilibrium, thermal_equilibrium, Les};
use crate::lattice::W9;

/// The domain's initial state on fluid cells; `ρ = 1`, `u = 0` elsewhere. With a free surface,
/// gas cells hold `ρ = 1`, `u = 0` too, and the liquid starts hydrostatic (ADR-032):
/// `ρ = ρ_init + 3 |g| d`, with `d` the depth of the cell centre below the first non-liquid cell
/// above it along the axis closest to g (line by line, so overhangs are handled).
pub fn initial_fields(domain: &Domain) -> LatticeFields {
    let mut f = LatticeFields::initial(domain);
    for (i, &fl) in domain.flags.iter().enumerate() {
        let gas = f.phase.as_ref().is_some_and(|p| !phase::wet(p[i]));
        if flags::flow(fl) != flags::FLUID || gas {
            (f.density[i], f.ux[i], f.uy[i]) = (1.0, 0.0, 0.0);
        }
    }
    if let Some(fs) = &domain.free_surface {
        let (w, h) = (domain.width as usize, domain.height as usize);
        let g = domain.physics.body_force;
        let along_y = g[1].abs() >= g[0].abs();
        let ga = if along_y { g[1] } else { g[0] };
        let (lines, len) = if along_y { (w, h) } else { (h, w) };
        for line in 0..lines {
            let mut depth = 0usize;
            for s in 0..len {
                // The s-th cell from the top (against g).
                let t = if ga < 0.0 { len - 1 - s } else { s };
                let k = if along_y { t * w + line } else { line * w + t };
                if phase::wet(fs.phase[k]) {
                    depth += 1;
                    let d = depth as f64 - 0.5;
                    f.density[k] = (domain.initial.density + 3.0 * ga.abs() * d) as f32;
                } else {
                    depth = 0;
                }
            }
        }
    }
    f
}

/// The free-surface planes of a free-surface run (ADR-032), `n` values each. `state` is the
/// input and output of a step (`state_a`), `state_b` the intermediate one.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfacePlanes {
    pub state: Vec<u32>,
    pub state_b: Vec<u32>,
    /// Mass m (lattice units): ρ on liquid cells, φρ on interface cells, 0 on gas cells.
    pub mass: Vec<f32>,
    /// Undivided excess mass, for the cell's receivers on the next step.
    pub massex: Vec<f32>,
    /// Fill fraction φ: 1 liquid, `m/ρ` interface, 0 gas and solid.
    pub phi: Vec<f32>,
}

/// The free-surface planes for `fields` (their `phase` and `fill`, else the domain's initial
/// phase with full interface cells), or `None` without a free surface.
pub fn surface_planes(domain: &Domain, fields: &LatticeFields) -> Option<SurfacePlanes> {
    let fs = domain.free_surface.as_ref()?;
    let phases = fields.phase.as_deref().unwrap_or(&fs.phase);
    let fill = |k: usize| match phases[k] {
        phase::LIQUID => 1.0,
        phase::INTERFACE => fields.fill.as_ref().map_or(1.0, |f| f[k]),
        _ => 0.0,
    };
    let n = phases.len();
    let state: Vec<u32> = phases.iter().map(|&p| p as u32).collect();
    let phi: Vec<f32> = (0..n).map(fill).collect();
    let mass = (0..n).map(|k| phi[k] * fields.density[k]).collect();
    Some(SurfacePlanes {
        state_b: state.clone(),
        state,
        mass,
        massex: vec![0.0; n],
        phi,
    })
}

/// The Smagorinsky model's constants, when the scene has it.
pub fn les_params(d: &Domain, config: LbmConfig) -> Option<Les> {
    let cs = d.physics.smagorinsky?;
    Some(Les {
        tau0: d.physics.tau() as f32,
        factor: (18.0 * 2f64.sqrt() * cs * cs) as f32,
        magic: match config.collision {
            Collision::Trt { magic } => Some(magic as f32),
            Collision::Bgk => None,
        },
    })
}

/// Equilibrium populations of `fields` in the SoA layout `[i * n + cell]`: shifted D2Q9 `f̃`
/// and, when `thermal`, D2Q5 `g` (θ = 0 where `fields.theta` is `None`). `g` is empty otherwise.
pub fn equilibrium_populations(fields: &LatticeFields, thermal: bool) -> (Vec<f32>, Vec<f32>) {
    let n = fields.density.len();
    let mut f = vec![0.0; 9 * n];
    let mut g = vec![0.0; if thermal { 5 * n } else { 0 }];
    for c in 0..n {
        let (rho, u) = (fields.density[c], [fields.ux[c], fields.uy[c]]);
        for (i, v) in equilibrium(rho, u).into_iter().enumerate() {
            f[i * n + c] = v;
        }
        if thermal {
            let theta = fields.theta.as_ref().map_or(0.0, |t| t[c]);
            for (i, v) in thermal_equilibrium(theta, u).into_iter().enumerate() {
                g[i * n + c] = v;
            }
        }
    }
    (f, g)
}

/// A link through which population `dir` of the fluid cell `cell` is pulled across a bounce-back
/// boundary (wall, moving wall or velocity inlet) with parameter slot `slot`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallLink {
    pub cell: u32,
    pub dir: u8,
    pub slot: u32,
}

/// Every bounce-back link, by increasing cell and then direction (the summation order of
/// [`wall_forces`], so every backend adds the same numbers in the same order).
pub fn wall_links(d: &Domain) -> Vec<WallLink> {
    let (w, h) = (d.width as i64, d.height as i64);
    let mut links = Vec::new();
    for (k, &fl) in d.flags.iter().enumerate() {
        if flags::flow(fl) != flags::FLUID || fl & flags::LINK_MASK == 0 {
            continue;
        }
        let (x, y) = (k as i64 % w, k as i64 / w);
        for (i, &[cx, cy]) in LINKS.iter().enumerate().skip(1) {
            if !flags::is_boundary_link(fl, i) {
                continue;
            }
            let (sx, sy) = (x - cx as i64, y - cy as i64);
            let inside_x = (0..w).contains(&sx) || d.periodic[0];
            let inside_y = (0..h).contains(&sy) || d.periodic[1];
            let (flow, slot) = if inside_x && inside_y {
                let src = (sy.rem_euclid(h) * w + sx.rem_euclid(w)) as usize;
                (flags::flow(d.flags[src]), d.bc_slot[src])
            } else {
                (flags::BOUNCE_BACK, 0)
            };
            if flow == flags::BOUNCE_BACK {
                links.push(WallLink {
                    cell: k as u32,
                    dir: i as u8,
                    slot,
                });
            }
        }
    }
    links
}

/// Momentum-exchange forces per element from the post-collision population `f̃*_ī` that each link
/// sends towards the wall (`outgoing[l]` for `links[l]`).
///
/// The population returns as `f_i = f*_ī + 6 wᵢ cᵢ·u_w`, so the wall receives
/// `c_ī (2 f*_ī + 6 wᵢ cᵢ·u_w)` per step. Using the shifted population `f̃*_ī = f*_ī − w_ī`
/// removes the reference pressure ρ₀ c_s² (which cancels on closed bodies), so open elements such
/// as inlets and single walls report gauge forces.
pub fn wall_forces(d: &Domain, links: &[WallLink], outgoing: &[f32]) -> Vec<ElementForce> {
    let mut acc: BTreeMap<u16, [f64; 2]> = BTreeMap::new();
    for (l, &out) in links.iter().zip(outgoing) {
        let i = l.dir as usize;
        let [cx, cy] = LINKS[i];
        let uw = d.bc_params[l.slot as usize].velocity;
        let cu = (cx as f32 * uw[0] + cy as f32 * uw[1]) as f64;
        let m = 2.0 * out as f64 + 6.0 * W9[i] as f64 * cu;
        let e = acc.entry(d.slot_element[l.slot as usize]).or_default();
        e[0] -= cx as f64 * m;
        e[1] -= cy as f64 * m;
    }
    acc.into_iter()
        .map(|(element, lattice)| ElementForce { element, lattice })
        .collect()
}

/// Index of the population each link sends towards the wall, in the SoA layout.
pub fn outgoing_index(l: &WallLink, n: usize) -> usize {
    OPPOSITE[l.dir as usize] * n + l.cell as usize
}

/// Relaxation rates `(ω⁺, ω⁻)` of the flow and thermal lattices, as stored by the kernels
/// (the thermal pair is `[1, 1]` without the thermal lattice).
pub fn relaxation_rates(d: &Domain, config: LbmConfig) -> ([f32; 2], [f32; 2]) {
    let omega = flow_rates(config.collision, d.physics.tau()).map(|v| v as f32);
    let omega_g = d
        .physics
        .tau_thermal()
        .map_or([1.0; 2], |tau| thermal_rates(config.collision, tau))
        .map(|v| v as f32);
    (omega, omega_g)
}

/// Values per workgroup/block in the GPU diagnostics partials: fluid cells, Σ(ρ − 1), ½Σρ|u|²,
/// max |u|, Σθ, non-finite flag, 2 × padding.
pub const DIAGNOSTIC_PARTIALS: usize = 8;

/// Finishes the per-workgroup partials of the GPU diagnostics reductions in f64. `groups` is the
/// number of records expected; a short read (device lost) reports non-finite diagnostics.
pub fn diagnostics_from_partials(
    step: u64,
    partials: &[f32],
    groups: usize,
    thermal: bool,
) -> Diagnostics {
    let (mut cells, mut dmass, mut energy, mut umax, mut heat) = (0.0, 0.0, 0.0, 0.0f64, 0.0);
    let mut finite = partials.len() >= groups * DIAGNOSTIC_PARTIALS;
    for w in partials.chunks_exact(DIAGNOSTIC_PARTIALS).take(groups) {
        cells += w[0] as f64;
        dmass += w[1] as f64;
        energy += w[2] as f64;
        umax = umax.max(w[3] as f64);
        heat += w[4] as f64;
        finite &= w[5] == 0.0;
    }
    Diagnostics {
        step,
        fluid_cells: cells as usize,
        mass: cells + dmass,
        kinetic_energy: energy,
        max_velocity: umax,
        thermal_energy: thermal.then_some(heat),
        finite: finite && umax.is_finite() && energy.is_finite(),
        liquid: None,
    }
}

/// Completes read-back fields of a free-surface run: the phase from the state plane, the fill
/// fraction, and gas cells reported in the gas state (`ρ = 1`, `u = 0`; their planes hold whatever
/// their last update stored). Every backend's `lattice_fields` ends here.
pub fn surface_fields(lf: &mut LatticeFields, state: &[u32], phi: Vec<f32>) {
    let phases: Vec<u8> = state
        .iter()
        .map(|&s| crate::surface::state::phase(s))
        .collect();
    for (k, &p) in phases.iter().enumerate() {
        if !phase::wet(p) {
            (lf.density[k], lf.ux[k], lf.uy[k]) = (1.0, 0.0, 0.0);
        }
    }
    lf.fill = Some(phi);
    lf.phase = Some(phases);
}

/// The bounce-back links of liquid and interface cells, with their outgoing populations (gas
/// cells hold stale populations and exert no force).
pub fn wet_links(links: &[WallLink], outgoing: &[f32], state: &[u32]) -> (Vec<WallLink>, Vec<f32>) {
    links
        .iter()
        .zip(outgoing)
        .filter(|(l, _)| phase::wet(crate::surface::state::phase(state[l.cell as usize])))
        .map(|(l, &o)| (*l, o))
        .unzip()
}

/// Finishes the GPU partials of a free-surface run (`diagnostics_fs`): wet cells, Σ(m + m_ex − φ),
/// ½Σρ|u|², max |u|, Σφ, non-finite flag, interface cells, padding. The liquid mass is
/// Σ(m + m_ex − φ) + Σφ.
pub fn diagnostics_from_surface_partials(
    step: u64,
    partials: &[f32],
    groups: usize,
) -> Diagnostics {
    let (mut cells, mut excess, mut energy, mut umax, mut volume, mut interface) =
        (0.0, 0.0, 0.0, 0.0f64, 0.0, 0.0);
    let mut finite = partials.len() >= groups * DIAGNOSTIC_PARTIALS;
    for w in partials.chunks_exact(DIAGNOSTIC_PARTIALS).take(groups) {
        cells += w[0] as f64;
        excess += w[1] as f64;
        energy += w[2] as f64;
        umax = umax.max(w[3] as f64);
        volume += w[4] as f64;
        finite &= w[5] == 0.0;
        interface += w[6] as f64;
    }
    let mass = excess + volume;
    Diagnostics {
        step,
        fluid_cells: cells as usize,
        mass,
        kinetic_energy: energy,
        max_velocity: umax,
        thermal_energy: None,
        finite: finite && umax.is_finite() && energy.is_finite() && mass.is_finite(),
        liquid: Some(cfd_core::solver::LiquidDiagnostics {
            volume,
            interface_cells: interface as usize,
        }),
    }
}
