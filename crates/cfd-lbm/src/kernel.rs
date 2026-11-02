//! The fused stream-collide kernel (pull scheme), for one block of rows.
//!
//! Memory layout (identical in every backend):
//! - populations are SoA: `f[i * n + cell]`, `cell = y * width + x`;
//! - `f` stores **shifted** populations `f̃ᵢ = fᵢ − wᵢ` (equilibrium at rest subtracted), which
//!   keeps ~3 more significant digits of the density in f32 (FluidX3D's trick);
//! - the source buffer holds post-collision values of the previous step, the destination
//!   buffer receives the new post-collision values (ping-pong).
//!
//! Per fluid cell: pull `g` → θ; pull `f` (applying link-wise boundary rules) → ρ, u;
//! Guo forcing (body force + Boussinesq) split into TRT symmetric/antisymmetric parts;
//! collide `f` and `g`; write populations and macroscopic fields. Non-fluid cells are skipped.
//!
//! With a free surface (`FREE_SURFACE`, ADR-032) this is pass 1 of four (the others are in
//! [`crate::surface`]): gas cells are skipped, populations coming from gas cells are rebuilt,
//! the mass exchange and the neighbours' excess mass are added to the cell's mass, and full or
//! empty interface cells are marked for conversion.

use cfd_core::domain::{flags, phase, BcParams, LINKS, OPPOSITE};

use crate::lattice::{W5, W9};
use crate::surface::{self, state};

/// Smagorinsky LES constants (Hou et al. 1996).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Les {
    /// Molecular relaxation time τ₀.
    pub tau0: f32,
    /// `18 √2 C_s²`: `τ = ½ (τ₀ + √(τ₀² + factor · ‖Π^neq‖ / ρ))`.
    pub factor: f32,
    /// TRT's Λ, so that ω⁻ follows the local τ; `None` for BGK (ω⁻ = ω⁺).
    pub magic: Option<f32>,
}

/// Constants of one step.
pub(crate) struct StepParams {
    pub w: usize,
    pub h: usize,
    /// Cells per population plane.
    pub n: usize,
    pub periodic: [bool; 2],
    /// Flow relaxation rates (ω⁺, ω⁻).
    pub omega: [f32; 2],
    /// Thermal relaxation rates (ω⁺, ω⁻).
    pub omega_g: [f32; 2],
    /// Body acceleration.
    pub body_force: [f32; 2],
    /// Boussinesq force per unit θ.
    pub buoyancy: [f32; 2],
    /// Local relaxation time from the LES model; `None` uses `omega` everywhere.
    pub les: Option<Les>,
}

/// The free-surface planes a step reads (all cells).
pub(crate) struct SurfaceSource<'a> {
    /// Cell states at the start of the step ([`crate::surface::state`]).
    pub state: &'a [u32],
    /// Undivided excess mass of each cell, with its receiver count in `state`.
    pub massex: &'a [f32],
    /// Fill fraction φ written by the previous step's last pass.
    pub phi: &'a [f32],
}

/// The free-surface planes pass 1 writes, for the rows of a [`Block`].
pub(crate) struct SurfaceBlock<'a> {
    /// The state with this step's marks (IF/IG), receiver counts cleared.
    pub state: &'a mut [u32],
    /// Read and written: the cell's own mass.
    pub mass: &'a mut [f32],
}

/// Read-only inputs of a step.
pub(crate) struct Source<'a> {
    pub f: &'a [f32],
    /// Empty without the thermal lattice.
    pub g: &'a [f32],
    pub flags: &'a [u16],
    pub slot: &'a [u32],
    pub bc: &'a [BcParams],
    /// `Some` with a free surface.
    pub surf: Option<SurfaceSource<'a>>,
}

/// Outputs for rows `y0 .. y0 + rows`: each slice covers exactly those rows.
pub(crate) struct Block<'a> {
    pub y0: usize,
    pub f: [&'a mut [f32]; 9],
    pub g: Option<[&'a mut [f32]; 5]>,
    pub rho: &'a mut [f32],
    pub ux: &'a mut [f32],
    pub uy: &'a mut [f32],
    pub theta: &'a mut [f32],
    pub surf: Option<SurfaceBlock<'a>>,
}

/// What sits across a boundary link: the flags and parameters of the source cell, or the
/// implicit wall (slot 0) when the link leaves the domain through a non-periodic edge.
#[inline]
fn link_target<'a>(
    p: &StepParams,
    s: &Source<'a>,
    x: usize,
    y: usize,
    i: usize,
    src: usize,
) -> (u16, &'a BcParams) {
    let [cx, cy] = LINKS[i];
    let outside_x = !p.periodic[0] && ((x == 0 && cx == 1) || (x + 1 == p.w && cx == -1));
    let outside_y = !p.periodic[1] && ((y == 0 && cy == 1) || (y + 1 == p.h && cy == -1));
    if outside_x || outside_y {
        (flags::BOUNCE_BACK, &s.bc[0])
    } else {
        (s.flags[src], &s.bc[s.slot[src] as usize])
    }
}

