// Helpers of the stream-collide kernels, shared by `lbm.wgsl` (single phase) and `surface.wgsl`
// (free surface): link-wise boundary rules, the collision and the Smagorinsky rates. They use the
// module-scope bindings `P`, `cell_flags`, `bc_slot`, `bc`, `f_dst` and `mac`, which both modules
// declare at the same places.

fn flags_of(c: u32) -> u32 {
    return unpack_flags(cell_flags[c >> 1u], c);
}

// What sits across a boundary link: (flags, slot) of the source cell, or the implicit wall
// (slot 0) when the link leaves the domain through a non-periodic edge.
fn link_target(x: u32, y: u32, i: u32, src: u32) -> vec2<u32> {
    let cx = LINK_X[i];
    let cy = LINK_Y[i];
    let outside_x = (P.periodic & 1u) == 0u && ((x == 0u && cx == 1) || (x + 1u == P.w && cx == -1));
    let outside_y = (P.periodic & 2u) == 0u && ((y == 0u && cy == 1) || (y + 1u == P.h && cy == -1));
    if (outside_x || outside_y) {
        return vec2<u32>(BOUNCE_BACK, 0u);
    }
    return vec2<u32>(flags_of(src), bc_slot[src]);
}

// Interior cell next to the outlet cell (sx, sy) along its inward axis, or `own` when the outlet
// has no inward direction.
fn inward_cell(sx: u32, sy: u32, nf: u32, own: u32) -> u32 {
    let d = (nf & INWARD_MASK) >> INWARD_SHIFT;
    if (d == 0u) {
        return own;
    }
    let x = u32((i32(sx) + LINK_X[d] + i32(P.w)) % i32(P.w));
    let y = u32((i32(sy) + LINK_Y[d] + i32(P.h)) % i32(P.h));
    return y * P.w + x;
}

// Local relaxation rates (ω⁺, ω⁻) of the Smagorinsky model from the non-equilibrium momentum flux of
// the pulled populations (`cfd_lbm::kernel::les_rates`).
fn les_rates(f: array<f32, 9>, drho: f32, rho: f32, u: vec2<f32>) -> vec2<f32> {
    // Σ cₓ² f̃^eq = Δρ/3 + ρuₓ², Σ cₓc_y f̃^eq = ρuₓu_y.
    let pxx = f[1] + f[3] + f[5] + f[6] + f[7] + f[8] - (drho / 3.0 + rho * u.x * u.x);
    let pyy = f[2] + f[4] + f[5] + f[6] + f[7] + f[8] - (drho / 3.0 + rho * u.y * u.y);
    let pxy = f[5] - f[6] + f[7] - f[8] - rho * u.x * u.y;
    let q = sqrt(pxx * pxx + pyy * pyy + 2.0 * pxy * pxy);
    let tau = 0.5 * (P.les_tau0 + sqrt(P.les_tau0 * P.les_tau0 + P.les_factor * q / rho));
    let wp = 1.0 / tau;
    let wm = select(1.0 / (0.5 + P.les_magic / (tau - 0.5)), wp, P.les_magic < 0.0);
    return vec2<f32>(wp, wm);
}

// The rates of a cell: the run's, or the LES model's when it is on.
fn relaxation(f: array<f32, 9>, drho: f32, rho: f32, u: vec2<f32>) -> vec2<f32> {
    if (P.les_factor > 0.0) {
        return les_rates(f, drho, rho, u);
    }
    return P.omega;
}

// TRT collision with Guo forcing of the pair (i, ī) with rates (wp, wm); writes both populations and
// returns their sum.
fn collide_pair(
    i: u32, k: u32, fi: f32, fo: f32, drho: f32, rho: f32, u: vec2<f32>, usq: f32,
    force: vec2<f32>, uf: f32, wp: f32, wm: f32,
) -> f32 {
    let o = OPPOSITE[i];
    let wi = W9[i];
    let sp = 1.0 - 0.5 * wp;
    let sm = 1.0 - 0.5 * wm;
    let cu = dot_c(i, u);
    let cf = dot_c(i, force);
    let eq_p = wi * (drho + rho * (4.5 * cu * cu - 1.5 * usq));
    let eq_m = wi * 3.0 * rho * cu;
    let src_p = wi * (9.0 * cu * cf - 3.0 * uf);
    let src_m = wi * 3.0 * cf;
    let f_p = 0.5 * (fi + fo);
    let f_m = 0.5 * (fi - fo);
    let relax_p = -wp * (f_p - eq_p) + sp * src_p;
    let relax_m = -wm * (f_m - eq_m) + sm * src_m;
    let new_i = fi + relax_p + relax_m;
    let new_o = fo + relax_p - relax_m;
    f_dst[i * P.n + k] = new_i;
    f_dst[o * P.n + k] = new_o;
    return new_i + new_o;
}

