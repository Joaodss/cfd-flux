//! Encoding of per-pixel layers inside the JSON scene.
//!
//! Layers are raw little-endian arrays, optionally zstd-compressed, then base64-encoded.
//! Hand-drawn layers have large uniform areas, so zstd typically shrinks them by >50×.

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LayerData {
    pub encoding: LayerEncoding,
    pub dtype: LayerDtype,
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum LayerEncoding {
    #[serde(rename = "raw+base64")]
    RawBase64,
    #[serde(rename = "zstd+base64")]
    ZstdBase64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum LayerDtype {
    U8,
    U16,
}

impl LayerDtype {
    pub fn size(self) -> usize {
        match self {
            LayerDtype::U8 => 1,
            LayerDtype::U16 => 2,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LayerError {
    #[error("invalid base64: {0}")]
    Base64(#[from] base64::DecodeError),
    #[error("invalid zstd data: {0}")]
    Zstd(String),
    #[error("expected dtype {expected:?}, found {found:?}")]
    Dtype {
        expected: LayerDtype,
        found: LayerDtype,
    },
    #[error("expected {expected} cells, found {found}")]
    Length { expected: usize, found: usize },
}

impl LayerData {
    pub fn encode_u8(values: &[u8], encoding: LayerEncoding) -> Self {
        Self::encode_bytes(values, LayerDtype::U8, encoding)
    }

    pub fn encode_u16(values: &[u16], encoding: LayerEncoding) -> Self {
        let bytes: Vec<u8> = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        Self::encode_bytes(&bytes, LayerDtype::U16, encoding)
    }

    fn encode_bytes(bytes: &[u8], dtype: LayerDtype, encoding: LayerEncoding) -> Self {
        let payload = match encoding {
            LayerEncoding::RawBase64 => bytes.to_vec(),
            LayerEncoding::ZstdBase64 => ruzstd::encoding::compress_to_vec(
                bytes,
                ruzstd::encoding::CompressionLevel::Fastest,
            ),
        };
        Self {
            encoding,
            dtype,
            data: BASE64.encode(payload),
        }
    }

    fn decode_bytes(&self) -> Result<Vec<u8>, LayerError> {
        let payload = BASE64.decode(&self.data)?;
        match self.encoding {
            LayerEncoding::RawBase64 => Ok(payload),
            LayerEncoding::ZstdBase64 => {
                use std::io::Read as _;
                let mut decoder = ruzstd::decoding::StreamingDecoder::new(payload.as_slice())
                    .map_err(|e| LayerError::Zstd(e.to_string()))?;
                let mut out = Vec::new();
                decoder
                    .read_to_end(&mut out)
                    .map_err(|e| LayerError::Zstd(e.to_string()))?;
                Ok(out)
            }
        }
    }

    /// Decodes a `u8` layer and checks it has exactly `cells` values.
    pub fn decode_u8(&self, cells: usize) -> Result<Vec<u8>, LayerError> {
        self.expect_dtype(LayerDtype::U8)?;
        let bytes = self.decode_bytes()?;
        check_len(cells, bytes.len())?;
        Ok(bytes)
    }

    /// Decodes a `u16` layer and checks it has exactly `cells` values.
    pub fn decode_u16(&self, cells: usize) -> Result<Vec<u16>, LayerError> {
        self.expect_dtype(LayerDtype::U16)?;
        let bytes = self.decode_bytes()?;
        if bytes.len() % 2 != 0 {
            return Err(LayerError::Length {
                expected: cells * 2,
                found: bytes.len(),
            });
        }
        let values: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        check_len(cells, values.len())?;
        Ok(values)
    }

    fn expect_dtype(&self, expected: LayerDtype) -> Result<(), LayerError> {
        if self.dtype == expected {
            Ok(())
        } else {
            Err(LayerError::Dtype {
                expected,
                found: self.dtype,
            })
        }
    }
}

fn check_len(expected: usize, found: usize) -> Result<(), LayerError> {
    if expected == found {
        Ok(())
    } else {
        Err(LayerError::Length { expected, found })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn u8_round_trip_both_encodings() {
        let values: Vec<u8> = (0..1000).map(|i| (i / 100) as u8).collect();
        for enc in [LayerEncoding::RawBase64, LayerEncoding::ZstdBase64] {
            let layer = LayerData::encode_u8(&values, enc);
            assert_eq!(layer.decode_u8(values.len()).unwrap(), values);
        }
    }

    #[test]
    fn u16_round_trip_both_encodings() {
        let values: Vec<u16> = (0..1000).map(|i| (i * 37 % 700) as u16).collect();
        for enc in [LayerEncoding::RawBase64, LayerEncoding::ZstdBase64] {
            let layer = LayerData::encode_u16(&values, enc);
            assert_eq!(layer.decode_u16(values.len()).unwrap(), values);
        }
    }

    #[test]
    fn zstd_compresses_uniform_layers() {
        let values = vec![0u8; 512 * 512];
        let layer = LayerData::encode_u8(&values, LayerEncoding::ZstdBase64);
        assert!(
            layer.data.len() < values.len() / 50,
            "len = {}",
            layer.data.len()
        );
    }

    #[test]
    fn wrong_length_and_dtype_are_rejected() {
        let layer = LayerData::encode_u8(&[1, 2, 3], LayerEncoding::RawBase64);
        assert!(matches!(layer.decode_u8(4), Err(LayerError::Length { .. })));
        assert!(matches!(layer.decode_u16(3), Err(LayerError::Dtype { .. })));
    }
}