/// Interior cell next to the outlet cell `(sx, sy)` (see `flags::INWARD_SHIFT`), or `own` when
/// the outlet has no inward direction.
#[inline]
fn inward_cell(p: &StepParams, sx: usize, sy: usize, nf: u16, own: usize) -> usize {
    match flags::inward(nf) {
        0 => own,
        d => {
            let [cx, cy] = LINKS[d];
            let x = (sx as isize + cx as isize).rem_euclid(p.w as isize) as usize;
            let y = (sy as isize + cy as isize).rem_euclid(p.h as isize) as usize;
            y * p.w + x
        }
    }
}

#[inline]
fn dot(c: [i32; 2], v: [f32; 2]) -> f32 {
    c[0] as f32 * v[0] + c[1] as f32 * v[1]
}

/// Population `i` of cell `k = (x, y)` pulled across a boundary link from `src` (bounce-back,
/// anti-bounce-back or zero gradient); `u` is the cell's velocity of the previous step.
#[inline]
#[allow(clippy::too_many_arguments)]
fn boundary_population(
    p: &StepParams,
    s: &Source,
    x: usize,
    y: usize,
    i: usize,
    src: usize,
    k: usize,
    u: [f32; 2],
) -> f32 {
    let n = p.n;
    let (nf, bc) = link_target(p, s, x, y, i, src);
    let out = s.f[OPPOSITE[i] * n + k]; // f̃*_ī(x)
    match flags::flow(nf) {
        // Bounce-back with wall velocity: f_i = f*_ī + 6 w_i ρ₀ c_i·u_w (ρ₀ = 1). It imposes
        // the momentum ρ₀u_w; a local ρ_w would make velocity inlets unstable with
        // open (zero-gradient) outlets: the inflow would grow with the density.
        flags::BOUNCE_BACK => out + 6.0 * W9[i] * dot(LINKS[i], bc.velocity),
        // Anti-bounce-back with the outlet density and this cell's velocity.
        flags::PRESSURE => {
            let cu = dot(LINKS[i], u);
            let usq = u[0] * u[0] + u[1] * u[1];
            -out + 2.0 * W9[i] * (bc.density * (1.0 + 4.5 * cu * cu - 1.5 * usq) - 1.0)
        }
        // Zero gradient along the outlet normal: the outlet cell has the state of
        // its interior neighbour.
        _ => {
            let [cx, cy] = LINKS[i];
            let sx = (x as isize - cx as isize).rem_euclid(p.w as isize) as usize;
            let sy = (y as isize - cy as isize).rem_euclid(p.h as isize) as usize;
            s.f[i * n + inward_cell(p, sx, sy, nf, k)]
        }
    }
}

/// Velocity for the gas-side reconstruction of an interface cell: from its own post-collision
/// populations (all in the source buffer at its index) plus the Guo half-force shift, as
/// FluidX3D does, so it does not depend on the stored velocity (ADR-029's `skip_macro`).
#[inline]
fn outgoing_velocity(p: &StepParams, s: &Source, k: usize) -> [f32; 2] {
    let n = p.n;
    let fo: [f32; 9] = std::array::from_fn(|i| s.f[i * n + k]);
    let rho = 1.0 + fo.iter().sum::<f32>();
    let jx = fo[1] - fo[3] + fo[5] - fo[6] - fo[7] + fo[8];
    let jy = fo[2] - fo[4] + fo[5] + fo[6] - fo[7] - fo[8];
    [
        (jx + 0.5 * rho * p.body_force[0]) / rho,
        (jy + 0.5 * rho * p.body_force[1]) / rho,
    ]
}

