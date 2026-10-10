//! Backend selection: the only place that knows the concrete solver types. Everything else
//! works with `Box<dyn Solver>`.

use anyhow::{bail, Result};
use cfd_core::scene::BackendPreference;
use cfd_core::{Domain, Solver};
use cfd_lbm::{Collision, CpuLbm, LbmConfig};
use clap::{Args, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendArg {
    /// CUDA if available, then wgpu, then CPU (only the CPU exists in Phase 1).
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
}

/// What was actually chosen, for reports.
#[derive(Debug, Clone)]
pub struct SolverChoice {
    pub backend: &'static str,
    pub collision: &'static str,
    pub trt_magic: Option<f64>,
    pub threads: usize,
    config: LbmConfig,
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
        match backend {
            BackendArg::Auto | BackendArg::Cpu => {}
            BackendArg::Wgpu | BackendArg::Cuda => {
                bail!("the {backend:?} backend is not available yet (Phase 2); use --backend cpu")
            }
        }
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
            backend: "cpu",
            collision: name,
            trt_magic: magic,
            threads: rayon::current_num_threads(),
            config: LbmConfig { collision },
        })
    }
}

impl SolverChoice {
    /// Creates a solver for `domain` on the chosen backend.
    pub fn make(&self, domain: Domain) -> Result<Box<dyn Solver>> {
        Ok(Box::new(CpuLbm::new(domain, self.config)))
    }
}
