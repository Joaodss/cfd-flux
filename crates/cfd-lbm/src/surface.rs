//! Free surface (ADR-032, design proposal `docs/sessions/phase6a-design-proposal.md`): the
//! per-cell state and the conversion passes 2–4 that follow the stream-collide pass.
//!
//! Each non-solid cell is gas (not updated), interface (own mass `m`, fill `φ = m/ρ`) or liquid.
//! One step is four passes, each a per-cell *gather* that reads only what earlier passes wrote,
//! so the passes have no data races on a GPU and the CPU runs exactly the same arithmetic:
//!
//! 1. `stream_collide` ([`crate::kernel::update_block`]): mass exchange, gas-link
//!    reconstruction, collision; interface cells that fill are marked IF, those that empty IG
//!    (`state_a` → `state_b`);
//! 2. [`fill`]: gas next to an IF cell → GI; an IG next to an IF is cancelled (`state_b` →
//!    `state_a`);
//! 3. [`init`]: GI cells start at the equilibrium of their neighbours' mean ρ, u; liquid next to
//!    an IG becomes interface (`state_a` → `state_b`);
//! 4. [`finish`]: the conversions are done, the excess mass is set aside with its receiver
//!    count, φ is written (`state_b` → `state_a`, the next step's input).
//!
//! The planes (`n` values each) are the same on every backend: `state_a`, `state_b` (`u32`),
//! `mass`, `massex`, `phi` (`f32`). Across a link `i` the neighbour of cell `x` is `x − cᵢ`
//! (the source of population `i`), skipped when the link is a boundary link.

use cfd_core::domain::{flags, phase};

use crate::kernel::{equilibrium, StepParams};

/// Conversion offset κ (Thürey 2007): fill when `m > (1 + κ)ρ`, empty when `m < −κρ`. Zero is
/// FluidX3D's strict comparison; raise it only if cells flip back and forth.
pub const KAPPA: f32 = 0.0;

/// Bits of a cell's state word.
pub mod state {
    use cfd_core::domain::phase;

    /// Bits 0–1: the phase ([`phase`]).
    pub const PHASE_MASK: u32 = 0b11;
    /// Bits 2–3: a conversion pending within the step.
    pub const MARK_SHIFT: u32 = 2;
    pub const MARK_MASK: u32 = 0b11 << MARK_SHIFT;
    pub const MARK_NONE: u32 = 0;
    /// IF: interface → liquid.
    pub const TO_LIQUID: u32 = 1;
    /// IG: interface → gas.
    pub const TO_GAS: u32 = 2;
    /// GI: gas → interface.
    pub const TO_INTERFACE: u32 = 3;
    /// Bits 4–7: how many neighbours share the cell's excess mass (0–8; 0 when it has none).
    pub const COUNT_SHIFT: u32 = 4;
    pub const COUNT_MASK: u32 = 0b1111 << COUNT_SHIFT;

    #[inline]
    pub const fn word(phase: u8, mark: u32, receivers: u32) -> u32 {
        phase as u32 | mark << MARK_SHIFT | receivers << COUNT_SHIFT
    }

    #[inline]
    pub const fn phase(s: u32) -> u8 {
        (s & PHASE_MASK) as u8
    }

    #[inline]
    pub const fn mark(s: u32) -> u32 {
        (s & MARK_MASK) >> MARK_SHIFT
    }

    #[inline]
    pub const fn receivers(s: u32) -> u32 {
        (s & COUNT_MASK) >> COUNT_SHIFT
    }

    /// The phase once the pending conversion is done.
    #[inline]
    pub const fn settled(s: u32) -> u8 {
        match mark(s) {
            TO_LIQUID => phase::LIQUID,
            TO_GAS => phase::GAS,
            TO_INTERFACE => phase::INTERFACE,
            _ => phase(s),
        }
    }
}

/// Neighbour `x − cᵢ` of cell `(x, y)` for each link `i = 1…8` (index 0 unused), `None` across a
/// boundary link (a non-fluid cell, or outside a non-periodic edge).
#[inline]
fn neighbours(p: &StepParams, fl: u16, x: usize, y: usize) -> [Option<usize>; 9] {
    let mask = fl >> flags::LINK_SHIFT;
    // Wrapped coordinates, used only across links that are not boundary links.
    let xm = if x == 0 { p.w - 1 } else { x - 1 };
    let xp = if x + 1 == p.w { 0 } else { x + 1 };
    let ym = if y == 0 { p.h - 1 } else { y - 1 };
    let yp = if y + 1 == p.h { 0 } else { y + 1 };
    let xs = [x, xm, x, xp, x, xm, xp, xp, xm];
    let ys = [y, y, ym, y, yp, ym, ym, yp, yp];
    std::array::from_fn(|i| (i > 0 && mask & (1 << (i - 1)) == 0).then(|| ys[i] * p.w + xs[i]))
}