/// Local relaxation rates (ω⁺, ω⁻) of the Smagorinsky model, from the non-equilibrium momentum
/// flux `Π^neq = Σ cᵢcᵢ (fᵢ − fᵢ^eq)` of the pulled populations (shifted: the shift cancels).
#[inline]
pub(crate) fn les_rates(les: &Les, f: &[f32; 9], drho: f32, rho: f32, u: [f32; 2]) -> [f32; 2] {
    // Σ cₓ² f̃^eq = Δρ/3 + ρuₓ², Σ cₓc_y f̃^eq = ρuₓu_y.
    let pxx = f[1] + f[3] + f[5] + f[6] + f[7] + f[8] - (drho / 3.0 + rho * u[0] * u[0]);
    let pyy = f[2] + f[4] + f[5] + f[6] + f[7] + f[8] - (drho / 3.0 + rho * u[1] * u[1]);
    let pxy = f[5] - f[6] + f[7] - f[8] - rho * u[0] * u[1];
    let q = (pxx * pxx + pyy * pyy + 2.0 * pxy * pxy).sqrt();
    let tau = 0.5 * (les.tau0 + (les.tau0 * les.tau0 + les.factor * q / rho).sqrt());
    let wp = 1.0 / tau;
    let wm = match les.magic {
        Some(magic) => 1.0 / (0.5 + magic / (tau - 0.5)),
        None => wp,
    };
    [wp, wm]
}

