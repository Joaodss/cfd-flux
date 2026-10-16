//! Engine selection: the only crate that knows the concrete solver types. Everything else works
//! with `Box<dyn Solver>`.
//!
//! - [`EngineKind`]: the three backends (CPU reference, wgpu, CUDA).
//! - [`open`] / [`open_auto`]: open one engine (a device handle shared by every solver made on it),
//!   or the best available one in the order CUDA > wgpu (hardware only) > CPU.
//! - [`Inventory`]: every engine opened once (for the server), keyed by physical device, with
//!   memory checks.
//! - [`SolverChoice`]: an opened engine plus the solver options; [`SolverChoice::make`] builds a
//!   solver for a domain.
//!
//! Shared by `cfd-cli` (`run`, `bench`, `verify`, `compare`), `cfd-job` and `cfd-server`, so every
//! entry point opens engines the same way. (In C# terms: a small class library with a factory.)

mod inventory;

use std::sync::Arc;

use anyhow::{Context, Result};
use cfd_core::scene::BackendPreference;
use cfd_core::{Domain, Solver};
use cfd_gpu::{GpuContext, GpuOptions, WgpuLbm};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};
use serde::{Deserialize, Serialize};

pub use inventory::{bytes_per_cell, ChooseError, EngineInfo, Inventory};

/// Steps replayed per CUDA graph launch by default (see `cfd_cuda::CudaLbmOptions`).
pub const DEFAULT_GRAPH_STEPS: u32 = 50;

/// One concrete engine (no `auto`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
pub enum EngineKind {
    Cpu,
    Wgpu,
    Cuda,
}

impl EngineKind {
    /// Every engine, in the order `auto` prefers them.
    pub const BY_PREFERENCE: [EngineKind; 3] =
        [EngineKind::Cuda, EngineKind::Wgpu, EngineKind::Cpu];

    pub fn name(self) -> &'static str {
        match self {
            EngineKind::Cpu => "cpu",
            EngineKind::Wgpu => "wgpu",
            EngineKind::Cuda => "cuda",
        }
    }

    /// The engine a scene's backend preference names; `None` for `auto`.
    pub fn from_preference(pref: BackendPreference) -> Option<EngineKind> {
        match pref {
            BackendPreference::Auto => None,
            BackendPreference::Cpu => Some(EngineKind::Cpu),
            BackendPreference::Wgpu => Some(EngineKind::Wgpu),
            BackendPreference::Cuda => Some(EngineKind::Cuda),
        }
    }
}

impl std::fmt::Display for EngineKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// Everything about a solver except which engine runs it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EngineOptions {
    pub collision: Collision,
    /// Threads per workgroup (wgpu) or block (CUDA) of the GPU stream-collide kernel.
    pub workgroup: u32,
    /// Let an explicitly requested wgpu engine use a software adapter (WARP, lavapipe). `auto`
    /// never does: software adapters are slower than the CPU backend.
    pub allow_software: bool,
    /// Compile the CUDA kernels with fused multiply-adds (off keeps CUDA bit-identical to the CPU).
    pub fmad: bool,
    /// Steps replayed per CUDA graph launch (even; 0 launches every step on its own).
    pub graph_steps: u32,
    /// Skip the ρ/u/θ stores on steps whose fields are not read (`LbmConfig::skip_macro`).
    pub skip_macro: bool,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            collision: Collision::default(),
            workgroup: cfd_gpu::DEFAULT_WORKGROUP,
            allow_software: false,
            fmad: false,
            graph_steps: DEFAULT_GRAPH_STEPS,
            skip_macro: true,
        }
    }
}

/// An opened engine: the shared device handle solvers are made on. Cloning it is cheap (an `Arc`).
#[derive(Debug, Clone)]
pub enum Engine {
    Cpu,
    Wgpu(Arc<GpuContext>),
    #[cfg(feature = "cuda")]
    Cuda(Arc<cfd_cuda::CudaDevice>),
}

impl Engine {
    pub fn kind(&self) -> EngineKind {
        match self {
            Engine::Cpu => EngineKind::Cpu,
            Engine::Wgpu(_) => EngineKind::Wgpu,
            #[cfg(feature = "cuda")]
            Engine::Cuda(_) => EngineKind::Cuda,
        }
    }

    /// The GPU, e.g. "NVIDIA GeForce RTX 4060 (Vulkan, DiscreteGpu)"; `None` for the CPU.
    pub fn device(&self) -> Option<String> {
        match self {
            Engine::Cpu => None,
            Engine::Wgpu(ctx) => Some(ctx.description()),
            #[cfg(feature = "cuda")]
            Engine::Cuda(dev) => Some(dev.description()),
        }
    }
}

/// The outcome of `auto`: the first engine that opens, in the order CUDA > wgpu > CPU. Each
/// failure is recorded as a note. Generic over the device handles so it can be tested without
/// devices.
#[derive(Debug, PartialEq)]
pub enum Auto<C, W> {
    Cuda(C),
    Wgpu(W),
    Cpu,
}

