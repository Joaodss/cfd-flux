//! Backend selection: the only place that knows the concrete solver types. Everything else
//! works with `Box<dyn Solver>`.

use std::sync::Arc;

use anyhow::{bail, Result};
use cfd_core::scene::BackendPreference;
use cfd_core::{Domain, Solver};
use cfd_gpu::{GpuContext, GpuOptions, WgpuLbm};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};
use clap::{Args, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendArg {
    /// A hardware GPU through wgpu if there is one, else the CPU (CUDA joins in Phase 2b).
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
    /// Invocations per workgroup of the wgpu stream-collide kernel.
    #[arg(long, default_value_t = cfd_gpu::DEFAULT_WORKGROUP)]
    pub workgroup: u32,
    /// Let `--backend wgpu` use a software adapter (WARP, lavapipe) when there is no GPU.
    #[arg(long)]
    pub allow_software: bool,
}

/// What was actually chosen, for reports.
#[derive(Debug, Clone)]
pub struct SolverChoice {
    pub backend: &'static str,
    /// The GPU adapter, e.g. "NVIDIA GeForce RTX 4060 (Vulkan, DiscreteGpu)".
    pub device: Option<String>,
    pub collision: &'static str,
    pub trt_magic: Option<f64>,
    pub threads: usize,
    config: LbmConfig,
    gpu: Option<Arc<GpuContext>>,
    workgroup: u32,
}

impl SolverArgs {
    /// Resolves the backend (falling back from `auto` to the CPU) and sets up the thread pool.
    /// Call once per process: rayon's global pool can only be configured once.
    pub fn resolve(&self, scene_pref: Option<BackendPreference>) -> Result<SolverChoice> {
        let backend = self.backend.unwrap_or(match scene_pref {
            Some(BackendPreference::Cpu) => BackendArg::Cpu,
            Some(BackendPreference::Wgpu) => BackendArg::Wgpu,
            Some(BackendPreference::Cuda) => BackendArg::Cuda,
            Some(BackendPreference::Auto) | None => BackendArg::Auto,
        });
        let gpu = match backend {
            BackendArg::Cpu => None,
            BackendArg::Cuda => {
                bail!("the CUDA backend is not available yet (Phase 2b); use --backend wgpu or cpu")
            }
            BackendArg::Wgpu => Some(GpuContext::new(&GpuOptions {
                allow_software: self.allow_software,
                ..Default::default()
            })?),
            // Software adapters are slower than the CPU backend: `auto` never picks them.
            BackendArg::Auto => match GpuContext::new(&GpuOptions::default()) {
                Ok(ctx) => Some(ctx),
                Err(e) => {
                    eprintln!("note: using the CPU backend ({e})");
                    None
                }
            },
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
        Ok(SolverChoice {
            backend: if gpu.is_some() { "wgpu" } else { "cpu" },
            device: gpu.as_ref().map(|g| g.description()),
            collision: name,
            trt_magic: magic,
            threads: rayon::current_num_threads(),
            config: LbmConfig { collision },
            gpu,
            workgroup: self.workgroup,
        })
    }
}

impl SolverChoice {
    /// Creates a solver for `domain` on the chosen backend.
    pub fn make(&self, domain: Domain) -> Result<Box<dyn Solver>> {
        Ok(match &self.gpu {
            Some(ctx) => Box::new(WgpuLbm::with_workgroup(
                ctx.clone(),
                domain,
                self.config,
                self.workgroup,
            )?),
            None => Box::new(CpuLbm::new(domain, self.config)),
        })
    }

    /// "cpu" or "wgpu (adapter)", for log lines.
    pub fn describe(&self) -> String {
        match &self.device {
            Some(d) => format!("{} ({d})", self.backend),
            None => self.backend.to_string(),
        }
    }
}
