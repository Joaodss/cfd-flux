//! Copies from GPU buffers to the CPU.
//!
//! The GPU cannot write into ordinary CPU memory: data is first copied (on the GPU timeline)
//! into a *staging* buffer created with `MAP_READ`, then the buffer is *mapped* — wgpu calls back
//! once the copy has finished and the bytes are visible to the CPU (`map_async` is the
//! callback-style equivalent of awaiting a `Task`). [`device.poll`](wgpu::Device::poll) drives
//! those callbacks: `PollType::Poll` checks without blocking, `PollType::Wait` blocks until a
//! given submission has completed.

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use cfd_core::sample::{PendingSamples, SampleSet, SampleSpec};
use cfd_core::solver::{LatticeFields, PendingFields};
use cfd_lbm::abi::SampleLayout;

use crate::GpuContext;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// How long [`Mapping::wait`] waits for a callback that another thread's poll has collected
/// (normally microseconds; the bound only turns a wgpu bug into an error instead of a hang).
const CALLBACK_GRACE: Duration = Duration::from_secs(10);

/// A staging buffer being mapped, with the flag set by the map callback.
pub(crate) struct Mapping {
    pub buffer: wgpu::Buffer,
    pub index: wgpu::SubmissionIndex,
    state: Arc<AtomicU8>,
}

