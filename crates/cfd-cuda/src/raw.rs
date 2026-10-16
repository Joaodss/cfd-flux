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
use std::sync::Arc;

use cfd_lbm::abi::{KernelParams, SampleCoefficients, SampleParams};
use cudarc::driver::sys as driver_sys;
use cudarc::driver::{
    CudaContext, CudaFunction, CudaGraph, CudaModule, CudaSlice, CudaStream, CudaViewMut,
    DeviceRepr, DriverError, LaunchConfig, PinnedHostSlice, PushKernelArg,
};
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

// ---- Kernel launches ----
//
// A launch is a C call that hands the GPU a function and an array of pointers to the arguments.
// Rust sees neither the CUDA signature nor what the kernel does with the buffers, so `cudarc`
// makes `launch` unsafe: the caller vouches that (1) the arguments match the `extern "C"`
// signature in type, order and count, (2) every access the kernel makes is inside the buffers it
// gets, and (3) no other work touches a buffer the kernel writes while it runs. Here (1) is kept
// by writing each wrapper against its kernel's signature (and exercised by the parity tests),
// (2) by the invariants `Lattice::new` checks once (the only data-dependent indices are the
// boundary slots and the link cells) plus the checks in each wrapper, and (3) by issuing every
// launch on the solver's single compute stream, which runs them in order.

/// `KernelParams` as a kernel argument (the orphan rule forbids implementing `cudarc`'s trait on
/// a type of another crate, so it is wrapped).
#[repr(transparent)]
#[derive(Clone, Copy)]
struct ParamsArg(KernelParams);

// SAFETY: `KernelParams` is `#[repr(C)]`, made only of `u32`/`f32` fields with explicit padding
// and no pointers, so its bytes are a valid `Params` (checked by the generated `static_assert`s).
unsafe impl DeviceRepr for ParamsArg {}

/// `SampleParams` and `SampleCoefficients` as kernel arguments (wrapped for the same reason).
#[repr(transparent)]
#[derive(Clone, Copy)]
struct SampleParamsArg(SampleParams);

// SAFETY: `#[repr(C)]`, only `u32`/`f32` fields with explicit padding, no pointers; checked
// against the CUDA `SampleParams` by the generated `static_assert`s.
unsafe impl DeviceRepr for SampleParamsArg {}

#[repr(transparent)]
#[derive(Clone, Copy)]
struct CoefficientsArg(SampleCoefficients);

// SAFETY: `#[repr(C)]`, twelve `f64` fields, no padding, no pointers; checked against the CUDA
// `SampleCoefficients` by the generated `static_assert`s.
unsafe impl DeviceRepr for CoefficientsArg {}

/// `len` zeroed values of page-locked ("pinned") host memory, for downloads.
///
/// The OS may move or swap out ordinary (pageable) memory, so the GPU cannot write into it
/// directly: the driver copies through its own pinned buffer and the call returns only when the
/// copy is done. Pinned memory stays put — like a `GCHandle.Alloc(.., Pinned)` enforced by the OS —
/// so a copy engine can DMA into it while the CPU and the compute stream carry on. `cudarc`'s
/// `alloc_pinned` makes the memory *write-combined*, which is fast for uploads but uncached for
/// CPU reads (several times slower to read back), so downloads use the default flags.
pub(crate) fn pinned_zeroed(
    ctx: &Arc<CudaContext>,
    len: usize,
) -> Result<PinnedHostSlice<f32>, DriverError> {
    // SAFETY: the memory is uninitialised after the allocation; it is zeroed below, through a raw
    // pointer, before any reference to it exists.
    let mut pinned = unsafe { ctx.alloc_pinned_with_flags::<f32>(len, 0) }?;
    let ptr = pinned.as_mut_ptr()?;
    // SAFETY: `ptr` points to `len` `f32`s owned by `pinned` (allocated above, nothing queued
    // on it yet), and writing zero bytes produces valid `f32` values.
    unsafe { std::ptr::write_bytes(ptr, 0, len) };
    Ok(pinned)
}

/// Turns off `cudarc`'s per-buffer event tracking for buffers allocated afterwards.
///
/// With tracking on, every launch records and waits on events for each buffer argument — several
/// driver calls per launch, more than a whole step costs on small grids. The backend orders its
/// work itself: one compute stream per solver, an explicit event before the copy stream reads a
/// snapshot, and buffers freed only after the work using them has completed (`lbm.rs`).
pub(crate) fn disable_event_tracking(ctx: &CudaContext) {
    // SAFETY: called by `CudaDevice::new` before the device allocates anything, so every buffer
    // of the backend is untracked and synchronised as described above.
    unsafe { ctx.disable_event_tracking() }
}

