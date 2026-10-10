// Types and helpers shared by the kernels (appended after the generated prelude).

// Constants of a run; the Rust side is `lbm::params_words` (16 × 4 bytes).
struct Params {
    w: u32,
    h: u32,
    // Cells per population plane (w · h).
    n: u32,
    // Bit 0: periodic along x; bit 1: periodic along y.
    periodic: u32,
    // Flow relaxation rates (ω⁺, ω⁻).
    omega: vec2<f32>,
    // Thermal relaxation rates (ω⁺, ω⁻).
    omega_g: vec2<f32>,
    // Body acceleration.
    body_force: vec2<f32>,
    // Boussinesq force per unit θ.
    buoyancy: vec2<f32>,
    // Number of bounce-back links (forces).
    links: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

// `cfd_core::domain::BcParams`, `#[repr(C)]`, 32 bytes.
struct BcParams {
    velocity: vec2<f32>,
    density: f32,
    theta: f32,
    heat_flux: f32,
    _pad0: f32,
    _pad1: f32,
    _pad2: f32,
}

// The `u16` flags are uploaded as the bytes of the CPU's `Vec<u16>`: two cells per word, the
// even cell in the low half (little-endian).
fn unpack_flags(word: u32, cell: u32) -> u32 {
    return (word >> ((cell & 1u) << 4u)) & 0xffffu;
}

// Not NaN or ±Inf (exponent bits not all set). Only reliable on values loaded from memory: on
// arithmetic results compilers may fold it to `true`, like `x != x`.
fn is_finite(v: f32) -> bool {
    return (bitcast<u32>(v) & 0x7f800000u) != 0x7f800000u;
}

// c_i · v
fn dot_c(i: u32, v: vec2<f32>) -> f32 {
    return f32(LINK_X[i]) * v.x + f32(LINK_Y[i]) * v.y;
}
