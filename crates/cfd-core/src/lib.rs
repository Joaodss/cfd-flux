//! Core types shared by every part of live-fluids.
//!
//! - [`scene`]: the `Scene` exchange format (what the web editor sends to the server).
//! - [`layers`]: encoding/decoding of the per-pixel layers.
//! - [`validate`]: semantic validation of a scene beyond what the JSON Schema can express.
//! - [`units`]: physical ↔ lattice unit conversion and stability checks.
//! - [`domain`]: the scene converted for the solvers (`Scene → Domain`).
//! - [`solver`]: the `Solver` trait implemented by every backend.

pub mod domain;
pub mod examples;
pub mod layers;
pub mod scene;
pub mod solver;
pub mod units;
pub mod validate;

pub use domain::{Domain, DomainOptions};
pub use scene::Scene;
pub use solver::Solver;