/// The functions of `kernels::program()`.
pub(crate) struct Kernels {
    stream_collide: CudaFunction,
    stream_collide_thermal: CudaFunction,
    stream_collide_skip: CudaFunction,
    stream_collide_thermal_skip: CudaFunction,
    diagnostics: CudaFunction,
    gather_links: CudaFunction,
    sample: CudaFunction,
}

impl Kernels {
    pub fn load(module: &Arc<CudaModule>) -> Result<Self, DriverError> {
        Ok(Self {
            stream_collide: module.load_function("stream_collide")?,
            stream_collide_thermal: module.load_function("stream_collide_thermal")?,
            stream_collide_skip: module.load_function("stream_collide_skip")?,
            stream_collide_thermal_skip: module.load_function("stream_collide_thermal_skip")?,
            diagnostics: module.load_function("diagnostics")?,
            gather_links: module.load_function("gather_links")?,
            sample: module.load_function("sample")?,
        })
    }
}

/// Threads per block of the reductions (`RBLOCK` in `reduce.cu`) and values per block.
pub(crate) const REDUCE_BLOCK: u32 = 256;
pub(crate) const PARTIALS: usize = 8;

/// What `Lattice::new` uploads.
pub(crate) struct LatticeInit<'a> {
    pub flags: &'a [u16],
    pub slot: &'a [u32],
    /// `abi::bc_words`: 8 words per slot.
    pub bc_words: &'a [f32],
    /// `cell << 4 | direction` per bounce-back link.
    pub links: &'a [u32],
    pub thermal: bool,
    /// Blocks of the diagnostics reduction.
    pub blocks: u32,
}

/// A captured sequence of steps, replayed with a single launch.
///
/// Launching a kernel costs ≈ 9 µs of CPU time on Windows (WDDM), more than a step takes on a
/// 256² grid, so the GPU would wait for the CPU. A CUDA graph is recorded once by *capturing* the
/// launches queued on a stream (they are not executed then) and replayed with one call — the
/// counterpart of a recorded wgpu command buffer. The graph keeps the kernel arguments it was
/// captured with: the buffer pointers (stable for the lifetime of the `Lattice` that owns it) and
/// the parameters by value (so a parameter change needs a new capture).
struct StepGraph {
    graph: CudaGraph,
    steps: u32,
    params: KernelParams,
    block: u32,
}

// SAFETY: a CUDA graph may be used from any thread as long as calls on one graph are serialised
// (CUDA's graph thread-safety rules). A `StepGraph` is only reachable through `&mut Lattice`, so
// calls on it never overlap; moving it to another thread with its solver is therefore sound.
unsafe impl Send for StepGraph {}

/// The device buffers of one solver, in the layout of the CPU backend (see `lbm.rs`).
///
/// The buffers that hold indices (flags, slots, links) are private and never change after
/// [`Lattice::new`], which checks the invariants the kernels rely on; the population and
/// macroscopic buffers can be read and overwritten through views, but not replaced (their
/// lengths are fixed).
pub(crate) struct Lattice {
    /// Declared first, so they are dropped before the buffers their kernels point to. One per
    /// variant: `[skip_macro, write_macro]`.
    graphs: [Option<StepGraph>; 2],
    n: usize,
    thermal: bool,
    flags: CudaSlice<u16>,
    slot: CudaSlice<u32>,
    bc: CudaSlice<f32>,
    f: [CudaSlice<f32>; 2],
    g: [CudaSlice<f32>; 2],
    mac: CudaSlice<f32>,
    links: CudaSlice<u32>,
    link_count: usize,
    link_out: CudaSlice<f32>,
    partials: CudaSlice<f32>,
    blocks: u32,
}

