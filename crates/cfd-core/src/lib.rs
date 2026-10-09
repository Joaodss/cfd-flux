//! Core types shared by every part of live-fluids.
//!
//! - [`scene`]: the `Scene` exchange format (what the web editor sends to the server).
//! - [`layers`]: encoding/decoding of the per-pixel layers.
//! - [`validate`]: semantic validation of a scene beyond what the JSON Schema can express.

pub mod examples;
pub mod layers;
pub mod scene;
pub mod validate;

pub use scene::Scene;
