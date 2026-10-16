//! Copies of the macroscopic fields to the CPU that overlap with stepping.
//!
//! A GPU has separate *copy engines* (DMA units) next to its compute units, and CUDA exposes them
//! through streams: work on different streams may run at the same time. A field request
//! (`CudaLbm::request_fields`) therefore does two things:
//!
//! 1. on the compute stream, a device-to-device copy of `mac` into a *snapshot* buffer (16 MB at
//!    1024², ≈ 0.06 ms at the device's bandwidth), followed by an event marking its completion;
//! 2. on the copy stream, which first waits for that event, a copy of the snapshot into pinned
//!    host memory (≈ 1.2 ms over the RTX 4060's PCIe ×8 link), followed by a second event.
//!
//! The compute stream goes on stepping as soon as the snapshot is taken; the PCIe transfer runs
//! on a copy engine meanwhile. (wgpu has a single queue, so its copy waits for the steps and the
//! steps wait for the copy.) Events are the synchronisation points: the CPU polls or waits on
//! them, and one stream can wait for another's event — like `Task.ContinueWith` across two
//! schedulers. Snapshots go back to a pool only once their copy has completed.

use std::sync::{Arc, Mutex};

use cfd_core::sample::{PendingSamples, SampleSet, SampleSpec};
use cfd_core::solver::{LatticeFields, PendingFields};
use cfd_lbm::abi::SampleLayout;
use cudarc::driver::{CudaEvent, CudaSlice, PinnedHostSlice};

/// A device snapshot of the four macroscopic planes and the pinned buffer it is copied into.
pub(crate) struct Snapshot {
    pub device: CudaSlice<f32>,
    pub host: PinnedHostSlice<f32>,
}

/// Objects of one kind, reused across copies (allocating pinned memory takes milliseconds).
pub(crate) struct Pool<T> {
    free: Arc<Mutex<Vec<T>>>,
}

impl<T> Clone for Pool<T> {
    fn clone(&self) -> Self {
        Self {
            free: self.free.clone(),
        }
    }
}

impl<T> Default for Pool<T> {
    fn default() -> Self {
        Self {
            free: Arc::default(),
        }
    }
}

impl<T> Pool<T> {
    pub fn take(&self) -> Option<T> {
        self.free.lock().unwrap_or_else(|p| p.into_inner()).pop()
    }

    /// A free object that satisfies `keep`, if any.
    pub fn take_where(&self, keep: impl Fn(&T) -> bool) -> Option<T> {
        let mut free = self.free.lock().unwrap_or_else(|p| p.into_inner());
        let i = free.iter().position(keep)?;
        Some(free.swap_remove(i))
    }

    pub fn give(&self, item: T) {
        self.free
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .push(item);
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

/// A copy of the macroscopic fields in flight (see `Solver::request_fields`).
pub(crate) struct CudaFields {
    /// Recorded on the copy stream after the transfer to `snapshot.host`.
    pub done: CudaEvent,
    pub snapshot: Option<Snapshot>,
    pub pool: Pool<Snapshot>,
    pub step: u64,
    pub n: usize,
    pub thermal: bool,
}

impl CudaFields {
    fn collect(&mut self) -> LatticeFields {
        let snapshot = self.snapshot.take().expect("fields already taken");
        let fields = match snapshot.host.as_slice() {
            Ok(raw) => split_fields(raw, self.n, self.thermal),
            // Device error: report NaN fields rather than panicking; `step` returns the error.
            Err(_) => split_fields(&vec![f32::NAN; 4 * self.n], self.n, self.thermal),
        };
        self.pool.give(snapshot);
        fields
    }
}

impl PendingFields for CudaFields {
    fn step(&self) -> u64 {
        self.step
    }

    fn try_take(&mut self) -> Option<LatticeFields> {
        self.snapshot.as_ref()?;
        // An error (device lost) also ends the wait: `collect` then reports NaN fields.
        match self.done.try_is_complete() {
            Ok(false) => None,
            _ => Some(self.collect()),
        }
    }

    fn wait(mut self: Box<Self>) -> LatticeFields {
        let _ = self.done.synchronize();
        self.collect()
    }
}

impl Drop for CudaFields {
    fn drop(&mut self) {
        // Dropped before the fields were taken: the copy may still be reading the snapshot.
        if let Some(snapshot) = self.snapshot.take() {
            let _ = self.done.synchronize();
            self.pool.give(snapshot);
        }
    }
}

/// The output buffer of one sample on the device and the pinned buffer it is copied into
/// (`words` 32-bit words, held as `f32`s).
pub(crate) struct SampleBuffer {
    pub device: CudaSlice<f32>,
    pub host: PinnedHostSlice<f32>,
    pub words: usize,
}

/// Samples being copied (see `Solver::request_samples`): the sampling kernels ran on the compute
/// stream, the copies to pinned memory on the copy stream, which records `done`.
pub(crate) struct CudaSamples {
    pub done: CudaEvent,
    /// One per spec, with its layout.
    pub buffers: Vec<(SampleSpec, SampleLayout, Option<SampleBuffer>)>,
    pub pool: Pool<SampleBuffer>,
    pub step: u64,
    pub time: f64,
}

impl PendingSamples for CudaSamples {
    fn step(&self) -> u64 {
        self.step
    }

    fn wait(mut self: Box<Self>) -> SampleSet {
        let _ = self.done.synchronize();
        let (step, time) = (self.step, self.time);
        let samples = self
            .buffers
            .iter_mut()
            .map(|(spec, layout, buffer)| {
                let buffer = buffer.take().expect("samples taken once");
                let sample = match buffer.host.as_slice() {
                    Ok(raw) => layout.sample(spec, step, time, bytemuck::cast_slice(raw)),
                    // Device error: NaN samples rather than a panic; `step` reports the error.
                    Err(_) => {
                        let nan = vec![f32::NAN.to_bits(); layout.words];
                        layout.sample(spec, step, time, &nan)
                    }
                };
                self.pool.give(buffer);
                sample
            })
            .collect();
        SampleSet::Ready(samples)
    }
}

impl Drop for CudaSamples {
    fn drop(&mut self) {
        // Dropped before being collected: the copies may still be writing the buffers.
        if self.buffers.iter().any(|(_, _, b)| b.is_some()) {
            let _ = self.done.synchronize();
            for (_, _, b) in &mut self.buffers {
                if let Some(b) = b.take() {
                    self.pool.give(b);
                }
            }
        }
    }
}
