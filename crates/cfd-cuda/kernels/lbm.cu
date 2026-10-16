// Fused pull stream-collide step: a line-by-line translation of `cfd-gpu/shaders/lbm.wgsl` (itself
// a translation of `cfd-lbm/src/kernel.rs`), with the same memory layout and the same order of
// floating-point operations. Compiled with `--fmad=false`, so no multiply-add is fused and every
// operation rounds as in the CPU reference.
//
// - Populations are SoA, `f[i * n + cell]`, `cell = y * w + x`; `f` holds shifted populations
//   f̃ᵢ = fᵢ − wᵢ. The source buffers hold the post-collision values of the previous step, the
//   destination buffers receive the new ones (ping-pong: the host swaps the two pointers).
// - `flags` are the `u16` flags of `cfd_core::Domain`, read directly (CUDA has 16-bit loads).
// - `mac` holds four planes ρ, ux, uy, θ, then one status word (index 4n) set to 1.0 when a
//   non-finite population is read (see the end of `step`).
// - One thread per cell; non-fluid cells return immediately.
//
// `__restrict__` promises that the buffers do not overlap, which lets the compiler use the
// read-only data cache for the source populations.

// What sits across a boundary link: (flags, slot) of the source cell, or the implicit wall
// (slot 0) when the link leaves the domain through a non-periodic edge.
__device__ __forceinline__ uint2 link_target(
    const Params& P, const unsigned short* __restrict__ flags, const unsigned* __restrict__ bc_slot,
    unsigned x, unsigned y, unsigned i, unsigned src
) {
    const int cx = LINK_X[i];
    const int cy = LINK_Y[i];
    const bool outside_x = (P.periodic & 1u) == 0u && ((x == 0u && cx == 1) || (x + 1u == P.w && cx == -1));
    const bool outside_y = (P.periodic & 2u) == 0u && ((y == 0u && cy == 1) || (y + 1u == P.h && cy == -1));
    if (outside_x || outside_y) {
        return make_uint2(BOUNCE_BACK, 0u);
    }
    return make_uint2(flags[src], bc_slot[src]);
}

// Interior cell next to the outlet cell (sx, sy) along its inward axis, or `own` when the outlet
// has no inward direction.
__device__ __forceinline__ unsigned inward_cell(const Params& P, unsigned sx, unsigned sy, unsigned nf, unsigned own) {
    const unsigned d = (nf & INWARD_MASK) >> INWARD_SHIFT;
    if (d == 0u) {
        return own;
    }
    const unsigned x = unsigned((int(sx) + LINK_X[d] + int(P.w)) % int(P.w));
    const unsigned y = unsigned((int(sy) + LINK_Y[d] + int(P.h)) % int(P.h));
    return y * P.w + x;
}

// TRT collision with Guo forcing of the pair (i, ī); writes both populations and returns their sum.
__device__ __forceinline__ float collide_pair(
    const Params& P, float* __restrict__ f_dst, unsigned i, unsigned k, float fi, float fo,
    float drho, float rho, float2 u, float usq, float2 force, float uf
) {
    const unsigned o = OPPOSITE[i];
    const float wi = W9[i];
    const float wp = P.omega.x;
    const float wm = P.omega.y;
    const float sp = 1.0f - 0.5f * wp;
    const float sm = 1.0f - 0.5f * wm;
    const float cu = dot_c(i, u);
    const float cf = dot_c(i, force);
    const float eq_p = wi * (drho + rho * (4.5f * cu * cu - 1.5f * usq));
    const float eq_m = wi * 3.0f * rho * cu;
    const float src_p = wi * (9.0f * cu * cf - 3.0f * uf);
    const float src_m = wi * 3.0f * cf;
    const float f_p = 0.5f * (fi + fo);
    const float f_m = 0.5f * (fi - fo);
    const float relax_p = -wp * (f_p - eq_p) + sp * src_p;
    const float relax_m = -wm * (f_m - eq_m) + sm * src_m;
    const float new_i = fi + relax_p + relax_m;
    const float new_o = fo + relax_p - relax_m;
    f_dst[i * P.n + k] = new_i;
    f_dst[o * P.n + k] = new_o;
    return new_i + new_o;
}

