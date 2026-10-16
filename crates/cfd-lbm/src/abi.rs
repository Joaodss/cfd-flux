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
use cfd_core::sample::{
    components, OutputCoefficients, Precision, Sample, SampleData, SampleField, SampleSpec,
};
use cfd_core::scene::OutputField;

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

/// Fields the sampling kernels can produce (`SampleParams::fields[i][0]`).
pub fn sample_kind(field: OutputField) -> Option<u32> {
    Some(match field {
        OutputField::Velocity => 0,
        OutputField::Pressure => 1,
        OutputField::Density => 2,
        OutputField::Temperature => 3,
        OutputField::Vorticity => 4,
        OutputField::FillFraction => return None,
    })
}

const SAMPLE_KINDS: [(&str, OutputField); 5] = [
    ("SAMPLE_VELOCITY", OutputField::Velocity),
    ("SAMPLE_PRESSURE", OutputField::Pressure),
    ("SAMPLE_DENSITY", OutputField::Density),
    ("SAMPLE_TEMPERATURE", OutputField::Temperature),
    ("SAMPLE_VORTICITY", OutputField::Vorticity),
];

/// Most fields one sample can hold (every kind once).
pub const MAX_SAMPLE_FIELDS: usize = SAMPLE_KINDS.len();

/// One sampling launch (a [`SampleSpec`]): sizes, the output layout and the `f32` conversion
/// coefficients, `#[repr(C)]`, 176 bytes. wgpu uploads it as a uniform (`SampleParams` in
/// `cfd-gpu/shaders/sample.wgsl`); CUDA passes it by value (`cfd-cuda/kernels/sample.cu`), with
/// the `f64` coefficients in [`SampleCoefficients`].
///
/// The output is one buffer of 32-bit words: each field starts at a word offset and holds
/// `out_w · out_h · components` values, as `f32` or as pairs of f16 per word (the last word of
/// an odd count padded with 0).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct SampleParams {
    pub w: u32,
    pub h: u32,
    pub n: u32,
    /// Bit 0: periodic along x; bit 1: periodic along y.
    pub periodic: u32,
    /// Box-filter factor (≥ 1).
    pub factor: u32,
    pub out_w: u32,
    pub out_h: u32,
    /// 1: f16 output, 0: f32.
    pub f16: u32,
    /// Number of fields (entries of `fields`).
    pub count: u32,
    pub _pad0: [u32; 3],
    /// Per field: kind ([`sample_kind`]), components, first output word, number of words.
    pub fields: [[u32; 4]; MAX_SAMPLE_FIELDS],
    /// `dt / dx` (velocity: `u_lb / velocity_factor`).
    pub velocity_factor: f32,
    /// `c_s² ρ₀ / velocity_factor²` (gauge pressure per unit of `ρ − 1`).
    pub pressure_factor: f32,
    pub rho0: f32,
    pub t_ref: f32,
    pub delta_t: f32,
    /// `1 / dt`.
    pub vorticity_scale: f32,
    /// Hydrostatic pressure at cell `(x, y)`: `hydro_a + hydro_x · x + hydro_y · y`.
    pub hydro_a: f32,
    pub hydro_x: f32,
    pub hydro_y: f32,
    pub _pad1: [f32; 3],
}

impl SampleParams {
    /// The 44 words of the struct (`f32` fields by their bits), for byte uploads.
    pub fn to_words(&self) -> Vec<u32> {
        let mut w = vec![
            self.w,
            self.h,
            self.n,
            self.periodic,
            self.factor,
            self.out_w,
            self.out_h,
            self.f16,
            self.count,
        ];
        w.extend(self._pad0);
        w.extend(self.fields.iter().flatten());
        w.extend(
            [
                self.velocity_factor,
                self.pressure_factor,
                self.rho0,
                self.t_ref,
                self.delta_t,
                self.vorticity_scale,
                self.hydro_a,
                self.hydro_x,
                self.hydro_y,
            ]
            .map(f32::to_bits),
        );
        w.extend(self._pad1.map(f32::to_bits));
        w
    }
}

/// The `f64` coefficients of [`FieldSet::from_lattice`](cfd_core::solver::FieldSet::from_lattice),
/// for the CUDA kernel, which reproduces it bit for bit (96 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
#[repr(C)]
pub struct SampleCoefficients {
    pub velocity_factor: f64,
    pub cs2: f64,
    pub rho0: f64,
    pub velocity_factor2: f64,
    pub dx: f64,
    pub gravity_x: f64,
    pub gravity_y: f64,
    pub reference_x: f64,
    pub reference_y: f64,
    pub t_ref: f64,
    pub delta_t: f64,
    pub vorticity_scale: f64,
}

