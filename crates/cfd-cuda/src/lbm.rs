//! `CudaLbm`: the LBM solver on CUDA, with the memory layout of the CPU reference.
//!
//! Device buffers (`raw::Lattice`), the same as the CPU and wgpu backends:
//!
//! | buffer | content |
//! |---|---|
//! | `flags` | `Domain::flags` (`u16`), read directly as `unsigned short` |
//! | `slot`, `bc` | `Domain::bc_slot`; `Domain::bc_params` as 32-byte records |
//! | `f[a]`, `f[b]` | D2Q9 populations, SoA `f[i·n + cell]`, shifted (`f̃ = f − w`) |
//! | `g[a]`, `g[b]` | D2Q5 populations (a single dummy value without the thermal lattice) |
//! | `mac` | planes ρ, ux, uy, θ and a status word at `4n` |
//!
//! Ping-pong without copies: step `s` reads `f[s % 2]` and writes `f[(s + 1) % 2]`. Kernel
//! arguments are captured when a launch is queued, so swapping is just passing the other pointer
//! (wgpu needed two prebuilt bind groups for the same effect). The run constants
//! ([`KernelParams`]) are passed by value with every launch.
//!
//! Stepping never waits for the GPU unless it is too far behind: a launch only queues work on the
//! solver's compute stream (≈ a few µs of CPU time). `step(n)` queues the steps in batches of up to
//! `MAX_BATCH`, each followed by a 4-byte copy of the status word into pinned memory and an event;
//! the CPU waits only when more than `MAX_IN_FLIGHT` batches are queued, and checks each batch's
//! status word when its event has fired (ADR-020, as in the wgpu backend). Within a batch, steps
//! are replayed from a CUDA graph of `graph_steps` steps (see `raw::StepGraph`), which cuts the CPU
//! cost per step from ≈ 8 µs (one launch) to ≈ 0.6 µs.

use std::collections::VecDeque;
use std::sync::Arc;

use cfd_core::domain::Domain;
use cfd_core::solver::{
    BackendKind, Diagnostics, ElementForce, LatticeFields, PendingFields, ReadyFields, Solver,
    SolverError,
};
use cfd_lbm::abi::{bc_words, KernelParams};
use cfd_lbm::shared::{
    diagnostics_from_partials, equilibrium_populations, initial_fields, wall_forces, wall_links,
    WallLink,
};
use cfd_lbm::LbmConfig;
use cudarc::driver::sys::CUevent_flags;
use cudarc::driver::{CudaEvent, CudaStream, DriverError, PinnedHostSlice};

use crate::raw::{pinned_zeroed, Kernels, Lattice, LatticeInit, REDUCE_BLOCK};
use crate::readback::{split_fields, CudaFields, Pool, Snapshot};
use crate::{CudaDevice, CudaError};

/// Default threads per block of the stream-collide kernel.
pub const DEFAULT_BLOCK: u32 = 128;
/// Default number of steps per CUDA graph (see `raw::StepGraph`).
pub const DEFAULT_GRAPH_STEPS: u32 = 50;
/// Maximum number of blocks of the diagnostics reduction.
const MAX_REDUCE_BLOCKS: u32 = 1024;
/// Steps queued between two status checks.
const MAX_BATCH: u32 = 1000;
/// Queued batches the CPU may run ahead of the GPU.
const MAX_IN_FLIGHT: usize = 2;
/// Events the CPU waits on block the thread (instead of spinning on a core, the default).
const EVENT_FLAGS: Option<CUevent_flags> = Some(CUevent_flags::CU_EVENT_BLOCKING_SYNC);

/// A queued batch of steps and the copy of its status word.
struct Batch {
    done: CudaEvent,
    status: PinnedHostSlice<f32>,
    /// Steps done once the batch has run.
    end_step: u64,
}

