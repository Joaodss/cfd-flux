//! CUDA backend (NVIDIA only), compiled when the `cuda` feature is enabled.
//!
//! - [`CudaDevice`]: device discovery, compute capability and memory checks, kernels compiled at
//!   runtime by NVRTC (a cubin for the device's architecture when possible).
//!
//! `cudarc` loads the CUDA libraries at runtime, so this crate builds without the CUDA Toolkit;
//! running it needs an NVIDIA driver supporting CUDA ≥ 13.0 and the CUDA 13 NVRTC library
//! (`nvrtc64_130_0.dll` / `libnvrtc.so.13`, part of the toolkit).
//!
//! All `unsafe` code lives in the private `raw` module (see its documentation).

/// Whether this build includes the CUDA backend.
pub const ENABLED: bool = cfg!(feature = "cuda");

#[cfg(feature = "cuda")]
mod device;
#[cfg(feature = "cuda")]
#[allow(unsafe_code)]
mod raw;

#[cfg(feature = "cuda")]
pub use device::{
    choose_target, compile, CudaDevice, CudaOptions, Target, DEVICE_ENV, MIN_DRIVER_VERSION,
};

/// Errors of the CUDA backend.
#[derive(Debug, thiserror::Error)]
pub enum CudaError {
    #[error("this build has no CUDA backend (rebuild with the `cuda` feature)")]
    NotBuilt,
    #[error("no NVIDIA driver found (nvcuda.dll / libcuda.so): there is no NVIDIA GPU or its driver is not installed")]
    NoDriver,
    #[error("the NVIDIA driver supports CUDA {found}, but CUDA ≥ {required} is required; update the driver")]
    DriverTooOld { found: String, required: String },
    #[error("CUDA device {requested} requested, but {count} device(s) found")]
    NoDevice { requested: usize, count: usize },
    #[error(
        "NVRTC (the CUDA runtime compiler) was not found: install the CUDA Toolkit 13.x and put its \
         library directory (bin\\x64 on Windows, lib64 on Linux) on the PATH / LD_LIBRARY_PATH. \
         Searched for {names:?} on the library path and in {dirs:?}"
    )]
    NoNvrtc {
        names: Vec<String>,
        dirs: Vec<String>,
    },
    #[error("{device} is not supported: {reason}")]
    UnsupportedDevice { device: String, reason: String },
    #[error("compiling {name} failed ({error}):\n{log}")]
    Compile {
        name: String,
        error: String,
        log: String,
    },
    #[error("{what} need {bytes} bytes, but {device} has {free} bytes free")]
    OutOfMemory {
        what: &'static str,
        bytes: u64,
        free: u64,
        device: String,
    },
    #[error("CUDA driver error: {0}")]
    Driver(String),
}
