// Free surface (ADR-032): pass 1 (stream-collide with the mass exchange) and passes 2–4 (cell
// conversion), line-by-line translations of `cfd-lbm/src/kernel.rs` (`FREE_SURFACE`) and
// `cfd-lbm/src/surface.rs`, with the same order of floating-point operations, so that with
// `--fmad=false` every value is bit-identical to the CPU reference.
//
// The free-surface planes, `n` values each: `state_a` (the step's input and output), `state_b`
// (between the passes), `mass`, `massex` (undivided excess, its receiver count in the state) and
// `phi`. Every pass is a per-cell gather: it reads only buffers it does not write, or its own cell
// of a buffer it writes, so there are no races between neighbouring threads (FluidX3D updates its
// flags in place instead, which the CUDA memory model leaves undefined).
//
// Across a link i the neighbour of cell x is x − c_i, the source of population i; boundary links
// (the link mask of the static flags) have none.

__device__ __forceinline__ unsigned st_phase(unsigned s) {
    return s & PHASE_MASK;
}

__device__ __forceinline__ unsigned st_mark(unsigned s) {
    return (s & MARK_MASK) >> MARK_SHIFT;
}

__device__ __forceinline__ unsigned st_receivers(unsigned s) {
    return (s & COUNT_MASK) >> COUNT_SHIFT;
}

__device__ __forceinline__ unsigned st_word(unsigned phase, unsigned mark, unsigned receivers) {
    return phase | (mark << MARK_SHIFT) | (receivers << COUNT_SHIFT);
}

__device__ __forceinline__ bool is_wet(unsigned phase) {
    return phase == PHASE_INTERFACE || phase == PHASE_LIQUID;
}

// The phase once the pending conversion is done.
__device__ __forceinline__ unsigned st_settled(unsigned s) {
    switch (st_mark(s)) {
        case TO_LIQUID:
            return PHASE_LIQUID;
        case TO_GAS:
            return PHASE_GAS;
        case TO_INTERFACE:
            return PHASE_INTERFACE;
        default:
            return st_phase(s);
    }
}

// Neighbour x − c_i of cell k = (x, y) for each link i = 1…8, or `NO_CELL` across a boundary link.
constexpr unsigned NO_CELL = 0xffffffffu;

__device__ __forceinline__ void neighbours(const Params& P, unsigned fl, unsigned k, unsigned* nb) {
    const unsigned w = P.w;
    const unsigned x = k % w;
    const unsigned y = k / w;
    const unsigned xm = x == 0u ? w - 1u : x - 1u;
    const unsigned xp = x + 1u == w ? 0u : x + 1u;
    const unsigned ym = y == 0u ? P.h - 1u : y - 1u;
    const unsigned yp = y + 1u == P.h ? 0u : y + 1u;
    const unsigned xs[9] = {x, xm, x, xp, x, xm, xp, xp, xm};
    const unsigned ys[9] = {y, y, ym, y, yp, ym, ym, yp, yp};
    const unsigned mask = fl >> LINK_SHIFT;
#pragma unroll
    for (unsigned i = 1u; i < 9u; i++) {
        nb[i] = (mask & (1u << (i - 1u))) == 0u ? ys[i] * w + xs[i] : NO_CELL;
    }
}

// Shifted D2Q9 equilibrium f̃ᵢ^eq (`cfd_lbm::kernel::equilibrium`).
__device__ __forceinline__ float equilibrium_i(unsigned i, float rho, float2 u, float usq) {
    const float cu = dot_c(i, u);
    return W9[i] * (rho - 1.0f + rho * (3.0f * cu + 4.5f * cu * cu - 1.5f * usq));
}

// Velocity for the gas-side reconstruction of interface cell k: from its own post-collision
// populations plus the Guo half-force shift (`outgoing_velocity`).
__device__ __forceinline__ float2 outgoing_velocity(const Params& P, const float* __restrict__ f_src, unsigned k) {
    const unsigned n = P.n;
    float fo[9];
    float sum = 0.0f;
#pragma unroll
    for (unsigned i = 0u; i < 9u; i++) {
        fo[i] = f_src[i * n + k];
        sum += fo[i];
    }
    const float rho = 1.0f + sum;
    const float jx = fo[1] - fo[3] + fo[5] - fo[6] - fo[7] + fo[8];
    const float jy = fo[2] - fo[4] + fo[5] + fo[6] - fo[7] - fo[8];
    return make_float2(
        (jx + 0.5f * rho * P.body_force.x) / rho,
        (jy + 0.5f * rho * P.body_force.y) / rho
    );
}

