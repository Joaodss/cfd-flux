//! Device discovery, compute-capability and memory checks, and runtime kernel compilation.
//!
//! The CUDA objects, with the closest wgpu and .NET equivalents:
//! - [`CudaContext`]: one GPU's address space — its allocations and loaded modules
//!   (≈ `wgpu::Device`). `cudarc` uses the device's *primary* context, shared with any other
//!   CUDA code in the process.
//! - Stream: an in-order queue of kernels and copies (≈ `wgpu::Queue`, but a context can have many,
//!   which run concurrently — that is how copies overlap with computation).
//! - Module: compiled code loaded into a context; a function in it is a kernel.
//!
//! Kernels are CUDA C++ compiled at runtime by NVRTC. Two outputs are possible:
//! - **cubin**: native code (SASS) for one architecture, e.g. `sm_89` — like a ReadyToRun image;
//! - **PTX**: a virtual ISA that the *driver* compiles when the module is loaded — like IL and the
//!   JIT. An older driver may reject PTX produced by a newer NVRTC, as an old runtime rejects newer IL.
//!
//! So a cubin for the device's own architecture is preferred; a cubin for an older architecture
//! of the same major version also runs (binary compatibility within a major version); PTX is used
//! only when NVRTC does not know the device and the driver is at least as new as NVRTC.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use cudarc::driver::{CudaContext, CudaModule};
use cudarc::nvrtc::Ptx;

use crate::raw::{self, Output};
use crate::CudaError;

/// Environment variable that selects the device by ordinal (default 0).
pub const DEVICE_ENV: &str = "CFD_CUDA_DEVICE";
/// Oldest driver supported by the CUDA 13.0 bindings `cudarc` is built with.
pub const MIN_DRIVER_VERSION: i32 = 13000;

/// How to choose a device.
#[derive(Debug, Clone, Default)]
pub struct CudaOptions {
    /// Device ordinal; defaults to `CFD_CUDA_DEVICE`, then 0.
    pub device: Option<usize>,
}

/// What NVRTC compiles for: an architecture as `10 * major + minor` (e.g. 89).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Target {
    /// Native code for `sm_XY`.
    Cubin(u32),
    /// PTX for `compute_XY`, compiled by the driver at load time.
    Ptx(u32),
}

impl Target {
    fn option(self) -> String {
        match self {
            Target::Cubin(a) => format!("--gpu-architecture=sm_{a}"),
            Target::Ptx(a) => format!("--gpu-architecture=compute_{a}"),
        }
    }

    fn output(self) -> Output {
        match self {
            Target::Cubin(_) => Output::Cubin,
            Target::Ptx(_) => Output::Ptx,
        }
    }
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Cubin(a) => write!(f, "sm_{a} cubin"),
            Target::Ptx(a) => write!(f, "compute_{a} PTX"),
        }
    }
}

/// `13040` → "13.4".
fn version_string(v: i32) -> String {
    format!("{}.{}", v / 1000, (v % 1000) / 10)
}

/// Picks the compilation target for a device of compute capability `cc` (e.g. 89), given the
/// architectures NVRTC supports and the NVRTC and driver versions (as 13040 for 13.4).
pub fn choose_target(
    cc: u32,
    supported: &[u32],
    nvrtc_version: i32,
    driver_version: i32,
) -> Result<Target, String> {
    if supported.contains(&cc) {
        return Ok(Target::Cubin(cc));
    }
    // Cubins run on later minor versions of the same major architecture.
    if let Some(&a) = supported
        .iter()
        .filter(|&&a| a / 10 == cc / 10 && a < cc)
        .max()
    {
        return Ok(Target::Cubin(a));
    }
    let Some(&a) = supported.iter().filter(|&&a| a < cc).max() else {
        return Err(format!(
            "compute capability {}.{} is older than any architecture this NVRTC supports \
             ({supported:?})",
            cc / 10,
            cc % 10
        ));
    };
    if nvrtc_version > driver_version {
        return Err(format!(
            "NVRTC {} cannot build native code for compute capability {}.{}, and PTX from it \
             would need a driver supporting CUDA ≥ {} (this one supports {}); update the driver \
             or install a newer CUDA Toolkit",
            version_string(nvrtc_version),
            cc / 10,
            cc % 10,
            version_string(nvrtc_version),
            version_string(driver_version),
        ));
    }
    Ok(Target::Ptx(a))
}

