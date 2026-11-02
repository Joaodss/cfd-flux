//! CPU backend: the numerical reference, parallelised over blocks of rows with `rayon`.

use cfd_core::domain::{flags, phase, Domain};
use cfd_core::solver::{
    BackendKind, Diagnostics, ElementForce, LatticeFields, LiquidDiagnostics, Solver, SolverError,
};
use rayon::prelude::*;

use crate::config::LbmConfig;
use crate::kernel::{update_block, Block, Source, StepParams, SurfaceBlock, SurfaceSource};
use crate::shared::{
    equilibrium_populations, initial_fields, les_params, outgoing_index, relaxation_rates,
    surface_fields, surface_planes, wall_forces, wall_links, wet_links, SurfacePlanes, WallLink,
};
use crate::surface::{self, state, FinishBlock};

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
    /// Free-surface planes (`state` is `state_a`, the input and output of a step).
    surf: Option<SurfacePlanes>,
    steps: u64,
    /// Bounce-back links, for momentum-exchange forces.
    links: Vec<WallLink>,
}

impl CpuLbm {
    /// Creates the solver with every cell at the equilibrium of the domain's initial state.
    pub fn new(domain: Domain, config: LbmConfig) -> Self {
        let n = domain.cell_count();
        let thermal = domain.physics.thermal();
        let (omega, omega_g) = relaxation_rates(&domain, config);
        let params = StepParams {
            w: domain.width as usize,
            h: domain.height as usize,
            n,
            periodic: domain.periodic,
            omega,
            omega_g,
            body_force: domain.physics.body_force.map(|v| v as f32),
            buoyancy: domain.physics.buoyancy.map(|v| v as f32),
            les: les_params(&domain, config),
        };
        let links = wall_links(&domain);
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
            surf: None,
            steps: 0,
            links,
            domain,
        };
        let initial = initial_fields(&solver.domain);
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

    /// The free-surface planes after the last step (`None` without a free surface), for tests.
    pub fn surface(&self) -> Option<&SurfacePlanes> {
        self.surf.as_ref()
    }

    /// Rows per parallel block: ~4 blocks per thread balances the load without much overhead.
    fn block_rows(&self) -> usize {
        (self.params.h / (4 * rayon::current_num_threads())).max(1)
    }

    fn step_once(&mut self, write_macro: bool) {
        let (w, n) = (self.params.w, self.params.n);
        let thermal = !self.g.is_empty();
        let rows = self.block_rows();
        let block = rows * w;

        // Disjoint fields of `self.surf`, borrowed separately (the borrow checker accepts it
        // because no plane is both read and written).
        let (surf_src, surf_blocks): (_, Vec<SurfaceBlock>) = match &mut self.surf {
            Some(SurfacePlanes {
                state,
                state_b,
                mass,
                massex,
                phi,
            }) => (
                Some(SurfaceSource { state, massex, phi }),
                state_b
                    .chunks_mut(block)
                    .zip(mass.chunks_mut(block))
                    .map(|(state, mass)| SurfaceBlock { state, mass })
                    .collect(),
            ),
            None => (None, Vec::new()),
        };
        let free_surface = surf_src.is_some();
        let src = Source {
            f: &self.f,
            g: &self.g,
            flags: &self.domain.flags,
            slot: &self.domain.bc_slot,
            bc: &self.domain.bc_params,
            surf: surf_src,
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
        let mut surf_blocks = surf_blocks.into_iter();
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
                surf: surf_blocks.next(),
            })
            .collect();

