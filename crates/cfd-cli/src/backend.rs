//! Backend selection: the only place that knows the concrete solver types. Everything else
//! works with `Box<dyn Solver>`.

use std::sync::Arc;

use anyhow::Result;
use cfd_core::scene::BackendPreference;
use cfd_core::{Domain, Solver};
use cfd_gpu::{GpuContext, GpuOptions, WgpuLbm};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};
use clap::{Args, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendArg {
    /// The best available engine: CUDA, else a hardware GPU through wgpu, else the CPU.
    Auto,
    Cpu,
    Wgpu,
    Cuda,
}

/// One concrete engine (no `auto`), for commands that run several side by side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum EngineArg {
    Cpu,
    Wgpu,
    Cuda,
}

impl EngineArg {
    pub fn name(self) -> &'static str {
        match self {
            EngineArg::Cpu => "cpu",
            EngineArg::Wgpu => "wgpu",
            EngineArg::Cuda => "cuda",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CollisionArg {
    /// Two relaxation times with Λ = 3/16 (default).
    Trt,
    /// Single relaxation time.
    Bgk,
}

/// Solver options shared by `run`, `bench` and `verify`: the backend plus the engine options.
#[derive(Debug, Clone, Args)]
pub struct SolverArgs {
    /// Backend; defaults to the scene's preference (or `auto`).
    #[arg(long, value_enum)]
    pub backend: Option<BackendArg>,
    #[command(flatten)]
    pub engine: EngineArgs,
}

/// Everything about a solver except which backend runs it (shared by `compare` and
/// `bench --backends`, which open several backends in one process).
#[derive(Debug, Clone, Args)]
pub struct EngineArgs {
    /// Collision operator.
    #[arg(long, value_enum, default_value = "trt")]
    pub collision: CollisionArg,
    /// Number of CPU threads (default: all logical cores).
    #[arg(long)]
    pub threads: Option<usize>,
    /// Threads per workgroup (wgpu) or block (CUDA) of the GPU stream-collide kernel.
    #[arg(long, default_value_t = cfd_gpu::DEFAULT_WORKGROUP)]
    pub workgroup: u32,
    /// Let `--backend wgpu` use a software adapter (WARP, lavapipe) when there is no GPU.
    #[arg(long)]
    pub allow_software: bool,
    /// Compile the CUDA kernels with fused multiply-adds (`--fmad=true`). Off by default, which
    /// keeps CUDA bit-identical to the CPU backend.
    #[arg(long)]
    pub fmad: bool,
    /// Steps replayed per CUDA graph launch (even; 0 launches every step on its own).
    #[arg(long, default_value_t = 50)]
    pub graph_steps: u32,
}

/// The engine a choice runs on, with its shared device handle.
#[derive(Debug, Clone)]
enum Engine {
    Cpu,
    Wgpu(Arc<GpuContext>),
    #[cfg(feature = "cuda")]
    Cuda(Arc<cfd_cuda::CudaDevice>),
}

/// What was actually chosen, for reports.
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

impl SolverArgs {
    /// Resolves the backend (falling back from `auto` to the next engine) and sets up the thread
    /// pool. Call once per process: rayon's global pool can only be configured once.
    pub fn resolve(&self, scene_pref: Option<BackendPreference>) -> Result<SolverChoice> {
        let backend = self.backend.unwrap_or(match scene_pref {
            Some(BackendPreference::Cpu) => BackendArg::Cpu,
            Some(BackendPreference::Wgpu) => BackendArg::Wgpu,
            Some(BackendPreference::Cuda) => BackendArg::Cuda,
            Some(BackendPreference::Auto) | None => BackendArg::Auto,
        });
        let engine = match backend {
            BackendArg::Cpu => EngineArg::Cpu,
            BackendArg::Wgpu => EngineArg::Wgpu,
            BackendArg::Cuda => EngineArg::Cuda,
            // Software adapters are slower than the CPU backend: `auto` never picks them.
            BackendArg::Auto => {
                let mut notes = Vec::new();
                let picked = pick_auto(
                    open_cuda,
                    || open_wgpu(false).map_err(|e| e.to_string()),
                    &mut notes,
                );
                for n in &notes {
                    eprintln!("note: {n}");
                }
                let engine = match picked {
                    #[cfg(feature = "cuda")]
                    Auto::Cuda(dev) => Engine::Cuda(dev),
                    #[cfg(not(feature = "cuda"))]
                    Auto::Cuda(never) => match never {},
                    Auto::Wgpu(ctx) => Engine::Wgpu(ctx),
                    Auto::Cpu => {
                        eprintln!("note: using the CPU backend");
                        Engine::Cpu
                    }
                };
                self.engine.configure_threads()?;
                return Ok(self.engine.choice(engine));
            }
        };
        self.engine.configure_threads()?;
        self.engine.open(engine)
    }
}

impl EngineArgs {
    /// Sets the size of rayon's global thread pool (when `--threads` is given). Call once per
    /// process, before any solver runs: the global pool can only be configured once.
    pub fn configure_threads(&self) -> Result<()> {
        if let Some(n) = self.threads {
            // In C# terms: configuring the default TaskScheduler's degree of parallelism.
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build_global()?;
        }
        Ok(())
    }

    /// Opens one engine, with no fallback (an unavailable engine is an error). Can be called
    /// any number of times; each call returns an independent choice.
    pub fn open(&self, engine: EngineArg) -> Result<SolverChoice> {
        let engine = match engine {
            EngineArg::Cpu => Engine::Cpu,
            #[cfg(feature = "cuda")]
            EngineArg::Cuda => Engine::Cuda(open_cuda().map_err(anyhow::Error::msg)?),
            #[cfg(not(feature = "cuda"))]
            EngineArg::Cuda => anyhow::bail!(cfd_cuda::CudaError::NotBuilt),
            EngineArg::Wgpu => Engine::Wgpu(open_wgpu(self.allow_software)?),
        };
        Ok(self.choice(engine))
    }

    fn choice(&self, engine: Engine) -> SolverChoice {
        let collision = match self.collision {
            CollisionArg::Trt => Collision::default(),
            CollisionArg::Bgk => Collision::Bgk,
        };
        let (name, magic) = match collision {
            Collision::Bgk => ("bgk", None),
            Collision::Trt { magic } => ("trt", Some(magic)),
        };
        let (backend, device) = match &engine {
            Engine::Cpu => ("cpu", None),
            Engine::Wgpu(ctx) => ("wgpu", Some(ctx.description())),
            #[cfg(feature = "cuda")]
            Engine::Cuda(dev) => ("cuda", Some(dev.description())),
        };
        SolverChoice {
            backend,
            device,
            collision: name,
            trt_magic: magic,
            threads: rayon::current_num_threads(),
            config: LbmConfig { collision },
            engine,
            workgroup: self.workgroup,
            fmad: self.fmad,
            graph_steps: self.graph_steps,
        }
    }
}

impl SolverChoice {
    /// Creates a solver for `domain` on the chosen backend.
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
}