/// The cells of a block of whole rows starting at row `y0`: `(local index, cell, x, y)`.
fn cells(
    p: &StepParams,
    y0: usize,
    len: usize,
) -> impl Iterator<Item = (usize, usize, usize, usize)> {
    let w = p.w;
    (0..len / w)
        .flat_map(move |ry| (0..w).map(move |x| (ry * w + x, (y0 + ry) * w + x, x, y0 + ry)))
}

/// Pass 2, for the cells `y0 · w ..` of `out`: gas with an IF neighbour becomes GI; an IG with
/// an IF neighbour stays interface. `src` is `state_b`, `out` a block of `state_a`.
pub(crate) fn fill(p: &StepParams, flags_: &[u16], src: &[u32], y0: usize, out: &mut [u32]) {
    for (local, k, x, y) in cells(p, y0, out.len()) {
        let o = &mut out[local];
        let st = src[k];
        let (ph, mark) = (state::phase(st), state::mark(st));
        let candidate = ph == phase::GAS || mark == state::TO_GAS;
        *o = st;
        if !candidate || flags::flow(flags_[k]) != flags::FLUID {
            continue;
        }
        let filling = neighbours(p, flags_[k], x, y)
            .into_iter()
            .flatten()
            .any(|j| state::mark(src[j]) == state::TO_LIQUID);
        if filling {
            *o = if ph == phase::GAS {
                state::word(ph, state::TO_INTERFACE, 0)
            } else {
                state::word(ph, state::MARK_NONE, 0)
            };
        }
    }
}

/// Pass 3, part 1, for a block of `state_b`: liquid next to an IG becomes interface. `src` is
/// `state_a`.
pub(crate) fn init_states(p: &StepParams, flags_: &[u16], src: &[u32], y0: usize, out: &mut [u32]) {
    for (local, k, x, y) in cells(p, y0, out.len()) {
        let o = &mut out[local];
        let st = src[k];
        *o = st;
        if state::phase(st) != phase::LIQUID || flags::flow(flags_[k]) != flags::FLUID {
            continue;
        }
        let emptying = neighbours(p, flags_[k], x, y)
            .into_iter()
            .flatten()
            .any(|j| state::mark(src[j]) == state::TO_GAS);
        if emptying {
            *o = state::word(phase::INTERFACE, state::MARK_NONE, 0);
        }
    }
}

/// A new interface cell's initial ρ and u.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct NewInterface {
    pub cell: usize,
    pub rho: f32,
    pub u: [f32; 2],
}

/// Pass 3, part 2: the ρ and u a GI cell `k` starts with, the mean over its liquid and
/// interface neighbours (not those being emptied), summed in link order; `ρ = 1`, `u = 0` if
/// it has none. Its populations are the equilibrium of these ([`init_populations`]).
pub(crate) fn init_value(
    p: &StepParams,
    flags_: &[u16],
    src: &[u32],
    rho: &[f32],
    ux: &[f32],
    uy: &[f32],
    k: usize,
) -> NewInterface {
    let (mut r, mut vx, mut vy, mut count) = (0.0f32, 0.0f32, 0.0f32, 0u32);
    for j in neighbours(p, flags_[k], k % p.w, k / p.w)
        .into_iter()
        .flatten()
    {
        let st = src[j];
        if phase::wet(state::phase(st)) && state::mark(st) != state::TO_GAS {
            r += rho[j];
            vx += ux[j];
            vy += uy[j];
            count += 1;
        }
    }
    let (rho, u) = if count > 0 {
        let c = count as f32;
        (r / c, [vx / c, vy / c])
    } else {
        (1.0, [0.0; 2])
    };
    NewInterface { cell: k, rho, u }
}

