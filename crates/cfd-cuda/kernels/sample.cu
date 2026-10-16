// Output sampling (design proposal §6): the physical output fields computed from the macroscopic
// planes, box-downsampled and written as f32 or f16 — a translation of
// `cfd_core::solver::FieldSet::from_lattice` followed by `cfd_core::sample::downsample`, with the
// same operations in the same order:
//
// - the conversions run in **f64**, as on the CPU, and round to f32 at the end (`(float)` of a
//   double rounds to nearest, like Rust's `as f32`); with `--fmad=false` no multiply-add is fused
//   (that holds for doubles too), so every value is bit-identical to the CPU's;
// - the box filter sums f32 values in row order and divides by the cell count, as `downsample`;
// - f16 is rounded to nearest-even (`cvt.rn.f16.f32`, what `__float2half_rn` compiles to, written
//   as inline PTX so the program needs no `cuda_fp16.h`).
//
// The output is a buffer of 32-bit words; field `i` (`P.fields[i]` = kind, components, first word,
// words) holds its values row-major, components interleaved, one f32 per word or two f16 per word
// (low half first). One thread per output word, `blockIdx.y` = field.

__device__ __forceinline__ bool sample_fluid(const unsigned short* __restrict__ flags, unsigned c) {
    return (flags[c] & FLOW_MASK) == FLUID;
}

// Derivative of the plane `f` along x (`along_x`) or y at fluid cell (x, y), using fluid neighbours
// only: central where both exist, one-sided where one does, 0 otherwise (`vorticity`'s `deriv`).
__device__ double sample_deriv(
    const SampleParams& P, const unsigned short* __restrict__ flags, const float* __restrict__ f,
    unsigned x, unsigned y, bool along_x
) {
    const int n = int(along_x ? P.w : P.h);
    const int p = int(along_x ? x : y);
    const bool periodic = (P.periodic & (along_x ? 1u : 2u)) != 0u;
    int q[2];
    bool ok[2];
#pragma unroll
    for (int s = 0; s < 2; s++) {
        int v = p + (s == 0 ? -1 : 1);
        if (periodic) {
            v = (v % n + n) % n;
            ok[s] = true;
        } else {
            ok[s] = v >= 0 && v < n;
        }
        if (ok[s]) {
            const unsigned qx = along_x ? unsigned(v) : x;
            const unsigned qy = along_x ? y : unsigned(v);
            ok[s] = sample_fluid(flags, qy * P.w + qx);
        }
        q[s] = v;
    }
    const auto val = [&](int v) -> double {
        const unsigned qx = along_x ? unsigned(v) : x;
        const unsigned qy = along_x ? y : unsigned(v);
        return double(f[qy * P.w + qx]);
    };
    if (ok[0] && ok[1]) {
        return (val(q[1]) - val(q[0])) / 2.0;
    }
    if (ok[1]) {
        return val(q[1]) - val(p);
    }
    if (ok[0]) {
        return val(p) - val(q[0]);
    }
    return 0.0;
}

// Component `comp` of field `kind` at cell (x, y), in physical units.
__device__ float sample_cell(
    const SampleParams& P, const SampleCoefficients& C, const unsigned short* __restrict__ flags,
    const float* __restrict__ mac, unsigned kind, unsigned comp, unsigned x, unsigned y
) {
    const unsigned n = P.n;
    const unsigned c = y * P.w + x;
    const bool fluid = sample_fluid(flags, c);
    switch (kind) {
        case SAMPLE_VELOCITY:
            return fluid ? float(double(mac[(1u + comp) * n + c]) / C.velocity_factor) : 0.0f;
        case SAMPLE_PRESSURE: {
            if (!fluid) {
                return 0.0f;
            }
            const double p = (double(mac[c]) - 1.0) * C.cs2 * C.rho0 / C.velocity_factor2;
            const double rx = (double(x) + 0.5) * C.dx - C.reference_x;
            const double ry = (double(y) + 0.5) * C.dx - C.reference_y;
            return float(p + C.rho0 * (C.gravity_x * rx + C.gravity_y * ry));
        }
        case SAMPLE_DENSITY:
            return fluid ? float(double(mac[c]) * C.rho0) : 0.0f;
        case SAMPLE_TEMPERATURE: {
            const double t = fluid ? double(mac[3u * n + c]) : 0.0;
            return float(C.t_ref + t * C.delta_t);
        }
        default: { // SAMPLE_VORTICITY: ω = ∂v/∂x − ∂u/∂y
            if (!fluid) {
                return 0.0f;
            }
            const double omega = sample_deriv(P, flags, mac + 2u * n, x, y, true)
                - sample_deriv(P, flags, mac + n, x, y, false);
            return float(omega * C.vorticity_scale);
        }
    }
}

// Output value `v` (pixel v / components, component v % components) of field `kind`.
__device__ float sample_value(
    const SampleParams& P, const SampleCoefficients& C, const unsigned short* __restrict__ flags,
    const float* __restrict__ mac, unsigned kind, unsigned comps, unsigned v
) {
    const unsigned pixel = v / comps;
    const unsigned comp = v % comps;
    const unsigned ox = pixel % P.out_w;
    const unsigned oy = pixel / P.out_w;
    const unsigned k = P.factor;
    if (k == 1u) {
        return sample_cell(P, C, flags, mac, kind, comp, ox, oy);
    }
    const unsigned x0 = ox * k;
    const unsigned y0 = oy * k;
    const unsigned x1 = min(x0 + k, P.w);
    const unsigned y1 = min(y0 + k, P.h);
    const float cells = float((x1 - x0) * (y1 - y0));
    float sum = 0.0f;
    for (unsigned y = y0; y < y1; y++) {
        for (unsigned x = x0; x < x1; x++) {
            sum += sample_cell(P, C, flags, mac, kind, comp, x, y);
        }
    }
    return sum / cells;
}

__device__ __forceinline__ unsigned f16_bits(float v) {
    unsigned short h;
    asm("cvt.rn.f16.f32 %0, %1;" : "=h"(h) : "f"(v));
    return h;
}

extern "C" __global__ void sample(
    const SampleParams P,
    const SampleCoefficients C,
    const unsigned short* __restrict__ flags,
    const float* __restrict__ mac,
    unsigned* __restrict__ out
) {
    if (blockIdx.y >= P.count) {
        return;
    }
    const uint4 field = P.fields[blockIdx.y];
    const unsigned t = blockIdx.x * blockDim.x + threadIdx.x;
    if (t >= field.w) {
        return;
    }
    const unsigned kind = field.x;
    const unsigned comps = field.y;
    unsigned word;
    if (P.f16 != 0u) {
        const unsigned values = P.out_w * P.out_h * comps;
        const unsigned v = 2u * t;
        const unsigned lo = f16_bits(sample_value(P, C, flags, mac, kind, comps, v));
        const unsigned hi = v + 1u < values ? f16_bits(sample_value(P, C, flags, mac, kind, comps, v + 1u)) : 0u;
        word = lo | (hi << 16);
    } else {
        word = __float_as_uint(sample_value(P, C, flags, mac, kind, comps, t));
    }
    out[field.z + t] = word;
}
