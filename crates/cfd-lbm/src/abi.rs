//! What the GPU kernels see: the per-run parameter block and the lattice constants, generated
//! from the Rust definitions so that no constant, flag bit or struct offset is typed twice.
//!
//! - [`KernelParams`]: the constants of a run, `#[repr(C)]`, 64 bytes. wgpu uploads it as a
//!   uniform buffer (`Params` in `cfd-gpu/shaders/common.wgsl`); CUDA passes it by value as a
//!   kernel argument (`Params` in `cfd-cuda/kernels/common.cuh`).
//! - [`wgsl_prelude`] and [`cuda_prelude`]: `LINK_X/Y`, `OPPOSITE`, `W9`, `W5` and the flag bits.
//! - [`cuda_layout_checks`]: `static_assert`s that make NVRTC reject the kernels if the CUDA
//!   structs ever stop matching [`KernelParams`] and [`BcParams`] (naga does the same check for
//!   WGSL in `cfd-gpu`'s tests).

use std::fmt::Write as _;
use std::mem::{offset_of, size_of};

use cfd_core::domain::{flags, BcParams, Domain, LINKS, OPPOSITE};

use crate::config::LbmConfig;
use crate::lattice::{W5, W9};
use crate::shared::relaxation_rates;

/// The constants of a run, as the kernels read them.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct KernelParams {
    pub w: u32,
    pub h: u32,
    /// Cells per population plane (`w · h`).
    pub n: u32,
    /// Bit 0: periodic along x; bit 1: periodic along y.
    pub periodic: u32,
    /// Flow relaxation rates (ω⁺, ω⁻).
    pub omega: [f32; 2],
    /// Thermal relaxation rates (ω⁺, ω⁻).
    pub omega_g: [f32; 2],
    /// Body acceleration.
    pub body_force: [f32; 2],
    /// Boussinesq force per unit θ.
    pub buoyancy: [f32; 2],
    /// Number of bounce-back links (forces).
    pub links: u32,
    pub _pad: [u32; 3],
}

impl KernelParams {
    pub fn new(d: &Domain, config: LbmConfig, body_force: [f32; 2], links: usize) -> Self {
        let (omega, omega_g) = relaxation_rates(d, config);
        Self {
            w: d.width,
            h: d.height,
            n: d.width * d.height,
            periodic: d.periodic[0] as u32 | (d.periodic[1] as u32) << 1,
            omega,
            omega_g,
            body_force,
            buoyancy: d.physics.buoyancy.map(|v| v as f32),
            links: links as u32,
            _pad: [0; 3],
        }
    }

    /// The 16 words of the struct (`f32` fields by their bits), for byte uploads.
    pub fn to_words(&self) -> [u32; 16] {
        let b = f32::to_bits;
        [
            self.w,
            self.h,
            self.n,
            self.periodic,
            b(self.omega[0]),
            b(self.omega[1]),
            b(self.omega_g[0]),
            b(self.omega_g[1]),
            b(self.body_force[0]),
            b(self.body_force[1]),
            b(self.buoyancy[0]),
            b(self.buoyancy[1]),
            self.links,
            self._pad[0],
            self._pad[1],
            self._pad[2],
        ]
    }
}

