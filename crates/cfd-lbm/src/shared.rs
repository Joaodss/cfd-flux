//! Backend-independent pieces every LBM backend uses unchanged: the initial state, equilibrium
//! populations in the SoA layout, and the momentum-exchange bookkeeping. Keeping them here (and
//! not re-implemented per backend) means a parity difference can only come from a kernel.

use std::collections::BTreeMap;

use cfd_core::domain::{flags, Domain, LINKS, OPPOSITE};
use cfd_core::solver::{Diagnostics, ElementForce, LatticeFields};

use crate::config::{flow_rates, thermal_rates, LbmConfig};
use crate::kernel::{equilibrium, thermal_equilibrium};
use crate::lattice::W9;

/// The domain's initial state on fluid cells; `ρ = 1`, `u = 0` elsewhere.
pub fn initial_fields(domain: &Domain) -> LatticeFields {
    let mut f = LatticeFields::initial(domain);
    for (i, &fl) in domain.flags.iter().enumerate() {
        if flags::flow(fl) != flags::FLUID {
            (f.density[i], f.ux[i], f.uy[i]) = (1.0, 0.0, 0.0);
        }
    }
    f
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
    }
}