impl SampleCoefficients {
    pub fn new(c: &OutputCoefficients) -> Self {
        Self {
            velocity_factor: c.velocity_factor,
            cs2: c.cs2,
            rho0: c.rho0,
            velocity_factor2: c.velocity_factor2,
            dx: c.dx,
            gravity_x: c.gravity[0],
            gravity_y: c.gravity[1],
            reference_x: c.reference[0],
            reference_y: c.reference[1],
            t_ref: c.t_ref,
            delta_t: c.delta_t,
            vorticity_scale: c.vorticity_scale,
        }
    }
}

/// The layout of one sample in the output buffer of the sampling kernels.
#[derive(Debug, Clone, PartialEq)]
pub struct SampleLayout {
    pub params: SampleParams,
    /// The fields produced, in order (the spec's fields the domain can produce).
    pub fields: Vec<OutputField>,
    /// Total 32-bit words of the output.
    pub words: usize,
}

impl SampleLayout {
    pub fn new(d: &Domain, spec: &SampleSpec) -> Self {
        let c = OutputCoefficients::new(d);
        let fields = spec.resolved(d.physics.thermal());
        let (out_w, out_h) = spec.out_size(d.width, d.height);
        let f16 = spec.precision == Precision::F16;
        let mut p = SampleParams {
            w: d.width,
            h: d.height,
            n: d.width * d.height,
            periodic: d.periodic[0] as u32 | (d.periodic[1] as u32) << 1,
            factor: spec.factor.max(1),
            out_w,
            out_h,
            f16: f16 as u32,
            count: fields.len() as u32,
            velocity_factor: c.velocity_factor as f32,
            pressure_factor: (c.cs2 * c.rho0 / c.velocity_factor2) as f32,
            rho0: c.rho0 as f32,
            t_ref: c.t_ref as f32,
            delta_t: c.delta_t as f32,
            vorticity_scale: c.vorticity_scale as f32,
            hydro_a: (c.rho0
                * (c.gravity[0] * (0.5 * c.dx - c.reference[0])
                    + c.gravity[1] * (0.5 * c.dx - c.reference[1]))) as f32,
            hydro_x: (c.rho0 * c.gravity[0] * c.dx) as f32,
            hydro_y: (c.rho0 * c.gravity[1] * c.dx) as f32,
            ..Default::default()
        };
        let mut words = 0usize;
        for (i, &f) in fields.iter().enumerate() {
            let comps = components(f) as usize;
            let values = out_w as usize * out_h as usize * comps;
            let len = if f16 { values.div_ceil(2) } else { values };
            p.fields[i] = [
                sample_kind(f).expect("resolved fields have a kind"),
                comps as u32,
                words as u32,
                len as u32,
            ];
            words += len;
        }
        Self {
            params: p,
            fields,
            words,
        }
    }

    /// Splits the downloaded words into a [`Sample`].
    pub fn sample(&self, spec: &SampleSpec, step: u64, time: f64, words: &[u32]) -> Sample {
        let p = &self.params;
        let fields = self
            .fields
            .iter()
            .zip(&p.fields)
            .map(|(&field, &[_, comps, offset, len])| {
                let raw = &words[offset as usize..(offset + len) as usize];
                let values = (p.out_w * p.out_h * comps) as usize;
                let data = if p.f16 == 1 {
                    let mut v: Vec<u16> = bytemuck::cast_slice(raw).to_vec();
                    v.truncate(values);
                    SampleData::F16(v)
                } else {
                    SampleData::F32(bytemuck::cast_slice(raw).to_vec())
                };
                SampleField {
                    field,
                    components: comps as u8,
                    data,
                }
            })
            .collect();
        Sample {
            spec: spec.clone(),
            step,
            time,
            width: p.out_w,
            height: p.out_h,
            fields,
        }
    }
}

