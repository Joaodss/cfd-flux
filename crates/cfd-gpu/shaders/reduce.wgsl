// Reductions and gathers that feed `diagnostics()` and `forces()`. Each entry point uses its own
// bindings, so its pipeline layout is derived automatically.

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> cell_flags: array<u32>;
@group(0) @binding(2) var<storage, read> mac: array<f32>;
// Per workgroup: fluid cells, Σ(ρ − 1), ½Σρ|u|², max |u|, Σθ, non-finite flag, 2 × padding.
@group(0) @binding(3) var<storage, read_write> partials: array<f32>;

// Bounce-back links: `cell << 4 | direction`, in the order of `cfd_lbm::wall_links`.
@group(0) @binding(4) var<storage, read> links: array<u32>;
// Post-collision populations of the last step.
@group(0) @binding(5) var<storage, read> f_cur: array<f32>;
// Per link: the population f̃*_ī leaving the fluid cell towards the wall.
@group(0) @binding(6) var<storage, read_write> link_out: array<f32>;

const RWG: u32 = 256u;
const PARTIALS: u32 = 8u;

var<workgroup> sh_cells: array<f32, RWG>;
var<workgroup> sh_mass: array<f32, RWG>;
var<workgroup> sh_energy: array<f32, RWG>;
var<workgroup> sh_umax: array<f32, RWG>;
var<workgroup> sh_heat: array<f32, RWG>;
var<workgroup> sh_bad: array<f32, RWG>;

// Each invocation accumulates a strided subset of the cells in f32, then the workgroup combines
// its 256 values with a tree in shared memory; the CPU adds the per-workgroup partials in f64.
// Summing ρ − 1 instead of ρ keeps the f32 partial sums accurate to ~1e-7 of the mass deviation.
@compute @workgroup_size(RWG)
fn diagnostics(
    @builtin(local_invocation_index) li: u32,
    @builtin(workgroup_id) wid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let n = P.n;
    var cells = 0.0;
    var mass = 0.0;
    var energy = 0.0;
    var umax = 0.0;
    var heat = 0.0;
    var bad = 0.0;
    for (var c = wid.x * RWG + li; c < n; c += groups.x * RWG) {
        if ((unpack_flags(cell_flags[c >> 1u], c) & FLOW_MASK) != FLUID) {
            continue;
        }
        let r = mac[c];
        let x = mac[n + c];
        let y = mac[2u * n + c];
        let t = mac[3u * n + c];
        let usq = x * x + y * y;
        cells += 1.0;
        mass += r - 1.0;
        energy += 0.5 * r * usq;
        umax = max(umax, sqrt(usq));
        heat += t;
        // Tested on loaded values (see `stream_collide`).
        if (!(is_finite(r) && is_finite(x) && is_finite(y) && is_finite(t))) {
            bad = 1.0;
        }
    }
    sh_cells[li] = cells;
    sh_mass[li] = mass;
    sh_energy[li] = energy;
    sh_umax[li] = umax;
    sh_heat[li] = heat;
    sh_bad[li] = bad;
    for (var s = RWG / 2u; s > 0u; s >>= 1u) {
        workgroupBarrier();
        if (li < s) {
            sh_cells[li] += sh_cells[li + s];
            sh_mass[li] += sh_mass[li + s];
            sh_energy[li] += sh_energy[li + s];
            sh_umax[li] = max(sh_umax[li], sh_umax[li + s]);
            sh_heat[li] += sh_heat[li + s];
            sh_bad[li] = max(sh_bad[li], sh_bad[li + s]);
        }
    }
    if (li == 0u) {
        let o = wid.x * PARTIALS;
        partials[o] = sh_cells[0];
        partials[o + 1u] = sh_mass[0];
        partials[o + 2u] = sh_energy[0];
        partials[o + 3u] = sh_umax[0];
        partials[o + 4u] = sh_heat[0];
        partials[o + 5u] = sh_bad[0];
    }
}

@compute @workgroup_size(RWG)
fn gather_links(@builtin(global_invocation_id) gid: vec3<u32>) {
    if (gid.x >= P.links) {
        return;
    }
    let l = links[gid.x];
    link_out[gid.x] = f_cur[OPPOSITE[l & 15u] * P.n + (l >> 4u)];
}