/// With `write_macro == false` (the `skip_macro` steps, design proposal §7) the macroscopic outputs
/// are not stored, except u on cells with a boundary link (pressure-outlet links read their cell's
/// u on the next step): nothing else reads them between two `step` calls, and the last step of a
/// call always writes them. With a free surface, ρ is stored on every liquid and interface cell
/// and u on every interface cell (the conversion passes read them).
pub(crate) fn update_block<const THERMAL: bool, const FREE_SURFACE: bool>(
    p: &StepParams,
    s: &Source,
    b: Block,
    write_macro: bool,
) {
    let Block {
        y0,
        f: fo,
        g: mut go,
        rho: rho_o,
        ux: ux_o,
        uy: uy_o,
        theta: theta_o,
        surf: mut so,
    } = b;
    let (w, n) = (p.w, p.n);
    let rows = rho_o.len() / w;
    let [gwp, gwm] = p.omega_g;

    for ry in 0..rows {
        let y = y0 + ry;
        // Wrapped rows; only used for links that are not boundary links.
        let ym = if y == 0 { p.h - 1 } else { y - 1 };
        let yp = if y + 1 == p.h { 0 } else { y + 1 };
        for x in 0..w {
            let k = y * w + x;
            let fl = s.flags[k];
            if flags::flow(fl) != flags::FLUID {
                continue;
            }
            let local = ry * w + x;
            let own_phase = if FREE_SURFACE {
                let st = s.surf.as_ref().expect("free-surface source").state[k];
                let ph = state::phase(st);
                if ph == phase::GAS {
                    so.as_mut().expect("free-surface outputs").state[local] = ph as u32;
                    continue;
                }
                ph
            } else {
                phase::LIQUID
            };
            let xm = if x == 0 { w - 1 } else { x - 1 };
            let xp = if x + 1 == w { 0 } else { x + 1 };
            // Source cell of population i is x − c_i.
            let xs = [x, xm, x, xp, x, xm, xp, xp, xm];
            let ys = [y, y, ym, y, yp, ym, ym, yp, yp];
            let mask = fl >> flags::LINK_SHIFT;

            // ---- Thermal pull: θ ----
            let mut g = [0.0f32; 5];
            let mut theta = 0.0f32;
            if THERMAL {
                for i in 0..5 {
                    let src = ys[i] * w + xs[i];
                    g[i] = if i == 0 || mask & (1 << (i - 1)) == 0 {
                        s.g[i * n + src]
                    } else {
                        let (nf, bc) = link_target(p, s, x, y, i, src);
                        let out = s.g[OPPOSITE[i] * n + k]; // g*_ī(x)
                        match flags::thermal(nf) {
                            flags::ADIABATIC => out,
                            flags::FIXED_TEMPERATURE => -out + 2.0 * W5[i] * bc.theta,
                            flags::HEAT_FLUX => out + bc.heat_flux,
                            _ => s.g[i * n + inward_cell(p, xs[i], ys[i], nf, k)], // zero gradient
                        }
                    };
                    theta += g[i];
                }
            }

            // ---- Flow pull ----
            let mut f = [0.0f32; 9];
            // Free surface: this step's mass change, and which neighbours are gas or liquid.
            let (mut dm, mut gas_neighbour, mut liquid_neighbour) = (0.0f32, false, false);
            if FREE_SURFACE {
                let sf = s.surf.as_ref().expect("free-surface source");
                let phi_k = sf.phi[k];
                let mut u_gas: Option<[f32; 2]> = None;
                f[0] = s.f[k];
                for i in 1..9 {
                    let src = ys[i] * w + xs[i];
                    if mask & (1 << (i - 1)) != 0 {
                        let u = [ux_o[local], uy_o[local]];
                        f[i] = boundary_population(p, s, x, y, i, src, k, u);
                        continue;
                    }
                    let nst = sf.state[src];
                    let np = state::phase(nst);
                    let out = s.f[OPPOSITE[i] * n + k]; // f̃*_ī(x)
                    if np == phase::GAS {
                        // The gas imposes ρ_G = 1 and the interface's velocity:
                        // f_i = f_i^eq(1, u) + f_ī^eq(1, u) − f*_ī (in shifted populations).
                        gas_neighbour = true;
                        let u = *u_gas.get_or_insert_with(|| outgoing_velocity(p, s, k));
                        let cu = dot(LINKS[i], u);
                        let usq = u[0] * u[0] + u[1] * u[1];
                        f[i] = -out + 2.0 * W9[i] * (4.5 * cu * cu - 1.5 * usq);
                    } else {
                        let fin = s.f[i * n + src];
                        f[i] = fin;
                        liquid_neighbour |= np == phase::LIQUID;
                        // Weight 1 if either cell is liquid, else the mean fill: the same seen
                        // from both cells, so each link's exchange is exactly antisymmetric.
                        let weight = if own_phase == phase::LIQUID || np == phase::LIQUID {
                            1.0
                        } else {
                            0.5 * (phi_k + sf.phi[src])
                        };
                        dm += weight * (fin - out);
                    }
                    // The neighbour's excess, split equally over its receivers (gas and
                    // boundary links carry none: they were not counted).
                    let receivers = state::receivers(nst);
                    if receivers > 0 {
                        dm += sf.massex[src] / receivers as f32;
                    }
                }
            } else if mask == 0 {
                for i in 0..9 {
                    f[i] = s.f[i * n + ys[i] * w + xs[i]];
                }
            } else {
                for i in 0..9 {
                    let src = ys[i] * w + xs[i];
                    if i == 0 || mask & (1 << (i - 1)) == 0 {
                        f[i] = s.f[i * n + src];
                        continue;
                    }
                    let u = [ux_o[local], uy_o[local]];
                    f[i] = boundary_population(p, s, x, y, i, src, k, u);
                }
            }

            // ---- Moments and force ----
            let drho: f32 = f.iter().sum();
            let rho = 1.0 + drho;
            let jx = f[1] - f[3] + f[5] - f[6] - f[7] + f[8];
            let jy = f[2] - f[4] + f[5] + f[6] - f[7] - f[8];
            let force = [
                rho * p.body_force[0] + p.buoyancy[0] * theta,
                rho * p.body_force[1] + p.buoyancy[1] * theta,
            ];
            let u = [(jx + 0.5 * force[0]) / rho, (jy + 0.5 * force[1]) / rho];
            let usq = u[0] * u[0] + u[1] * u[1];
            let uf = u[0] * force[0] + u[1] * force[1];
            let [wp, wm] = match &p.les {
                Some(les) => les_rates(les, &f, drho, rho, u),
                None => p.omega,
            };

            // ---- TRT collision with Guo forcing ----
            let (sp, sm) = (1.0 - 0.5 * wp, 1.0 - 0.5 * wm);
            let mut moving = 0.0;
            for i in [1, 2, 5, 6] {
                let o = OPPOSITE[i];
                let wi = W9[i];
                let cu = dot(LINKS[i], u);
                let cf = dot(LINKS[i], force);
                let eq_p = wi * (drho + rho * (4.5 * cu * cu - 1.5 * usq));
                let eq_m = wi * 3.0 * rho * cu;
                let src_p = wi * (9.0 * cu * cf - 3.0 * uf);
                let src_m = wi * 3.0 * cf;
                let f_p = 0.5 * (f[i] + f[o]);
                let f_m = 0.5 * (f[i] - f[o]);
                let relax_p = -wp * (f_p - eq_p) + sp * src_p;
                let relax_m = -wm * (f_m - eq_m) + sm * src_m;
                let (fi, fo_) = (f[i] + relax_p + relax_m, f[o] + relax_p - relax_m);
                fo[i][local] = fi;
                fo[o][local] = fo_;
                moving += fi + fo_;
            }
            // Rest population by difference: Σ f̃* = Δρ exactly (the Guo source sums to zero).
            fo[0][local] = drho - moving;
            if write_macro || FREE_SURFACE {
                rho_o[local] = rho;
            }
            if write_macro || mask != 0 || (FREE_SURFACE && own_phase == phase::INTERFACE) {
                ux_o[local] = u[0];
                uy_o[local] = u[1];
            }

            if FREE_SURFACE {
                let so = so.as_mut().expect("free-surface outputs");
                let m = so.mass[local] + dm;
                so.mass[local] = m;
                let mark = if own_phase != phase::INTERFACE {
                    state::MARK_NONE
                } else if m > (1.0 + surface::KAPPA) * rho || !gas_neighbour {
                    state::TO_LIQUID
                } else if m < -surface::KAPPA * rho || !liquid_neighbour {
                    state::TO_GAS
                } else {
                    state::MARK_NONE
                };
                so.state[local] = state::word(own_phase, mark, 0);
            }

            if THERMAL {
                let go = go.as_mut().expect("thermal outputs");
                // The rest population takes what is left, so Σ g* = θ holds by construction:
                // relaxing it directly leaves an f32 rounding bias that drifts the heat content.
                let mut moving = 0.0;
                for i in [1, 2] {
                    let o = OPPOSITE[i];
                    let cu = dot(LINKS[i], u);
                    let eq_p = W5[i] * theta;
                    let eq_m = W5[i] * theta * 3.0 * cu;
                    let relax_p = -gwp * (0.5 * (g[i] + g[o]) - eq_p);
                    let relax_m = -gwm * (0.5 * (g[i] - g[o]) - eq_m);
                    let (gi, go_) = (g[i] + relax_p + relax_m, g[o] + relax_p - relax_m);
                    go[i][local] = gi;
                    go[o][local] = go_;
                    moving += gi + go_;
                }
                go[0][local] = theta - moving;
                if write_macro {
                    theta_o[local] = theta;
                }
            }
        }
    }
}