/// Launch and compilation options (for benchmarks and the FMA comparison).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CudaLbmOptions {
    /// Threads per block of the stream-collide kernel (1–1024).
    pub block: u32,
    /// Let the compiler fuse multiply-adds (`--fmad=true`). Off by default: every operation then
    /// rounds as in the CPU reference.
    pub fmad: bool,
    /// Steps replayed per CUDA graph launch (an even number; 0 launches every step on its own).
    pub graph_steps: u32,
}

impl Default for CudaLbmOptions {
    fn default() -> Self {
        Self {
            block: DEFAULT_BLOCK,
            fmad: false,
            graph_steps: DEFAULT_GRAPH_STEPS,
        }
    }
}

pub struct CudaLbm {
    dev: Arc<CudaDevice>,
    /// The compute stream: every kernel and copy of this solver, in order.
    stream: Arc<CudaStream>,
    /// Transfers of field snapshots to the CPU, concurrent with the compute stream.
    copy_stream: Arc<CudaStream>,
    in_flight: VecDeque<Batch>,
    status_pool: Vec<PinnedHostSlice<f32>>,
    snapshots: Pool<Snapshot>,
    kernels: Kernels,
    domain: Domain,
    config: LbmConfig,
    options: CudaLbmOptions,
    thermal: bool,
    n: usize,
    steps: u64,
    params: KernelParams,
    lattice: Lattice,
    links: Vec<WallLink>,
    reduce_blocks: u32,
}

fn driver(e: DriverError) -> SolverError {
    SolverError::Backend(format!("CUDA: {e}"))
}

impl From<CudaError> for SolverError {
    fn from(e: CudaError) -> Self {
        SolverError::Backend(e.to_string())
    }
}

impl CudaLbm {
    /// Creates the solver on `dev` with every cell at the equilibrium of the domain's initial
    /// state (exactly the CPU backend's initial populations).
    pub fn new(dev: Arc<CudaDevice>, domain: Domain, config: LbmConfig) -> Result<Self, CudaError> {
        Self::with_options(dev, domain, config, CudaLbmOptions::default())
    }

    pub fn with_options(
        dev: Arc<CudaDevice>,
        domain: Domain,
        config: LbmConfig,
        options: CudaLbmOptions,
    ) -> Result<Self, CudaError> {
        if !options.graph_steps.is_multiple_of(2) {
            return Err(CudaError::Driver(format!(
                "graph_steps must be even (got {})",
                options.graph_steps
            )));
        }
        if !(1..=1024).contains(&options.block) {
            return Err(CudaError::Driver(format!(
                "block size {} is not supported (1–1024)",
                options.block
            )));
        }
        let n = domain.cell_count();
        let thermal = domain.physics.thermal();
        let links = wall_links(&domain);
        dev.check_memory(
            "the lattice buffers",
            n as u64 * Lattice::bytes_per_cell(thermal) + links.len() as u64 * 8,
        )?;
        let kernels = dev.kernels(options.fmad)?;
        let err = |e: DriverError| CudaError::Driver(e.to_string());
        let stream = dev.context().new_stream().map_err(err)?;
        let copy_stream = dev.context().new_stream().map_err(err)?;
        let reduce_blocks = (n.div_ceil(REDUCE_BLOCK as usize) as u32).clamp(1, MAX_REDUCE_BLOCKS);
        let packed: Vec<u32> = links.iter().map(|l| l.cell << 4 | l.dir as u32).collect();
        let lattice = Lattice::new(
            &stream,
            &LatticeInit {
                flags: &domain.flags,
                slot: &domain.bc_slot,
                bc_words: &bc_words(&domain.bc_params),
                links: &packed,
                thermal,
                blocks: reduce_blocks,
            },
        )
        .map_err(err)?;
        let body_force = domain.physics.body_force.map(|v| v as f32);
        let params = KernelParams::new(&domain, config, body_force, links.len());
        let mut solver = Self {
            dev,
            stream,
            copy_stream,
            in_flight: VecDeque::new(),
            status_pool: Vec::new(),
            snapshots: Pool::default(),
            kernels,
            domain,
            config,
            options,
            thermal,
            n,
            steps: 0,
            params,
            lattice,
            links,
            reduce_blocks,
        };
        let initial = initial_fields(&solver.domain);
        solver.upload_equilibrium(&initial).map_err(err)?;
        Ok(solver)
    }

