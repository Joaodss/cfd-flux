// Free surface (ADR-032): pass 1 (stream-collide with the mass exchange) and passes 2–4 (cell
// conversion), line-by-line translations of `cfd-lbm/src/kernel.rs` (`FREE_SURFACE`) and
// `cfd-lbm/src/surface.rs` (and of `cfd-cuda/kernels/surface.cu`), with the same order of
// floating-point operations. The driver may still fuse multiply-adds, and WGSL's division and
// square root are not correctly rounded, so the results agree with the CPU's to round-off.
//
// `surf` holds five planes of `n` words (`cfd_lbm::surface::plane`): `state_a` (the step's input
// and output), `state_b` (between the passes), `mass`, `massex` (undivided excess, its receiver
// count in the state) and `phi`; the `f32` planes are stored by their bits (WGSL has no unions).
// Every pass is a per-cell gather: it reads only planes it does not write, or its own cell of a
// plane it writes. A WGSL module cannot declare two buffers at one binding, so this module binds
// `surf` where `lbm.wgsl` binds `g_src` (a free surface excludes the thermal lattice): the
// pipelines keep 8 storage buffers at most (ADR-019). All four entry points share one layout.
//
// Across a link i the neighbour of cell x is x − c_i, the source of population i; boundary links
// (the link mask of the static flags) have none.

override WG: u32 = 128u;
override WRITE_MACRO: bool = true;

@group(0) @binding(0) var<uniform> P: Params;
@group(0) @binding(1) var<storage, read> cell_flags: array<u32>;
@group(0) @binding(2) var<storage, read> bc_slot: array<u32>;
@group(0) @binding(3) var<storage, read> bc: array<BcParams>;

@group(1) @binding(0) var<storage, read> f_src: array<f32>;
@group(1) @binding(1) var<storage, read_write> f_dst: array<f32>;
@group(1) @binding(2) var<storage, read_write> surf: array<u32>;
@group(1) @binding(4) var<storage, read_write> mac: array<f32>;

const NO_CELL: u32 = 0xffffffffu;

fn plane(p: u32, c: u32) -> u32 {
    return p * P.n + c;
}

fn surf_f32(p: u32, c: u32) -> f32 {
    return bitcast<f32>(surf[plane(p, c)]);
}

fn set_surf_f32(p: u32, c: u32, v: f32) {
    surf[plane(p, c)] = bitcast<u32>(v);
}

fn st_phase(s: u32) -> u32 {
    return s & PHASE_MASK;
}

fn st_mark(s: u32) -> u32 {
    return (s & MARK_MASK) >> MARK_SHIFT;
}

fn st_receivers(s: u32) -> u32 {
    return (s & COUNT_MASK) >> COUNT_SHIFT;
}

fn st_word(phase: u32, mark: u32, receivers: u32) -> u32 {
    return phase | (mark << MARK_SHIFT) | (receivers << COUNT_SHIFT);
}

fn is_wet(phase: u32) -> bool {
    return phase == PHASE_INTERFACE || phase == PHASE_LIQUID;
}

// The phase once the pending conversion is done.
fn st_settled(s: u32) -> u32 {
    switch (st_mark(s)) {
        case TO_LIQUID: {
            return PHASE_LIQUID;
        }
        case TO_GAS: {
            return PHASE_GAS;
        }
        case TO_INTERFACE: {
            return PHASE_INTERFACE;
        }
        default: {
            return st_phase(s);
        }
    }
}

// Neighbour x − c_i of cell k for each link i = 1…8, or `NO_CELL` across a boundary link.
fn neighbours(fl: u32, k: u32) -> array<u32, 9> {
    let w = P.w;
    let x = k % w;
    let y = k / w;
    let xm = select(x - 1u, w - 1u, x == 0u);
    let xp = select(x + 1u, 0u, x + 1u == w);
    let ym = select(y - 1u, P.h - 1u, y == 0u);
    let yp = select(y + 1u, 0u, y + 1u == P.h);
    let xs = array<u32, 9>(x, xm, x, xp, x, xm, xp, xp, xm);
    let ys = array<u32, 9>(y, y, ym, y, yp, ym, ym, yp, yp);
    let mask = fl >> LINK_SHIFT;
    var nb = array<u32, 9>(NO_CELL, NO_CELL, NO_CELL, NO_CELL, NO_CELL, NO_CELL, NO_CELL, NO_CELL, NO_CELL);
    for (var i = 1u; i < 9u; i++) {
        if ((mask & (1u << (i - 1u))) == 0u) {
            nb[i] = ys[i] * w + xs[i];
        }
    }
    return nb;
}