// The same for the D2Q5 pair (i, ī) with TRT rates P.omega_g.
__device__ __forceinline__ float collide_thermal_pair(
    const Params& P, float* __restrict__ g_dst, unsigned i, unsigned k, float gi, float go,
    float theta, float2 u
) {
    const unsigned o = OPPOSITE[i];
    const float cu = dot_c(i, u);
    const float eq_p = W5[i] * theta;
    const float eq_m = W5[i] * theta * 3.0f * cu;
    const float relax_p = -P.omega_g.x * (0.5f * (gi + go) - eq_p);
    const float relax_m = -P.omega_g.y * (0.5f * (gi - go) - eq_m);
    const float new_i = gi + relax_p + relax_m;
    const float new_o = go + relax_p - relax_m;
    g_dst[i * P.n + k] = new_i;
    g_dst[o * P.n + k] = new_o;
    return new_i + new_o;
}

// The step, with the thermal lattice compiled in or out and the macroscopic stores on or off (template
// arguments play the role of WGSL's pipeline-overridable `THERMAL` and `WRITE_MACRO` constants).
// Without `WRITE_MACRO` (the `skip_macro` steps, design proposal §7) ρ, u and θ are not stored,
// except u on cells with a boundary link (pressure-outlet links read their cell's u on the next
// step); the status word is still written. The last step of every `step` call stores them, so every read sees fresh values.
template <bool THERMAL, bool WRITE_MACRO>
__device__ __forceinline__ void step(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const float* __restrict__ g_src,
    float* __restrict__ g_dst,
    float* __restrict__ mac
) {
    const unsigned k = blockIdx.x * blockDim.x + threadIdx.x;
    if (k >= P.n) {
        return;
    }
    const unsigned fl = flags[k];
    if ((fl & FLOW_MASK) != FLUID) {
        return;
    }
    const unsigned w = P.w;
    const unsigned n = P.n;
    const unsigned x = k % w;
    const unsigned y = k / w;
    // Wrapped neighbours; only used for links that are not boundary links.
    const unsigned xm = x == 0u ? w - 1u : x - 1u;
    const unsigned xp = x + 1u == w ? 0u : x + 1u;
    const unsigned ym = y == 0u ? P.h - 1u : y - 1u;
    const unsigned yp = y + 1u == P.h ? 0u : y + 1u;
    // Source cell of population i is x − c_i.
    const unsigned xs[9] = {x, xm, x, xp, x, xm, xp, xp, xm};
    const unsigned ys[9] = {y, y, ym, y, yp, ym, ym, yp, yp};
    const unsigned mask = fl >> LINK_SHIFT;

    // ---- Thermal pull: θ ----
    float g[5];
    float theta = 0.0f;
    if (THERMAL) {
#pragma unroll
        for (unsigned i = 0u; i < 5u; i++) {
            const unsigned src = ys[i] * w + xs[i];
            if (i == 0u || (mask & (1u << (i - 1u))) == 0u) {
                g[i] = g_src[i * n + src];
            } else {
                const uint2 t = link_target(P, flags, bc_slot, x, y, i, src);
                const float out = g_src[OPPOSITE[i] * n + k]; // g*_ī(x)
                const BcParams b = bc[t.y];
                switch ((t.x & THERMAL_MASK) >> THERMAL_SHIFT) {
                    case ADIABATIC:
                        g[i] = out;
                        break;
                    case FIXED_TEMPERATURE:
                        g[i] = -out + 2.0f * W5[i] * b.theta;
                        break;
                    case HEAT_FLUX:
                        g[i] = out + b.heat_flux;
                        break;
                    default: // zero gradient
                        g[i] = g_src[i * n + inward_cell(P, xs[i], ys[i], t.x, k)];
                        break;
                }
            }
            theta += g[i];
        }
    }

    // ---- Flow pull ----
    float f[9];
    if (mask == 0u) {
#pragma unroll
        for (unsigned i = 0u; i < 9u; i++) {
            f[i] = f_src[i * n + ys[i] * w + xs[i]];
        }
    } else {
#pragma unroll
        for (unsigned i = 0u; i < 9u; i++) {
            const unsigned src = ys[i] * w + xs[i];
            if (i == 0u || (mask & (1u << (i - 1u))) == 0u) {
                f[i] = f_src[i * n + src];
                continue;
            }
            const uint2 t = link_target(P, flags, bc_slot, x, y, i, src);
            const float out = f_src[OPPOSITE[i] * n + k]; // f̃*_ī(x)
            const BcParams b = bc[t.y];
            switch (t.x & FLOW_MASK) {
                // Bounce-back with wall velocity: f_i = f*_ī + 6 w_i ρ₀ c_i·u_w (ρ₀ = 1).
                case BOUNCE_BACK:
                    f[i] = out + 6.0f * W9[i] * dot_c(i, b.velocity);
                    break;
                // Anti-bounce-back with the outlet density and this cell's velocity (previous step).
                case PRESSURE: {
                    const float2 u = make_float2(mac[n + k], mac[2u * n + k]);
                    const float cu = dot_c(i, u);
                    const float usq = u.x * u.x + u.y * u.y;
                    f[i] = -out + 2.0f * W9[i] * (b.density * (1.0f + 4.5f * cu * cu - 1.5f * usq) - 1.0f);
                    break;
                }
                // Zero gradient along the outlet normal.
                default:
                    f[i] = f_src[i * n + inward_cell(P, xs[i], ys[i], t.x, k)];
                    break;
            }
        }
    }

    // ---- Moments and force ----
    float drho = 0.0f;
#pragma unroll
    for (unsigned i = 0u; i < 9u; i++) {
        drho += f[i];
    }
    const float rho = 1.0f + drho;
    const float jx = f[1] - f[3] + f[5] - f[6] - f[7] + f[8];
    const float jy = f[2] - f[4] + f[5] + f[6] - f[7] - f[8];
    const float2 force = make_float2(
        rho * P.body_force.x + P.buoyancy.x * theta,
        rho * P.body_force.y + P.buoyancy.y * theta
    );
    const float2 u = make_float2((jx + 0.5f * force.x) / rho, (jy + 0.5f * force.y) / rho);
    const float usq = u.x * u.x + u.y * u.y;
    const float uf = u.x * force.x + u.y * force.y;

    // ---- TRT collision with Guo forcing ----
    float moving = 0.0f;
    moving += collide_pair(P, f_dst, 1u, k, f[1], f[3], drho, rho, u, usq, force, uf);
    moving += collide_pair(P, f_dst, 2u, k, f[2], f[4], drho, rho, u, usq, force, uf);
    moving += collide_pair(P, f_dst, 5u, k, f[5], f[7], drho, rho, u, usq, force, uf);
    moving += collide_pair(P, f_dst, 6u, k, f[6], f[8], drho, rho, u, usq, force, uf);
    // Rest population by difference: Σ f̃* = Δρ exactly (the Guo source sums to zero).
    f_dst[k] = drho - moving;
    if (WRITE_MACRO) {
        mac[k] = rho;
    }
    if (WRITE_MACRO || mask != 0u) {
        mac[n + k] = u.x;
        mac[2u * n + k] = u.y;
    }
    // Divergence check on values read from memory (ADR-019): a NaN anywhere in a cell reaches its
    // rest population, so this flags a divergence one step after it appears.
    bool finite = finite_bits(f[0]);

    if (THERMAL) {
        // Rest population as the remainder: Σ g* = θ by construction.
        float moving_g = 0.0f;
        moving_g += collide_thermal_pair(P, g_dst, 1u, k, g[1], g[3], theta, u);
        moving_g += collide_thermal_pair(P, g_dst, 2u, k, g[2], g[4], theta, u);
        g_dst[k] = theta - moving_g;
        if (WRITE_MACRO) {
            mac[3u * n + k] = theta;
        }
        finite = finite && finite_bits(g[0]);
    }
    if (!finite) {
        mac[4u * n] = 1.0f;
    }
}

