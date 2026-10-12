//! `WgpuLbm`: the LBM solver on wgpu, with the memory layout of the CPU reference.
//!
//! Buffers (all `f32`/`u32` storage buffers; the kernels are in `shaders/`):
//!
//! | group | binding | buffer | content |
//! |---|---|---|---|
//! | 0 | 0 | `params` (uniform) | [`params_words`]: sizes, relaxation rates, forces |
//! | 0 | 1 | `flags` | the bytes of `Domain::flags` (`u16`), two cells per word |
//! | 0 | 2 | `slot` | `Domain::bc_slot` |
//! | 0 | 3 | `bc` | `Domain::bc_params`, 32 bytes each |
//! | 1 | 0, 1 | `f[a]`, `f[b]` | D2Q9 populations, read / written |
//! | 1 | 2, 3 | `g[a]`, `g[b]` | D2Q5 populations (a 16-byte dummy without the thermal lattice) |
//! | 1 | 4 | `mac` | planes ρ, ux, uy, θ and a status word |
//!
//! Ping-pong without copies: two bind groups for group 1 are built once, A = (f0 → f1) and
//! B = (f1 → f0); step `s` binds `s % 2`. After `steps` steps the latest populations are in
//! `f[steps % 2]`. wgpu inserts the barrier between consecutive dispatches, so each step sees
//! every write of the previous one.

use std::collections::VecDeque;
use std::sync::Arc;

use cfd_core::domain::Domain;
use cfd_core::solver::{
    BackendKind, Diagnostics, ElementForce, LatticeFields, PendingFields, Solver, SolverError,
};
use cfd_lbm::abi::{bc_words, KernelParams};
use cfd_lbm::shared::{equilibrium_populations, initial_fields, wall_forces, wall_links, WallLink};
use cfd_lbm::LbmConfig;
use wgpu::util::DeviceExt;

use crate::prelude::{source, LBM_WGSL, REDUCE_WGSL};
use crate::readback::{GpuFields, Mapping, Pool};
use crate::{GpuContext, GpuError};

/// Default invocations per workgroup of the stream-collide kernel.
pub const DEFAULT_WORKGROUP: u32 = 128;
/// Steps recorded into one command buffer.
const MAX_BATCH: u32 = 1000;
/// Submitted batches the CPU may run ahead of the GPU.
const MAX_IN_FLIGHT: usize = 2;
/// Workgroup size and maximum number of workgroups of the diagnostics reduction.
const REDUCE_WG: u32 = 256;
const MAX_REDUCE_GROUPS: u32 = 1024;
/// Values per workgroup in the diagnostics partials (see `reduce.wgsl`).
const PARTIALS: usize = 8;

/// The uniform `Params` of the kernels (`shaders/common.wgsl`): the words of
/// [`KernelParams`]. Layout checked against the WGSL struct by a test.
pub(crate) fn params_words(
    d: &Domain,
    config: LbmConfig,
    body_force: [f32; 2],
    links: usize,
) -> [u32; 16] {
    KernelParams::new(d, config, body_force, links).to_words()
}

/// `Domain::flags` as uploaded: the same bytes, padded to whole 32-bit words.
pub(crate) fn flag_bytes(flags: &[u16]) -> Vec<u8> {
    let mut bytes = bytemuck::cast_slice::<u16, u8>(flags).to_vec();
    bytes.resize(bytes.len().next_multiple_of(4), 0);
    bytes
}

/// A submitted batch of steps and the copy of its status word.
struct Batch {
    status: Mapping,
    /// Steps done once the batch has run.
    end_step: u64,
}

pub struct WgpuLbm {
    ctx: Arc<GpuContext>,
    domain: Domain,
    config: LbmConfig,
    thermal: bool,
    n: usize,
    steps: u64,
    workgroup: u32,
    /// Workgroups of the stream-collide dispatch along x and y.
    dispatch: [u32; 2],

    params: wgpu::Buffer,
    body_force: [f32; 2],
    f: [wgpu::Buffer; 2],
    g: [wgpu::Buffer; 2],
    mac: wgpu::Buffer,
    step_pipeline: wgpu::ComputePipeline,
    static_group: wgpu::BindGroup,
    step_groups: [wgpu::BindGroup; 2],

    diag_pipeline: wgpu::ComputePipeline,
    diag_group: wgpu::BindGroup,
    diag_groups: u32,
    partials: wgpu::Buffer,