// Shifted D2Q9 equilibrium f̃ᵢ^eq (`cfd_lbm::kernel::equilibrium`).
fn equilibrium_i(i: u32, rho: f32, u: vec2<f32>, usq: f32) -> f32 {
    let cu = dot_c(i, u);
    return W9[i] * (rho - 1.0 + rho * (3.0 * cu + 4.5 * cu * cu - 1.5 * usq));
}

// Velocity for the gas-side reconstruction of interface cell k: from its own post-collision
// populations plus the Guo half-force shift (`outgoing_velocity`).
fn outgoing_velocity(k: u32) -> vec2<f32> {
    let n = P.n;
    var fo: array<f32, 9>;
    var sum = 0.0;
    for (var i = 0u; i < 9u; i++) {
        fo[i] = f_src[i * n + k];
        sum += fo[i];
    }
    let rho = 1.0 + sum;
    let jx = fo[1] - fo[3] + fo[5] - fo[6] - fo[7] + fo[8];
    let jy = fo[2] - fo[4] + fo[5] + fo[6] - fo[7] - fo[8];
    return vec2<f32>((jx + 0.5 * rho * P.body_force.x) / rho, (jy + 0.5 * rho * P.body_force.y) / rho);
}

fn cell_index(gid: vec3<u32>, groups: vec3<u32>) -> u32 {
    // 2-D dispatch: a dimension is limited to 65535 workgroups.
    return gid.y * groups.x * WG + gid.x;
}

