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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum CollisionArg {
    /// Two relaxation times with Λ = 3/16 (default).
    Trt,
    /// Single relaxation time.
    Bgk,
}

/// Solver options shared by `run`, `bench` and `verify`.
#[derive(Debug, Clone, Args)]
pub struct SolverArgs {
    /// Backend; defaults to the scene's preference (or `auto`).
    #[arg(long, value_enum)]
    pub backend: Option<BackendArg>,
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
        let wgpu = |allow_software: bool| {
            GpuContext::new(&GpuOptions {
                allow_software,
                ..Default::default()
            })
        };
        let engine = match backend {
            BackendArg::Cpu => Engine::Cpu,
            #[cfg(feature = "cuda")]
            BackendArg::Cuda => Engine::Cuda(open_cuda().map_err(anyhow::Error::msg)?),
            #[cfg(not(feature = "cuda"))]
            BackendArg::Cuda => anyhow::bail!(cfd_cuda::CudaError::NotBuilt),
            BackendArg::Wgpu => Engine::Wgpu(wgpu(self.allow_software)?),
            // Software adapters are slower than the CPU backend: `auto` never picks them.
            BackendArg::Auto => {
                let mut notes = Vec::new();
                let picked = pick_auto(
                    open_cuda,
                    || wgpu(false).map_err(|e| e.to_string()),
                    &mut notes,
                );
                for n in &notes {
                    eprintln!("note: {n}");
                }
                match picked {
                    #[cfg(feature = "cuda")]
                    Auto::Cuda(dev) => Engine::Cuda(dev),
                    #[cfg(not(feature = "cuda"))]
                    Auto::Cuda(never) => match never {},
                    Auto::Wgpu(ctx) => Engine::Wgpu(ctx),
                    Auto::Cpu => {
                        eprintln!("note: using the CPU backend");
                        Engine::Cpu
                    }
                }
            }
        };
        if let Some(n) = self.threads {
            // In C# terms: configuring the default TaskScheduler's degree of parallelism.
            rayon::ThreadPoolBuilder::new()
                .num_threads(n)
                .build_global()?;
        }
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
        Ok(SolverChoice {
            backend,
            device,
            collision: name,
            trt_magic: magic,
            threads: rayon::current_num_threads(),
            config: LbmConfig { collision },
            engine,
            workgroup: self.workgroup,
            fmad: self.fmad,
        })
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
                },
            )?),
        })
    }

    /// Whether the solver runs on a GPU (the workgroup/block size applies).
    pub fn is_gpu(&self) -> bool {
        !matches!(self.engine, Engine::Cpu)
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