/// Byte offset of every named member of `KernelParams` (the names of the GPU structs).
pub fn params_fields() -> [(&'static str, usize); 9] {
    [
        ("w", offset_of!(KernelParams, w)),
        ("h", offset_of!(KernelParams, h)),
        ("n", offset_of!(KernelParams, n)),
        ("periodic", offset_of!(KernelParams, periodic)),
        ("omega", offset_of!(KernelParams, omega)),
        ("omega_g", offset_of!(KernelParams, omega_g)),
        ("body_force", offset_of!(KernelParams, body_force)),
        ("buoyancy", offset_of!(KernelParams, buoyancy)),
        ("links", offset_of!(KernelParams, links)),
    ]
}

/// Byte offset of every named member of `BcParams`.
pub fn bc_fields() -> [(&'static str, usize); 4] {
    [
        ("velocity", offset_of!(BcParams, velocity)),
        ("density", offset_of!(BcParams, density)),
        ("theta", offset_of!(BcParams, theta)),
        ("heat_flux", offset_of!(BcParams, heat_flux)),
    ]
}

/// The words of `BcParams` records as uploaded (8 per slot: velocity, density, θ, heat flux,
/// padding).
pub fn bc_words(bc: &[BcParams]) -> Vec<f32> {
    bc.iter()
        .flat_map(|p| {
            let [ux, uy] = p.velocity;
            [ux, uy, p.density, p.theta, p.heat_flux, 0.0, 0.0, 0.0]
        })
        .collect()
}

/// Flag bits shared with the kernels.
const FLAG_CONSTANTS: [(&str, u16); 14] = [
    ("FLOW_MASK", flags::FLOW_MASK),
    ("FLUID", flags::FLUID),
    ("BOUNCE_BACK", flags::BOUNCE_BACK),
    ("PRESSURE", flags::PRESSURE),
    ("ZERO_GRADIENT", flags::ZERO_GRADIENT),
    ("THERMAL_SHIFT", flags::THERMAL_SHIFT),
    ("THERMAL_MASK", flags::THERMAL_MASK),
    ("ADIABATIC", flags::ADIABATIC),
    ("FIXED_TEMPERATURE", flags::FIXED_TEMPERATURE),
    ("HEAT_FLUX", flags::HEAT_FLUX),
    ("THERMAL_ZERO_GRADIENT", flags::THERMAL_ZERO_GRADIENT),
    ("INWARD_SHIFT", flags::INWARD_SHIFT),
    ("INWARD_MASK", flags::INWARD_MASK),
    ("LINK_SHIFT", flags::LINK_SHIFT),
];

/// `f32` as a source literal (without suffix). Rust prints the shortest decimal that
/// round-trips, so the value on the GPU is bit-identical to the CPU constant.
pub fn float_literal(v: f32) -> String {
    let s = format!("{v:?}");
    if s.contains(['.', 'e']) {
        s
    } else {
        format!("{s}.0")
    }
}

/// The lattice arrays as `(name, element type, values)`, in a language-neutral form.
fn arrays() -> [(&'static str, Ty, Vec<String>); 5] {
    let ints = |v: [i32; 9]| v.iter().map(|x| x.to_string()).collect();
    [
        ("LINK_X", Ty::Int, ints(LINKS.map(|c| c[0]))),
        ("LINK_Y", Ty::Int, ints(LINKS.map(|c| c[1]))),
        (
            "OPPOSITE",
            Ty::Uint,
            OPPOSITE.iter().map(|o| o.to_string()).collect(),
        ),
        (
            "W9",
            Ty::Float,
            W9.iter().map(|&w| float_literal(w)).collect(),
        ),
        (
            "W5",
            Ty::Float,
            W5.iter().map(|&w| float_literal(w)).collect(),
        ),
    ]
}

#[derive(Clone, Copy)]
enum Ty {
    Int,
    Uint,
    Float,
}

/// The generated WGSL constants (followed by `common.wgsl` in every shader module).
pub fn wgsl_prelude() -> String {
    let mut s = String::from("// Generated by cfd-lbm/src/abi.rs from cfd-core and cfd-lbm.\n");
    for (name, ty, values) in arrays() {
        let (ty, suffix) = match ty {
            Ty::Int => ("i32", ""),
            Ty::Uint => ("u32", "u"),
            Ty::Float => ("f32", ""),
        };
        let items: Vec<String> = values.iter().map(|v| format!("{v}{suffix}")).collect();
        let _ = writeln!(
            s,
            "const {name} = array<{ty}, {}>({});",
            values.len(),
            items.join(", ")
        );
    }
    for (name, v) in FLAG_CONSTANTS {
        let _ = writeln!(s, "const {name}: u32 = {v}u;");
    }
    s
}

/// The generated CUDA C++ constants (followed by `common.cuh`). Arrays live in `__constant__`
/// memory: a lookup that is not resolved at compile time reads through the constant cache, which
/// broadcasts one value to a whole warp.
pub fn cuda_prelude() -> String {
    let mut s = String::from("// Generated by cfd-lbm/src/abi.rs from cfd-core and cfd-lbm.\n");
    for (name, ty, values) in arrays() {
        let (ty, suffix) = match ty {
            Ty::Int => ("int", ""),
            Ty::Uint => ("unsigned", "u"),
            Ty::Float => ("float", "f"),
        };
        let items: Vec<String> = values.iter().map(|v| format!("{v}{suffix}")).collect();
        let _ = writeln!(
            s,
            "__constant__ const {ty} {name}[{}] = {{{}}};",
            values.len(),
            items.join(", ")
        );
    }
    for (name, v) in FLAG_CONSTANTS {
        let _ = writeln!(s, "constexpr unsigned {name} = {v}u;");
    }
    s
}

/// `static_assert`s on the sizes and member offsets of the CUDA structs `Params` and `BcParams`,
/// with the values taken from the Rust structs (appended after `common.cuh`).
pub fn cuda_layout_checks() -> String {
    let mut s = String::from("// Generated by cfd-lbm/src/abi.rs: the Rust layouts.\n");
    let mut check = |ty: &str, size: usize, fields: &[(&str, usize)]| {
        let _ = writeln!(
            s,
            "static_assert(sizeof({ty}) == {size}, \"{ty}: size differs from Rust\");"
        );
        for (f, off) in fields {
            let _ = writeln!(
                s,
                "static_assert(__builtin_offsetof({ty}, {f}) == {off}, \"{ty}.{f}: offset differs from Rust\");"
            );
        }
    };
    check("Params", size_of::<KernelParams>(), &params_fields());
    check("BcParams", size_of::<BcParams>(), &bc_fields());
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floats_round_trip() {
        for v in W9.into_iter().chain(W5) {
            assert_eq!(float_literal(v).parse::<f32>().unwrap(), v);
        }
        assert_eq!(float_literal(1.0), "1.0");
    }

    #[test]
    fn params_are_64_bytes_of_words() {
        assert_eq!(size_of::<KernelParams>(), 64);
        let p = KernelParams {
            w: 3,
            body_force: [0.5, -0.25],
            links: 7,
            ..Default::default()
        };
        let words = p.to_words();
        assert_eq!(words[0], 3);
        assert_eq!(f32::from_bits(words[9]), -0.25);
        assert_eq!(words[12], 7);
        // `to_words` follows the field offsets.
        for (name, off) in params_fields() {
            assert_eq!(off % 4, 0, "{name}");
        }
        assert_eq!(params_fields()[8], ("links", 48));
    }

    #[test]
    fn wgsl_prelude_has_every_constant() {
        let p = wgsl_prelude();
        assert!(p.contains("const LINK_X = array<i32, 9>(0, 1, 0, -1, 0, 1, -1, -1, 1);"));
        assert!(p.contains("const OPPOSITE = array<u32, 9>(0u, 3u, 4u, 1u, 2u, 7u, 8u, 5u, 6u);"));
        assert!(p.contains("const LINK_SHIFT: u32 = 8u;"));
    }

    #[test]
    fn cuda_prelude_has_every_constant() {
        let p = cuda_prelude();
        assert!(p.contains("__constant__ const int LINK_Y[9] = {0, 0, 1, 0, -1, 1, 1, -1, -1};"));
        assert!(p.contains(
            "__constant__ const unsigned OPPOSITE[9] = {0u, 3u, 4u, 1u, 2u, 7u, 8u, 5u, 6u};"
        ));
        assert!(p.contains("__constant__ const float W5[5] = {0.33333334f, 0.16666667f,"));
        assert!(p.contains("constexpr unsigned INWARD_MASK = 224u;"));
        let c = cuda_layout_checks();
        assert!(c.contains("static_assert(sizeof(Params) == 64,"));
        assert!(c.contains("__builtin_offsetof(BcParams, heat_flux) == 16"));
    }
}
