//! Command-line options for choosing and configuring a backend. Opening engines and making
//! solvers is `cfd-engines`' job; this module only maps flags onto it.

use anyhow::Result;
use cfd_core::scene::BackendPreference;
use cfd_engines::{EngineKind, EngineOptions, SolverChoice};
use cfd_lbm::Collision;
use clap::{Args, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum BackendArg {
    /// The best available engine: CUDA, else a hardware GPU through wgpu, else the CPU.
    Auto,
    Cpu,
    Wgpu,
    Cuda,
}

impl BackendArg {
    fn preference(self) -> BackendPreference {
        match self {
            BackendArg::Auto => BackendPreference::Auto,
            BackendArg::Cpu => BackendPreference::Cpu,
            BackendArg::Wgpu => BackendPreference::Wgpu,
            BackendArg::Cuda => BackendPreference::Cuda,
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
    #[arg(long, default_value_t = cfd_engines::DEFAULT_GRAPH_STEPS)]
    pub graph_steps: u32,
    /// Store ρ, u and θ on every step instead of only on the last step of each batch (the
    /// results are identical; for comparisons and benchmarks).
    #[arg(long)]
    pub no_skip_macro: bool,
}

impl SolverArgs {
    /// Resolves the backend (falling back from `auto` to the next engine, with a note on stderr
    /// for each engine skipped) and sets up the thread pool. Call once per process: rayon's
    /// global pool can only be configured once.
    pub fn resolve(&self, scene_pref: Option<BackendPreference>) -> Result<SolverChoice> {
        let pref = match self.backend {
            Some(b) => b.preference(),
            None => scene_pref.unwrap_or(BackendPreference::Auto),
        };
        let opts = self.engine.options();
        let mut notes = Vec::new();
        let engine = cfd_engines::open_preferred(pref, &opts, &mut notes);
        for n in &notes {
            eprintln!("note: {n}");
        }
        let engine = engine?;
        self.engine.configure_threads()?;
        Ok(SolverChoice::new(engine, &opts))
    }
}

impl EngineArgs {
    pub fn options(&self) -> EngineOptions {
        EngineOptions {
            collision: match self.collision {
                CollisionArg::Trt => Collision::default(),
                CollisionArg::Bgk => Collision::Bgk,
            },
            workgroup: self.workgroup,
            allow_software: self.allow_software,
            fmad: self.fmad,
            graph_steps: self.graph_steps,
            skip_macro: !self.no_skip_macro,
        }
    }

    /// Sets the size of rayon's global thread pool (when `--threads` is given). Call once per
    /// process, before any solver runs.
    pub fn configure_threads(&self) -> Result<()> {
        cfd_engines::configure_threads(self.threads)
    }

    /// Opens one engine, with no fallback (an unavailable engine is an error). Can be called
    /// any number of times; each call returns an independent choice.
    pub fn open(&self, kind: EngineKind) -> Result<SolverChoice> {
        let opts = self.options();
        Ok(SolverChoice::new(cfd_engines::open(kind, &opts)?, &opts))
    }

    /// Opens every engine of `list` (an unavailable one is an error), labelled as in
    /// [`cfd_engines::label`].
    pub fn open_all(&self, list: &[EngineKind]) -> Result<Vec<(String, SolverChoice)>> {
        cfd_engines::open_all(list, &self.options())
    }
}
