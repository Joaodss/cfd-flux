//! Result formats.
//!
//! - [`frame`]: the snapshot format used both for WebSocket streaming and for files on disk.
//! - [`image`]: colour maps and PNG encoding of scalar fields.

pub mod frame;
pub mod image;