// Pass 1. Gas cells copy their phase into `state_b` and return; liquid and interface cells pull
// (rebuilding the populations that come from gas), add the mass exchange and the neighbours'
// shares of excess to their mass, collide, and interface cells are marked IF or IG. ρ is stored on
// every liquid and interface cell, u on interface cells (the conversion passes read them).
template <bool WRITE_MACRO>
__device__ __forceinline__ void step_fs(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const unsigned* __restrict__ state_a,
    unsigned* __restrict__ state_b,
    float* __restrict__ mass,
    const float* __restrict__ massex,
    const float* __restrict__ phi,
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
    const unsigned own = st_phase(state_a[k]);
    if (own == PHASE_GAS) {
        state_b[k] = own;
        return;
    }
    const unsigned w = P.w;
    const unsigned n = P.n;
    const unsigned x = k % w;
    const unsigned y = k / w;
    const unsigned xm = x == 0u ? w - 1u : x - 1u;
    const unsigned xp = x + 1u == w ? 0u : x + 1u;
    const unsigned ym = y == 0u ? P.h - 1u : y - 1u;
    const unsigned yp = y + 1u == P.h ? 0u : y + 1u;
    const unsigned xs[9] = {x, xm, x, xp, x, xm, xp, xp, xm};
    const unsigned ys[9] = {y, y, ym, y, yp, ym, ym, yp, yp};
    const unsigned mask = fl >> LINK_SHIFT;

    // ---- Pull, mass exchange ----
    float f[9];
    float dm = 0.0f;
    bool gas_neighbour = false;
    bool liquid_neighbour = false;
    bool have_u_gas = false;
    float2 u_gas = make_float2(0.0f, 0.0f);
    const float phi_k = phi[k];
    f[0] = f_src[k];
#pragma unroll
    for (unsigned i = 1u; i < 9u; i++) {
        const unsigned src = ys[i] * w + xs[i];
        const float out = f_src[OPPOSITE[i] * n + k]; // f̃*_ī(x)
        if ((mask & (1u << (i - 1u))) != 0u) {
            const uint2 t = link_target(P, flags, bc_slot, x, y, i, src);
            const BcParams b = bc[t.y];
            switch (t.x & FLOW_MASK) {
                case BOUNCE_BACK:
                    f[i] = out + 6.0f * W9[i] * dot_c(i, b.velocity);
                    break;
                case PRESSURE: {
                    const float2 u = make_float2(mac[n + k], mac[2u * n + k]);
                    const float cu = dot_c(i, u);
                    const float usq = u.x * u.x + u.y * u.y;
                    f[i] = -out + 2.0f * W9[i] * (b.density * (1.0f + 4.5f * cu * cu - 1.5f * usq) - 1.0f);
                    break;
                }
                default:
                    f[i] = f_src[i * n + inward_cell(P, xs[i], ys[i], t.x, k)];
                    break;
            }
            continue;
        }
        const unsigned nst = state_a[src];
        const unsigned np = st_phase(nst);
        if (np == PHASE_GAS) {
            // The gas imposes ρ_G = 1 and the interface's velocity.
            gas_neighbour = true;
            if (!have_u_gas) {
                u_gas = outgoing_velocity(P, f_src, k);
                have_u_gas = true;
            }
            const float cu = dot_c(i, u_gas);
            const float usq = u_gas.x * u_gas.x + u_gas.y * u_gas.y;
            f[i] = -out + 2.0f * W9[i] * (4.5f * cu * cu - 1.5f * usq);
        } else {
            const float fin = f_src[i * n + src];
            f[i] = fin;
            liquid_neighbour = liquid_neighbour || np == PHASE_LIQUID;
            const float weight = own == PHASE_LIQUID || np == PHASE_LIQUID ? 1.0f : 0.5f * (phi_k + phi[src]);
            dm += weight * (fin - out);
        }
        const unsigned receivers = st_receivers(nst);
        if (receivers > 0u) {
            dm += massex[src] / float(receivers);
        }
    }

    // ---- Moments, force, collision (as `step` in lbm.cu, without the thermal lattice) ----
    float drho = 0.0f;
#pragma unroll
    for (unsigned i = 0u; i < 9u; i++) {
        drho += f[i];
    }
    const float rho = 1.0f + drho;
    const float jx = f[1] - f[3] + f[5] - f[6] - f[7] + f[8];
    const float jy = f[2] - f[4] + f[5] + f[6] - f[7] - f[8];
    const float theta = 0.0f;
    const float2 force = make_float2(
        rho * P.body_force.x + P.buoyancy.x * theta,
        rho * P.body_force.y + P.buoyancy.y * theta
    );
    const float2 u = make_float2((jx + 0.5f * force.x) / rho, (jy + 0.5f * force.y) / rho);
    const float usq = u.x * u.x + u.y * u.y;
    const float uf = u.x * force.x + u.y * force.y;
    const float2 om = relaxation(P, f, drho, rho, u);
    float moving = 0.0f;
    moving += collide_pair(P, f_dst, 1u, k, f[1], f[3], drho, rho, u, usq, force, uf, om.x, om.y);
    moving += collide_pair(P, f_dst, 2u, k, f[2], f[4], drho, rho, u, usq, force, uf, om.x, om.y);
    moving += collide_pair(P, f_dst, 5u, k, f[5], f[7], drho, rho, u, usq, force, uf, om.x, om.y);
    moving += collide_pair(P, f_dst, 6u, k, f[6], f[8], drho, rho, u, usq, force, uf, om.x, om.y);
    f_dst[k] = drho - moving;
    mac[k] = rho;
    if (WRITE_MACRO || mask != 0u || own == PHASE_INTERFACE) {
        mac[n + k] = u.x;
        mac[2u * n + k] = u.y;
    }

    // ---- Mass and conversion marks ----
    const float m = mass[k] + dm;
    mass[k] = m;
    unsigned mark = MARK_NONE;
    if (own == PHASE_INTERFACE) {
        if (m > (1.0f + KAPPA) * rho || !gas_neighbour) {
            mark = TO_LIQUID;
        } else if (m < -KAPPA * rho || !liquid_neighbour) {
            mark = TO_GAS;
        }
    }
    state_b[k] = st_word(own, mark, 0u);
    if (!finite_bits(f[0])) {
        mac[4u * n] = 1.0f;
    }
}