        let params = &self.params;
        match (thermal, free_surface) {
            (true, _) => blocks
                .into_par_iter()
                .for_each(|b| update_block::<true, false>(params, &src, b, write_macro)),
            (false, true) => blocks
                .into_par_iter()
                .for_each(|b| update_block::<false, true>(params, &src, b, write_macro)),
            (false, false) => blocks
                .into_par_iter()
                .for_each(|b| update_block::<false, false>(params, &src, b, write_macro)),
        }
        if free_surface {
            self.surface_passes();
        }
        std::mem::swap(&mut self.f, &mut self.f_next);
        std::mem::swap(&mut self.g, &mut self.g_next);
    }

    /// Passes 2–4 of a free-surface step (see [`crate::surface`]); `f_next` holds pass 1's output.
    fn surface_passes(&mut self) {
        let (w, n) = (self.params.w, self.params.n);
        let rows = self.block_rows();
        let block = rows * w;
        let p = &self.params;
        let fl = &self.domain.flags;
        let s = self.surf.as_mut().expect("free surface");

        // 2: state_b → state_a.
        s.state
            .par_chunks_mut(block)
            .enumerate()
            .for_each(|(b, out)| surface::fill(p, fl, &s.state_b, b * rows, out));
        // 3: state_a → state_b; then the new interface cells' populations, from values gathered
        // before any is written.
        s.state_b
            .par_chunks_mut(block)
            .enumerate()
            .for_each(|(b, out)| surface::init_states(p, fl, &s.state, b * rows, out));
        let (state_a, rho, ux, uy) = (&s.state, &self.rho, &self.ux, &self.uy);
        let new: Vec<_> = (0..n)
            .into_par_iter()
            .with_min_len(4096)
            .filter(|&k| {
                state::mark(state_a[k]) == state::TO_INTERFACE && flags::flow(fl[k]) == flags::FLUID
            })
            .map(|k| surface::init_value(p, fl, state_a, rho, ux, uy, k))
            .collect();
        for c in &new {
            surface::init_populations(
                n,
                c,
                &mut self.f_next,
                &mut self.rho,
                &mut self.ux,
                &mut self.uy,
            );
        }
        // 4: state_b → state_a, mass, massex, φ.
        let rho = &self.rho;
        let SurfacePlanes {
            state,
            state_b,
            mass,
            massex,
            phi,
        } = s;
        state
            .par_chunks_mut(block)
            .zip(mass.par_chunks_mut(block))
            .zip(massex.par_chunks_mut(block))
            .zip(phi.par_chunks_mut(block))
            .enumerate()
            .for_each(|(b, (((state, mass), massex), phi))| {
                let out = FinishBlock {
                    y0: b * rows,
                    state,
                    mass,
                    massex,
                    phi,
                };
                surface::finish(p, fl, state_b, rho, out)
            });
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
        // With `skip_macro`, only the last step stores ρ, u, θ (they are read only between calls).
        for s in 0..n {
            self.step_once(!self.config.skip_macro || s + 1 == n);
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
        let thermal = !self.g.is_empty();
        (self.f, self.g) = equilibrium_populations(fields, thermal);
        if thermal {
            if let Some(t) = &fields.theta {
                self.theta.copy_from_slice(t);
            } else {
                self.theta.fill(0.0);
            }
        }
        self.rho.copy_from_slice(&fields.density);
        self.ux.copy_from_slice(&fields.ux);
        self.uy.copy_from_slice(&fields.uy);
        self.surf = surface_planes(&self.domain, fields);
        Ok(())
    }

    fn lattice_fields(&mut self) -> LatticeFields {
        let mut lf = LatticeFields {
            density: self.rho.clone(),
            ux: self.ux.clone(),
            uy: self.uy.clone(),
            theta: (!self.g.is_empty()).then(|| self.theta.clone()),
            fill: None,
            phase: None,
        };
        if let Some(s) = &self.surf {
            surface_fields(&mut lf, &s.state, s.phi.clone());
        }
        lf
    }

    fn diagnostics(&mut self) -> Diagnostics {
        #[derive(Clone, Copy)]
        struct Acc {
            cells: usize,
            mass: f64,
            energy: f64,
            umax: f64,
            heat: f64,
            volume: f64,
            interface: usize,
            finite: bool,
        }
        let zero = Acc {
            cells: 0,
            mass: 0.0,
            energy: 0.0,
            umax: 0.0,
            heat: 0.0,
            volume: 0.0,
            interface: 0,
            finite: true,
        };
        let flags = &self.domain.flags;
        let (rho, ux, uy, theta) = (&self.rho, &self.ux, &self.uy, &self.theta);
        let surf = self.surf.as_ref();
        let acc = (0..self.params.n)
            .into_par_iter()
            .with_min_len(4096)
            .filter(|&i| flags::flow(flags[i]) == flags::FLUID)
            .fold(
                || zero,
                |mut a, i| {
                    if let Some(s) = surf {
                        // Liquid mass: every cell's m + m_ex (gas cells may hold the mass of a
                        // cell that emptied with no neighbour to give it to).
                        a.mass += s.mass[i] as f64 + s.massex[i] as f64;
                        a.volume += s.phi[i] as f64;
                        let p = state::phase(s.state[i]);
                        a.interface += (p == phase::INTERFACE) as usize;
                        if !phase::wet(p) {
                            return a;
                        }
                    }
                    let (r, x, y, t) = (rho[i] as f64, ux[i] as f64, uy[i] as f64, theta[i]);
                    let usq = x * x + y * y;
                    a.cells += 1;
                    if surf.is_none() {
                        a.mass += r;
                    }
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
                    volume: a.volume + b.volume,
                    interface: a.interface + b.interface,
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
            finite: acc.finite && acc.mass.is_finite(),
            liquid: surf.map(|_| LiquidDiagnostics {
                volume: acc.volume,
                interface_cells: acc.interface,
            }),
        }
    }

    fn forces(&mut self) -> Vec<ElementForce> {
        let n = self.params.n;
        let outgoing: Vec<f32> = self
            .links
            .iter()
            .map(|l| self.f[outgoing_index(l, n)])
            .collect();
        match &self.surf {
            Some(s) => {
                let (links, outgoing) = wet_links(&self.links, &outgoing, &s.state);
                wall_forces(&self.domain, &links, &outgoing)
            }
            None => wall_forces(&self.domain, &self.links, &outgoing),
        }
    }
}
