//! CPU ↔ wgpu parity: the shared cases of `cfd_verify::parity` (docs/04-numerical-methods.md §7).
//!
//! Tests are skipped (with a message) when no wgpu adapter exists; with `CFD_REQUIRE_GPU=1` they
//! fail instead, so CI notices a missing software adapter.

use std::sync::{Arc, OnceLock};

use cfd_gpu::{GpuContext, GpuOptions, WgpuLbm};
use cfd_verify::parity::Factory;

fn context() -> Option<Arc<GpuContext>> {
    static CTX: OnceLock<Option<Arc<GpuContext>>> = OnceLock::new();
    CTX.get_or_init(|| {
        let opts = GpuOptions {
            allow_software: true,
            ..Default::default()
        };
        match GpuContext::new(&opts) {
            Ok(c) => {
                eprintln!("wgpu adapter: {}", c.description());
                Some(c)
            }
            Err(e) if std::env::var_os("CFD_REQUIRE_GPU").is_some() => {
                panic!("CFD_REQUIRE_GPU is set but there is no wgpu adapter: {e}")
            }
            Err(e) => {
                eprintln!("skipping wgpu parity tests: {e}");
                None
            }
        }
    })
    .clone()
}

fn factory() -> Option<Box<Factory>> {
    let ctx = context()?;
    Some(Box::new(move |d, config| {
        Box::new(WgpuLbm::new(ctx.clone(), d, config).expect("wgpu solver"))
    }))
}

cfd_verify::parity_tests!(factory);
