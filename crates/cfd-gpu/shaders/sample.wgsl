// Output sampling (design proposal §6): the physical output fields computed from the macroscopic
// planes, box-downsampled and written as f32 or f16 — the WGSL counterpart of
// `cfd-cuda/kernels/sample.cu`, itself a translation of `FieldSet::from_lattice` followed by
// `cfd_core::sample::downsample`. WGSL has no portable f64, so the conversions run in f32 with
// coefficients precomputed in f64 on the CPU (`cfd_lbm::abi::SampleParams`): results agree with
// the CPU to ~1e-7 relative, not bit for bit. The box filter sums in the CPU's order.
//
// Output: 32-bit words; field `i` (`S.fields[i]` = kind, components, first word, words) holds its
// values row-major, components interleaved, one f32 per word or two f16 per word
// (`pack2x16float`, low half first). One invocation per output word; `workgroup_id.z` = field.

// `cfd_lbm::abi::SampleParams` (176 bytes; layout checked against Rust by a test).
struct SampleParams {
    w: u32,
    h: u32,
    n: u32,
    periodic: u32,
    factor: u32,
    out_w: u32,
    out_h: u32,
    f16: u32,
    count: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
    // Per field: kind (SAMPLE_*), components, first output word, number of words.
    fields: array<vec4<u32>, 5>,
    velocity_factor: f32,
    pressure_factor: f32,
    rho0: f32,
    t_ref: f32,
    delta_t: f32,
    vorticity_scale: f32,
    hydro_a: f32,
    hydro_x: f32,
    hydro_y: f32,
    _pad3: f32,
    _pad4: f32,
    _pad5: f32,
}

const SAMPLE_WG: u32 = 128u;

@group(0) @binding(0) var<uniform> S: SampleParams;
@group(0) @binding(1) var<storage, read> cell_flags: array<u32>;
@group(0) @binding(2) var<storage, read> mac: array<f32>;
@group(0) @binding(3) var<storage, read_write> out: array<u32>;

fn sample_fluid(c: u32) -> bool {
    return (unpack_flags(cell_flags[c >> 1u], c) & FLOW_MASK) == FLUID;
}

// Cell at position `v` along the axis through (x, y).
fn axis_cell(along_x: bool, v: i32, x: u32, y: u32) -> u32 {
    if (along_x) {
        return y * S.w + u32(v);
    }
    return u32(v) * S.w + x;
}

// Derivative of macroscopic plane `plane` along x (`along_x`) or y at fluid cell (x, y), from fluid
// neighbours only: central, one-sided, or 0 (`vorticity`'s `deriv`).
fn sample_deriv(plane: u32, x: u32, y: u32, along_x: bool) -> f32 {
    let n = i32(select(S.h, S.w, along_x));
    let p = i32(select(y, x, along_x));
    let periodic = (S.periodic & select(2u, 1u, along_x)) != 0u;
    var q = array<i32, 2>(p - 1, p + 1);
    var ok = array<bool, 2>(false, false);
    for (var s = 0u; s < 2u; s = s + 1u) {
        var good = true;
        if (periodic) {
            q[s] = (q[s] % n + n) % n;
        } else {
            good = q[s] >= 0 && q[s] < n;
        }
        if (good) {
            good = sample_fluid(axis_cell(along_x, q[s], x, y));
        }
        ok[s] = good;
    }
    let base = plane * S.n;
    let here = mac[base + axis_cell(along_x, p, x, y)];
    if (ok[0] && ok[1]) {
        return (mac[base + axis_cell(along_x, q[1], x, y)] - mac[base + axis_cell(along_x, q[0], x, y)]) * 0.5;
    }
    if (ok[1]) {
        return mac[base + axis_cell(along_x, q[1], x, y)] - here;
    }
    if (ok[0]) {
        return here - mac[base + axis_cell(along_x, q[0], x, y)];
    }
    return 0.0;
}

// Component `comp` of field `kind` at cell (x, y), in physical units.
fn sample_cell(kind: u32, comp: u32, x: u32, y: u32) -> f32 {
    let n = S.n;
    let c = y * S.w + x;
    let fluid = sample_fluid(c);
    // One result and a single return: with a `return` in every case, DX12's FXC compiler (after
    // naga's translation to HLSL) rejects the function ("not all control paths return a value").
    var value = 0.0;
    if (kind == SAMPLE_TEMPERATURE) {
        let t = select(0.0, mac[3u * n + c], fluid);
        value = S.t_ref + t * S.delta_t;
    } else if (fluid) {
        switch (kind) {
            case SAMPLE_VELOCITY: {
                value = mac[(1u + comp) * n + c] / S.velocity_factor;
            }
            case SAMPLE_PRESSURE: {
                value = (mac[c] - 1.0) * S.pressure_factor + (S.hydro_a + S.hydro_x * f32(x) + S.hydro_y * f32(y));
            }
            case SAMPLE_DENSITY: {
                value = mac[c] * S.rho0;
            }
            default: { // SAMPLE_VORTICITY: ω = ∂v/∂x − ∂u/∂y
                value = (sample_deriv(2u, x, y, true) - sample_deriv(1u, x, y, false)) * S.vorticity_scale;
            }
        }
    }
    return value;
}

// Output value `v` (pixel v / components, component v % components) of field `kind`.
fn sample_value(kind: u32, comps: u32, v: u32) -> f32 {
    let pixel = v / comps;
    let comp = v % comps;
    let ox = pixel % S.out_w;
    let oy = pixel / S.out_w;
    let k = S.factor;
    if (k == 1u) {
        return sample_cell(kind, comp, ox, oy);
    }
    let x0 = ox * k;
    let y0 = oy * k;
    let x1 = min(x0 + k, S.w);
    let y1 = min(y0 + k, S.h);
    let cells = f32((x1 - x0) * (y1 - y0));
    var sum = 0.0;
    for (var y = y0; y < y1; y = y + 1u) {
        for (var x = x0; x < x1; x = x + 1u) {
            sum = sum + sample_cell(kind, comp, x, y);
        }
    }
    return sum / cells;
}

@compute @workgroup_size(128)
fn sample(
    @builtin(workgroup_id) wg: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
    @builtin(local_invocation_index) li: u32,
) {
    if (wg.z >= S.count) {
        return;
    }
    let field = S.fields[wg.z];
    let t = (wg.y * groups.x + wg.x) * SAMPLE_WG + li;
    if (t >= field.w) {
        return;
    }
    let kind = field.x;
    let comps = field.y;
    var word: u32;
    if (S.f16 != 0u) {
        let values = S.out_w * S.out_h * comps;
        let v = 2u * t;
        let lo = sample_value(kind, comps, v);
        var hi = 0.0;
        if (v + 1u < values) {
            hi = sample_value(kind, comps, v + 1u);
        }
        word = pack2x16float(vec2<f32>(lo, hi));
    } else {
        word = bitcast<u32>(sample_value(kind, comps, t));
    }
    out[field.z + t] = word;
}
