// Reductions and gathers that feed `diagnostics()` and `forces()`; the same outputs as
// `cfd-gpu/shaders/reduce.wgsl`, so the host code that finishes them is shared.

constexpr unsigned RBLOCK = 256u;
constexpr unsigned WARPS = RBLOCK / 32u;
// Values per block in `partials`: fluid cells, Σ(ρ − 1), ½Σρ|u|², max |u|, Σθ, non-finite flag,
// 2 × padding.
constexpr unsigned PARTIALS = 8u;

// A warp is 32 threads that execute in lockstep. `__shfl_down_sync` reads a register of the lane
// `offset` places higher, without going through memory: after the five steps (16, 8, 4, 2, 1)
// lane 0 holds the combination of all 32 values. WGSL can only do this with the subgroups
// extension; `reduce.wgsl` uses a tree in workgroup memory with a barrier per level instead.
__device__ __forceinline__ float warp_sum(float v) {
#pragma unroll
    for (int offset = 16; offset > 0; offset >>= 1) {
        v += __shfl_down_sync(0xffffffffu, v, offset);
    }
    return v;
}

__device__ __forceinline__ float warp_max(float v) {
#pragma unroll
    for (int offset = 16; offset > 0; offset >>= 1) {
        v = fmaxf(v, __shfl_down_sync(0xffffffffu, v, offset));
    }
    return v;
}

// Each thread accumulates a strided subset of the cells in f32; each warp combines its 32 values
// with shuffles; the 8 warp results go through shared memory to the first warp, which combines
// them again; the host adds the per-block partials in f64. Summing ρ − 1 instead of ρ keeps the
// f32 partial sums accurate to ~1e-7 of the mass deviation.
extern "C" __global__ void __launch_bounds__(RBLOCK) diagnostics(
    const Params P,
    const unsigned short* __restrict__ flags,
    const float* __restrict__ mac,
    float* __restrict__ partials
) {
    const unsigned n = P.n;
    const unsigned li = threadIdx.x;
    float v[6] = {0.0f, 0.0f, 0.0f, 0.0f, 0.0f, 0.0f}; // cells, mass, energy, umax, heat, bad
    for (unsigned c = blockIdx.x * RBLOCK + li; c < n; c += gridDim.x * RBLOCK) {
        if ((flags[c] & FLOW_MASK) != FLUID) {
            continue;
        }
        const float r = mac[c];
        const float x = mac[n + c];
        const float y = mac[2u * n + c];
        const float t = mac[3u * n + c];
        const float usq = x * x + y * y;
        v[0] += 1.0f;
        v[1] += r - 1.0f;
        v[2] += 0.5f * r * usq;
        v[3] = fmaxf(v[3], sqrtf(usq));
        v[4] += t;
        // Tested on loaded values (see `lbm.cu`).
        if (!(finite_bits(r) && finite_bits(x) && finite_bits(y) && finite_bits(t))) {
            v[5] = 1.0f;
        }
    }
    __shared__ float warp_values[6][WARPS];
    const unsigned lane = li % 32u;
    const unsigned warp = li / 32u;
#pragma unroll
    for (int q = 0; q < 6; q++) {
        v[q] = (q == 3 || q == 5) ? warp_max(v[q]) : warp_sum(v[q]);
        if (lane == 0u) {
            warp_values[q][warp] = v[q];
        }
    }
    __syncthreads();
    if (warp == 0u) {
#pragma unroll
        for (int q = 0; q < 6; q++) {
            float w = lane < WARPS ? warp_values[q][lane] : 0.0f;
            w = (q == 3 || q == 5) ? warp_max(w) : warp_sum(w);
            if (lane == 0u) {
                partials[blockIdx.x * PARTIALS + q] = w;
            }
        }
    }
}

// Bounce-back links: `cell << 4 | direction`, in the order of `cfd_lbm::shared::wall_links`. Per
// link, the population f̃*_ī leaving the fluid cell towards the wall after the last step.
extern "C" __global__ void gather_links(
    const Params P,
    const unsigned* __restrict__ links,
    const float* __restrict__ f_cur,
    float* __restrict__ link_out
) {
    const unsigned l = blockIdx.x * blockDim.x + threadIdx.x;
    if (l >= P.links) {
        return;
    }
    const unsigned link = links[l];
    link_out[l] = f_cur[OPPOSITE[link & 15u] * P.n + (link >> 4u)];
}