// Pass 1. Gas cells copy their phase into `state_b` and return; liquid and interface cells pull
// (rebuilding the populations that come from gas), add the mass exchange and the neighbours'
// shares of excess to their mass, collide, and interface cells are marked IF or IG. ρ is stored on
// every liquid and interface cell, u on interface cells (the conversion passes read them).
@compute @workgroup_size(WG)
fn stream_collide_fs(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let k = cell_index(gid, groups);
    if (k >= P.n) {
        return;
    }
    let fl = flags_of(k);
    if ((fl & FLOW_MASK) != FLUID) {
        return;
    }
    let own = st_phase(surf[plane(SURF_STATE_A, k)]);
    if (own == PHASE_GAS) {
        surf[plane(SURF_STATE_B, k)] = own;
        return;
    }
    let w = P.w;
    let n = P.n;
    let x = k % w;
    let y = k / w;
    let xm = select(x - 1u, w - 1u, x == 0u);
    let xp = select(x + 1u, 0u, x + 1u == w);
    let ym = select(y - 1u, P.h - 1u, y == 0u);
    let yp = select(y + 1u, 0u, y + 1u == P.h);
    var xs = array<u32, 9>(x, xm, x, xp, x, xm, xp, xp, xm);
    var ys = array<u32, 9>(y, y, ym, y, yp, ym, ym, yp, yp);
    let mask = fl >> LINK_SHIFT;

    // ---- Pull, mass exchange ----
    var f: array<f32, 9>;
    var dm = 0.0;
    var gas_neighbour = false;
    var liquid_neighbour = false;
    var have_u_gas = false;
    var u_gas = vec2<f32>(0.0, 0.0);
    let phi_k = surf_f32(SURF_PHI, k);
    f[0] = f_src[k];
    for (var i = 1u; i < 9u; i++) {
        let src = ys[i] * w + xs[i];
        let out = f_src[OPPOSITE[i] * n + k]; // f̃*_ī(x)
        if ((mask & (1u << (i - 1u))) != 0u) {
            let t = link_target(x, y, i, src);
            let b = bc[t.y];
            switch (t.x & FLOW_MASK) {
                case BOUNCE_BACK: {
                    f[i] = out + 6.0 * W9[i] * dot_c(i, b.velocity);
                }
                case PRESSURE: {
                    let u = vec2<f32>(mac[n + k], mac[2u * n + k]);
                    let cu = dot_c(i, u);
                    let usq = u.x * u.x + u.y * u.y;
                    f[i] = -out + 2.0 * W9[i] * (b.density * (1.0 + 4.5 * cu * cu - 1.5 * usq) - 1.0);
                }
                default: {
                    f[i] = f_src[i * n + inward_cell(xs[i], ys[i], t.x, k)];
                }
            }
            continue;
        }
        let nst = surf[plane(SURF_STATE_A, src)];
        let np = st_phase(nst);
        if (np == PHASE_GAS) {
            // The gas imposes ρ_G = 1 and the interface's velocity.
            gas_neighbour = true;
            if (!have_u_gas) {
                u_gas = outgoing_velocity(k);
                have_u_gas = true;
            }
            let cu = dot_c(i, u_gas);
            let usq = u_gas.x * u_gas.x + u_gas.y * u_gas.y;
            f[i] = -out + 2.0 * W9[i] * (4.5 * cu * cu - 1.5 * usq);
        } else {
            let fin = f_src[i * n + src];
            f[i] = fin;
            liquid_neighbour = liquid_neighbour || np == PHASE_LIQUID;
            var weight = 1.0;
            if (own != PHASE_LIQUID && np != PHASE_LIQUID) {
                weight = 0.5 * (phi_k + surf_f32(SURF_PHI, src));
            }
            dm += weight * (fin - out);
        }
        let receivers = st_receivers(nst);
        if (receivers > 0u) {
            dm += surf_f32(SURF_MASSEX, src) / f32(receivers);
        }
    }

    // ---- Moments, force, collision (as `stream_collide` in lbm.wgsl, without θ) ----
    var drho = 0.0;
    for (var i = 0u; i < 9u; i++) {
        drho += f[i];
    }
    let rho = 1.0 + drho;
    let jx = f[1] - f[3] + f[5] - f[6] - f[7] + f[8];
    let jy = f[2] - f[4] + f[5] + f[6] - f[7] - f[8];
    let theta = 0.0;
    let force = vec2<f32>(
        rho * P.body_force.x + P.buoyancy.x * theta,
        rho * P.body_force.y + P.buoyancy.y * theta,
    );
    let u = vec2<f32>((jx + 0.5 * force.x) / rho, (jy + 0.5 * force.y) / rho);
    let usq = u.x * u.x + u.y * u.y;
    let uf = u.x * force.x + u.y * force.y;
    let om = relaxation(f, drho, rho, u);
    var moving = 0.0;
    moving += collide_pair(1u, k, f[1], f[3], drho, rho, u, usq, force, uf, om.x, om.y);
    moving += collide_pair(2u, k, f[2], f[4], drho, rho, u, usq, force, uf, om.x, om.y);
    moving += collide_pair(5u, k, f[5], f[7], drho, rho, u, usq, force, uf, om.x, om.y);
    moving += collide_pair(6u, k, f[6], f[8], drho, rho, u, usq, force, uf, om.x, om.y);
    f_dst[k] = drho - moving;
    mac[k] = rho;
    if (WRITE_MACRO || mask != 0u || own == PHASE_INTERFACE) {
        mac[n + k] = u.x;
        mac[2u * n + k] = u.y;
    }

    // ---- Mass and conversion marks ----
    let m = surf_f32(SURF_MASS, k) + dm;
    set_surf_f32(SURF_MASS, k, m);
    var mark = MARK_NONE;
    if (own == PHASE_INTERFACE) {
        if (m > (1.0 + KAPPA) * rho || !gas_neighbour) {
            mark = TO_LIQUID;
        } else if (m < -KAPPA * rho || !liquid_neighbour) {
            mark = TO_GAS;
        }
    }
    surf[plane(SURF_STATE_B, k)] = st_word(own, mark, 0u);
    if (!is_finite(f[0])) {
        mac[4u * n] = 1.0;
    }
}

// Pass 2 (`surface::fill`), `state_b` → `state_a`: gas next to an IF cell becomes GI; an IG next to
// an IF cell stays interface.
@compute @workgroup_size(WG)
fn surface_fill(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let k = cell_index(gid, groups);
    if (k >= P.n) {
        return;
    }
    let st = surf[plane(SURF_STATE_B, k)];
    let ph = st_phase(st);
    let candidate = ph == PHASE_GAS || st_mark(st) == TO_GAS;
    let fl = flags_of(k);
    var result = st;
    if (candidate && (fl & FLOW_MASK) == FLUID) {
        let nb = neighbours(fl, k);
        var filling = false;
        for (var i = 1u; i < 9u; i++) {
            if (nb[i] != NO_CELL && st_mark(surf[plane(SURF_STATE_B, nb[i])]) == TO_LIQUID) {
                filling = true;
            }
        }
        if (filling) {
            result = select(st_word(ph, MARK_NONE, 0u), st_word(ph, TO_INTERFACE, 0u), ph == PHASE_GAS);
        }
    }
    surf[plane(SURF_STATE_A, k)] = result;
}

