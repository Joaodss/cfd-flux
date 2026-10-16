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

// One sampling launch: `cfd_lbm::abi::SampleParams` (176 bytes), passed by value.
struct SampleParams {
    unsigned w;
    unsigned h;
    unsigned n;
    unsigned periodic;
    // Box-filter factor (≥ 1) and output size.
    unsigned factor;
    unsigned out_w;
    unsigned out_h;
    // 1: f16 output (two values per word), 0: f32.
    unsigned f16;
    unsigned count;
    unsigned _pad0[3];
    // Per field: kind (SAMPLE_*), components, first output word, number of words.
    uint4 fields[5];
    // f32 coefficients (used by the WGSL kernel; CUDA uses `SampleCoefficients`).
    float velocity_factor;
    float pressure_factor;
    float rho0;
    float t_ref;
    float delta_t;
    float vorticity_scale;
    float hydro_a;
    float hydro_x;
    float hydro_y;
    float _pad1[3];
};

// `cfd_lbm::abi::SampleCoefficients`: the f64 coefficients of `FieldSet::from_lattice` (96 bytes).
struct SampleCoefficients {
    double velocity_factor;
    double cs2;
    double rho0;
    double velocity_factor2;
    double dx;
    double gravity_x;
    double gravity_y;
    double reference_x;
    double reference_y;
    double t_ref;
    double delta_t;
    double vorticity_scale;
};