extern "C" __global__ void stream_collide_fs(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const unsigned* __restrict__ state_a,
    unsigned* __restrict__ state_b,
    float* __restrict__ mass,
    const float* __restrict__ massex,
    const float* __restrict__ phi,
    float* __restrict__ mac
) {
    step_fs<true>(P, flags, bc_slot, bc, f_src, f_dst, state_a, state_b, mass, massex, phi, mac);
}

extern "C" __global__ void stream_collide_fs_skip(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ bc_slot,
    const BcParams* __restrict__ bc,
    const float* __restrict__ f_src,
    float* __restrict__ f_dst,
    const unsigned* __restrict__ state_a,
    unsigned* __restrict__ state_b,
    float* __restrict__ mass,
    const float* __restrict__ massex,
    const float* __restrict__ phi,
    float* __restrict__ mac
) {
    step_fs<false>(P, flags, bc_slot, bc, f_src, f_dst, state_a, state_b, mass, massex, phi, mac);
}

// Pass 2 (`surface::fill`), `state_b` → `state_a`: gas next to an IF cell becomes GI; an IG next to
// an IF cell stays interface.
extern "C" __global__ void surface_fill(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ src,
    unsigned* __restrict__ out
) {
    const unsigned k = blockIdx.x * blockDim.x + threadIdx.x;
    if (k >= P.n) {
        return;
    }
    const unsigned st = src[k];
    const unsigned ph = st_phase(st);
    const bool candidate = ph == PHASE_GAS || st_mark(st) == TO_GAS;
    const unsigned fl = flags[k];
    unsigned result = st;
    if (candidate && (fl & FLOW_MASK) == FLUID) {
        unsigned nb[9];
        neighbours(P, fl, k, nb);
        bool filling = false;
#pragma unroll
        for (unsigned i = 1u; i < 9u; i++) {
            filling = filling || (nb[i] != NO_CELL && st_mark(src[nb[i]]) == TO_LIQUID);
        }
        if (filling) {
            result = ph == PHASE_GAS ? st_word(ph, TO_INTERFACE, 0u) : st_word(ph, MARK_NONE, 0u);
        }
    }
    out[k] = result;
}