    links: Vec<WallLink>,
    /// Gather of the outgoing populations, one bind group per `f` buffer; `None` without links.
    link_gather: Option<(wgpu::ComputePipeline, [wgpu::BindGroup; 2], wgpu::Buffer)>,

    in_flight: VecDeque<Batch>,
    status_pool: Pool,
    field_pool: Pool,
    small_pool: Pool,
}

impl WgpuLbm {
    /// Creates the solver on `ctx` with every cell at the equilibrium of the domain's initial
    /// state (exactly the CPU backend's initial populations).
    pub fn new(ctx: Arc<GpuContext>, domain: Domain, config: LbmConfig) -> Result<Self, GpuError> {
        Self::with_workgroup(ctx, domain, config, DEFAULT_WORKGROUP)
    }

    /// As [`new`](Self::new), with a given number of invocations per workgroup (a power of two
    /// up to the device limit; for benchmarks).
    pub fn with_workgroup(
        ctx: Arc<GpuContext>,
        domain: Domain,
        config: LbmConfig,
        workgroup: u32,
    ) -> Result<Self, GpuError> {
        let limits = ctx.limits().clone();
        if workgroup == 0
            || workgroup > limits.max_compute_invocations_per_workgroup
            || workgroup > limits.max_compute_workgroup_size_x
        {
            return Err(GpuError::Validation(format!(
                "workgroup size {workgroup} is not supported by {}",
                ctx.description()
            )));
        }
        let n = domain.cell_count();
        let thermal = domain.physics.thermal();
        let bytes = |values: usize| (values * 4) as u64;
        ctx.check_storage_size("the D2Q9 populations", bytes(9 * n))?;
        if thermal {
            ctx.check_storage_size("the D2Q5 populations", bytes(5 * n))?;
        }
        ctx.check_storage_size("the macroscopic fields", bytes(4 * n + 4))?;

        let groups = n.div_ceil(workgroup as usize) as u64;
        let max_dim = limits.max_compute_workgroups_per_dimension as u64;
        let gx = groups.min(max_dim);
        let gy = groups.div_ceil(gx);
        if gy > max_dim {
            return Err(GpuError::TooLarge {
                what: "the dispatch grid",
                bytes: groups,
                limit: max_dim * max_dim,
                adapter: ctx.description(),
            });
        }

        ctx.scoped(|| {
            Self::build(
                ctx.clone(),
                domain,
                config,
                workgroup,
                [gx as u32, gy as u32],
            )
        })?
    }

