// Fused pull stream-collide step: a line-by-line translation of `cfd-lbm/src/kernel.rs`
// (`update_block`), with the same memory layout and the same order of floating-point operations.
//
// - Populations are SoA, `f[i * n + cell]`, `cell = y * w + x`; `f` holds shifted populations
//   f̃ᵢ = fᵢ − wᵢ. The source buffers hold the post-collision values of the previous step, the
//   destination buffers receive the new ones (ping-pong by swapping bind groups on the CPU side).
// - `mac` holds four planes ρ, ux, uy, θ, then one status word (index 4n) set to 1.0 when a
//   non-finite population is read (see the end of `stream_collide`).
// - One invocation per cell; non-fluid cells return immediately.

// Invocations per workgroup and whether the D2Q5 thermal lattice is solved: pipeline-overridable
// constants, fixed when the pipeline is created (like a C++ template argument), so the unused
// thermal code is compiled out.
override WG: u32 = 128u;
override THERMAL: bool = false;

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> cell_flags: array<u32>;
@group(0) @binding(2) var<storage, read> bc_slot: array<u32>;
@group(0) @binding(3) var<storage, read> bc: array<BcParams>;

@group(1) @binding(0) var<storage, read> f_src: array<f32>;
@group(1) @binding(1) var<storage, read_write> f_dst: array<f32>;
@group(1) @binding(2) var<storage, read> g_src: array<f32>;
@group(1) @binding(3) var<storage, read_write> g_dst: array<f32>;
@group(1) @binding(4) var<storage, read_write> mac: array<f32>;

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