/// Byte offset of every named member of `SampleParams`.
pub fn sample_params_fields() -> [(&'static str, usize); 19] {
    [
        ("w", offset_of!(SampleParams, w)),
        ("h", offset_of!(SampleParams, h)),
        ("n", offset_of!(SampleParams, n)),
        ("periodic", offset_of!(SampleParams, periodic)),
        ("factor", offset_of!(SampleParams, factor)),
        ("out_w", offset_of!(SampleParams, out_w)),
        ("out_h", offset_of!(SampleParams, out_h)),
        ("f16", offset_of!(SampleParams, f16)),
        ("count", offset_of!(SampleParams, count)),
        ("fields", offset_of!(SampleParams, fields)),
        ("velocity_factor", offset_of!(SampleParams, velocity_factor)),
        ("pressure_factor", offset_of!(SampleParams, pressure_factor)),
        ("rho0", offset_of!(SampleParams, rho0)),
        ("t_ref", offset_of!(SampleParams, t_ref)),
        ("delta_t", offset_of!(SampleParams, delta_t)),
        ("vorticity_scale", offset_of!(SampleParams, vorticity_scale)),
        ("hydro_a", offset_of!(SampleParams, hydro_a)),
        ("hydro_x", offset_of!(SampleParams, hydro_x)),
        ("hydro_y", offset_of!(SampleParams, hydro_y)),
    ]
}

/// Byte offset of every member of `SampleCoefficients`.
pub fn sample_coefficients_fields() -> [(&'static str, usize); 12] {
    [
        (
            "velocity_factor",
            offset_of!(SampleCoefficients, velocity_factor),
        ),
        ("cs2", offset_of!(SampleCoefficients, cs2)),
        ("rho0", offset_of!(SampleCoefficients, rho0)),
        (
            "velocity_factor2",
            offset_of!(SampleCoefficients, velocity_factor2),
        ),
        ("dx", offset_of!(SampleCoefficients, dx)),
        ("gravity_x", offset_of!(SampleCoefficients, gravity_x)),
        ("gravity_y", offset_of!(SampleCoefficients, gravity_y)),
        ("reference_x", offset_of!(SampleCoefficients, reference_x)),
        ("reference_y", offset_of!(SampleCoefficients, reference_y)),
        ("t_ref", offset_of!(SampleCoefficients, t_ref)),
        ("delta_t", offset_of!(SampleCoefficients, delta_t)),
        (
            "vorticity_scale",
            offset_of!(SampleCoefficients, vorticity_scale),
        ),
    ]
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
    for (name, f) in SAMPLE_KINDS {
        let _ = writeln!(
            s,
            "const {name}: u32 = {}u;",
            sample_kind(f).expect("a kind")
        );
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
    for (name, f) in SAMPLE_KINDS {
        let _ = writeln!(
            s,
            "constexpr unsigned {name} = {}u;",
            sample_kind(f).expect("a kind")
        );
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
    check(
        "SampleParams",
        size_of::<SampleParams>(),
        &sample_params_fields(),
    );
    check(
        "SampleCoefficients",
        size_of::<SampleCoefficients>(),
        &sample_coefficients_fields(),
    );
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
        assert!(c.contains("static_assert(sizeof(SampleParams) == 176,"));
        assert!(c.contains("static_assert(sizeof(SampleCoefficients) == 96,"));
        assert!(p.contains("constexpr unsigned SAMPLE_VORTICITY = 4u;"));
    }

    #[test]
    fn sample_params_are_words_in_member_order() {
        assert_eq!(size_of::<SampleParams>(), 176);
        let p = SampleParams {
            count: 2,
            fields: [[1, 2, 3, 4]; MAX_SAMPLE_FIELDS],
            velocity_factor: 0.5,
            hydro_y: -2.0,
            ..Default::default()
        };
        let words = p.to_words();
        assert_eq!(words.len(), 44);
        let at = |name: &str| {
            sample_params_fields()
                .iter()
                .find(|f| f.0 == name)
                .unwrap()
                .1
                / 4
        };
        assert_eq!(words[at("count")], 2);
        assert_eq!(words[at("fields") + 5], 2);
        assert_eq!(f32::from_bits(words[at("velocity_factor")]), 0.5);
        assert_eq!(f32::from_bits(words[at("hydro_y")]), -2.0);
    }

    #[test]
    fn sample_layouts_pack_fields_in_order() {
        let d = Domain::from_scene(
            &cfd_core::examples::channel(),
            &cfd_core::DomainOptions::default(),
        )
        .unwrap();
        let spec = SampleSpec {
            fields: vec![
                OutputField::Pressure,
                OutputField::Temperature,
                OutputField::Velocity,
            ],
            factor: 3,
            precision: Precision::F16,
        };
        let l = SampleLayout::new(&d, &spec);
        let (ow, oh) = (d.width.div_ceil(3), d.height.div_ceil(3));
        // No thermal lattice in the channel: temperature is skipped.
        assert_eq!(l.fields, [OutputField::Pressure, OutputField::Velocity]);
        let p_words = (ow * oh).div_ceil(2);
        assert_eq!(l.params.fields[0], [1, 1, 0, p_words]);
        assert_eq!(l.params.fields[1], [0, 2, p_words, ow * oh]);
        assert_eq!(l.words, (p_words + ow * oh) as usize);
        // Splitting the output drops the padding half-word of an odd count.
        let s = l.sample(&spec, 7, 0.5, &vec![0x3c00_3c00; l.words]);
        assert_eq!(s.fields[0].data.len(), (ow * oh) as usize);
        assert_eq!(s.fields[0].data.to_f32()[0], 1.0);
    }
}
