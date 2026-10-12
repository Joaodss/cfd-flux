//! Every `unsafe` operation of the CUDA backend, behind safe functions.
//!
//! Why this module exists: the workspace denies `unsafe_code`, and Rust cannot check what happens
//! on the other side of a C call (≈ C# P/Invoke). `cudarc` marks as `unsafe` exactly the calls
//! whose correctness depends on facts it cannot see: that a library exports the right symbols, that
//! an NVRTC handle is still alive, or — for kernel launches — that the arguments match the CUDA C
//! signature and that the kernel stays inside the buffers it is given. Each safe wrapper here
//! states, in a `// SAFETY:` comment, why those facts hold; the rest of the crate is safe code.
#![deny(clippy::undocumented_unsafe_blocks)]

use std::ffi::{c_char, CString};
use std::path::Path;

use cudarc::driver::sys as driver_sys;
use cudarc::nvrtc::{result as nvrtc, sys as nvrtc_sys};

// ---- Libraries and versions ----

/// Whether the NVIDIA driver library (`nvcuda.dll` / `libcuda.so`) can be loaded.
pub(crate) fn driver_present() -> bool {
    // SAFETY: `is_culib_present` only tries to open the library by name (running its loader
    // initialisation, which the driver supports) and closes it again; no symbol is called.
    unsafe { driver_sys::is_culib_present() }
}

/// Whether the NVRTC library can be loaded.
pub(crate) fn nvrtc_present() -> bool {
    // SAFETY: as in `driver_present`, for the NVRTC library.
    unsafe { nvrtc_sys::is_culib_present() }
}

/// Opens a library by full path and keeps it loaded for the rest of the process. On Windows a later
/// `LoadLibrary` by bare file name returns the module already loaded, so this lets `cudarc` find
/// NVRTC in `%CUDA_PATH%\bin\x64` even when that directory is not on `PATH`.
pub(crate) fn preload(path: &Path) -> bool {
    // SAFETY: loading runs the library's initialisation code; only NVRTC libraries found in the
    // CUDA Toolkit directory are passed here. The handle is leaked, so the code is never unloaded
    // while `cudarc` (which opens it again by name) may call into it.
    match unsafe { libloading::Library::new(path) } {
        Ok(lib) => {
            std::mem::forget(lib);
            true
        }
        Err(_) => false,
    }
}

/// The driver's CUDA version, e.g. 13030 for 13.3. Requires `driver_present()`.
pub(crate) fn driver_version() -> Result<i32, String> {
    if !driver_present() {
        return Err("the NVIDIA driver library is not present".into());
    }
    let mut version = 0;
    // SAFETY: the library is present (checked above), and the call writes one `int` through a
    // pointer to a live local variable. It does not need `cuInit`.
    unsafe { driver_sys::cuDriverGetVersion(&mut version) }
        .result()
        .map_err(|e| e.to_string())?;
    Ok(version)
}

/// NVRTC's version `(major, minor)`. Requires `nvrtc_present()`.
pub(crate) fn nvrtc_version() -> Result<(i32, i32), String> {
    let (mut major, mut minor) = (0, 0);
    // SAFETY: NVRTC is present (checked by the caller before any NVRTC call); the call writes two
    // `int`s through pointers to live locals.
    unsafe { nvrtc_sys::nvrtcVersion(&mut major, &mut minor) }
        .result()
        .map_err(|e| format!("{e:?}"))?;
    Ok((major, minor))
}

/// The real architectures NVRTC can compile for, as `10 * major + minor` (e.g. 89).
pub(crate) fn nvrtc_supported_archs() -> Result<Vec<u32>, String> {
    let mut count = 0;
    // SAFETY: NVRTC is present; writes one `int` through a pointer to a live local.
    unsafe { nvrtc_sys::nvrtcGetNumSupportedArchs(&mut count) }
        .result()
        .map_err(|e| format!("{e:?}"))?;
    let mut archs = vec![0i32; count.max(0) as usize];
    // SAFETY: `archs` holds exactly the number of entries NVRTC just reported.
    unsafe { nvrtc_sys::nvrtcGetSupportedArchs(archs.as_mut_ptr()) }
        .result()
        .map_err(|e| format!("{e:?}"))?;
    Ok(archs.into_iter().map(|a| a as u32).collect())
}

// ---- NVRTC compilation ----

/// What NVRTC hands back: a cubin (native code for one architecture) or PTX (virtual ISA, compiled
/// by the driver's JIT when the module is loaded).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Output {
    Cubin,
    Ptx,
}

/// A compilation failure: the NVRTC error and the compiler log.
#[derive(Debug)]
pub(crate) struct CompileFailure {
    pub error: String,
    pub log: String,
}

/// Destroys an NVRTC program when dropped, also on early returns.
struct Program(nvrtc_sys::nvrtcProgram);

impl Drop for Program {
    fn drop(&mut self) {
        // SAFETY: `self.0` was created by `create_program` and is destroyed exactly once, here.
        let _ = unsafe { nvrtc::destroy_program(self.0) };
    }
}

fn to_string(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Compiles `source` with `options` and returns the cubin or the PTX (NUL-terminated, as the
/// driver's module loader expects it), plus the compiler log (warnings).
pub(crate) fn compile(
    source: &str,
    name: &str,
    options: &[String],
    output: Output,
) -> Result<(Vec<u8>, String), CompileFailure> {
    let failure = |error: String, log: String| CompileFailure { error, log };
    let src = CString::new(source).map_err(|e| failure(e.to_string(), String::new()))?;
    let name = CString::new(name).map_err(|e| failure(e.to_string(), String::new()))?;
    let prog = Program(
        nvrtc::create_program(&src, Some(&name))
            .map_err(|e| failure(format!("{e:?}"), String::new()))?,
    );
    // `src` and `name` outlive `prog`: they are declared before it, so they are dropped after it.

    // SAFETY: `prog.0` is a live program created above (this holds for every block below) and
    // destroyed only when `prog` is dropped at the end of this function; `options` are plain
    // strings that `compile_program` copies into C strings.
    let compiled = unsafe { nvrtc::compile_program(prog.0, options) };
    // SAFETY: see above.
    let log = unsafe { nvrtc::get_program_log(prog.0) }
        .map(|l| to_string(&l))
        .unwrap_or_default();
    if let Err(e) = compiled {
        return Err(failure(format!("{e:?}"), log));
    }
    let image = match output {
        Output::Ptx => {
            // SAFETY: see above; the program compiled successfully.
            let ptx = unsafe { nvrtc::get_ptx(prog.0) }
                .map_err(|e| failure(format!("{e:?}"), log.clone()))?;
            ptx.into_iter().map(|c| c as u8).collect()
        }
        Output::Cubin => {
            let mut size = 0usize;
            // SAFETY: see above; writes one `usize` through a pointer to a live local.
            unsafe { nvrtc_sys::nvrtcGetCUBINSize(prog.0, &mut size) }
                .result()
                .map_err(|e| failure(format!("{e:?}"), log.clone()))?;
            if size == 0 {
                return Err(failure(
                    "NVRTC produced no cubin (was a virtual `compute_XY` architecture requested?)"
                        .into(),
                    log,
                ));
            }
            let mut cubin = vec![0u8; size];
            // SAFETY: `cubin` holds exactly the size NVRTC reported for this program.
            unsafe { nvrtc_sys::nvrtcGetCUBIN(prog.0, cubin.as_mut_ptr().cast()) }
                .result()
                .map_err(|e| failure(format!("{e:?}"), log.clone()))?;
            cubin
        }
    };
    Ok((image, log))
}
