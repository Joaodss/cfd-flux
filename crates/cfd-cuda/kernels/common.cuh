// Types and helpers shared by the kernels (appended after the generated prelude; the generated
// `static_assert`s that follow this file check both structs against the Rust ones).

// Constants of a run: `cfd_lbm::abi::KernelParams` (64 bytes), passed by value to every kernel.
// Kernel arguments live in the constant bank, so reading them costs nothing and there is no
// uniform buffer to keep up to date.
struct Params {
    unsigned w;
    unsigned h;
    // Cells per population plane (w · h).
    unsigned n;
    // Bit 0: periodic along x; bit 1: periodic along y.
    unsigned periodic;
    // Flow relaxation rates (ω⁺, ω⁻).
    float2 omega;
    // Thermal relaxation rates (ω⁺, ω⁻).
    float2 omega_g;
    // Body acceleration.
    float2 body_force;
    // Boussinesq force per unit θ.
    float2 buoyancy;
    // Number of bounce-back links (forces).
    unsigned links;
    unsigned _pad0, _pad1, _pad2;
};

// `cfd_core::domain::BcParams`, `#[repr(C)]`, 32 bytes.
struct BcParams {
    float2 velocity;
    float density;
    float theta;
    float heat_flux;
    float _pad0, _pad1, _pad2;
};

// Not NaN or ±Inf (exponent bits not all set). Applied to values loaded from memory, as in the
// WGSL kernels (ADR-019), so a divergence is reported at the same step on every GPU backend.
__device__ __forceinline__ bool finite_bits(float v) {
    return (__float_as_uint(v) & 0x7f800000u) != 0x7f800000u;
}

// c_i · v
__device__ __forceinline__ float dot_c(unsigned i, float2 v) {
    return float(LINK_X[i]) * v.x + float(LINK_Y[i]) * v.y;
}
