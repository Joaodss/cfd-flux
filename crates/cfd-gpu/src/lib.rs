//! wgpu/WGSL backend.
//!
//! - [`GpuContext`]: adapter discovery and ranking, device creation with the adapter's limits.
//! - [`WgpuLbm`]: the LBM solver (D2Q9 TRT + Guo forcing, link-wise boundaries, D2Q5 thermal)
//!   with the memory layout of the CPU reference `cfd_lbm::CpuLbm`.
//! - `prelude`: the WGSL constants, generated from the Rust definitions.
//!
//! Kernels live in `shaders/*.wgsl`; every one of them has a parity test against the CPU
//! backend (`tests/parity.rs`).

mod context;
mod lbm;
pub mod prelude;
mod readback;
#[cfg(test)]
mod shader_tests;

pub use context::{adapters, describe, is_software, GpuContext, GpuOptions, ADAPTER_ENV};
pub use lbm::{WgpuLbm, DEFAULT_WORKGROUP};

/// Errors of the wgpu backend.
#[derive(Debug, thiserror::Error)]
pub enum GpuError {
    #[error(
        "no suitable wgpu adapter{}; available: {}",
        if *software_allowed { "" } else { " (software adapters excluded)" },
        if available.is_empty() { "none".to_string() } else { available.join("; ") }
    )]
    NoAdapter {
        available: Vec<String>,
        software_allowed: bool,
    },
    #[error("could not open the GPU device: {0}")]
    Device(String),
    #[error("{what} need {bytes} bytes, but {adapter} allows at most {limit} per buffer")]
    TooLarge {
        what: &'static str,
        bytes: u64,
        limit: u64,
        adapter: String,
    },
    #[error("wgpu validation error: {0}")]
    Validation(String),
    #[error("GPU out of memory: {0}")]
    OutOfMemory(String),
}