impl Mapping {
    /// Requests the mapping of `buffer` after the submission `index` (which copies into it).
    pub fn start(buffer: wgpu::Buffer, index: wgpu::SubmissionIndex) -> Self {
        let state = Arc::new(AtomicU8::new(PENDING));
        let flag = state.clone();
        buffer.map_async(wgpu::MapMode::Read, .., move |r| {
            flag.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
        Self {
            buffer,
            index,
            state,
        }
    }

    /// Whether the callback has run (call `device.poll` first to give it a chance).
    pub fn is_done(&self) -> bool {
        self.state.load(Ordering::Acquire) != PENDING
    }

    /// Blocks until the submission has completed and the callback has run; `Err` (with the
    /// reason) if the poll fails or the callback still has not run after [`CALLBACK_GRACE`].
    ///
    /// `poll(Wait)` alone is not enough: a poll collects the callbacks of *every* finished
    /// submission and runs them after it returns, on the thread that polled. When another thread
    /// polls the same device (solvers sharing one context on several threads), it can collect
    /// this callback, and our `Wait` then returns before the callback has run.
    pub fn wait(&self, ctx: &GpuContext) -> Result<(), String> {
        let deadline = Instant::now() + CALLBACK_GRACE;
        while !self.is_done() {
            let polled = ctx.device.poll(wgpu::PollType::Wait {
                submission_index: Some(self.index.clone()),
                timeout: None,
            });
            if self.is_done() {
                break;
            }
            if let Err(e) = polled {
                return Err(format!("device poll failed: {e}"));
            }
            // The submission has completed, so the callback is about to run on the thread that
            // collected it. Further `Wait`s return at once: this loop yields rather than blocks.
            if Instant::now() >= deadline {
                return Err(format!(
                    "map callback did not run within {CALLBACK_GRACE:?} of the copy completing"
                ));
            }
            std::thread::yield_now();
        }
        Ok(())
    }

    /// Reads the mapped bytes as `f32`, unmaps, and returns the buffer for reuse.
    /// `None` if mapping failed (device lost).
    pub fn read(self, out: &mut Vec<f32>) -> Option<wgpu::Buffer> {
        if self.state.load(Ordering::Acquire) != MAPPED {
            return None;
        }
        {
            let view = self.buffer.get_mapped_range(..).ok()?;
            out.clear();
            out.extend_from_slice(bytemuck::cast_slice::<u8, f32>(&view));
        }
        self.buffer.unmap();
        Some(self.buffer)
    }
}

/// Staging buffers of one size, reused across copies.
#[derive(Clone)]
pub(crate) struct Pool {
    size: u64,
    free: Arc<Mutex<Vec<wgpu::Buffer>>>,
}

impl Pool {
    pub fn new(size: u64) -> Self {
        Self {
            size,
            free: Arc::default(),
        }
    }

    pub fn take(&self, ctx: &GpuContext) -> wgpu::Buffer {
        let reused = self.free.lock().unwrap_or_else(|p| p.into_inner()).pop();
        reused.unwrap_or_else(|| {
            ctx.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("staging"),
                size: self.size,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        })
    }

    pub fn give(&self, buffer: wgpu::Buffer) {
        self.free
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(buffer);
    }
}

/// A copy of the macroscopic fields in flight (see `Solver::request_fields`).
pub(crate) struct GpuFields {
    pub ctx: Arc<GpuContext>,
    pub mapping: Option<Mapping>,
    pub pool: Pool,
    pub step: u64,
    pub n: usize,
    pub thermal: bool,
}

impl GpuFields {
    fn collect(&mut self) -> LatticeFields {
        let mapping = self.mapping.take().expect("fields already taken");
        let mut raw = Vec::new();
        match mapping.read(&mut raw) {
            Some(buffer) => self.pool.give(buffer),
            // Device lost: report NaN fields rather than panicking; `step` returns the error.
            None => raw = vec![f32::NAN; 4 * self.n],
        }
        split_fields(&raw, self.n, self.thermal)
    }
}

/// Splits the four planes ρ, ux, uy, θ.
pub(crate) fn split_fields(raw: &[f32], n: usize, thermal: bool) -> LatticeFields {
    LatticeFields {
        density: raw[..n].to_vec(),
        ux: raw[n..2 * n].to_vec(),
        uy: raw[2 * n..3 * n].to_vec(),
        theta: thermal.then(|| raw[3 * n..4 * n].to_vec()),
    }
}

impl PendingFields for GpuFields {
    fn step(&self) -> u64 {
        self.step
    }

    fn try_take(&mut self) -> Option<LatticeFields> {
        let m = self.mapping.as_ref()?;
        if !m.is_done() {
            let _ = self.ctx.device.poll(wgpu::PollType::Poll);
            if !m.is_done() {
                return None;
            }
        }
        Some(self.collect())
    }

    fn wait(mut self: Box<Self>) -> LatticeFields {
        if let Some(m) = &self.mapping {
            // A failed wait leaves the mapping unread: `collect` reports NaN fields.
            let _ = m.wait(&self.ctx);
        }
        self.collect()
    }
}

/// Samples being copied (see `Solver::request_samples`): one staging buffer per spec, all
/// filled by one submission (the sampling dispatches and the copies of their outputs).
pub(crate) struct GpuSamples {
    pub ctx: Arc<GpuContext>,
    pub items: Vec<SampleCopy>,
    pub step: u64,
    pub time: f64,
}

pub(crate) struct SampleCopy {
    pub spec: SampleSpec,
    pub layout: SampleLayout,
    pub mapping: Mapping,
    pub pool: Pool,
}

impl PendingSamples for GpuSamples {
    fn step(&self) -> u64 {
        self.step
    }

    fn wait(self: Box<Self>) -> SampleSet {
        let samples = self
            .items
            .into_iter()
            .map(|c| {
                // A failed wait leaves the mapping unread: NaN samples below.
                let _ = c.mapping.wait(&self.ctx);
                let mut raw = Vec::new();
                match c.mapping.read(&mut raw) {
                    Some(buffer) => c.pool.give(buffer),
                    // Device lost: NaN samples rather than a panic; `step` reports the error.
                    None => raw = vec![f32::NAN; c.layout.words],
                }
                let words: &[u32] = bytemuck::cast_slice(&raw);
                c.layout.sample(&c.spec, self.step, self.time, words)
            })
            .collect();
        SampleSet::Ready(samples)
    }
}