/// Writes a new interface cell's populations (SoA, `9 · n` values) and ρ, u.
pub(crate) fn init_populations(
    n: usize,
    c: &NewInterface,
    f: &mut [f32],
    rho: &mut [f32],
    ux: &mut [f32],
    uy: &mut [f32],
) {
    for (i, v) in equilibrium(c.rho, c.u).into_iter().enumerate() {
        f[i * n + c.cell] = v;
    }
    rho[c.cell] = c.rho;
    ux[c.cell] = c.u[0];
    uy[c.cell] = c.u[1];
}

/// The outputs of pass 4 for one block of rows.
pub(crate) struct FinishBlock<'a> {
    pub y0: usize,
    pub state: &'a mut [u32],
    pub mass: &'a mut [f32],
    pub massex: &'a mut [f32],
    pub phi: &'a mut [f32],
}

/// Pass 4: IF → liquid, IG → gas, GI → interface; the excess mass (liquid: `m − ρ`; interface:
/// above ρ or below 0; gas: all of it) is set aside for the liquid and interface neighbours to
/// gather on the next step, undivided, with their count in the state word, so the liquid mass
/// is exactly `Σ (m + m_ex)`. Without receivers a cell keeps its excess in `m` (FluidX3D).
/// `src` is `state_b`; `rho` is this step's density.
pub(crate) fn finish(p: &StepParams, flags_: &[u16], src: &[u32], rho: &[f32], b: FinishBlock) {
    for (local, k, x, y) in cells(p, b.y0, b.state.len()) {
        let st = src[k];
        if state::phase(st) == phase::SOLID || flags::flow(flags_[k]) != flags::FLUID {
            b.state[local] = st;
            continue;
        }
        let settled = state::settled(st);
        let (r, m) = (rho[k], b.mass[local]);
        let (mut m, mut excess, phi) = match settled {
            phase::LIQUID => (r, m - r, 1.0),
            phase::INTERFACE => {
                let excess = if m > r {
                    m - r
                } else if m < 0.0 {
                    m
                } else {
                    0.0
                };
                let m = m.clamp(0.0, r);
                (m, excess, m / r)
            }
            _ => (0.0, m, 0.0),
        };
        // Only a cell with an excess needs its receivers (a zero excess shares nothing).
        let receivers = if excess == 0.0 {
            0
        } else {
            neighbours(p, flags_[k], x, y)
                .into_iter()
                .flatten()
                .filter(|&j| phase::wet(state::settled(src[j])))
                .count() as u32
        };
        if receivers == 0 {
            m += excess;
            excess = 0.0;
        }
        b.state[local] = state::word(settled, state::MARK_NONE, receivers);
        b.mass[local] = m;
        b.massex[local] = excess;
        b.phi[local] = phi;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A periodic 5 × 5 grid of fluid cells (no boundary links): every cell has 8 neighbours.
    fn params() -> StepParams {
        StepParams {
            w: 5,
            h: 5,
            n: 25,
            periodic: [true; 2],
            omega: [1.0; 2],
            omega_g: [1.0; 2],
            body_force: [0.0; 2],
            buoyancy: [0.0; 2],
            les: None,
        }
    }

    const G: u32 = phase::GAS as u32;
    const I: u32 = phase::INTERFACE as u32;
    const L: u32 = phase::LIQUID as u32;

    fn at(x: usize, y: usize) -> usize {
        y * 5 + x
    }

    #[test]
    fn filling_cells_open_the_gas_and_cancel_emptying_neighbours() {
        let p = params();
        let flags = [flags::FLUID; 25];
        let mut src = [G; 25];
        src[at(2, 2)] = state::word(phase::INTERFACE, state::TO_LIQUID, 0);
        src[at(3, 2)] = state::word(phase::INTERFACE, state::TO_GAS, 0);
        src[at(0, 0)] = state::word(phase::INTERFACE, state::TO_GAS, 0);
        let mut out = [0u32; 25];
        fill(&p, &flags, &src, 0, &mut out);
        // The 8 neighbours of (2, 2): gas → GI, the IG → plain interface.
        for (x, y) in [(1, 1), (2, 1), (3, 1), (1, 2), (1, 3), (2, 3), (3, 3)] {
            assert_eq!(
                out[at(x, y)],
                state::word(phase::GAS, state::TO_INTERFACE, 0)
            );
        }
        assert_eq!(out[at(3, 2)], I);
        // Unchanged: the IF itself, an IG away from it, gas away from it.
        assert_eq!(out[at(2, 2)], src[at(2, 2)]);
        assert_eq!(out[at(0, 0)], src[at(0, 0)]);
        assert_eq!(out[at(4, 4)], G);
    }

    #[test]
    fn liquid_next_to_an_emptying_cell_becomes_interface() {
        let p = params();
        let flags = [flags::FLUID; 25];
        let mut src = [L; 25];
        src[at(2, 2)] = state::word(phase::INTERFACE, state::TO_GAS, 0);
        let mut out = [0u32; 25];
        init_states(&p, &flags, &src, 0, &mut out);
        for (x, y) in [
            (1, 1),
            (2, 1),
            (3, 1),
            (1, 2),
            (3, 2),
            (1, 3),
            (2, 3),
            (3, 3),
        ] {
            assert_eq!(out[at(x, y)], I, "({x}, {y})");
        }
        assert_eq!(out[at(0, 0)], L);
        assert_eq!(out[at(2, 2)], src[at(2, 2)]);
    }

    #[test]
    fn new_interface_cells_average_their_wet_neighbours() {
        let p = params();
        let flags = [flags::FLUID; 25];
        let mut src = [G; 25];
        src[at(2, 2)] = state::word(phase::GAS, state::TO_INTERFACE, 0);
        src[at(1, 2)] = state::word(phase::INTERFACE, state::TO_LIQUID, 0);
        src[at(3, 2)] = I;
        src[at(2, 3)] = state::word(phase::INTERFACE, state::TO_GAS, 0); // excluded
        let mut rho = [1.0f32; 25];
        let (mut ux, uy) = ([0.0f32; 25], [0.0f32; 25]);
        (rho[at(1, 2)], ux[at(1, 2)]) = (1.02, 0.01);
        (rho[at(3, 2)], ux[at(3, 2)]) = (1.04, 0.03);
        (rho[at(2, 3)], ux[at(2, 3)]) = (9.0, 9.0);
        let c = init_value(&p, &flags, &src, &rho, &ux, &uy, at(2, 2));
        assert!(
            (c.rho - 1.03).abs() < 1e-6 && (c.u[0] - 0.02).abs() < 1e-7,
            "{c:?}"
        );
        // Alone, it starts at rest with the gas density.
        let lone = init_value(&p, &flags, &[G; 25], &rho, &ux, &uy, at(2, 2));
        assert_eq!((lone.rho, lone.u), (1.0, [0.0; 2]));
    }

    #[test]
    fn finishing_sets_the_excess_aside_with_its_receivers() {
        let p = params();
        let flags = [flags::FLUID; 25];
        let mut src = [G; 25];
        // (2, 2) fills with m = 1.1 ρ next to a new interface cell and a liquid cell.
        src[at(2, 2)] = state::word(phase::INTERFACE, state::TO_LIQUID, 0);
        src[at(3, 2)] = state::word(phase::GAS, state::TO_INTERFACE, 0);
        src[at(1, 2)] = L;
        // A lone interface cell far away empties with m = 0.2: no receivers, it keeps it.
        src[at(0, 4)] = state::word(phase::INTERFACE, state::TO_GAS, 0);
        let rho = [1.0f32; 25];
        let mut mass = [0.0f32; 25];
        (mass[at(2, 2)], mass[at(0, 4)], mass[at(1, 2)]) = (1.1, 0.2, 1.0);
        let (mut state_a, mut massex, mut phi) = ([0u32; 25], [0.0f32; 25], [0.0f32; 25]);
        let before: f32 = mass.iter().sum();
        finish(
            &p,
            &flags,
            &src,
            &rho,
            FinishBlock {
                y0: 0,
                state: &mut state_a,
                mass: &mut mass,
                massex: &mut massex,
                phi: &mut phi,
            },
        );
        let k = at(2, 2);
        assert_eq!(state::phase(state_a[k]), phase::LIQUID);
        assert_eq!(state::receivers(state_a[k]), 2);
        assert_eq!((mass[k], phi[k]), (1.0, 1.0));
        assert!((massex[k] - 0.1).abs() < 1e-6);
        let new = at(3, 2);
        assert_eq!(state::phase(state_a[new]), phase::INTERFACE);
        assert_eq!((mass[new], phi[new]), (0.0, 0.0));
        let lone = at(0, 4);
        assert_eq!(state_a[lone], G);
        assert_eq!((mass[lone], massex[lone], phi[lone]), (0.2, 0.0, 0.0));
        let after: f32 = mass.iter().chain(&massex).sum();
        assert!((after - before).abs() < 1e-6);
    }
}
