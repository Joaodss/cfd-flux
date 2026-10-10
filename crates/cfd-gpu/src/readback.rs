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

use cfd_core::solver::{LatticeFields, PendingFields};

use crate::GpuContext;

const PENDING: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

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

    /// Blocks until the submission has completed and the callback has run.
    pub fn wait(&self, ctx: &GpuContext) {
        if !self.is_done() {
            let _ = ctx.device.poll(wgpu::PollType::Wait {
                submission_index: Some(self.index.clone()),
                timeout: None,
            });
        }
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
            m.wait(&self.ctx);
        }
        self.collect()
    }
}
