//! Lattice Boltzmann solver — CPU reference implementation.
//!
//! - [`lattice`]: D2Q9 (flow) and D2Q5 (temperature) velocity sets.
//! - [`config`]: collision operator (BGK or TRT, the default).
//! - [`CpuLbm`]: the CPU backend of the [`cfd_core::Solver`] trait: fused stream-collide
//!   (pull, ping-pong, SoA), link-wise boundaries, Guo forcing, Boussinesq coupling.
//!
//! This backend is the numerical reference that the GPU backends are tested against; the
//! memory layout is documented in `kernel.rs` and must stay identical in WGSL and CUDA.

pub mod config;
mod cpu;
mod kernel;
pub mod lattice;

pub use config::{Collision, LbmConfig};
pub use cpu::CpuLbm;