/// Shifted D2Q9 equilibrium `f̃ᵢ^eq = fᵢ^eq − wᵢ`.
pub(crate) fn equilibrium(rho: f32, u: [f32; 2]) -> [f32; 9] {
    let usq = u[0] * u[0] + u[1] * u[1];
    std::array::from_fn(|i| {
        let cu = dot(LINKS[i], u);
        W9[i] * (rho - 1.0 + rho * (3.0 * cu + 4.5 * cu * cu - 1.5 * usq))
    })
}

/// D2Q5 equilibrium `gᵢ^eq = wᵢ θ (1 + 3 cᵢ·u)`.
pub(crate) fn thermal_equilibrium(theta: f32, u: [f32; 2]) -> [f32; 5] {
    std::array::from_fn(|i| W5[i] * theta * (1.0 + 3.0 * dot(LINKS[i], u)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_relaxation_time_solves_the_smagorinsky_closure() {
        // Equilibrium plus a shear perturbation of the diagonal populations.
        let (rho, u) = (1.01f32, [0.03f32, -0.01]);
        let mut f = equilibrium(rho, u);
        for (i, d) in [(5, 1e-3), (7, 1e-3), (6, -1e-3), (8, -1e-3)] {
            f[i] += d;
        }
        let drho: f32 = f.iter().sum();
        let (tau0, cs) = (0.50003f64, 0.1f64);
        let les = Les {
            tau0: tau0 as f32,
            factor: (18.0 * 2f64.sqrt() * cs * cs) as f32,
            magic: Some(3.0 / 16.0),
        };
        let [wp, wm] = les_rates(&les, &f, drho, 1.0 + drho, u);
        // Π^neq_xy = 4e-3, Π^neq_xx = Π^neq_yy = 0 (to f32 rounding): τ from
        // τ² − τ₀τ − 9 C_s² ‖Π‖ / (√2 ρ) = 0.
        let q = (2.0f64 * 4e-3f64.powi(2)).sqrt();
        let c = 9.0 * cs * cs * q / (2f64.sqrt() * (1.0 + drho as f64));
        let tau = 0.5 * (tau0 + (tau0 * tau0 + 4.0 * c).sqrt());
        assert!(
            ((1.0 / wp as f64) / tau - 1.0).abs() < 1e-5,
            "{} vs {tau}",
            1.0 / wp
        );
        let magic = (1.0 / wp as f64 - 0.5) * (1.0 / wm as f64 - 0.5);
        assert!((magic - 3.0 / 16.0).abs() < 1e-4, "Λ = {magic}");
        assert!(tau > tau0 + 5e-4, "the shear raises τ: {tau}");
    }
}