/// Directories where the CUDA Toolkit keeps NVRTC, from the usual environment variables.
fn nvrtc_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for var in ["CUDA_PATH", "CUDA_HOME", "CUDA_ROOT"] {
        if let Some(root) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            let root = PathBuf::from(root);
            dirs.extend(["bin/x64", "bin", "lib64", "lib"].map(|d| root.join(d)));
        }
    }
    if cfg!(unix) {
        dirs.push("/usr/local/cuda/lib64".into());
    }
    dirs
}

/// Loads NVRTC from the toolkit directory when it is not on the library search path.
fn find_nvrtc() -> Result<(), CudaError> {
    if raw::nvrtc_present() {
        return Ok(());
    }
    let mut names = cudarc::get_lib_name_candidates("nvrtc");
    let mut seen = std::collections::HashSet::new();
    names.retain(|n| seen.insert(n.clone()));
    let dirs = nvrtc_dirs();
    let found = dirs.iter().find(|d| {
        names
            .iter()
            .map(|n| d.join(n))
            .any(|p| p.is_file() && raw::preload(&p))
    });
    if let Some(dir) = found {
        // NVRTC opens its builtins library (`nvrtc-builtins64_134.dll`) by name when compiling.
        let builtins = std::fs::read_dir(dir).into_iter().flatten().flatten();
        for entry in builtins {
            let file = entry.file_name().to_string_lossy().to_lowercase();
            if file.contains("nvrtc-builtins") && file.contains(std::env::consts::DLL_EXTENSION) {
                raw::preload(&entry.path());
            }
        }
        if raw::nvrtc_present() {
            return Ok(());
        }
    }
    Err(CudaError::NoNvrtc {
        names,
        dirs: dirs.iter().map(|d| d.display().to_string()).collect(),
    })
}

/// Runs a `cudarc` call, turning a panic (it panics when a library symbol is missing) into an
/// error instead of aborting the caller.
fn guarded<T>(what: &str, f: impl FnOnce() -> Result<T, String>) -> Result<T, CudaError> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(CudaError::Driver(format!("{what}: {e}"))),
        Err(panic) => {
            let msg = panic
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            Err(CudaError::Driver(format!("{what}: {msg}")))
        }
    }
}

/// Compiles `source` with NVRTC for `target` (`-std=c++17` plus `options`) and returns the cubin or
/// the NUL-terminated PTX. Needs NVRTC but no GPU, so layout checks can run anywhere NVRTC is
/// installed.
pub fn compile(
    target: Target,
    name: &str,
    source: &str,
    options: &[&str],
) -> Result<Vec<u8>, CudaError> {
    find_nvrtc()?;
    let mut opts: Vec<String> = vec![target.option(), "-std=c++17".into()];
    opts.extend(options.iter().map(|s| s.to_string()));
    raw::compile(source, name, &opts, target.output())
        .map(|(image, _log)| image)
        .map_err(|f| CudaError::Compile {
            name: name.to_string(),
            error: f.error,
            log: f.log,
        })
}

/// A CUDA device ready to run solvers. Meant to be shared (`Arc`) by every solver of a process:
/// kernels are compiled once per device and options.
pub struct CudaDevice {
    ctx: Arc<CudaContext>,
    name: String,
    /// Compute capability as `10 * major + minor`.
    compute: u32,
    total_memory: usize,
    driver_version: i32,
    nvrtc_version: i32,
    target: Target,
    modules: Mutex<HashMap<u64, Arc<CudaModule>>>,
}

impl std::fmt::Debug for CudaDevice {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.description())
    }
}

impl CudaDevice {
    /// Opens the device chosen by `opts`, after checking the driver, the device and NVRTC.
    pub fn new(opts: &CudaOptions) -> Result<Arc<Self>, CudaError> {
        if !raw::driver_present() {
            return Err(CudaError::NoDriver);
        }
        let driver_version = raw::driver_version().map_err(CudaError::Driver)?;
        if driver_version < MIN_DRIVER_VERSION {
            return Err(CudaError::DriverTooOld {
                found: version_string(driver_version),
                required: version_string(MIN_DRIVER_VERSION),
            });
        }
        let ordinal = match opts.device {
            Some(d) => d,
            None => match std::env::var(DEVICE_ENV) {
                Ok(s) => s.trim().parse().map_err(|_| {
                    CudaError::Driver(format!("{DEVICE_ENV}={s:?} is not a device ordinal"))
                })?,
                Err(_) => 0,
            },
        };
        let count = guarded("counting devices", || {
            CudaContext::device_count().map_err(|e| e.to_string())
        })?;
        if ordinal >= count.max(0) as usize {
            return Err(CudaError::NoDevice {
                requested: ordinal,
                count: count.max(0) as usize,
            });
        }
        let ctx = guarded("opening the device", || {
            CudaContext::new(ordinal).map_err(|e| e.to_string())
        })?;
        let (name, (major, minor), total_memory) = guarded("querying the device", || {
            let s = |e: cudarc::driver::DriverError| e.to_string();
            Ok((
                ctx.name().map_err(s)?,
                ctx.compute_capability().map_err(s)?,
                ctx.total_mem().map_err(s)?,
            ))
        })?;
        let compute = (10 * major + minor) as u32;

        find_nvrtc()?;
        let (nv_major, nv_minor) = raw::nvrtc_version().map_err(CudaError::Driver)?;
        let nvrtc_version = 1000 * nv_major + 10 * nv_minor;
        let supported = raw::nvrtc_supported_archs().map_err(CudaError::Driver)?;
        let target = choose_target(compute, &supported, nvrtc_version, driver_version).map_err(
            |reason| CudaError::UnsupportedDevice {
                device: name.clone(),
                reason,
            },
        )?;
        Ok(Arc::new(Self {
            ctx,
            name,
            compute,
            total_memory,
            driver_version,
            nvrtc_version,
            target,
            modules: Mutex::default(),
        }))
    }

