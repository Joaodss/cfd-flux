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

use cfd_core::solver::{LatticeFields, PendingFields};
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