    pub fn device(&self) -> &Arc<CudaDevice> {
        &self.dev
    }

    pub fn config(&self) -> &LbmConfig {
        &self.config
    }

    pub fn options(&self) -> CudaLbmOptions {
        self.options
    }

    /// Updates the body force (lattice units), e.g. for validation cases. Launches capture the
    /// parameters when they are queued, so steps already queued keep the old force.
    pub fn set_body_force(&mut self, force: [f64; 2]) {
        self.domain.physics.body_force = force;
        self.params.body_force = force.map(|v| v as f32);
    }

    /// Writes the equilibrium of `fields` into the current population buffers and the
    /// macroscopic planes, and clears the status word (queued after earlier work).
    fn upload_equilibrium(&mut self, fields: &LatticeFields) -> Result<(), DriverError> {
        let cur = (self.steps % 2) as usize;
        let (f, g) = equilibrium_populations(fields, self.thermal);
        self.stream.memcpy_htod(&f, &mut self.lattice.f_mut(cur))?;
        if self.thermal {
            self.stream.memcpy_htod(&g, &mut self.lattice.g_mut(cur))?;
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
        self.stream.memcpy_htod(&mac, &mut self.lattice.mac_mut())
    }

    /// Queues `steps` steps, then a copy of the status word and an event. Whole graphs are
    /// replayed while the populations are in `f[0]`; single launches do the rest.
    fn submit_batch(&mut self, steps: u32) -> Result<(), DriverError> {
        let graph = self.options.graph_steps;
        let mut left = steps;
        while left > 0 {
            if graph > 0 && left >= graph && self.steps.is_multiple_of(2) {
                self.lattice.replay_steps(
                    &self.kernels,
                    &self.stream,
                    self.options.block,
                    &self.params,
                    graph,
                )?;
                self.steps += graph as u64;
                left -= graph;
            } else {
                let parity = (self.steps % 2) as usize;
                self.lattice.stream_collide(
                    &self.kernels,
                    &self.stream,
                    self.options.block,
                    &self.params,
                    parity,
                )?;
                self.steps += 1;
                left -= 1;
            }
        }
        let mut status = match self.status_pool.pop() {
            Some(s) => s,
            None => pinned_zeroed(self.dev.context(), 1)?,
        };
        let n = self.n;
        self.stream
            .memcpy_dtoh(&self.lattice.mac().slice(4 * n..4 * n + 1), &mut status)?;
        let done = self.stream.record_event(EVENT_FLAGS)?;
        self.in_flight.push_back(Batch {
            done,
            status,
            end_step: self.steps,
        });
        Ok(())
    }

    /// Waits for a batch and checks its status word (non-zero once a step has read a
    /// non-finite population).
    fn retire(&mut self, batch: Batch) -> Result<(), SolverError> {
        batch.done.synchronize().map_err(driver)?;
        let status = batch.status.as_slice().map_err(driver)?[0];
        self.status_pool.push(batch.status);
        if status != 0.0 {
            return Err(SolverError::NonFinite {
                step: batch.end_step,
            });
        }
        Ok(())
    }

    /// Retires every batch the GPU has already finished, without blocking.
    fn retire_finished(&mut self) -> Result<(), SolverError> {
        while let Some(b) = self.in_flight.front() {
            // An error (e.g. a failed kernel) is reported by `retire`.
            if matches!(b.done.try_is_complete(), Ok(false)) {
                break;
            }
            let b = self.in_flight.pop_front().expect("front exists");
            self.retire(b)?;
        }
        Ok(())
    }

    /// Takes a snapshot of the macroscopic planes on the compute stream and starts copying it
    /// to pinned memory on the copy stream (see `readback.rs`).
    fn start_copy(&mut self) -> Result<CudaFields, DriverError> {
        let n = self.n;
        let mut snapshot = match self.snapshots.take() {
            Some(s) => s,
            None => Snapshot {
                device: self.stream.alloc_zeros(4 * n)?,
                host: pinned_zeroed(self.dev.context(), 4 * n)?,
            },
        };
        self.stream
            .memcpy_dtod(&self.lattice.mac().slice(..4 * n), &mut snapshot.device)?;
        let taken = self.stream.record_event(EVENT_FLAGS)?;
        self.copy_stream.wait(&taken)?;
        self.copy_stream
            .memcpy_dtoh(&snapshot.device, &mut snapshot.host)?;
        let done = self.copy_stream.record_event(EVENT_FLAGS)?;
        Ok(CudaFields {
            done,
            snapshot: Some(snapshot),
            pool: self.snapshots.clone(),
            step: self.steps,
            n,
            thermal: self.thermal,
        })
    }
}

impl Solver for CudaLbm {
    fn backend(&self) -> BackendKind {
        BackendKind::Cuda
    }

