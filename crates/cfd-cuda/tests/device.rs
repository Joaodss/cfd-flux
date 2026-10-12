//! Device discovery and NVRTC. The tests that need an NVIDIA GPU (or NVRTC) print `skipping`
//! without one; `CFD_REQUIRE_CUDA=1` makes them fail instead.
#![cfg(feature = "cuda")]

use cfd_cuda::{compile, CudaDevice, CudaError, CudaOptions, Target};

const KERNEL: &str = r#"extern "C" __global__ void scale(float* x, unsigned n, float a) {
    unsigned i = blockIdx.x * blockDim.x + threadIdx.x;
    if (i < n) x[i] *= a;
}"#;

fn required() -> bool {
    std::env::var_os("CFD_REQUIRE_CUDA").is_some()
}

#[test]
fn device_is_found_and_compiles_a_module() {
    let dev = match CudaDevice::new(&CudaOptions::default()) {
        Ok(d) => d,
        Err(e) if !required() => return eprintln!("skipping: {e}"),
        Err(e) => panic!("CFD_REQUIRE_CUDA is set: {e}"),
    };
    eprintln!("{}", dev.description());
    let (major, _) = dev.compute_capability();
    assert!(major >= 7);
    let m1 = dev.module("scale.cu", KERNEL, &[]).expect("compiles");
    m1.load_function("scale").expect("kernel exists");
    // The second request is served from the cache.
    let m2 = dev.module("scale.cu", KERNEL, &[]).unwrap();
    assert!(std::sync::Arc::ptr_eq(&m1, &m2));
    dev.check_memory("a small buffer", 1 << 20).unwrap();
    assert!(matches!(
        dev.check_memory("an absurd buffer", 1 << 50),
        Err(CudaError::OutOfMemory { .. })
    ));
}

#[test]
fn a_missing_device_is_a_clear_error() {
    match CudaDevice::new(&CudaOptions { device: Some(99) }) {
        Err(CudaError::NoDevice { requested: 99, .. }) => {}
        // Without a driver the earlier checks fail first, which is also fine here.
        Err(e @ (CudaError::NoDriver | CudaError::DriverTooOld { .. })) if !required() => {
            eprintln!("skipping: {e}")
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn nvrtc_builds_cubin_and_ptx_without_a_gpu() {
    let cubin = match compile(Target::Cubin(75), "scale.cu", KERNEL, &[]) {
        Ok(c) => c,
        Err(e @ CudaError::NoNvrtc { .. }) if !required() => return eprintln!("skipping: {e}"),
        Err(e) => panic!("{e}"),
    };
    assert_eq!(&cubin[..4], b"\x7fELF", "a cubin is an ELF image");
    let ptx = compile(Target::Ptx(75), "scale.cu", KERNEL, &[]).unwrap();
    let text = String::from_utf8_lossy(&ptx);
    assert!(text.contains(".target sm_75") && text.contains(".entry scale"));
    match compile(Target::Cubin(75), "bad.cu", "this is not C++", &[]) {
        Err(CudaError::Compile { log, .. }) => assert!(log.contains("bad.cu"), "{log}"),
        other => panic!("{other:?}"),
    }
}
