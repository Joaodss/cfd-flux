//! CUDA backend (NVIDIA only), compiled when the `cuda` feature is enabled.
//!
//! Planned for Phase 2b (see `docs/05-implementation-plan.md`): `cudarc` + NVRTC kernels
//! that mirror the WGSL kernels so the engines can be compared with `cfd-cli compare`.

/// Whether this build includes the CUDA backend.
pub const ENABLED: bool = cfg!(feature = "cuda");