    pub fn context(&self) -> &Arc<CudaContext> {
        &self.ctx
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// Compute capability `(major, minor)`.
    pub fn compute_capability(&self) -> (u32, u32) {
        (self.compute / 10, self.compute % 10)
    }

    pub fn target(&self) -> Target {
        self.target
    }

    /// e.g. "NVIDIA GeForce RTX 4060 (CUDA, sm_89, 8.0 GiB, driver 13.3, NVRTC 13.4)".
    pub fn description(&self) -> String {
        format!(
            "{} (CUDA, sm_{}, {:.1} GiB, driver {}, NVRTC {})",
            self.name,
            self.compute,
            self.total_memory as f64 / (1u64 << 30) as f64,
            version_string(self.driver_version),
            version_string(self.nvrtc_version),
        )
    }

    /// Fails with a clear message when `bytes` more cannot be allocated on the device.
    pub fn check_memory(&self, what: &'static str, bytes: u64) -> Result<(), CudaError> {
        let (free, _) = self
            .ctx
            .mem_get_info()
            .map_err(|e| CudaError::Driver(e.to_string()))?;
        if bytes > free as u64 {
            return Err(CudaError::OutOfMemory {
                what,
                bytes,
                free: free as u64,
                device: self.description(),
            });
        }
        Ok(())
    }

    /// Compiles `source` for this device with the given extra NVRTC options and loads it, or
    /// returns the module compiled earlier from the same source and options.
    pub fn module(
        &self,
        name: &str,
        source: &str,
        options: &[&str],
    ) -> Result<Arc<CudaModule>, CudaError> {
        let mut h = DefaultHasher::new();
        (source, options).hash(&mut h);
        let key = h.finish();
        let mut cache = self.modules.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(m) = cache.get(&key) {
            return Ok(m.clone());
        }
        let image = compile(self.target, name, source, options)?;
        let module = self
            .ctx
            .load_module(Ptx::from_binary(image))
            .map_err(|e| CudaError::Driver(format!("loading {name}: {e}")))?;
        cache.insert(key, module.clone());
        Ok(module)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NVRTC_13_4: [u32; 13] = [75, 80, 86, 87, 88, 89, 90, 100, 103, 107, 110, 120, 121];

    #[test]
    fn native_cubin_when_nvrtc_knows_the_device() {
        assert_eq!(
            choose_target(89, &NVRTC_13_4, 13040, 13030),
            Ok(Target::Cubin(89))
        );
    }

    #[test]
    fn older_cubin_of_the_same_major_version() {
        assert_eq!(
            choose_target(89, &[75, 80, 86], 12040, 12000),
            Ok(Target::Cubin(86))
        );
    }

    #[test]
    fn ptx_only_when_the_driver_is_new_enough() {
        // A device newer than NVRTC: PTX for the newest known architecture...
        assert_eq!(
            choose_target(130, &NVRTC_13_4, 13040, 13050),
            Ok(Target::Ptx(121))
        );
        // ...but not when the driver is older than NVRTC (it would reject the PTX).
        let e = choose_target(130, &NVRTC_13_4, 13040, 13030).unwrap_err();
        assert!(e.contains("update the driver"), "{e}");
    }

    #[test]
    fn devices_older_than_nvrtc_are_rejected() {
        assert!(choose_target(61, &NVRTC_13_4, 13040, 13040).is_err());
    }

    #[test]
    fn versions_print_as_major_minor() {
        assert_eq!(version_string(13030), "13.3");
        assert_eq!(version_string(12080), "12.8");
    }
}
