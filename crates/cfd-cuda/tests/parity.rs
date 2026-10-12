//! CPU ↔ CUDA parity: the shared cases of `cfd_verify::parity` (docs/04-numerical-methods.md §7).
//!
//! Tests are skipped (with a message) without an NVIDIA GPU or NVRTC; with `CFD_REQUIRE_CUDA=1`
//! they fail instead.
#![cfg(feature = "cuda")]

use std::sync::{Arc, OnceLock};

use cfd_cuda::{CudaDevice, CudaLbm, CudaLbmOptions, CudaOptions};
use cfd_verify::parity::Factory;

fn device() -> Option<Arc<CudaDevice>> {
    static DEV: OnceLock<Option<Arc<CudaDevice>>> = OnceLock::new();
    DEV.get_or_init(|| match CudaDevice::new(&CudaOptions::default()) {
        Ok(d) => {
            eprintln!("CUDA device: {}", d.description());
            Some(d)
        }
        Err(e) if std::env::var_os("CFD_REQUIRE_CUDA").is_some() => {
            panic!("CFD_REQUIRE_CUDA is set but CUDA is unavailable: {e}")
        }
        Err(e) => {
            eprintln!("skipping CUDA parity tests: {e}");
            None
        }
    })
    .clone()
}

/// `CFD_CUDA_FMAD=1` compiles the kernels with fused multiply-adds (for the FMA comparison).
fn options() -> CudaLbmOptions {
    CudaLbmOptions {
        fmad: std::env::var_os("CFD_CUDA_FMAD").is_some_and(|v| v == "1"),
        ..Default::default()
    }
}

fn factory() -> Option<Box<Factory>> {
    let dev = device()?;
    Some(Box::new(move |d, config| {
        Box::new(CudaLbm::with_options(dev.clone(), d, config, options()).expect("CUDA solver"))
    }))
}

cfd_verify::parity_tests!(factory);