// Entry points (`extern "C"`: unmangled names the host looks up). The argument lists must match
// the launch wrappers in `src/raw.rs`.
extern "C" __global__ void stream_collide(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const float* __restrict__ g_src,
    float* __restrict__ g_dst,
    float* __restrict__ mac
) {
    step<false, true>(P, flags, bc_slot, bc, f_src, f_dst, g_src, g_dst, mac);
}

extern "C" __global__ void stream_collide_thermal(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const float* __restrict__ g_src,
    float* __restrict__ g_dst,
    float* __restrict__ mac
) {
    step<true, true>(P, flags, bc_slot, bc, f_src, f_dst, g_src, g_dst, mac);
}

// The `skip_macro` variants (no ρ/u/θ stores but the ones pressure outlets read).
extern "C" __global__ void stream_collide_skip(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const float* __restrict__ g_src,
    float* __restrict__ g_dst,
    float* __restrict__ mac
) {
    step<false, false>(P, flags, bc_slot, bc, f_src, f_dst, g_src, g_dst, mac);
}

extern "C" __global__ void stream_collide_thermal_skip(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const float* __restrict__ g_src,
    float* __restrict__ g_dst,
    float* __restrict__ mac
) {
    step<true, false>(P, flags, bc_slot, bc, f_src, f_dst, g_src, g_dst, mac);
}