// Pass 3 (`surface::init_states`, `init_value`, `init_populations`), `state_a` → `state_b`: liquid
// next to an IG becomes interface; a GI cell starts at the equilibrium of its liquid and interface
// neighbours' mean ρ and u (not of those being emptied), summed in link order. GI cells write their
// own populations and macroscopic values only, and read only those of wet cells, which are never
// GI: no invocation reads what another writes.
@compute @workgroup_size(WG)
fn surface_init(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let k = cell_index(gid, groups);
    if (k >= P.n) {
        return;
    }
    let n = P.n;
    let st = surf[plane(SURF_STATE_A, k)];
    let fl = flags_of(k);
    var result = st;
    if ((fl & FLOW_MASK) == FLUID) {
        let ph = st_phase(st);
        if (ph == PHASE_LIQUID) {
            let nb = neighbours(fl, k);
            var emptying = false;
            for (var i = 1u; i < 9u; i++) {
                if (nb[i] != NO_CELL && st_mark(surf[plane(SURF_STATE_A, nb[i])]) == TO_GAS) {
                    emptying = true;
                }
            }
            if (emptying) {
                result = st_word(PHASE_INTERFACE, MARK_NONE, 0u);
            }
        } else if (st_mark(st) == TO_INTERFACE) {
            let nb = neighbours(fl, k);
            var r = 0.0;
            var vx = 0.0;
            var vy = 0.0;
            var count = 0u;
            for (var i = 1u; i < 9u; i++) {
                if (nb[i] == NO_CELL) {
                    continue;
                }
                let j = nb[i];
                let sj = surf[plane(SURF_STATE_A, j)];
                if (is_wet(st_phase(sj)) && st_mark(sj) != TO_GAS) {
                    r += mac[j];
                    vx += mac[n + j];
                    vy += mac[2u * n + j];
                    count += 1u;
                }
            }
            var rho = 1.0;
            var u = vec2<f32>(0.0, 0.0);
            if (count > 0u) {
                let c = f32(count);
                rho = r / c;
                u = vec2<f32>(vx / c, vy / c);
            }
            let usq = u.x * u.x + u.y * u.y;
            for (var i = 0u; i < 9u; i++) {
                f_dst[i * n + k] = equilibrium_i(i, rho, u, usq);
            }
            mac[k] = rho;
            mac[n + k] = u.x;
            mac[2u * n + k] = u.y;
        }
    }
    surf[plane(SURF_STATE_B, k)] = result;
}

// Pass 4 (`surface::finish`), `state_b` → `state_a`: IF → liquid, IG → gas, GI → interface; the
// excess mass is set aside undivided for the next step, with the number of liquid and interface
// neighbours that share it (counted only when there is an excess); without receivers the cell keeps
// it in its mass.
@compute @workgroup_size(WG)
fn surface_finish(
    @builtin(global_invocation_id) gid: vec3<u32>,
    @builtin(num_workgroups) groups: vec3<u32>,
) {
    let k = cell_index(gid, groups);
    if (k >= P.n) {
        return;
    }
    let st = surf[plane(SURF_STATE_B, k)];
    let fl = flags_of(k);
    if (st_phase(st) == PHASE_SOLID || (fl & FLOW_MASK) != FLUID) {
        surf[plane(SURF_STATE_A, k)] = st;
        return;
    }
    let settled = st_settled(st);
    let r = mac[k];
    let m0 = surf_f32(SURF_MASS, k);
    var m: f32;
    var excess: f32;
    var fill: f32;
    if (settled == PHASE_LIQUID) {
        m = r;
        excess = m0 - r;
        fill = 1.0;
    } else if (settled == PHASE_INTERFACE) {
        excess = 0.0;
        if (m0 > r) {
            excess = m0 - r;
        } else if (m0 < 0.0) {
            excess = m0;
        }
        // Rust's `clamp(0, ρ)`.
        m = m0;
        if (m < 0.0) {
            m = 0.0;
        }
        if (m > r) {
            m = r;
        }
        fill = m / r;
    } else {
        m = 0.0;
        excess = m0;
        fill = 0.0;
    }
    var receivers = 0u;
    if (excess != 0.0) {
        let nb = neighbours(fl, k);
        for (var i = 1u; i < 9u; i++) {
            if (nb[i] != NO_CELL && is_wet(st_settled(surf[plane(SURF_STATE_B, nb[i])]))) {
                receivers += 1u;
            }
        }
    }
    if (receivers == 0u) {
        m += excess;
        excess = 0.0;
    }
    surf[plane(SURF_STATE_A, k)] = st_word(settled, MARK_NONE, receivers);
    set_surf_f32(SURF_MASS, k, m);
    set_surf_f32(SURF_MASSEX, k, excess);
    set_surf_f32(SURF_PHI, k, fill);
}