    fn domain(&self) -> &Domain {
        &self.domain
    }

    fn steps_done(&self) -> u64 {
        self.steps
    }

    /// Queues the steps and returns; the CPU waits only when more than `MAX_IN_FLIGHT` batches
    /// are queued. Divergence is detected by the kernel (status word) and reported when the batch
    /// is retired.
    fn step(&mut self, n: u32) -> Result<(), SolverError> {
        self.retire_finished()?;
        let mut left = n;
        while left > 0 {
            let chunk = left.min(MAX_BATCH);
            self.submit_batch(chunk).map_err(driver)?;
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
        self.stream.synchronize().map_err(driver)
    }

    fn set_equilibrium(&mut self, fields: &LatticeFields) -> Result<(), SolverError> {
        fields.check_len(self.n)?;
        self.upload_equilibrium(fields).map_err(driver)
    }

    fn lattice_fields(&mut self) -> LatticeFields {
        self.request_fields().wait()
    }

    fn request_fields(&mut self) -> Box<dyn PendingFields> {
        match self.start_copy() {
            Ok(pending) => Box::new(pending),
            // Device error: NaN fields, as a lost device gives on wgpu; `step` reports the error.
            Err(_) => Box::new(ReadyFields {
                step: self.steps,
                fields: Some(split_fields(
                    &vec![f32::NAN; 4 * self.n],
                    self.n,
                    self.thermal,
                )),
            }),
        }
    }

    fn diagnostics(&mut self) -> Diagnostics {
        let partials = self
            .lattice
            .diagnostics(&self.kernels, &self.stream, &self.params)
            .and_then(|_| self.stream.clone_dtoh(self.lattice.partials()))
            .unwrap_or_default();
        diagnostics_from_partials(
            self.steps,
            &partials,
            self.reduce_blocks as usize,
            self.thermal,
        )
    }

    fn forces(&mut self) -> Vec<ElementForce> {
        if self.links.is_empty() {
            return Vec::new();
        }
        let current = (self.steps % 2) as usize;
        let outgoing = self
            .lattice
            .gather_links(&self.kernels, &self.stream, &self.params, current)
            .and_then(|_| self.stream.clone_dtoh(self.lattice.link_out()))
            .unwrap_or_else(|_| vec![f32::NAN; self.links.len()]);
        wall_forces(&self.domain, &self.links, &outgoing)
    }
}

impl Drop for CudaLbm {
    fn drop(&mut self) {
        // Event tracking is off (see `raw::disable_event_tracking`): make sure no queued work
        // still uses the buffers before they are freed. Pending field copies own their snapshot
        // and wait for their own copy when dropped.
        let _ = self.stream.synchronize();
        let _ = self.copy_stream.synchronize();
    }
}

// `Solver: Send`: the solver may move to a worker thread (`cudarc` handles are `Send + Sync`).
const _: fn() = || {
    fn is_send<T: Send>() {}
    is_send::<CudaLbm>();
};