pub fn pick_auto<C, W>(
    cuda: impl FnOnce() -> Result<C, String>,
    wgpu: impl FnOnce() -> Result<W, String>,
    notes: &mut Vec<String>,
) -> Auto<C, W> {
    match cuda() {
        Ok(c) => return Auto::Cuda(c),
        Err(e) => notes.push(format!("CUDA unavailable: {e}")),
    }
    match wgpu() {
        Ok(w) => return Auto::Wgpu(w),
        Err(e) => notes.push(format!("wgpu unavailable: {e}")),
    }
    Auto::Cpu
}

#[cfg(feature = "cuda")]
fn open_cuda() -> Result<Arc<cfd_cuda::CudaDevice>, String> {
    cfd_cuda::CudaDevice::new(&cfd_cuda::CudaOptions::default()).map_err(|e| e.to_string())
}

#[cfg(not(feature = "cuda"))]
fn open_cuda() -> Result<std::convert::Infallible, String> {
    Err(cfd_cuda::CudaError::NotBuilt.to_string())
}

fn open_wgpu(allow_software: bool) -> Result<Arc<GpuContext>> {
    Ok(GpuContext::new(&GpuOptions {
        allow_software,
        ..Default::default()
    })?)
}

/// Opens one engine, with no fallback (an unavailable engine is an error).
pub fn open(kind: EngineKind, opts: &EngineOptions) -> Result<Engine> {
    Ok(match kind {
        EngineKind::Cpu => Engine::Cpu,
        #[cfg(feature = "cuda")]
        EngineKind::Cuda => Engine::Cuda(open_cuda().map_err(anyhow::Error::msg)?),
        #[cfg(not(feature = "cuda"))]
        EngineKind::Cuda => anyhow::bail!(cfd_cuda::CudaError::NotBuilt),
        EngineKind::Wgpu => Engine::Wgpu(open_wgpu(opts.allow_software)?),
    })
}

/// The best available engine: CUDA, else a hardware GPU through wgpu, else the CPU. Why each
/// engine was skipped is added to `notes`.
pub fn open_auto(notes: &mut Vec<String>) -> Engine {
    // Software adapters are slower than the CPU backend: `auto` never picks them.
    let picked = pick_auto(
        open_cuda,
        || open_wgpu(false).map_err(|e| e.to_string()),
        notes,
    );
    match picked {
        #[cfg(feature = "cuda")]
        Auto::Cuda(dev) => Engine::Cuda(dev),
        #[cfg(not(feature = "cuda"))]
        Auto::Cuda(never) => match never {},
        Auto::Wgpu(ctx) => Engine::Wgpu(ctx),
        Auto::Cpu => {
            notes.push("using the CPU backend".into());
            Engine::Cpu
        }
    }
}

/// Opens the engine a preference names (`auto`: see [`open_auto`]).
pub fn open_preferred(
    pref: BackendPreference,
    opts: &EngineOptions,
    notes: &mut Vec<String>,
) -> Result<Engine> {
    match EngineKind::from_preference(pref) {
        Some(kind) => open(kind, opts),
        None => Ok(open_auto(notes)),
    }
}

/// Sets the size of rayon's global thread pool. Call once per process, before any solver runs:
/// the global pool can only be configured once.
pub fn configure_threads(threads: Option<usize>) -> Result<()> {
    if let Some(n) = threads {
        // In C# terms: configuring the default TaskScheduler's degree of parallelism.
        rayon::ThreadPoolBuilder::new()
            .num_threads(n)
            .build_global()?;
    }
    Ok(())
}

