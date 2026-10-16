//! Output samples: physical fields, optionally box-downsampled, in f32 or f16, ready to be
//! written into a frame (design proposal §6).
//!
//! A [`SampleSpec`] says what to produce; [`Solver::request_samples`](crate::Solver::request_samples)
//! starts producing several of them at once (the stored frame and one per stream format) and
//! returns a [`PendingSamples`] handle. GPU backends compute the samples on the device and copy
//! only the packed output; the default implementation copies the lattice fields and converts
//! them on the CPU with [`Sample::from_lattice`], which is also the reference the GPU kernels are
//! tested against.

use crate::domain::Domain;
use crate::scene::OutputField;
use crate::solver::{FieldSet, LatticeFields, PendingFields, SampleRequest};
use crate::units::CS2;

/// Precision of the sampled values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Precision {
    F32,
    /// IEEE binary16, rounded to nearest-even.
    F16,
}

/// What to sample.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SampleSpec {
    /// In output order; fields the domain cannot produce are skipped (see [`Self::resolved`]).
    pub fields: Vec<OutputField>,
    /// Box-filter factor (≥ 1): the output has `ceil(w / k) × ceil(h / k)` values per component.
    pub factor: u32,
    pub precision: Precision,
}

impl SampleSpec {
    /// Every value at full resolution.
    pub fn full(fields: Vec<OutputField>, precision: Precision) -> Self {
        Self {
            fields,
            factor: 1,
            precision,
        }
    }

    /// The fields actually produced: temperature only with the thermal lattice; no fill fraction
    /// (free surface, Phase 6) yet.
    pub fn resolved(&self, thermal: bool) -> Vec<OutputField> {
        self.fields
            .iter()
            .copied()
            .filter(|f| match f {
                OutputField::Temperature => thermal,
                OutputField::FillFraction => false,
                _ => true,
            })
            .collect()
    }

    /// Output size for a `w × h` grid.
    pub fn out_size(&self, w: u32, h: u32) -> (u32, u32) {
        let k = self.factor.max(1);
        (w.div_ceil(k), h.div_ceil(k))
    }
}

/// Components per value of an output field.
pub fn components(field: OutputField) -> u8 {
    match field {
        OutputField::Velocity => 2,
        _ => 1,
    }
}

/// Sampled values, as they go into a frame.
#[derive(Debug, Clone, PartialEq)]
pub enum SampleData {
    F32(Vec<f32>),
    /// f16 bit patterns.
    F16(Vec<u16>),
}

