//! Frame format (LFFR) v2: one snapshot of several fields at one instant.
//!
//! Layout (little-endian):
//!
//! ```text
//! offset  size         content
//! 0       4            magic b"LFFR"
//! 4       2            format version (u16) = 2 (1 is still read)
//! 6       4            header length H (u32)
//! 10      H            header, UTF-8 JSON (`FrameHeader`, with `codec`)
//! 10+H    rest         payload, by codec:
//!                        "shuffle+lz4": per field, in header order: u32 length L, then L bytes of
//!                                       lz4 block( byte-shuffled field bytes )
//!                        "zstd":        zstd( field 0 bytes ‖ field 1 bytes ‖ … )   (all v1 frames)
//!                        "none":        field 0 bytes ‖ field 1 bytes ‖ …
//! ```
//!
//! Each field holds `width * height * components` values, row-major from the bottom-left
//! corner, components interleaved (`vx, vy, vx, vy, …`), stored as f16 or f32.
//! The JSON header keeps the format easy to extend and to decode in the browser.
//!
//! **Byte shuffle** (as in HDF5/Blosc): a field of `m` values of `s` bytes is stored as the `m`
//! first bytes of every value, then the `m` second bytes, and so on. The high bytes (sign,
//! exponent, top of the mantissa) of neighbouring values are alike and compress well; the low
//! mantissa bytes are noise. Shuffling makes these fields 25–35% smaller with any codec (Q13).
//! Each field is its own lz4 block, so fields are compressed in parallel and can be decoded one
//! at a time.

use rayon::prelude::*;
use serde::{Deserialize, Serialize};

pub const MAGIC: [u8; 4] = *b"LFFR";
/// The version written.
pub const VERSION: u16 = 2;
const PREFIX_LEN: usize = 10;

// Field bytes are cast from `f32`/`u16` slices in place, which is their little-endian encoding
// only on little-endian targets (every target this project builds for).
const _: () = assert!(cfg!(target_endian = "little"));

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

/// How the payload is compressed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Codec {
    #[serde(rename = "none")]
    None,
    /// The only codec of v1 frames (`ruzstd`, ADR-007 before Q13); read, not written by default.
    #[serde(rename = "zstd")]
    Zstd,
    /// Byte shuffle by element width, then one `lz4_flex` block per field (Q13).
    #[default]
    #[serde(rename = "shuffle+lz4")]
    ShuffleLz4,
}

impl Codec {
    /// v1 headers have no `codec`: their payload is always zstd.
    fn v1() -> Self {
        Codec::Zstd
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
    #[serde(default = "Codec::v1")]
    pub codec: Codec,
    pub fields: Vec<FieldDesc>,
}

/// Everything in a frame header but the fields and the codec.
#[derive(Debug, Clone, PartialEq)]
pub struct FrameInfo {
    pub job_id: String,
    pub frame_index: u32,
    pub step: u64,
    pub sim_time: f64,
    pub width: u32,
    pub height: u32,
}

/// A field already in its wire precision: `bytes` are the little-endian values (f16 bits or
/// f32), e.g. `bytemuck::cast_slice(&values)`.
#[derive(Debug, Clone, Copy)]
pub struct RawField<'a> {
    pub desc: &'a FieldDesc,
    pub bytes: &'a [u8],
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
    Compressed(String),
    #[error("field '{name}' has {found} values, expected {expected}")]
    FieldLength {
        name: String,
        expected: usize,
        found: usize,
    },
    #[error("payload has {found} bytes, expected {expected}")]
    PayloadLength { expected: usize, found: usize },
}

/// Encodes a frame from fields already packed in their wire precision (see [`RawField`]).
pub fn encode_raw(
    info: &FrameInfo,
    fields: &[RawField],
    codec: Codec,
) -> Result<Vec<u8>, FrameError> {
    let n = info.width as usize * info.height as usize;
    for f in fields {
        let expected = n * f.desc.components as usize;
        let found = f.bytes.len() / f.desc.dtype.size();
        if found != expected || f.bytes.len() % f.desc.dtype.size() != 0 {
            return Err(FrameError::FieldLength {
                name: f.desc.name.clone(),
                expected,
                found,
            });
        }
    }
    let header = FrameHeader {
        job_id: info.job_id.clone(),
        frame_index: info.frame_index,
        step: info.step,
        sim_time: info.sim_time,
        width: info.width,
        height: info.height,
        codec,
        fields: fields.iter().map(|f| f.desc.clone()).collect(),
    };
    let header = serde_json::to_vec(&header)?;
    let mut out = Vec::with_capacity(PREFIX_LEN + header.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&(header.len() as u32).to_le_bytes());
    out.extend_from_slice(&header);
    match codec {
        Codec::None => fields.iter().for_each(|f| out.extend_from_slice(f.bytes)),
        Codec::Zstd => {
            let raw: Vec<u8> = fields.iter().flat_map(|f| f.bytes).copied().collect();
            out.extend(ruzstd::encoding::compress_to_vec(
                raw.as_slice(),
                ruzstd::encoding::CompressionLevel::Fastest,
            ));
        }
        Codec::ShuffleLz4 => {
            // One task per field on rayon's thread pool (≈ `Parallel.ForEach`); `collect` keeps
            // the header order.
            let blocks: Vec<Vec<u8>> = fields
                .par_iter()
                .map(|f| lz4_flex::block::compress(&shuffle(f.bytes, f.desc.dtype.size())))
                .collect();
            out.reserve(blocks.iter().map(|b| 4 + b.len()).sum());
            for b in blocks {
                out.extend_from_slice(&(b.len() as u32).to_le_bytes());
                out.extend_from_slice(&b);
            }
        }
    }
    Ok(out)
}

