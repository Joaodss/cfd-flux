//! CPU backend: the numerical reference, parallelised over blocks of rows with `rayon`.

use std::collections::BTreeMap;

use cfd_core::domain::{flags, Domain, LINKS, OPPOSITE};
use cfd_core::solver::{
    BackendKind, Diagnostics, ElementForce, LatticeFields, Solver, SolverError,
};
use rayon::prelude::*;

use crate::config::{flow_rates, thermal_rates, LbmConfig};
use crate::kernel::{equilibrium, thermal_equilibrium, update_block, Block, Source, StepParams};
use crate::lattice::W9;

pub struct CpuLbm {
    domain: Domain,
    config: LbmConfig,
    params: StepParams,
    /// Post-collision populations of the last step, and the buffer the next step writes to.
    f: Vec<f32>,
    f_next: Vec<f32>,
    /// Thermal populations (empty without the thermal lattice).
    g: Vec<f32>,
    g_next: Vec<f32>,
    rho: Vec<f32>,
    ux: Vec<f32>,
    uy: Vec<f32>,
    theta: Vec<f32>,
    steps: u64,
    /// Fluid cells with at least one boundary link (for forces).
    boundary_cells: Vec<usize>,
}

impl CpuLbm {
    /// Creates the solver with every cell at the equilibrium of the domain's initial state.
    pub fn new(domain: Domain, config: LbmConfig) -> Self {
        let n = domain.cell_count();
        let thermal = domain.physics.thermal();
        let omega = flow_rates(config.collision, domain.physics.tau()).map(|v| v as f32);
        let omega_g = domain
            .physics
            .tau_thermal()
            .map_or([1.0; 2], |tau| thermal_rates(config.collision, tau))
            .map(|v| v as f32);
        let params = StepParams {
            w: domain.width as usize,
            h: domain.height as usize,
            n,
            periodic: domain.periodic,
            omega,
            omega_g,
            body_force: domain.physics.body_force.map(|v| v as f32),
            buoyancy: domain.physics.buoyancy.map(|v| v as f32),
        };
        let boundary_cells = (0..n)
            .filter(|&i| {
                let fl = domain.flags[i];
                flags::flow(fl) == flags::FLUID && fl & flags::LINK_MASK != 0
            })
            .collect();
        let g_len = if thermal { 5 * n } else { 0 };
        let mut solver = CpuLbm {
            config,
            params,
            f: vec![0.0; 9 * n],
            f_next: vec![0.0; 9 * n],
            g: vec![0.0; g_len],
            g_next: vec![0.0; g_len],
            rho: vec![1.0; n],
            ux: vec![0.0; n],
            uy: vec![0.0; n],
            theta: vec![0.0; n],
            steps: 0,
            boundary_cells,
            domain,
        };
        let mut initial = LatticeFields::initial(&solver.domain);
        for i in 0..n {
            if flags::flow(solver.domain.flags[i]) != flags::FLUID {
                (initial.density[i], initial.ux[i], initial.uy[i]) = (1.0, 0.0, 0.0);
            }
        }
        solver
            .set_equilibrium(&initial)
            .expect("initial fields match the domain");
        solver
    }

    pub fn config(&self) -> &LbmConfig {
        &self.config
    }

    /// Updates the body force (lattice units), e.g. for validation cases.
    pub fn set_body_force(&mut self, force: [f64; 2]) {
        self.domain.physics.body_force = force;
        self.params.body_force = force.map(|v| v as f32);
    }

    fn step_once(&mut self) {
        let (w, n) = (self.params.w, self.params.n);
        let thermal = !self.g.is_empty();
        // ~4 blocks per thread balances the load without much overhead.
        let rows = (self.params.h / (4 * rayon::current_num_threads())).max(1);
        let block = rows * w;

        let src = Source {
            f: &self.f,
            g: &self.g,
            flags: &self.domain.flags,
            slot: &self.domain.bc_slot,
            bc: &self.domain.bc_params,
        };
        let f_blocks = plane_blocks::<9>(&mut self.f_next, n, block);
        let mut g_blocks = if thermal {
            plane_blocks::<5>(&mut self.g_next, n, block)
                .into_iter()
                .map(Some)
                .collect()
        } else {
            Vec::new()
        }
        .into_iter();
        let blocks: Vec<Block> = f_blocks
            .into_iter()
            .zip(self.rho.chunks_mut(block))
            .zip(self.ux.chunks_mut(block))
            .zip(self.uy.chunks_mut(block))
            .zip(self.theta.chunks_mut(block))
            .enumerate()
            .map(|(b, ((((f, rho), ux), uy), theta))| Block {
                y0: b * rows,
                f,
                g: g_blocks.next().flatten(),
                rho,
                ux,
                uy,
                theta,
            })
            .collect();

        let params = &self.params;
        if thermal {
            blocks
                .into_par_iter()
                .for_each(|b| update_block::<true>(params, &src, b));
        } else {
            blocks
                .into_par_iter()
                .for_each(|b| update_block::<false>(params, &src, b));
        }
        std::mem::swap(&mut self.f, &mut self.f_next);
        std::mem::swap(&mut self.g, &mut self.g_next);
    }
}

/// Splits `Q` planes of `n` values into blocks of `block` values, returning for each block the
/// `Q` disjoint mutable slices. The borrow checker then guarantees that parallel blocks never
/// write to the same memory, without `unsafe`.
fn plane_blocks<const Q: usize>(buf: &mut [f32], n: usize, block: usize) -> Vec<[&mut [f32]; Q]> {
    let mut planes: Vec<_> = buf.chunks_mut(n).map(|p| p.chunks_mut(block)).collect();
    (0..n.div_ceil(block))
        .map(|_| std::array::from_fn(|i| planes[i].next().expect("same block count per plane")))
        .collect()
}