    fn build(
        ctx: Arc<GpuContext>,
        domain: Domain,
        config: LbmConfig,
        workgroup: u32,
        dispatch: [u32; 2],
    ) -> Result<Self, GpuError> {
        use wgpu::BufferUsages as U;
        let device = &ctx.device;
        let n = domain.cell_count();
        let thermal = domain.physics.thermal();
        let links = wall_links(&domain);
        let body_force = domain.physics.body_force.map(|v| v as f32);

        let init = |label: &str, contents: &[u8], usage: U| {
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some(label),
                contents,
                usage,
            })
        };
        let empty = |label: &str, size: u64, usage: U| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size,
                usage,
                mapped_at_creation: false,
            })
        };

        let params = init(
            "params",
            bytemuck::cast_slice(&params_words(&domain, config, body_force, links.len())),
            U::UNIFORM | U::COPY_DST,
        );
        let flags = init("flags", &flag_bytes(&domain.flags), U::STORAGE);
        let slot = init("bc_slot", bytemuck::cast_slice(&domain.bc_slot), U::STORAGE);
        let bc_words = bc_words(&domain.bc_params);
        let bc = init("bc_params", bytemuck::cast_slice(&bc_words), U::STORAGE);

        let pop = U::STORAGE | U::COPY_DST | U::COPY_SRC;
        let f = [
            empty("f0", (36 * n) as u64, pop),
            empty("f1", (36 * n) as u64, pop),
        ];
        let g_size = if thermal { (20 * n) as u64 } else { 16 };
        let g = [empty("g0", g_size, pop), empty("g1", g_size, pop)];
        let mac = empty("mac", (16 * n + 16) as u64, pop);

        // ---- Stream-collide pipeline, with explicit layouts (the thermal bindings must exist
        // even when the thermal code is compiled out) ----
        let entry = |binding: u32, ty: wgpu::BufferBindingType| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let static_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lbm static"),
            entries: &[
                entry(0, wgpu::BufferBindingType::Uniform),
                entry(1, ro),
                entry(2, ro),
                entry(3, ro),
            ],
        });
        let step_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("lbm step"),
            entries: &[
                entry(0, ro),
                entry(1, rw),
                entry(2, ro),
                entry(3, rw),
                entry(4, rw),
            ],
        });
        let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("lbm"),
            bind_group_layouts: &[Some(&static_layout), Some(&step_layout)],
            immediate_size: 0,
        });
        let lbm_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("lbm.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source(LBM_WGSL).into()),
        });
        let step_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("stream_collide"),
            layout: Some(&layout),
            module: &lbm_module,
            entry_point: Some("stream_collide"),
            compilation_options: wgpu::PipelineCompilationOptions {
                constants: &[
                    ("WG", workgroup as f64),
                    ("THERMAL", if thermal { 1.0 } else { 0.0 }),
                ],
                ..Default::default()
            },
            cache: None,
        });
        let bind = |layout: &wgpu::BindGroupLayout, label: &str, buffers: &[&wgpu::Buffer]| {
            let entries: Vec<wgpu::BindGroupEntry> = buffers
                .iter()
                .enumerate()
                .map(|(i, b)| wgpu::BindGroupEntry {
                    binding: i as u32,
                    resource: b.as_entire_binding(),
                })
                .collect();
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout,
                entries: &entries,
            })
        };
        let static_group = bind(&static_layout, "lbm static", &[&params, &flags, &slot, &bc]);
        let step_groups = [
            bind(&step_layout, "step A", &[&f[0], &f[1], &g[0], &g[1], &mac]),
            bind(&step_layout, "step B", &[&f[1], &f[0], &g[1], &g[0], &mac]),
        ];

        // ---- Reductions: layouts derived from each entry point's bindings ----
        let reduce_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("reduce.wgsl"),
            source: wgpu::ShaderSource::Wgsl(source(REDUCE_WGSL).into()),
        });
        let reduce_pipeline = |entry: &str| {
            device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                label: Some(entry),
                layout: None,
                module: &reduce_module,
                entry_point: Some(entry),
                compilation_options: Default::default(),
                cache: None,
            })
        };
        let bind_at = |pipeline: &wgpu::ComputePipeline, entries: &[(u32, &wgpu::Buffer)]| {
            let entries: Vec<wgpu::BindGroupEntry> = entries
                .iter()
                .map(|&(binding, b)| wgpu::BindGroupEntry {
                    binding,
                    resource: b.as_entire_binding(),
                })
                .collect();
            device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &entries,
            })
        };
        let diag_groups = (n.div_ceil(REDUCE_WG as usize) as u32).clamp(1, MAX_REDUCE_GROUPS);
        let partial_bytes = (diag_groups as usize * PARTIALS * 4) as u64;
        let partials = empty("partials", partial_bytes, U::STORAGE | U::COPY_SRC);
        let diag_pipeline = reduce_pipeline("diagnostics");
        let diag_group = bind_at(
            &diag_pipeline,
            &[(0, &params), (1, &flags), (2, &mac), (3, &partials)],
        );
        let link_gather = if links.is_empty() {
            None
        } else {
            let packed: Vec<u32> = links.iter().map(|l| l.cell << 4 | l.dir as u32).collect();
            let link_buf = init("links", bytemuck::cast_slice(&packed), U::STORAGE);
            let out = empty(
                "link_out",
                (links.len() * 4) as u64,
                U::STORAGE | U::COPY_SRC,
            );
            let pipeline = reduce_pipeline("gather_links");
            let groups = [0, 1].map(|i| {
                bind_at(
                    &pipeline,
                    &[(0, &params), (4, &link_buf), (5, &f[i]), (6, &out)],
                )
            });
            Some((pipeline, groups, out))
        };

        let small = partial_bytes.max(links.len() as u64 * 4).max(16);
        let mut solver = Self {
            status_pool: Pool::new(16),
            field_pool: Pool::new((16 * n) as u64),
            small_pool: Pool::new(small),
            ctx,
            domain,
            config,
            thermal,
            n,
            steps: 0,
            workgroup,
            dispatch,
            params,
            body_force,
            f,
            g,
            mac,
            step_pipeline,
            static_group,
            step_groups,
            diag_pipeline,
            diag_group,
            diag_groups,
            partials,
            links,
            link_gather,
            in_flight: VecDeque::new(),
        };
        let initial = initial_fields(&solver.domain);
        solver.upload_equilibrium(&initial);
        Ok(solver)
    }

    pub fn context(&self) -> &Arc<GpuContext> {
        &self.ctx
    }

    pub fn config(&self) -> &LbmConfig {
        &self.config
    }

    pub fn workgroup_size(&self) -> u32 {
        self.workgroup
    }

    /// Updates the body force (lattice units), e.g. for validation cases.
    pub fn set_body_force(&mut self, force: [f64; 2]) {
        self.domain.physics.body_force = force;
        self.body_force = force.map(|v| v as f32);
        let words = params_words(&self.domain, self.config, self.body_force, self.links.len());
        self.ctx
            .queue
            .write_buffer(&self.params, 0, bytemuck::cast_slice(&words));
    }

    /// Writes the equilibrium of `fields` into the current population buffers and the
    /// macroscopic planes, and clears the status word. Queue writes are ordered with the
    /// submissions, so steps already queued still see the old state.
    fn upload_equilibrium(&mut self, fields: &LatticeFields) {
        let q = &self.ctx.queue;
        let cur = (self.steps % 2) as usize;
        let (f, g) = equilibrium_populations(fields, self.thermal);
        q.write_buffer(&self.f[cur], 0, bytemuck::cast_slice(&f));
        if self.thermal {
            q.write_buffer(&self.g[cur], 0, bytemuck::cast_slice(&g));
        }
        let n = self.n;
        let mut mac = Vec::with_capacity(4 * n + 4);
        mac.extend_from_slice(&fields.density);
        mac.extend_from_slice(&fields.ux);
        mac.extend_from_slice(&fields.uy);
        match (&fields.theta, self.thermal) {
            (Some(t), true) => mac.extend_from_slice(t),
            _ => mac.resize(4 * n, 0.0),
        }
        mac.resize(4 * n + 4, 0.0);
        q.write_buffer(&self.mac, 0, bytemuck::cast_slice(&mac));
    }

    fn device_error(&self) -> Result<(), SolverError> {
        match self.ctx.take_error() {
            Some(e) => Err(SolverError::Backend(e)),
            None => Ok(()),
        }
    }

    /// Checks the status word of a finished batch.
    fn retire(&mut self, batch: Batch) -> Result<(), SolverError> {
        batch.status.wait(&self.ctx);
        let mut status = Vec::new();
        match batch.status.read(&mut status) {
            Some(buffer) => self.status_pool.give(buffer),
            None => {
                self.device_error()?;
                return Err(SolverError::Backend("status readback failed".into()));
            }
        }
        if status[0] != 0.0 {
            return Err(SolverError::NonFinite {
                step: batch.end_step,
            });
        }
        Ok(())
    }

    /// Retires every batch the GPU has already finished, without blocking.
    fn retire_finished(&mut self) -> Result<(), SolverError> {
        let _ = self.ctx.device.poll(wgpu::PollType::Poll);
        while self.in_flight.front().is_some_and(|b| b.status.is_done()) {
            let b = self.in_flight.pop_front().expect("front exists");
            self.retire(b)?;
        }
        Ok(())
    }

    /// Records and submits up to `MAX_BATCH` steps, followed by a copy of the status word.
    fn submit_batch(&mut self, steps: u32) {
        let device = &self.ctx.device;
        let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("steps"),
        });
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("stream_collide"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.step_pipeline);
            pass.set_bind_group(0, &self.static_group, &[]);
            for s in 0..steps as u64 {
                let parity = ((self.steps + s) % 2) as usize;
                pass.set_bind_group(1, &self.step_groups[parity], &[]);
                pass.dispatch_workgroups(self.dispatch[0], self.dispatch[1], 1);
            }
        }
        let staging = self.status_pool.take(&self.ctx);
        enc.copy_buffer_to_buffer(&self.mac, (16 * self.n) as u64, &staging, 0, 4);
        let index = self.ctx.queue.submit([enc.finish()]);
        self.steps += steps as u64;
        self.in_flight.push_back(Batch {
            status: Mapping::start(staging, index),
            end_step: self.steps,
        });
    }

    /// Copies `size` bytes of `src` into a staging buffer of `pool` and waits for them.
    fn read_now(
        &self,
        pool: &Pool,
        src: &wgpu::Buffer,
        size: u64,
        mut enc: wgpu::CommandEncoder,
    ) -> Vec<f32> {
        let staging = pool.take(&self.ctx);
        enc.copy_buffer_to_buffer(src, 0, &staging, 0, size);
        let index = self.ctx.queue.submit([enc.finish()]);
        let mapping = Mapping::start(staging, index);
        mapping.wait(&self.ctx);
        let mut out = Vec::new();
        if let Some(buffer) = mapping.read(&mut out) {
            pool.give(buffer);
        }
        out.truncate(size as usize / 4);
        out
    }

    fn encoder(&self) -> wgpu::CommandEncoder {
        self.ctx
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: None })
    }
}

