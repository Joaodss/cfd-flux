//! The CUDA C++ sources and how they are put together into one NVRTC program: the constants
//! generated from Rust (`cfd_lbm::abi::cuda_prelude`), `common.cuh`, the generated layout checks,
//! then the kernels. `#line` directives keep compiler messages pointing at the real files.

use cfd_lbm::abi::{cuda_layout_checks, cuda_prelude};

pub const COMMON_CUH: &str = include_str!("../kernels/common.cuh");
pub const LBM_CU: &str = include_str!("../kernels/lbm.cu");
pub const REDUCE_CU: &str = include_str!("../kernels/reduce.cu");
pub const SAMPLE_CU: &str = include_str!("../kernels/sample.cu");

/// NVRTC options of the layout tests (the solver picks `--fmad` per `CudaLbmOptions`).
pub const OPTIONS: &[&str] = &["--fmad=false"];

/// The complete program: generated constants, common types, layout checks, then `kernels`
/// as `(file name, source)`.
pub fn source(kernels: &[(&str, &str)]) -> String {
    let mut s = cuda_prelude();
    s += "#line 1 \"common.cuh\"\n";
    s += COMMON_CUH;
    s += "\n#line 1 \"layout-checks (generated)\"\n";
    s += &cuda_layout_checks();
    for (name, src) in kernels {
        s += &format!("\n#line 1 \"{name}\"\n");
        s += src;
    }
    s
}

/// The program of the LBM solver: every kernel `CudaLbm` launches.
pub fn program() -> String {
    source(&[
        ("lbm.cu", LBM_CU),
        ("reduce.cu", REDUCE_CU),
        ("sample.cu", SAMPLE_CU),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compile, CudaError, Target};

    /// `CFD_REQUIRE_NVRTC=1` (CI, NVRTC without a GPU) or `CFD_REQUIRE_CUDA=1`: fail instead of skip.
    fn nvrtc_required() -> bool {
        ["CFD_REQUIRE_NVRTC", "CFD_REQUIRE_CUDA"]
            .iter()
            .any(|v| std::env::var_os(v).is_some())
    }

    fn compiles(src: &str) -> Option<Result<Vec<u8>, CudaError>> {
        match compile(Target::Cubin(75), "test.cu", src, OPTIONS) {
            Err(e @ CudaError::NoNvrtc { .. }) if !nvrtc_required() => {
                eprintln!("skipping: {e}");
                None
            }
            r => Some(r),
        }
    }

    #[test]
    fn common_types_match_the_rust_layouts() {
        if let Some(r) = compiles(&source(&[])) {
            r.unwrap();
        }
    }

    #[test]
    fn the_solver_kernels_compile() {
        if let Some(r) = compiles(&program()) {
            assert_eq!(&r.unwrap()[..4], b"ELF");
        }
    }

    #[test]
    fn a_layout_mismatch_fails_to_compile() {
        // Swap two members of `Params`: the generated static_asserts must catch it.
        let broken = source(&[]).replace(
            "    float2 omega;\n    // Thermal relaxation rates (ω⁺, ω⁻).\n    float2 omega_g;",
            "    float2 omega_g;\n    float2 omega;",
        );
        assert_ne!(broken, source(&[]), "the replacement must apply");
        if let Some(r) = compiles(&broken) {
            match r {
                Err(CudaError::Compile { log, .. }) => {
                    assert!(
                        log.contains("Params.omega: offset differs from Rust"),
                        "{log}"
                    )
                }
                other => panic!("{other:?}"),
            }
        }
    }
}