// TRT collision with Guo forcing of the pair (i, ī); writes both populations and returns their sum.
fn collide_pair(
    i: u32, k: u32, fi: f32, fo: f32, drho: f32, rho: f32, u: vec2<f32>, usq: f32,
    force: vec2<f32>, uf: f32,
) -> f32 {
    let o = OPPOSITE[i];
    let wi = W9[i];
    let wp = P.omega.x;
    let wm = P.omega.y;
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

// The same for the D2Q5 pair (i, ī) with TRT rates P.omega_g.
fn collide_thermal_pair(i: u32, k: u32, gi: f32, go: f32, theta: f32, u: vec2<f32>) -> f32 {
    let o = OPPOSITE[i];
    let cu = dot_c(i, u);
    let eq_p = W5[i] * theta;
    let eq_m = W5[i] * theta * 3.0 * cu;
    let relax_p = -P.omega_g.x * (0.5 * (gi + go) - eq_p);
    let relax_m = -P.omega_g.y * (0.5 * (gi - go) - eq_m);
    let new_i = gi + relax_p + relax_m;
    let new_o = go + relax_p - relax_m;
    g_dst[i * P.n + k] = new_i;
    g_dst[o * P.n + k] = new_o;
    return new_i + new_o;
}

@compute @workgroup_size(WG)
fn stream_collide(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    // 2-D dispatch: a dimension is limited to 65535 workgroups.
    let k = gid.y * groups.x * WG + gid.x;
    if (k >= P.n) {
        return;
    }
    let fl = flags_of(k);
    if ((fl & FLOW_MASK) != FLUID) {
        return;
    }
    let w = P.w;
    let n = P.n;
    let x = k % w;
    let y = k / w;
    // Wrapped neighbours; only used for links that are not boundary links.
    let xm = select(x - 1u, w - 1u, x == 0u);
    let xp = select(x + 1u, 0u, x + 1u == w);
    let ym = select(y - 1u, P.h - 1u, y == 0u);
    let yp = select(y + 1u, 0u, y + 1u == P.h);
    // Source cell of population i is x − c_i.
    var xs = array<u32, 9>(x, xm, x, xp, x, xm, xp, xp, xm);
    var ys = array<u32, 9>(y, y, ym, y, yp, ym, ym, yp, yp);
    let mask = fl >> LINK_SHIFT;

    // ---- Thermal pull: θ ----
    var g: array<f32, 5>;
    var theta = 0.0;
    if (THERMAL) {
        for (var i = 0u; i < 5u; i++) {
            let src = ys[i] * w + xs[i];
            if (i == 0u || (mask & (1u << (i - 1u))) == 0u) {
                g[i] = g_src[i * n + src];
            } else {
                let t = link_target(x, y, i, src);
                let out = g_src[OPPOSITE[i] * n + k]; // g*_ī(x)
                let b = bc[t.y];
                switch ((t.x & THERMAL_MASK) >> THERMAL_SHIFT) {
                    case ADIABATIC: {
                        g[i] = out;
                    }
                    case FIXED_TEMPERATURE: {
                        g[i] = -out + 2.0 * W5[i] * b.theta;
                    }
                    case HEAT_FLUX: {
                        g[i] = out + b.heat_flux;
                    }
                    default: { // zero gradient
                        g[i] = g_src[i * n + inward_cell(xs[i], ys[i], t.x, k)];
                    }
                }
            }
            theta += g[i];
        }
    }

    // ---- Flow pull ----
    var f: array<f32, 9>;
    if (mask == 0u) {
        for (var i = 0u; i < 9u; i++) {
            f[i] = f_src[i * n + ys[i] * w + xs[i]];
        }
    } else {
        for (var i = 0u; i < 9u; i++) {
            let src = ys[i] * w + xs[i];
            if (i == 0u || (mask & (1u << (i - 1u))) == 0u) {
                f[i] = f_src[i * n + src];
                continue;
            }
            let t = link_target(x, y, i, src);
            let out = f_src[OPPOSITE[i] * n + k]; // f̃*_ī(x)
            let b = bc[t.y];
            switch (t.x & FLOW_MASK) {
                // Bounce-back with wall velocity: f_i = f*_ī + 6 w_i ρ₀ c_i·u_w (ρ₀ = 1).
                case BOUNCE_BACK: {
                    f[i] = out + 6.0 * W9[i] * dot_c(i, b.velocity);
                }
                // Anti-bounce-back with the outlet density and this cell's velocity (previous step).
                case PRESSURE: {
                    let u = vec2<f32>(mac[n + k], mac[2u * n + k]);
                    let cu = dot_c(i, u);
                    let usq = u.x * u.x + u.y * u.y;
                    f[i] = -out + 2.0 * W9[i] * (b.density * (1.0 + 4.5 * cu * cu - 1.5 * usq) - 1.0);
                }
                // Zero gradient along the outlet normal.
                default: {
                    f[i] = f_src[i * n + inward_cell(xs[i], ys[i], t.x, k)];
                }
            }
        }
    }

    // ---- Moments and force ----
    var drho = 0.0;
    for (var i = 0u; i < 9u; i++) {
        drho += f[i];
    }
    let rho = 1.0 + drho;
    let jx = f[1] - f[3] + f[5] - f[6] - f[7] + f[8];
    let jy = f[2] - f[4] + f[5] + f[6] - f[7] - f[8];
    let force = vec2<f32>(
        rho * P.body_force.x + P.buoyancy.x * theta,
        rho * P.body_force.y + P.buoyancy.y * theta,
    );
    let u = vec2<f32>((jx + 0.5 * force.x) / rho, (jy + 0.5 * force.y) / rho);
    let usq = u.x * u.x + u.y * u.y;
    let uf = u.x * force.x + u.y * force.y;

    // ---- TRT collision with Guo forcing ----
    var moving = 0.0;
    moving += collide_pair(1u, k, f[1], f[3], drho, rho, u, usq, force, uf);
    moving += collide_pair(2u, k, f[2], f[4], drho, rho, u, usq, force, uf);
    moving += collide_pair(5u, k, f[5], f[7], drho, rho, u, usq, force, uf);
    moving += collide_pair(6u, k, f[6], f[8], drho, rho, u, usq, force, uf);
    // Rest population by difference: Σ f̃* = Δρ exactly (the Guo source sums to zero).
    f_dst[k] = drho - moving;
    mac[k] = rho;
    mac[n + k] = u.x;
    mac[2u * n + k] = u.y;
    // Divergence check on values read from memory: GPU compilers may assume that arithmetic
    // results are never NaN/Inf (Vulkan does not require their preservation) and fold the test
    // away, but they cannot assume it of loads. A NaN anywhere in a cell reaches its rest
    // population, so this flags a divergence one step after it appears.
    var finite = is_finite(f[0]);

    if (THERMAL) {
        // Rest population as the remainder: Σ g* = θ by construction.
        var moving_g = 0.0;
        moving_g += collide_thermal_pair(1u, k, g[1], g[3], theta, u);
        moving_g += collide_thermal_pair(2u, k, g[2], g[4], theta, u);
        g_dst[k] = theta - moving_g;
        mac[3u * n + k] = theta;
        finite = finite && is_finite(g[0]);
    }
    if (!finite) {
        mac[4u * n] = 1.0;
    }
}