// Pass 3 (`surface::init_states`, `init_value`, `init_populations`), `state_a` → `state_b`: liquid
// next to an IG becomes interface; a GI cell starts at the equilibrium of its liquid and interface
// neighbours' mean ρ and u (not of those being emptied), summed in link order. GI cells write their
// own populations and macroscopic values only, and read only those of wet cells, which are never
// GI: no thread reads what another writes.
extern "C" __global__ void surface_init(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ src,
    unsigned* __restrict__ out,
    float* __restrict__ f_dst,
    float* __restrict__ mac
) {
    const unsigned k = blockIdx.x * blockDim.x + threadIdx.x;
    if (k >= P.n) {
        return;
    }
    const unsigned n = P.n;
    const unsigned st = src[k];
    const unsigned fl = flags[k];
    unsigned result = st;
    if ((fl & FLOW_MASK) == FLUID) {
        const unsigned ph = st_phase(st);
        if (ph == PHASE_LIQUID) {
            unsigned nb[9];
            neighbours(P, fl, k, nb);
            bool emptying = false;
#pragma unroll
            for (unsigned i = 1u; i < 9u; i++) {
                emptying = emptying || (nb[i] != NO_CELL && st_mark(src[nb[i]]) == TO_GAS);
            }
            if (emptying) {
                result = st_word(PHASE_INTERFACE, MARK_NONE, 0u);
            }
        } else if (st_mark(st) == TO_INTERFACE) {
            unsigned nb[9];
            neighbours(P, fl, k, nb);
            float r = 0.0f;
            float vx = 0.0f;
            float vy = 0.0f;
            unsigned count = 0u;
#pragma unroll
            for (unsigned i = 1u; i < 9u; i++) {
                if (nb[i] == NO_CELL) {
                    continue;
                }
                const unsigned j = nb[i];
                const unsigned sj = src[j];
                if (is_wet(st_phase(sj)) && st_mark(sj) != TO_GAS) {
                    r += mac[j];
                    vx += mac[n + j];
                    vy += mac[2u * n + j];
                    count += 1u;
                }
            }
            float rho = 1.0f;
            float2 u = make_float2(0.0f, 0.0f);
            if (count > 0u) {
                const float c = float(count);
                rho = r / c;
                u = make_float2(vx / c, vy / c);
            }
            const float usq = u.x * u.x + u.y * u.y;
#pragma unroll
            for (unsigned i = 0u; i < 9u; i++) {
                f_dst[i * n + k] = equilibrium_i(i, rho, u, usq);
            }
            mac[k] = rho;
            mac[n + k] = u.x;
            mac[2u * n + k] = u.y;
        }
    }
    out[k] = result;
}