impl SampleData {
    pub fn len(&self) -> usize {
        match self {
            SampleData::F32(v) => v.len(),
            SampleData::F16(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The values as `f32` (f16 converted exactly).
    pub fn to_f32(&self) -> Vec<f32> {
        match self {
            SampleData::F32(v) => v.clone(),
            SampleData::F16(v) => v
                .iter()
                .map(|&b| half::f16::from_bits(b).to_f32())
                .collect(),
        }
    }

    /// The little-endian bytes of the values (no copy).
    pub fn bytes(&self) -> &[u8] {
        const _: () = assert!(cfg!(target_endian = "little"));
        match self {
            SampleData::F32(v) => bytemuck::cast_slice(v),
            SampleData::F16(v) => bytemuck::cast_slice(v),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SampleField {
    pub field: OutputField,
    pub components: u8,
    /// Row-major from the bottom-left corner, components interleaved.
    pub data: SampleData,
}

/// One sample: the fields of a [`SampleSpec`] at one step.
#[derive(Debug, Clone, PartialEq)]
pub struct Sample {
    pub spec: SampleSpec,
    pub step: u64,
    /// Physical time (s).
    pub time: f64,
    /// Output size (after downsampling).
    pub width: u32,
    pub height: u32,
    pub fields: Vec<SampleField>,
}

impl Sample {
    /// The reference implementation: [`FieldSet::from_lattice`], then [`downsample`], then
    /// round-to-nearest-even f16 if asked.
    pub fn from_lattice(domain: &Domain, lf: &LatticeFields, step: u64, spec: &SampleSpec) -> Self {
        let req = SampleRequest {
            fields: spec.fields.clone(),
        };
        Self::from_field_set(&FieldSet::from_lattice(domain, lf, step, &req), spec)
    }

    /// Picks, downsamples and converts the fields of `spec` from a full-resolution field set
    /// (fields `fs` lacks are skipped).
    pub fn from_field_set(fs: &FieldSet, spec: &SampleSpec) -> Self {
        let (w, h) = (fs.width as usize, fs.height as usize);
        let k = spec.factor.max(1) as usize;
        let (ow, oh) = spec.out_size(fs.width, fs.height);
        let fields = spec
            .fields
            .iter()
            .filter_map(|&field| fs.fields.iter().find(|f| f.field == field))
            .map(|f| {
                let values = downsample(&f.values, f.components as usize, w, h, k);
                SampleField {
                    field: f.field,
                    components: f.components,
                    data: match spec.precision {
                        Precision::F32 => SampleData::F32(values),
                        Precision::F16 => SampleData::F16(f16_bits(&values)),
                    },
                }
            })
            .collect();
        Sample {
            spec: spec.clone(),
            step: fs.step,
            time: fs.time,
            width: ow,
            height: oh,
            fields,
        }
    }

    pub fn field(&self, field: OutputField) -> Option<&SampleField> {
        self.fields.iter().find(|f| f.field == field)
    }
}

/// f32 → f16 bits, round-to-nearest-even (vectorised with F16C when the CPU has it).
pub fn f16_bits(values: &[f32]) -> Vec<u16> {
    use half::slice::HalfFloatSliceExt as _;
    let mut h = vec![half::f16::ZERO; values.len()];
    h.convert_from_f32_slice(values);
    h.reinterpret_cast().to_vec()
}

/// Averages `k × k` blocks (partial blocks at the right and top edges average what they have):
/// an `f32` sum in row order, divided by the number of cells. `values` are row-major with `c`
/// interleaved components. The GPU sampling kernels use the same order of operations.
pub fn downsample(values: &[f32], c: usize, w: usize, h: usize, k: usize) -> Vec<f32> {
    if k <= 1 {
        return values.to_vec();
    }
    let (ow, oh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = vec![0.0f32; ow * oh * c];
    for oy in 0..oh {
        for ox in 0..ow {
            let (x0, y0) = (ox * k, oy * k);
            let (x1, y1) = ((x0 + k).min(w), (y0 + k).min(h));
            let n = ((x1 - x0) * (y1 - y0)) as f32;
            for comp in 0..c {
                let mut sum = 0.0f32;
                for y in y0..y1 {
                    for x in x0..x1 {
                        sum += values[(y * w + x) * c + comp];
                    }
                }
                out[(oy * ow + ox) * c + comp] = sum / n;
            }
        }
    }
    out
}

/// The coefficients of [`FieldSet::from_lattice`]'s conversions, computed exactly as it does
/// (for kernels that must reproduce it bit for bit, in f64).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OutputCoefficients {
    /// `dt / dx`: lattice velocity per physical velocity.
    pub velocity_factor: f64,
    pub cs2: f64,
    pub rho0: f64,
    /// `velocity_factor²`.
    pub velocity_factor2: f64,
    pub dx: f64,
    pub gravity: [f64; 2],
    /// Point (m) where the hydrostatic pressure is zero.
    pub reference: [f64; 2],
    pub t_ref: f64,
    pub delta_t: f64,
    /// `1 / dt`: lattice vorticity (per step) → 1/s.
    pub vorticity_scale: f64,
}

impl OutputCoefficients {
    pub fn new(domain: &Domain) -> Self {
        let u = &domain.units;
        let vf = u.velocity_factor();
        Self {
            velocity_factor: vf,
            cs2: CS2,
            rho0: u.rho0,
            velocity_factor2: vf.powi(2),
            dx: u.dx,
            gravity: domain.hydrostatic.gravity,
            reference: domain.hydrostatic.reference,
            t_ref: u.t_ref,
            delta_t: u.delta_t,
            vorticity_scale: 1.0 / u.dt,
        }
    }
}

/// What a [`PendingSamples`] hands back: samples computed on the device, or lattice fields still
/// to be converted — on the caller's output thread, so the solver thread never does it.
pub enum SampleSet {
    Ready(Vec<Sample>),
    Lattice {
        step: u64,
        fields: LatticeFields,
        specs: Vec<SampleSpec>,
    },
}

impl SampleSet {
    /// One sample per requested spec, in request order.
    pub fn into_samples(self, domain: &Domain) -> Vec<Sample> {
        match self {
            SampleSet::Ready(samples) => samples,
            SampleSet::Lattice {
                step,
                fields,
                specs,
            } => {
                // Convert every field any spec wants once, then pick per spec.
                let mut all: Vec<OutputField> = Vec::new();
                for f in specs.iter().flat_map(|s| &s.fields) {
                    if !all.contains(f) {
                        all.push(*f);
                    }
                }
                let fs =
                    FieldSet::from_lattice(domain, &fields, step, &SampleRequest { fields: all });
                specs
                    .iter()
                    .map(|s| Sample::from_field_set(&fs, s))
                    .collect()
            }
        }
    }
}

/// Samples being produced (see [`Solver::request_samples`](crate::Solver::request_samples)),
/// in C# terms a `Task<SampleSet>`.
pub trait PendingSamples: Send {
    /// Number of steps done when the samples were requested.
    fn step(&self) -> u64;

    /// Waits for the device's copy (not for any CPU conversion: see [`SampleSet`]).
    fn wait(self: Box<Self>) -> SampleSet;
}

/// The default: a copy of the lattice fields, converted later by [`SampleSet::into_samples`].
pub struct LatticeSamples {
    pub pending: Box<dyn PendingFields>,
    pub specs: Vec<SampleSpec>,
}

impl PendingSamples for LatticeSamples {
    fn step(&self) -> u64 {
        self.pending.step()
    }

    fn wait(self: Box<Self>) -> SampleSet {
        let step = self.pending.step();
        SampleSet::Lattice {
            step,
            fields: self.pending.wait(),
            specs: self.specs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downsampling_averages_blocks_and_partial_edges() {
        // 3 × 2 grid, 1 component, k = 2 → 2 × 1.
        let v = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        assert_eq!(downsample(&v, 1, 3, 2, 2), [3.0, 4.5]);
        // Two components stay interleaved.
        let v2 = [1.0, 10.0, 3.0, 30.0];
        assert_eq!(downsample(&v2, 2, 2, 1, 2), [2.0, 20.0]);
        assert_eq!(downsample(&v, 1, 3, 2, 1), v);
    }

    #[test]
    fn f16_rounds_to_nearest_even() {
        // 1 + 2⁻¹¹ is halfway between 1 and 1 + 2⁻¹⁰: ties go to the even mantissa (1.0).
        let half_ulp = 1.0 + 2f32.powi(-11);
        let above = 1.0 + 3.0 * 2f32.powi(-11); // halfway between 1+2⁻¹⁰ and 1+2⁻⁹ → 1+2⁻⁹
        assert_eq!(
            f16_bits(&[half_ulp, above, 65520.0]),
            [0x3c00, 0x3c02, 0x7c00]
        );
    }

    #[test]
    fn specs_skip_fields_the_domain_cannot_produce() {
        let s = SampleSpec::full(
            vec![
                OutputField::Temperature,
                OutputField::Velocity,
                OutputField::FillFraction,
            ],
            Precision::F16,
        );
        assert_eq!(s.resolved(false), [OutputField::Velocity]);
        assert_eq!(s.resolved(true).len(), 2);
        let k3 = SampleSpec { factor: 3, ..s };
        assert_eq!(k3.out_size(10, 9), (4, 3));
    }
}