impl Frame {
    fn info(&self) -> FrameInfo {
        FrameInfo {
            job_id: self.job_id.clone(),
            frame_index: self.frame_index,
            step: self.step,
            sim_time: self.sim_time,
            width: self.width,
            height: self.height,
        }
    }

    /// Encodes with the default codec (byte shuffle + lz4).
    pub fn encode(&self) -> Result<Vec<u8>, FrameError> {
        self.encode_with(Codec::default())
    }

    pub fn encode_with(&self, codec: Codec) -> Result<Vec<u8>, FrameError> {
        let n = self.width as usize * self.height as usize;
        let mut packed = Vec::with_capacity(self.fields.len());
        for f in &self.fields {
            let expected = n * f.desc.components as usize;
            if f.values.len() != expected {
                return Err(FrameError::FieldLength {
                    name: f.desc.name.clone(),
                    expected,
                    found: f.values.len(),
                });
            }
            packed.push(match f.desc.dtype {
                Dtype::F32 => bytemuck::cast_slice::<f32, u8>(&f.values).to_vec(),
                Dtype::F16 => {
                    use half::slice::HalfFloatSliceExt as _; // `convert_from_f32_slice`, `reinterpret_cast`
                    let mut h = vec![half::f16::ZERO; f.values.len()];
                    // Round-to-nearest-even, vectorised (F16C) when the CPU has it.
                    h.convert_from_f32_slice(&f.values);
                    bytemuck::cast_slice::<u16, u8>(h.reinterpret_cast()).to_vec()
                }
            });
        }
        let raw: Vec<RawField> = self
            .fields
            .iter()
            .zip(&packed)
            .map(|(f, bytes)| RawField {
                desc: &f.desc,
                bytes,
            })
            .collect();
        encode_raw(&self.info(), &raw, codec)
    }