// Pass 4 (`surface::finish`), `state_b` → `state_a`: IF → liquid, IG → gas, GI → interface; the
// excess mass is set aside undivided for the next step, with the number of liquid and interface
// neighbours that share it (counted only when there is an excess); without receivers the cell keeps
// it in its mass.
extern "C" __global__ void surface_finish(
    const Params P,
    const unsigned short* __restrict__ flags,
    const unsigned* __restrict__ src,
    const float* __restrict__ mac,
    unsigned* __restrict__ out,
    float* __restrict__ mass,
    float* __restrict__ massex,
    float* __restrict__ phi
) {
    const unsigned k = blockIdx.x * blockDim.x + threadIdx.x;
    if (k >= P.n) {
        return;
    }
    const unsigned st = src[k];
    const unsigned fl = flags[k];
    if (st_phase(st) == PHASE_SOLID || (fl & FLOW_MASK) != FLUID) {
        out[k] = st;
        return;
    }
    const unsigned settled = st_settled(st);
    const float r = mac[k];
    const float m0 = mass[k];
    float m;
    float excess;
    float fill;
    if (settled == PHASE_LIQUID) {
        m = r;
        excess = m0 - r;
        fill = 1.0f;
    } else if (settled == PHASE_INTERFACE) {
        excess = m0 > r ? m0 - r : (m0 < 0.0f ? m0 : 0.0f);
        // Rust's `clamp(0, ρ)`.
        m = m0 < 0.0f ? 0.0f : m0;
        m = m > r ? r : m;
        fill = m / r;
    } else {
        m = 0.0f;
        excess = m0;
        fill = 0.0f;
    }
    unsigned receivers = 0u;
    if (excess != 0.0f) {
        unsigned nb[9];
        neighbours(P, fl, k, nb);
#pragma unroll
        for (unsigned i = 1u; i < 9u; i++) {
            if (nb[i] != NO_CELL && is_wet(st_settled(src[nb[i]]))) {
                receivers += 1u;
            }
        }
    }
    if (receivers == 0u) {
        m += excess;
        excess = 0.0f;
    }
    out[k] = st_word(settled, MARK_NONE, receivers);
    mass[k] = m;
    massex[k] = excess;
    phi[k] = fill;
}

// Diagnostics of a free-surface run, per block (`diagnostics` in reduce.cu): wet cells,
// Σ(m + m_ex − φ), ½Σρ|u|² and max |u| over wet cells, Σφ, non-finite flag, interface cells,
// padding. The liquid mass is Σ(m + m_ex − φ) + Σφ, each in f64 on the host: the f32 partial sums
// then keep their digits (m + m_ex − φ is small on most cells).
extern "C" __global__ void __launch_bounds__(RBLOCK) diagnostics_fs(
    const Params P,
    const unsigned short* __restrict__ flags,
    const float* __restrict__ mac,
    const unsigned* __restrict__ state,
    const float* __restrict__ mass,
    const float* __restrict__ massex,
    const float* __restrict__ phi,
    float* __restrict__ partials
) {
    const unsigned n = P.n;
    const unsigned li = threadIdx.x;
    float v[7] = {0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f};
    for (unsigned c = blockIdx.x * RBLOCK + li; c < n; c += gridDim.x * RBLOCK) {
        if ((flags[c] & FLOW_MASK) != FLUID) {
            continue;
        }
        const float m = mass[c];
        const float e = massex[c];
        const float f = phi[c];
        v[1] += m + e - f;
        v[4] += f;
        const unsigned p = st_phase(state[c]);
        v[6] += p == PHASE_INTERFACE ? 1.0f : 0.0f;
        if (!(finite_bits(m) && finite_bits(e))) {
            v[5] = 1.0f;
        }
        if (!is_wet(p)) {
            continue;
        }
        const float r = mac[c];
        const float x = mac[n + c];
        const float y = mac[2u * n + c];
        const float usq = x * x + y * y;
        v[0] += 1.0f;
        v[2] += 0.5f * r * usq;
        v[3] = fmaxf(v[3], sqrtf(usq));
        if (!(finite_bits(r) && finite_bits(x) && finite_bits(y))) {
            v[5] = 1.0f;
        }
    }
    __shared__ float warp_values[7][WARPS];
    const unsigned lane = li % 32u;
    const unsigned warp = li / 32u;
#pragma unroll
    for (int q = 0; q < 7; q++) {
        v[q] = (q == 3 || q == 5) ? warp_max(v[q]) : warp_sum(v[q]);
        if (lane == 0u) {
            warp_values[q][warp] = v[q];
        }
    }
    __syncthreads();
    if (warp == 0u) {
#pragma unroll
        for (int q = 0; q < 7; q++) {
            float w = lane < WARPS ? warp_values[q][lane] : 0.0f;
            w = (q == 3 || q == 5) ? warp_max(w) : warp_sum(w);
            if (lane == 0u) {
                partials[blockIdx.x * PARTIALS + q] = w;
            }
        }
    }
}