impl From<GpuError> for SolverError {
    fn from(e: GpuError) -> Self {
        SolverError::Backend(e.to_string())
    }
}

impl Solver for WgpuLbm {
    fn backend(&self) -> BackendKind {
        BackendKind::Wgpu
    }

    fn domain(&self) -> &Domain {
        &self.domain
    }

    fn steps_done(&self) -> u64 {
        self.steps
    }

    /// Submits the steps and returns; the CPU waits only when more than `MAX_IN_FLIGHT`
    /// batches are queued. Divergence is detected by the kernel (status word) and reported when
    /// the batch is retired.
    fn step(&mut self, n: u32) -> Result<(), SolverError> {
        self.device_error()?;
        self.retire_finished()?;
        let mut left = n;
        while left > 0 {
            let chunk = left.min(MAX_BATCH);
            self.submit_batch(chunk);
            left -= chunk;
            while self.in_flight.len() > MAX_IN_FLIGHT {
                let b = self.in_flight.pop_front().expect("in flight");
                self.retire(b)?;
            }
        }
        self.retire_finished()
    }

    fn finish(&mut self) -> Result<(), SolverError> {
        while let Some(b) = self.in_flight.pop_front() {
            self.retire(b)?;
        }
        self.device_error()
    }

    fn set_equilibrium(&mut self, fields: &LatticeFields) -> Result<(), SolverError> {
        fields.check_len(self.n)?;
        self.upload_equilibrium(fields);
        Ok(())
    }

