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

use cfd_core::domain::{flags, BcParams, LINKS, OPPOSITE};

use crate::lattice::{W5, W9};

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
}

/// Read-only inputs of a step.
pub(crate) struct Source<'a> {
    pub f: &'a [f32],
    /// Empty without the thermal lattice.
    pub g: &'a [f32],
    pub flags: &'a [u16],
    pub slot: &'a [u32],
    pub bc: &'a [BcParams],
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

pub(crate) fn update_block<const THERMAL: bool>(p: &StepParams, s: &Source, b: Block) {
    let Block {
        y0,
        f: fo,
        g: mut go,
        rho: rho_o,
        ux: ux_o,
        uy: uy_o,
        theta: theta_o,
    } = b;
    let (w, n) = (p.w, p.n);
    let rows = rho_o.len() / w;
    let [wp, wm] = p.omega;
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
            if mask == 0 {
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
                    let (nf, bc) = link_target(p, s, x, y, i, src);
                    let out = s.f[OPPOSITE[i] * n + k]; // f̃*_ī(x)
                    f[i] = match flags::flow(nf) {
                        // Bounce-back with wall velocity: f_i = f*_ī + 6 w_i ρ₀ c_i·u_w (ρ₀ = 1). It imposes
                        // the momentum ρ₀u_w; a local ρ_w would make velocity inlets unstable with
                        // open (zero-gradient) outlets: the inflow would grow with the density.
                        flags::BOUNCE_BACK => out + 6.0 * W9[i] * dot(LINKS[i], bc.velocity),
                        // Anti-bounce-back with the outlet density and this cell's velocity.
                        flags::PRESSURE => {
                            let u = [ux_o[local], uy_o[local]];
                            let cu = dot(LINKS[i], u);
                            let usq = u[0] * u[0] + u[1] * u[1];
                            -out + 2.0
                                * W9[i]
                                * (bc.density * (1.0 + 4.5 * cu * cu - 1.5 * usq) - 1.0)
                        }
                        // Zero gradient along the outlet normal: the outlet cell has the state of
                        // its interior neighbour.
                        _ => s.f[i * n + inward_cell(p, xs[i], ys[i], nf, k)],
                    };
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
            rho_o[local] = rho;
            ux_o[local] = u[0];
            uy_o[local] = u[1];

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
                theta_o[local] = theta;
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