/// Opens every engine of `list` (an unavailable one is an error), labelled as in [`label`].
pub fn open_all(list: &[EngineKind], opts: &EngineOptions) -> Result<Vec<(String, SolverChoice)>> {
    let opened = list
        .iter()
        .map(|&kind| {
            let engine =
                open(kind, opts).with_context(|| format!("cannot open the {kind} backend"))?;
            Ok((kind, SolverChoice::new(engine, opts)))
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(label(opened))
}

/// Unique labels for engines opened side by side: the engine name, then `cpu-2`, `cpu-3`, … for
/// repeats (usable in file names and Markdown links).
pub fn label<T>(opened: Vec<(EngineKind, T)>) -> Vec<(String, T)> {
    let mut seen: Vec<EngineKind> = Vec::new();
    opened
        .into_iter()
        .map(|(e, v)| {
            seen.push(e);
            let label = match seen.iter().filter(|&&s| s == e).count() {
                1 => e.name().to_string(),
                n => format!("{}-{n}", e.name()),
            };
            (label, v)
        })
        .collect()
}

/// What was actually chosen: an engine plus the solver options, for making solvers and for
/// reports.
#[derive(Debug, Clone)]
pub struct SolverChoice {
    pub backend: &'static str,
    /// The GPU, e.g. "NVIDIA GeForce RTX 4060 (Vulkan, DiscreteGpu)".
    pub device: Option<String>,
    pub collision: &'static str,
    pub trt_magic: Option<f64>,
    pub threads: usize,
    config: LbmConfig,
    engine: Engine,
    workgroup: u32,
    #[cfg_attr(not(feature = "cuda"), allow(dead_code))]
    fmad: bool,
    #[cfg_attr(not(feature = "cuda"), allow(dead_code))]
    graph_steps: u32,
}

impl SolverChoice {
    pub fn new(engine: Engine, opts: &EngineOptions) -> Self {
        let (collision, trt_magic) = match opts.collision {
            Collision::Bgk => ("bgk", None),
            Collision::Trt { magic } => ("trt", Some(magic)),
        };
        SolverChoice {
            backend: engine.kind().name(),
            device: engine.device(),
            collision,
            trt_magic,
            threads: rayon::current_num_threads(),
            config: LbmConfig {
                collision: opts.collision,
                skip_macro: opts.skip_macro,
            },
            engine,
            workgroup: opts.workgroup,
            fmad: opts.fmad,
            graph_steps: opts.graph_steps,
        }
    }

    pub fn kind(&self) -> EngineKind {
        self.engine.kind()
    }

    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Creates a solver for `domain` on the chosen engine.
    pub fn make(&self, domain: Domain) -> Result<Box<dyn Solver>> {
        Ok(match &self.engine {
            Engine::Cpu => Box::new(CpuLbm::new(domain, self.config)),
            Engine::Wgpu(ctx) => Box::new(WgpuLbm::with_workgroup(
                ctx.clone(),
                domain,
                self.config,
                self.workgroup,
            )?),
            #[cfg(feature = "cuda")]
            Engine::Cuda(dev) => Box::new(cfd_cuda::CudaLbm::with_options(
                dev.clone(),
                domain,
                self.config,
                cfd_cuda::CudaLbmOptions {
                    block: self.workgroup,
                    fmad: self.fmad,
                    graph_steps: self.graph_steps,
                },
            )?),
        })
    }

    /// Whether the solver runs on a GPU (the workgroup/block size applies).
    pub fn is_gpu(&self) -> bool {
        !matches!(self.engine, Engine::Cpu)
    }

    /// The workgroup/block size, on GPU backends.
    pub fn workgroup(&self) -> Option<u32> {
        self.is_gpu().then_some(self.workgroup)
    }

    /// "cpu", "wgpu (adapter)" or "cuda (device)", for log lines.
    pub fn describe(&self) -> String {
        match &self.device {
            Some(d) => format!("{} ({d})", self.backend),
            None => self.backend.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(v: u8) -> impl FnOnce() -> Result<u8, String> {
        move || Ok(v)
    }
    fn missing(what: &'static str) -> impl FnOnce() -> Result<u8, String> {
        move || Err(format!("no {what}"))
    }

    #[test]
    fn auto_prefers_cuda_then_wgpu_then_cpu() {
        let mut notes = Vec::new();
        assert_eq!(pick_auto(ok(1), ok(2), &mut notes), Auto::Cuda(1));
        assert!(notes.is_empty());

        assert_eq!(
            pick_auto(missing("NVIDIA GPU"), ok(2), &mut notes),
            Auto::Wgpu(2)
        );
        assert_eq!(notes, ["CUDA unavailable: no NVIDIA GPU"]);

        notes.clear();
        assert_eq!(
            pick_auto(missing("NVIDIA GPU"), missing("adapter"), &mut notes),
            Auto::<u8, u8>::Cpu
        );
        assert_eq!(
            notes,
            [
                "CUDA unavailable: no NVIDIA GPU",
                "wgpu unavailable: no adapter"
            ]
        );
    }

    #[test]
    fn wgpu_is_not_tried_when_cuda_opens() {
        let mut notes = Vec::new();
        let wgpu = || -> Result<u8, String> { panic!("wgpu must not be opened") };
        assert_eq!(pick_auto(ok(1), wgpu, &mut notes), Auto::Cuda(1));
    }

    #[test]
    fn repeated_engines_get_numbered_labels() {
        use EngineKind::*;
        let labels: Vec<String> = label(vec![(Cpu, ()), (Cuda, ()), (Cpu, ()), (Cpu, ())])
            .into_iter()
            .map(|(l, _)| l)
            .collect();
        assert_eq!(labels, ["cpu", "cuda", "cpu-2", "cpu-3"]);
    }

    #[test]
    fn engine_kinds_serialise_in_lowercase() {
        assert_eq!(
            serde_json::to_string(&EngineKind::Cuda).unwrap(),
            "\"cuda\""
        );
        assert_eq!(
            EngineKind::from_preference(BackendPreference::Wgpu),
            Some(EngineKind::Wgpu)
        );
        assert_eq!(EngineKind::from_preference(BackendPreference::Auto), None);
    }
}