impl Lattice {
    /// Uploads the static data and allocates zeroed populations and macroscopic planes.
    /// Panics if the data break the invariants the kernels index with.
    pub fn new(stream: &Arc<CudaStream>, init: &LatticeInit) -> Result<Self, DriverError> {
        let n = init.flags.len();
        let slots = init.bc_words.len() / 8;
        assert!(n > 0 && init.slot.len() == n, "one slot per cell");
        assert!(
            slots > 0 && init.bc_words.len() == 8 * slots,
            "whole BcParams records"
        );
        assert!(
            init.slot.iter().all(|&s| (s as usize) < slots),
            "every boundary slot has parameters"
        );
        assert!(
            init.links
                .iter()
                .all(|&l| ((l >> 4) as usize) < n && (1..9).contains(&(l & 15))),
            "links name a cell and a moving direction"
        );
        assert!(init.blocks > 0);
        let g_len = if init.thermal { 5 * n } else { 1 };
        let links: &[u32] = if init.links.is_empty() {
            &[0x11]
        } else {
            init.links
        };
        Ok(Self {
            graphs: [None, None],
            n,
            thermal: init.thermal,
            flags: stream.clone_htod(init.flags)?,
            slot: stream.clone_htod(init.slot)?,
            bc: stream.clone_htod(init.bc_words)?,
            f: [stream.alloc_zeros(9 * n)?, stream.alloc_zeros(9 * n)?],
            g: [stream.alloc_zeros(g_len)?, stream.alloc_zeros(g_len)?],
            mac: stream.alloc_zeros(4 * n + 4)?,
            links: stream.clone_htod(links)?,
            link_count: init.links.len(),
            link_out: stream.alloc_zeros(init.links.len().max(1))?,
            partials: stream.alloc_zeros(init.blocks as usize * PARTIALS)?,
            blocks: init.blocks,
        })
    }

    /// Bytes allocated per cell (flags 2, slot 4, f 2 × 36, g 2 × 20, mac 16), for memory
    /// checks before `new`.
    pub fn bytes_per_cell(thermal: bool) -> u64 {
        2 + 4 + 72 + if thermal { 40 } else { 0 } + 16
    }

