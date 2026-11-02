//! WGSL sources. The constants shared with the CPU reference (`LINK_X/Y`, `OPPOSITE`, `W9`, `W5`,
//! flag and free-surface state bits) are generated from the Rust definitions by
//! `cfd_lbm::abi::wgsl_prelude`, which also generates the CUDA header, so no lattice constant or
//! flag bit is ever typed twice. The kernels (`shaders/*.wgsl`) are appended to the prelude when
//! the shader module is created.

pub use cfd_lbm::abi::wgsl_prelude as prelude;

/// Shader sources: `common` follows the prelude in every module.
pub const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
pub const LBM_WGSL: &str = include_str!("../shaders/lbm.wgsl");
/// Helpers of both stream-collide modules (`lbm.wgsl`, `surface.wgsl`).
pub const LBM_CORE_WGSL: &str = include_str!("../shaders/lbm_core.wgsl");
pub const SURFACE_WGSL: &str = include_str!("../shaders/surface.wgsl");
pub const REDUCE_WGSL: &str = include_str!("../shaders/reduce.wgsl");
pub const SAMPLE_WGSL: &str = include_str!("../shaders/sample.wgsl");

/// The complete source of one shader module: prelude + common types + kernel.
pub fn source(kernel: &str) -> String {
    format!("{}\n{COMMON_WGSL}\n{kernel}", prelude())
}

/// The single-phase stream-collide module (WGSL declarations may come in any order).
pub fn lbm_source() -> String {
    source(&format!("{LBM_WGSL}\n{LBM_CORE_WGSL}"))
}

/// The free-surface module: pass 1 and the conversion passes.
pub fn surface_source() -> String {
    source(&format!("{SURFACE_WGSL}\n{LBM_CORE_WGSL}"))
}