impl Solver for CpuLbm {
    fn backend(&self) -> BackendKind {
        BackendKind::Cpu
    }

    fn domain(&self) -> &Domain {
        &self.domain
    }

    fn steps_done(&self) -> u64 {
        self.steps
    }

    fn step(&mut self, n: u32) -> Result<(), SolverError> {
        for _ in 0..n {
            self.step_once();
            self.steps += 1;
        }
        if self.rho.par_iter().any(|v| !v.is_finite()) {
            return Err(SolverError::NonFinite { step: self.steps });
        }
        Ok(())
    }

    fn set_equilibrium(&mut self, fields: &LatticeFields) -> Result<(), SolverError> {
        let n = self.params.n;
        fields.check_len(n)?;
        for c in 0..n {
            let (rho, u) = (fields.density[c], [fields.ux[c], fields.uy[c]]);
            for (i, v) in equilibrium(rho, u).into_iter().enumerate() {
                self.f[i * n + c] = v;
            }
            if !self.g.is_empty() {
                let theta = fields.theta.as_ref().map_or(0.0, |t| t[c]);
                for (i, v) in thermal_equilibrium(theta, u).into_iter().enumerate() {
                    self.g[i * n + c] = v;
                }
                self.theta[c] = theta;
            }
        }
        self.rho.copy_from_slice(&fields.density);
        self.ux.copy_from_slice(&fields.ux);
        self.uy.copy_from_slice(&fields.uy);
        Ok(())
    }

    fn lattice_fields(&mut self) -> LatticeFields {
        LatticeFields {
            density: self.rho.clone(),
            ux: self.ux.clone(),
            uy: self.uy.clone(),
            theta: (!self.g.is_empty()).then(|| self.theta.clone()),
        }
    }

    fn diagnostics(&mut self) -> Diagnostics {
        #[derive(Clone, Copy)]
        struct Acc {
            cells: usize,
            mass: f64,
            energy: f64,
            umax: f64,
            heat: f64,
            finite: bool,
        }
        let zero = Acc {
            cells: 0,
            mass: 0.0,
            energy: 0.0,
            umax: 0.0,
            heat: 0.0,
            finite: true,
        };
        let flags = &self.domain.flags;
        let (rho, ux, uy, theta) = (&self.rho, &self.ux, &self.uy, &self.theta);
        let acc = (0..self.params.n)
            .into_par_iter()
            .with_min_len(4096)
            .filter(|&i| flags::flow(flags[i]) == flags::FLUID)
            .fold(
                || zero,
                |mut a, i| {
                    let (r, x, y, t) = (rho[i] as f64, ux[i] as f64, uy[i] as f64, theta[i]);
                    let usq = x * x + y * y;
                    a.cells += 1;
                    a.mass += r;
                    a.energy += 0.5 * r * usq;
                    a.umax = a.umax.max(usq.sqrt());
                    a.heat += t as f64;
                    a.finite &= r.is_finite() && usq.is_finite() && t.is_finite();
                    a
                },
            )
            .reduce(
                || zero,
                |a, b| Acc {
                    cells: a.cells + b.cells,
                    mass: a.mass + b.mass,
                    energy: a.energy + b.energy,
                    umax: a.umax.max(b.umax),
                    heat: a.heat + b.heat,
                    finite: a.finite && b.finite,
                },
            );
        Diagnostics {
            step: self.steps,
            fluid_cells: acc.cells,
            mass: acc.mass,
            kinetic_energy: acc.energy,
            max_velocity: acc.umax,
            thermal_energy: (!self.g.is_empty()).then_some(acc.heat),
            finite: acc.finite,
        }
    }

    fn forces(&mut self) -> Vec<ElementForce> {
        // Momentum exchange over every link to a bounce-back cell: the population f*_ī leaving
        // the fluid cell towards the wall returns as f_i = f*_ī + 6 wᵢ cᵢ·u_w, so the wall
        // receives c_ī (2 f*_ī + 6 wᵢ cᵢ·u_w) per step.
        let p = &self.params;
        let d = &self.domain;
        let mut acc: BTreeMap<u16, [f64; 2]> = BTreeMap::new();
        for &k in &self.boundary_cells {
            let fl = d.flags[k];
            let (x, y) = (k % p.w, k / p.w);
            for i in 1..9 {
                if !flags::is_boundary_link(fl, i) {
                    continue;
                }
                let [cx, cy] = LINKS[i];
                let sx = x as i64 - cx as i64;
                let sy = y as i64 - cy as i64;
                let inside = |v: i64, len: usize| (0..len as i64).contains(&v);
                let (flow, slot) =
                    if (inside(sx, p.w) || p.periodic[0]) && (inside(sy, p.h) || p.periodic[1]) {
                        let src = sy.rem_euclid(p.h as i64) as usize * p.w
                            + sx.rem_euclid(p.w as i64) as usize;
                        (flags::flow(d.flags[src]), d.bc_slot[src] as usize)
                    } else {
                        (flags::BOUNCE_BACK, 0)
                    };
                if flow != flags::BOUNCE_BACK {
                    continue;
                }
                let o = OPPOSITE[i];
                let out = (self.f[o * p.n + k] + W9[o]) as f64;
                let uw = d.bc_params[slot].velocity;
                let cu = (cx as f32 * uw[0] + cy as f32 * uw[1]) as f64;
                let m = 2.0 * out + 6.0 * W9[i] as f64 * cu;
                let e = acc.entry(d.slot_element[slot]).or_default();
                e[0] -= cx as f64 * m;
                e[1] -= cy as f64 * m;
            }
        }
        acc.into_iter()
            .map(|(element, lattice)| ElementForce { element, lattice })
            .collect()
    }
}