    fn lattice_fields(&mut self) -> LatticeFields {
        self.request_fields().wait()
    }

    fn request_fields(&mut self) -> Box<dyn PendingFields> {
        let staging = self.field_pool.take(&self.ctx);
        let mut enc = self.encoder();
        enc.copy_buffer_to_buffer(&self.mac, 0, &staging, 0, (16 * self.n) as u64);
        let index = self.ctx.queue.submit([enc.finish()]);
        Box::new(GpuFields {
            ctx: self.ctx.clone(),
            mapping: Some(Mapping::start(staging, index)),
            pool: self.field_pool.clone(),
            step: self.steps,
            n: self.n,
            thermal: self.thermal,
        })
    }

    fn diagnostics(&mut self) -> Diagnostics {
        let mut enc = self.encoder();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("diagnostics"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.diag_pipeline);
            pass.set_bind_group(0, &self.diag_group, &[]);
            pass.dispatch_workgroups(self.diag_groups, 1, 1);
        }
        let size = (self.diag_groups as usize * PARTIALS * 4) as u64;
        let p = self.read_now(&self.small_pool, &self.partials, size, enc);
        let (mut cells, mut dmass, mut energy, mut umax, mut heat) = (0.0, 0.0, 0.0, 0.0f64, 0.0);
        let mut finite = p.len() == self.diag_groups as usize * PARTIALS;
        for w in p.chunks_exact(PARTIALS) {
            cells += w[0] as f64;
            dmass += w[1] as f64;
            energy += w[2] as f64;
            umax = umax.max(w[3] as f64);
            heat += w[4] as f64;
            finite &= w[5] == 0.0;
        }
        Diagnostics {
            step: self.steps,
            fluid_cells: cells as usize,
            mass: cells + dmass,
            kinetic_energy: energy,
            max_velocity: umax,
            thermal_energy: self.thermal.then_some(heat),
            finite: finite && umax.is_finite() && energy.is_finite(),
        }
    }

    fn forces(&mut self) -> Vec<ElementForce> {
        let Some((pipeline, groups, out)) = &self.link_gather else {
            return Vec::new();
        };
        let mut enc = self.encoder();
        {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gather_links"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &groups[(self.steps % 2) as usize], &[]);
            pass.dispatch_workgroups((self.links.len() as u32).div_ceil(REDUCE_WG), 1, 1);
        }
        let outgoing = self.read_now(&self.small_pool, out, (self.links.len() * 4) as u64, enc);
        wall_forces(&self.domain, &self.links, &outgoing)
    }
}

// `Solver: Send`: the solver may move to a worker thread (wgpu handles are `Send + Sync`).
const _: fn() = || {
    fn is_send<T: Send>() {}
    is_send::<WgpuLbm>();
};