    /// Reads the header only (e.g. to find the codec or the size of a frame).
    pub fn decode_header(bytes: &[u8]) -> Result<FrameHeader, FrameError> {
        Ok(split(bytes)?.0)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, FrameError> {
        let (header, payload) = split(bytes)?;
        let n = header.width as usize * header.height as usize;
        let sizes: Vec<usize> = header
            .fields
            .iter()
            .map(|d| n * d.components as usize * d.dtype.size())
            .collect();
        let expected: usize = sizes.iter().sum();
        let raw: Vec<Vec<u8>> = match header.codec {
            Codec::None => split_concatenated(payload.to_vec(), &sizes)?,
            Codec::Zstd => {
                use std::io::Read as _;
                let mut decoder = ruzstd::decoding::StreamingDecoder::new(payload)
                    .map_err(|e| FrameError::Compressed(e.to_string()))?;
                let mut raw = Vec::new();
                decoder
                    .read_to_end(&mut raw)
                    .map_err(|e| FrameError::Compressed(e.to_string()))?;
                split_concatenated(raw, &sizes)?
            }
            Codec::ShuffleLz4 => {
                let mut rest = payload;
                let mut out = Vec::with_capacity(sizes.len());
                for (d, &size) in header.fields.iter().zip(&sizes) {
                    if rest.len() < 4 {
                        return Err(FrameError::Truncated);
                    }
                    let len = u32::from_le_bytes([rest[0], rest[1], rest[2], rest[3]]) as usize;
                    let block = rest.get(4..4 + len).ok_or(FrameError::Truncated)?;
                    rest = &rest[4 + len..];
                    let mut shuffled = vec![0u8; size];
                    let found = lz4_flex::block::decompress_into(block, &mut shuffled)
                        .map_err(|e| FrameError::Compressed(e.to_string()))?;
                    if found != size {
                        return Err(FrameError::PayloadLength { expected, found });
                    }
                    out.push(unshuffle(&shuffled, d.dtype.size()));
                }
                if !rest.is_empty() {
                    return Err(FrameError::PayloadLength {
                        expected,
                        found: expected + rest.len(),
                    });
                }
                out
            }
        };

        let fields = header
            .fields
            .into_iter()
            .zip(raw)
            .map(|(desc, bytes)| {
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
                Field { desc, values }
            })
            .collect();

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

/// The header and the payload of a v1 or v2 frame.
fn split(bytes: &[u8]) -> Result<(FrameHeader, &[u8]), FrameError> {
    if bytes.len() < PREFIX_LEN {
        return Err(FrameError::Truncated);
    }
    if bytes[0..4] != MAGIC {
        return Err(FrameError::Magic);
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if !(1..=VERSION).contains(&version) {
        return Err(FrameError::Version(version));
    }
    let header_len = u32::from_le_bytes([bytes[6], bytes[7], bytes[8], bytes[9]]) as usize;
    let header_end = PREFIX_LEN
        .checked_add(header_len)
        .ok_or(FrameError::Truncated)?;
    if bytes.len() < header_end {
        return Err(FrameError::Truncated);
    }
    let mut header: FrameHeader = serde_json::from_slice(&bytes[PREFIX_LEN..header_end])?;
    if version == 1 {
        header.codec = Codec::Zstd;
    }
    Ok((header, &bytes[header_end..]))
}

/// Cuts a concatenated payload into fields of the given sizes.
fn split_concatenated(raw: Vec<u8>, sizes: &[usize]) -> Result<Vec<Vec<u8>>, FrameError> {
    let expected: usize = sizes.iter().sum();
    if raw.len() != expected {
        return Err(FrameError::PayloadLength {
            expected,
            found: raw.len(),
        });
    }
    let mut offset = 0;
    Ok(sizes
        .iter()
        .map(|&s| {
            offset += s;
            raw[offset - s..offset].to_vec()
        })
        .collect())
}

/// [`shuffle`] into an existing buffer of the same length (no allocation).
pub fn shuffle_into(input: &[u8], width: usize, out: &mut [u8]) {
    assert_eq!(input.len(), out.len(), "shuffle buffers differ in length");
    match width {
        2 => shuffle_n::<2>(input, out),
        4 => shuffle_n::<4>(input, out),
        _ => out.copy_from_slice(input),
    }
}

/// Byte shuffle of values `width` bytes wide: byte `b` of value `i` goes to `b · m + i`.
pub fn shuffle(input: &[u8], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; input.len()];
    match width {
        2 => shuffle_n::<2>(input, &mut out),
        4 => shuffle_n::<4>(input, &mut out),
        _ => out.copy_from_slice(input),
    }
    out
}

/// The inverse of [`shuffle`].
pub fn unshuffle(input: &[u8], width: usize) -> Vec<u8> {
    let mut out = vec![0u8; input.len()];
    match width {
        2 => unshuffle_n::<2>(input, &mut out),
        4 => unshuffle_n::<4>(input, &mut out),
        _ => out.copy_from_slice(input),
    }
    out
}

/// Values per block of the shuffle loops: the inner loops have a constant trip count and no
/// bounds checks, so LLVM turns them into SIMD byte shuffles (≈ what `Vector128.Shuffle` would do
/// by hand in C#) without any `unsafe` code.
const BLOCK: usize = 64;

fn shuffle_n<const W: usize>(input: &[u8], out: &mut [u8]) {
    let m = input.len() / W;
    let blocks = m / BLOCK;
    for (blk, src) in input[..blocks * W * BLOCK]
        .chunks_exact(W * BLOCK)
        .enumerate()
    {
        for b in 0..W {
            let dst = &mut out[b * m + blk * BLOCK..][..BLOCK];
            for (i, d) in dst.iter_mut().enumerate() {
                *d = src[i * W + b];
            }
        }
    }
    for i in blocks * BLOCK..m {
        for b in 0..W {
            out[b * m + i] = input[i * W + b];
        }
    }
    // A trailing partial value (never in a valid field) is copied as it is.
    let tail = m * W;
    out[tail..].copy_from_slice(&input[tail..]);
}

fn unshuffle_n<const W: usize>(input: &[u8], out: &mut [u8]) {
    let m = input.len() / W;
    let blocks = m / BLOCK;
    for (blk, dst) in out[..blocks * W * BLOCK]
        .chunks_exact_mut(W * BLOCK)
        .enumerate()
    {
        for b in 0..W {
            let src = &input[b * m + blk * BLOCK..][..BLOCK];
            for (i, s) in src.iter().enumerate() {
                dst[i * W + b] = *s;
            }
        }
    }
    for i in blocks * BLOCK..m {
        for b in 0..W {
            out[i * W + b] = input[b * m + i];
        }
    }
    let tail = m * W;
    out[tail..].copy_from_slice(&input[tail..]);
}