    pub fn f_mut(&mut self, i: usize) -> CudaViewMut<'_, f32> {
        self.f[i].slice_mut(..)
    }
    pub fn g_mut(&mut self, i: usize) -> CudaViewMut<'_, f32> {
        self.g[i].slice_mut(..)
    }
    pub fn mac(&self) -> &CudaSlice<f32> {
        &self.mac
    }
    pub fn mac_mut(&mut self) -> CudaViewMut<'_, f32> {
        self.mac.slice_mut(..)
    }
    pub fn partials(&self) -> &CudaSlice<f32> {
        &self.partials
    }
    pub fn link_out(&self) -> &CudaSlice<f32> {
        &self.link_out
    }

    fn check_params(&self, p: &KernelParams) {
        assert_eq!(p.n as usize, self.n, "params for another grid");
        assert_eq!((p.w * p.h) as usize, self.n, "inconsistent grid size");
        assert_eq!(p.links as usize, self.link_count, "params for other links");
    }

    /// One stream-collide step from `f[parity]` into `f[1 − parity]` (and the same for `g`), storing
    /// the macroscopic fields if `write` (else only what pressure outlets read).
    #[allow(clippy::too_many_arguments)]
    pub fn stream_collide(
        &mut self,
        k: &Kernels,
        stream: &CudaStream,
        block: u32,
        p: &KernelParams,
        parity: usize,
        write: bool,
    ) -> Result<(), DriverError> {
        self.check_params(p);
        assert!((1..=1024).contains(&block), "block size");
        let [f0, f1] = &mut self.f;
        let (f_src, f_dst) = if parity == 0 { (&*f0, f1) } else { (&*f1, f0) };
        let [g0, g1] = &mut self.g;
        let (g_src, g_dst) = if parity == 0 { (&*g0, g1) } else { (&*g1, g0) };
        let func = match (self.thermal, write) {
            (true, true) => &k.stream_collide_thermal,
            (false, true) => &k.stream_collide,
            (true, false) => &k.stream_collide_thermal_skip,
            (false, false) => &k.stream_collide_skip,
        };
        let cfg = LaunchConfig {
            grid_dim: ((self.n as u32).div_ceil(block), 1, 1),
            block_dim: (block, 1, 1),
            shared_mem_bytes: 0,
        };
        let arg = ParamsArg(*p);
        let mut launch = stream.launch_builder(func);
        launch
            .arg(&arg)
            .arg(&self.flags)
            .arg(&self.slot)
            .arg(&self.bc)
            .arg(f_src)
            .arg(f_dst)
            .arg(g_src)
            .arg(g_dst)
            .arg(&mut self.mac);
        // SAFETY: (1) the arguments are, in order, `Params` by value and the eight pointers of
        // `stream_collide(_thermal)(_skip)` in `lbm.cu` (`BcParams*` receives the `bc` words, 32 bytes
        // per record). (2) With `p.n == n == w·h` (checked above) threads `k ≥ n` return; every
        // population index is `i·n + c` with `i < 9` (`< 5` for `g`, which has 5n values when
        // thermal) and `c` a cell index wrapped into `[0, n)`; `mac` indices are `≤ 4n`; `bc` is
        // indexed by slots, all `< slots` (checked in `new`). (3) `f_src` and `f_dst` are distinct
        // buffers (the borrow checker guarantees it), and the launch is queued on the solver's
        // compute stream after all earlier work on these buffers.
        unsafe { launch.launch(cfg) }.map(|_| ())
    }

    /// `steps` stream-collide steps (an even number, starting from `f[0]`, so they end in `f[0]`)
    /// replayed from a CUDA graph of the `write` variant, captured on first use and again whenever
    /// the parameters, the block size or the number of steps change.
    #[allow(clippy::too_many_arguments)]
    pub fn replay_steps(
        &mut self,
        k: &Kernels,
        stream: &Arc<CudaStream>,
        block: u32,
        p: &KernelParams,
        steps: u32,
        write: bool,
    ) -> Result<(), DriverError> {
        assert!(
            steps > 0 && steps.is_multiple_of(2),
            "graphs hold whole ping-pong cycles"
        );
        let v = write as usize;
        let current = self.graphs[v]
            .as_ref()
            .is_some_and(|g| g.steps == steps && g.block == block && g.params == *p);
        if !current {
            self.graphs[v] = None;
            self.graphs[v] = Some(self.capture(k, stream, block, p, steps, write)?);
        }
        self.graphs[v].as_ref().expect("captured").graph.launch()
    }

    #[allow(clippy::too_many_arguments)]
    fn capture(
        &mut self,
        k: &Kernels,
        stream: &Arc<CudaStream>,
        block: u32,
        p: &KernelParams,
        steps: u32,
        write: bool,
    ) -> Result<StepGraph, DriverError> {
        // Thread-local mode: only this thread's CUDA calls are restricted during the capture, so
        // solvers on other threads keep working.
        stream
            .begin_capture(driver_sys::CUstreamCaptureMode::CU_STREAM_CAPTURE_MODE_THREAD_LOCAL)?;
        let mut launched = Ok(());
        for s in 0..steps {
            launched = self.stream_collide(k, stream, block, p, (s % 2) as usize, write);
            if launched.is_err() {
                break;
            }
        }
        // End the capture even after an error, so the stream is usable again.
        let graph = stream.end_capture(driver_sys::CUgraphInstantiate_flags(0));
        launched?;
        let graph = graph?.ok_or(DriverError(
            driver_sys::cudaError_enum::CUDA_ERROR_STREAM_CAPTURE_INVALIDATED,
        ))?;
        Ok(StepGraph {
            graph,
            steps,
            params: *p,
            block,
        })
    }

    /// Per-block partial sums of the diagnostics into `partials`.
    pub fn diagnostics(
        &mut self,
        k: &Kernels,
        stream: &CudaStream,
        p: &KernelParams,
    ) -> Result<(), DriverError> {
        self.check_params(p);
        let cfg = LaunchConfig {
            grid_dim: (self.blocks, 1, 1),
            block_dim: (REDUCE_BLOCK, 1, 1),
            shared_mem_bytes: 0,
        };
        let arg = ParamsArg(*p);
        let mut launch = stream.launch_builder(&k.diagnostics);
        launch
            .arg(&arg)
            .arg(&self.flags)
            .arg(&self.mac)
            .arg(&mut self.partials);
        // SAFETY: (1) matches `diagnostics(Params, const unsigned short*, const float*, float*)`
        // in `reduce.cu`. (2) The grid-stride loop visits cells `< n` and reads `mac` below `4n`;
        // block `b` writes `partials[8b .. 8b + 6]`, and `partials` holds `8 · blocks` values for
        // exactly `blocks` blocks of `REDUCE_BLOCK` (= `RBLOCK`) threads. (3) Queued on the
        // compute stream after the steps that wrote `mac`.
        unsafe { launch.launch(cfg) }.map(|_| ())
    }

    /// The outgoing population of every bounce-back link after the last step (`f[current]`).
    pub fn gather_links(
        &mut self,
        k: &Kernels,
        stream: &CudaStream,
        p: &KernelParams,
        current: usize,
    ) -> Result<(), DriverError> {
        self.check_params(p);
        if self.link_count == 0 {
            return Ok(());
        }
        let cfg = LaunchConfig {
            grid_dim: ((self.link_count as u32).div_ceil(REDUCE_BLOCK), 1, 1),
            block_dim: (REDUCE_BLOCK, 1, 1),
            shared_mem_bytes: 0,
        };
        let arg = ParamsArg(*p);
        let mut launch = stream.launch_builder(&k.gather_links);
        launch
            .arg(&arg)
            .arg(&self.links)
            .arg(&self.f[current])
            .arg(&mut self.link_out);
        // SAFETY: (1) matches `gather_links(Params, const unsigned*, const float*, float*)` in
        // `reduce.cu`. (2) Threads `l ≥ p.links = link_count` return; `links` and `link_out` hold
        // `link_count` values; each link's cell is `< n` and its direction `< 9` (checked in
        // `new`), so `OPPOSITE[d]·n + cell < 9n`. (3) Queued on the compute stream after the steps.
        unsafe { launch.launch(cfg) }.map(|_| ())
    }

    /// One output sample (`sample.cu`) into `out`, which must hold `words` words (the total of
    /// `p.fields`' lengths).
    pub fn sample(
        &self,
        k: &Kernels,
        stream: &CudaStream,
        p: &SampleParams,
        c: &SampleCoefficients,
        out: &mut CudaSlice<f32>,
    ) -> Result<(), DriverError> {
        assert_eq!(p.n as usize, self.n, "sample params for another grid");
        assert_eq!((p.w * p.h) as usize, self.n, "inconsistent grid size");
        assert!(
            p.factor >= 1 && p.out_w == p.w.div_ceil(p.factor) && p.out_h == p.h.div_ceil(p.factor)
        );
        let count = p.count as usize;
        assert!(count <= p.fields.len(), "at most one entry per kind");
        let mut words = 0u64;
        let mut widest = 0u32;
        for &[kind, comps, offset, len] in &p.fields[..count] {
            let values = p.out_w as u64 * p.out_h as u64 * comps as u64;
            let expected = if p.f16 == 1 {
                values.div_ceil(2)
            } else {
                values
            };
            assert!(
                kind < 5 && (1..=2).contains(&comps),
                "field kind and components"
            );
            assert!(
                kind != 3 || self.thermal,
                "temperature needs the thermal planes"
            );
            assert_eq!(len as u64, expected, "field length");
            assert_eq!(offset as u64, words, "fields are packed in order");
            words += len as u64;
            widest = widest.max(len);
        }
        assert!(words <= out.len() as u64, "output buffer too small");
        if count == 0 || widest == 0 {
            return Ok(());
        }
        const BLOCK: u32 = 128;
        let cfg = LaunchConfig {
            grid_dim: (widest.div_ceil(BLOCK), count as u32, 1),
            block_dim: (BLOCK, 1, 1),
            shared_mem_bytes: 0,
        };
        let (pa, ca) = (SampleParamsArg(*p), CoefficientsArg(*c));
        let mut launch = stream.launch_builder(&k.sample);
        launch
            .arg(&pa)
            .arg(&ca)
            .arg(&self.flags)
            .arg(&self.mac)
            .arg(out);
        // SAFETY: (1) matches `sample(SampleParams, SampleCoefficients, const unsigned short*,
        // const float*, unsigned*)` in `sample.cu` (the output words are `f32`-sized). (2) Block
        // `y < count` handles field `y`; threads `t ≥ len` return, the others write
        // `out[offset + t]`, below `words ≤ out.len()` (fields packed in order, checked above).
        // Cells read are `y · w + x` with `x < w`, `y < h` (output pixels cover the grid with
        // `factor`, neighbours are wrapped or bounds-checked), so `< n = flags.len()`; `mac`
        // planes are read below `4n` (temperature, plane 3, only with the thermal lattice).
        // (3) Queued on the compute stream after the steps that wrote `mac`; `out` is not used
        // by other work until the copy that follows on the same stream's event.
        unsafe { launch.launch(cfg) }.map(|_| ())
    }
}
