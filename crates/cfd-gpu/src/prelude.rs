//! WGSL sources. The constants shared with the CPU reference (`LINK_X/Y`, `OPPOSITE`, `W9`, `W5`,
//! flag bits) are generated from the Rust definitions by `cfd_lbm::abi::wgsl_prelude`, which also
//! generates the CUDA header, so no lattice constant or flag bit is ever typed twice. The kernels
//! (`shaders/*.wgsl`) are appended to the prelude when the shader module is created.

pub use cfd_lbm::abi::wgsl_prelude as prelude;

/// Shader sources: `common` follows the prelude in every module.
pub const COMMON_WGSL: &str = include_str!("../shaders/common.wgsl");
pub const LBM_WGSL: &str = include_str!("../shaders/lbm.wgsl");
pub const REDUCE_WGSL: &str = include_str!("../shaders/reduce.wgsl");
pub const SAMPLE_WGSL: &str = include_str!("../shaders/sample.wgsl");

/// The complete source of one shader module: prelude + common types + kernel.
pub fn source(kernel: &str) -> String {
    format!("{}\n{COMMON_WGSL}\n{kernel}", prelude())
}
