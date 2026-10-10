//! Validation cases for live-fluids solvers (`docs/08-validation-guide.md`).
//!
//! The cases only see the [`cfd_core::Solver`] trait: the caller supplies a [`SolverFactory`]
//! that builds a solver of any backend for a [`Domain`], so the same suite validates the CPU
//! reference now and the GPU backends in Phase 2.
//!
//! - [`scenes`]: the validation geometries as regular scenes (also used by `cfd-cli bench`).

pub mod scenes;

use cfd_core::{Domain, Solver};

/// Builds a solver for a domain on some backend (in C#: a `Func<Domain, ISolver>`).
pub type SolverFactory<'a> = &'a dyn Fn(Domain) -> anyhow::Result<Box<dyn Solver>>;
