//! Helpers shared by the end-to-end tests (each file under `tests/` is its own crate and uses a
//! different subset, hence the `allow`).
#![allow(dead_code)]

use std::path::PathBuf;

use cfd_core::examples::{self, Canvas};
use cfd_core::scene::CellType;
use cfd_core::Scene;

/// A 64 × 18 channel (walls, uniform inlet, pressure outlet) with a probe, in the units of the
/// `channel` example (dx = 1 mm, dt = 1 ms at the default lattice velocity).
pub fn small_channel() -> Scene {
    let (w, h) = (64, 18);
    let mut c = Canvas::new(w, h, CellType::Fluid);
    c.rect(0, 0, w, 1, CellType::Solid, 1);
    c.rect(0, h - 1, w, h, CellType::Solid, 1);
    c.rect(0, 1, 1, h - 1, CellType::Inlet, 2);
    c.rect(w - 1, 1, w, h - 1, CellType::Outlet, 3);
    let mut scene = examples::channel();
    scene.grid.width = w;
    scene.grid.height = h;
    scene.layers = c.layers();
    scene.probes[0].position = [32, 9];
    scene
}

pub fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("cfd-cli-test-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// Whether this build has the CUDA backend and the machine can run it.
pub fn cuda_available() -> Result<(), String> {
    #[cfg(feature = "cuda")]
    return cfd_cuda::CudaDevice::new(&cfd_cuda::CudaOptions::default())
        .map(|_| ())
        .map_err(|e| e.to_string());
    #[cfg(not(feature = "cuda"))]
    Err(cfd_cuda::CudaError::NotBuilt.to_string())
}

/// Whether `backend` can run here. A missing GPU backend skips the test (with a message)
/// unless `CFD_REQUIRE_CUDA` / `CFD_REQUIRE_GPU` is set, in which case the test fails.
pub fn available(backend: &str) -> bool {
    match backend {
        "cpu" => true,
        "cuda" => match cuda_available() {
            Ok(()) => true,
            Err(e) => {
                assert!(
                    std::env::var_os("CFD_REQUIRE_CUDA").is_none(),
                    "CFD_REQUIRE_CUDA is set but CUDA is unavailable: {e}"
                );
                eprintln!("skipping: {e}");
                false
            }
        },
        _ if cfd_gpu::adapters().is_empty() => {
            assert!(
                std::env::var_os("CFD_REQUIRE_GPU").is_none(),
                "CFD_REQUIRE_GPU is set but there is no wgpu adapter"
            );
            eprintln!("skipping: no wgpu adapter");
            false
        }
        _ => true,
    }
}

/// `--backend` arguments for `run`; `None` when the backend is unavailable (see [`available`]).
pub fn backend_args(backend: &str) -> Option<Vec<&'static str>> {
    if !available(backend) {
        return None;
    }
    Some(match backend {
        "cpu" => vec!["--backend", "cpu"],
        "cuda" => vec!["--backend", "cuda"],
        _ => vec!["--backend", "wgpu", "--allow-software"],
    })
}
