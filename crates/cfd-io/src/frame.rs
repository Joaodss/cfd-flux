//! Frame format v1: one snapshot of several fields at one instant.
//!
//! Layout (little-endian):
//!
//! ```text
//! offset  size         content
//! 0       4            magic b"LFFR"
//! 4       2            format version (u16) = 1
//! 6       4            header length H (u32)
//! 10      H            header, UTF-8 JSON (`FrameHeader`)
//! 10+H    rest         zstd( field 0 bytes ‖ field 1 bytes ‖ … ) in header order
//! ```
//!
//! Each field holds `width * height * components` values, row-major from the bottom-left
//! corner, components interleaved (`vx, vy, vx, vy, …`), stored as f16 or f32.
//! The JSON header keeps the format easy to extend and to decode in the browser.

use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"LFFR";
pub const VERSION: u16 = 1;
const PREFIX_LEN: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dtype {
    F16,
    F32,
}

impl Dtype {
    pub fn size(self) -> usize {
        match self {
            Dtype::F16 => 2,
            Dtype::F32 => 4,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldDesc {
    pub name: String,
    pub components: u8,
    pub dtype: Dtype,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameHeader {
    pub job_id: String,
    pub frame_index: u32,
    pub step: u64,
    /// Physical time in seconds.
    pub sim_time: f64,
    pub width: u32,
    pub height: u32,
    pub fields: Vec<FieldDesc>,
}

/// A field in memory. Values are always `f32` here; `desc.dtype` is the precision on the wire.
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    pub desc: FieldDesc,
    pub values: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub job_id: String,
    pub frame_index: u32,
    pub step: u64,
    pub sim_time: f64,
    pub width: u32,
    pub height: u32,
    pub fields: Vec<Field>,
}

#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("not a frame (bad magic)")]
    Magic,
    #[error("unsupported frame version {0}")]
    Version(u16),
    #[error("truncated frame")]
    Truncated,
    #[error("invalid header: {0}")]
    Header(#[from] serde_json::Error),
    #[error("invalid compressed payload: {0}")]
    Zstd(String),
    #[error("field '{name}' has {found} values, expected {expected}")]
    FieldLength {
        name: String,
        expected: usize,
        found: usize,
    },
    #[error("payload has {found} bytes, expected {expected}")]
    PayloadLength { expected: usize, found: usize },
}

impl Frame {
    fn values_per_component(&self) -> usize {
        self.width as usize * self.height as usize
    }

    fn header(&self) -> FrameHeader {
        FrameHeader {
            job_id: self.job_id.clone(),
            frame_index: self.frame_index,
            step: self.step,
            sim_time: self.sim_time,
            width: self.width,
            height: self.height,
            fields: self.fields.iter().map(|f| f.desc.clone()).collect(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        let mut raw = Vec::new();
        for f in &self.fields {
            let expected = self.values_per_component() * f.desc.components as usize;
            if f.values.len() != expected {
                return Err(FrameError::FieldLength {
                    name: f.desc.name.clone(),
                    expected,
                    found: f.values.len(),
                });
            }
            match f.desc.dtype {
                Dtype::F32 => raw.extend(f.values.iter().flat_map(|v| v.to_le_bytes())),
                Dtype::F16 => raw.extend(
                    f.values
                        .iter()
                        .flat_map(|v| half::f16::from_f32(*v).to_le_bytes()),
                ),
            }
        }
        let header = serde_json::to_vec(&self.header())?;
        let payload = ruzstd::encoding::compress_to_vec(
            raw.as_slice(),
            ruzstd::encoding::CompressionLevel::Fastest,
        );

        let mut out = Vec::with_capacity(PREFIX_LEN + header.len() + payload.len());
        out.extend_from_slice(&MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&(header.len() as u32).to_le_bytes());
        out.extend_from_slice(&header);
        out.extend_from_slice(&payload);
        Ok(out)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        if bytes.len() < PREFIX_LEN {
            return Err(FrameError::Truncated);
        }
        if bytes[0..4] != MAGIC {
            return Err(FrameError::Magic);
        }
        let version = u16::from_le_bytes([bytes[4], bytes[5]]);
        if version != VERSION {
            return Err(FrameError::Version(version));
        }
        let header_len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
        let header_end = PREFIX_LEN
            .checked_add(header_len)
            .ok_or(FrameError::Truncated)?;
        if bytes.len() < header_end {
            return Err(FrameError::Truncated);
        }
        let header: FrameHeader = serde_json::from_slice(&bytes[PREFIX_LEN..header_end])?;

        let raw = {
            use std::io::Read as _;
            let mut decoder = ruzstd::decoding::StreamingDecoder::new(&bytes[header_end..])
                .map_err(|e| FrameError::Zstd(e.to_string()))?;
            let mut raw = Vec::new();
            decoder
                .read_to_end(&mut raw)
                .map_err(|e| FrameError::Zstd(e.to_string()))?;
            raw
        };

        let n = header.width as usize * header.height as usize;
        let expected: usize = header
            .fields
            .iter()
            .map(|d| n * d.components as usize * d.dtype.size())
            .sum();
        if raw.len() != expected {
            return Err(FrameError::PayloadLength {
                expected,
                found: raw.len(),
            });
        }

        let mut offset = 0;
        let mut fields = Vec::with_capacity(header.fields.len());
        for desc in header.fields {
            let count = n * desc.components as usize;
            let bytes = &raw[offset..offset + count * desc.dtype.size()];
            offset += bytes.len();
            let values = match desc.dtype {
                Dtype::F32 => bytes
                    .chunks_exact(4)
                    .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
                    .collect(),
                Dtype::F16 => bytes
                    .chunks_exact(2)
                    .map(|c| half::f16::from_le_bytes([c[0], c[1]]).to_f32())
                    .collect(),
            };
            fields.push(Field { desc, values });
        }

        Ok(Frame {
            job_id: header.job_id,
            frame_index: header.frame_index,
            step: header.step,
            sim_time: header.sim_time,
            width: header.width,
            height: header.height,
            fields,
        })
    }
}
