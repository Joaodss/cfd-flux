//! Frames for the web viewer's decoder (contract test, Phase 5 design proposal §2.2).
//!
//! Writes `apps/web/tests/fixtures/frames/`: three LFFR v2 frames (shuffle+lz4 in f16 and f32,
//! and the uncompressed codec) and `frames.json` with their headers and the bits of every value.
//! The web app's Vitest suite decodes the frames and compares them with the JSON bit for bit.
//! This test rewrites the files with `UPDATE_FIXTURES=1` and otherwise fails if they differ from
//! what it would write (like the generated scenes and schema).

use std::path::PathBuf;

use cfd_io::frame::{Codec, Dtype, Field, FieldDesc, Frame};
use serde_json::{json, Value};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/web/tests/fixtures/frames")
}

/// Odd sizes (no accidental alignment); 851 cells.
const W: u32 = 37;
const H: u32 = 23;

/// A field of `n` values with long runs of zeros (long LZ4 matches with overlapping copies, match
/// lengths past 255) between noisy values (literal runs past 255), plus a few extreme values.
///
/// Only `+ − × ÷` on integers and floats, which IEEE 754 rounds exactly on every platform: the
/// first version used `f32::sin`, which comes from the platform's maths library, and the f32
/// fixture written on Windows differed by an ulp on Linux (CI).
fn values(n: usize, seed: u32) -> Vec<f32> {
    (0..n)
        .map(|i| match i {
            _ if (200..600).contains(&i) => 0.0,
            0 => 1.0e-6,  // an f16 subnormal
            1 => 60000.0, // near the largest f16
            2 => -0.0,
            _ => {
                // A small integer hash (an LCG step) → [0, 1), then a parabola for the shape.
                let h = (i as u32 ^ seed.wrapping_mul(0x9e37_79b9))
                    .wrapping_mul(1_664_525)
                    .wrapping_add(1_013_904_223);
                let noise = (h >> 8) as f32 / (1u32 << 24) as f32;
                let x = (i % 97) as f32 / 97.0;
                6.0 * x * (1.0 - x) - 1.5 + noise + 0.001 * i as f32
            }
        })
        .collect()
}

fn field(name: &str, components: u8, dtype: Dtype, seed: u32) -> Field {
    Field {
        desc: FieldDesc {
            name: name.into(),
            components,
            dtype,
        },
        values: values((W * H) as usize * components as usize, seed),
    }
}

fn frame(dtype: Dtype, frame_index: u32) -> Frame {
    let mut fields = vec![
        field("velocity", 2, dtype, 0),
        field("pressure", 1, dtype, 1),
    ];
    if dtype == Dtype::F32 {
        fields.push(field("vorticity", 1, dtype, 2));
    }
    Frame {
        job_id: "fixture".into(),
        frame_index,
        step: 1000 * frame_index as u64,
        sim_time: 0.125 * frame_index as f64,
        width: W,
        height: H,
        fields,
    }
}

/// The header and the bits of every value (u16 for f16, u32 for f32) of an encoded frame, as
/// Rust decodes it.
fn expected(bytes: &[u8]) -> Value {
    let header = Frame::decode_header(bytes).expect("header");
    let decoded = Frame::decode(bytes).expect("frame");
    let fields: Vec<Value> = decoded
        .fields
        .iter()
        .map(|f| {
            let bits: Vec<u32> = match f.desc.dtype {
                Dtype::F16 => f
                    .values
                    .iter()
                    .map(|v| half::f16::from_f32(*v).to_bits() as u32)
                    .collect(),
                Dtype::F32 => f.values.iter().map(|v| v.to_bits()).collect(),
            };
            json!({ "name": f.desc.name, "components": f.desc.components, "dtype": f.desc.dtype, "bits": bits })
        })
        .collect();
    json!({ "header": header, "fields": fields })
}

#[test]
fn browser_fixtures_are_up_to_date() {
    let files: Vec<(&str, Vec<u8>)> = vec![
        (
            "stream-f16.lffr",
            frame(Dtype::F16, 3).encode_with(Codec::ShuffleLz4).unwrap(),
        ),
        (
            "stored-f32.lffr",
            frame(Dtype::F32, 4).encode_with(Codec::ShuffleLz4).unwrap(),
        ),
        (
            "none-f16.lffr",
            frame(Dtype::F16, 5).encode_with(Codec::None).unwrap(),
        ),
    ];
    let manifest: serde_json::Map<String, Value> = files
        .iter()
        .map(|(name, bytes)| (name.to_string(), expected(bytes)))
        .collect();
    let manifest = serde_json::to_string(&Value::Object(manifest)).unwrap() + "\n";

    let dir = fixture_dir();
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::create_dir_all(&dir).unwrap();
        for (name, bytes) in &files {
            std::fs::write(dir.join(name), bytes).unwrap();
        }
        std::fs::write(dir.join("frames.json"), &manifest).unwrap();
    }
    for (name, bytes) in &files {
        let on_disk = std::fs::read(dir.join(name))
            .unwrap_or_else(|e| panic!("{name}: {e} (run with UPDATE_FIXTURES=1)"));
        assert!(
            &on_disk == bytes,
            "{name} is out of date: run `UPDATE_FIXTURES=1 cargo test -p cfd-io --test browser_fixtures`"
        );
    }
    let on_disk = std::fs::read_to_string(dir.join("frames.json")).unwrap();
    assert!(
        on_disk.replace("\r\n", "\n") == manifest,
        "frames.json is out of date: run `UPDATE_FIXTURES=1 cargo test -p cfd-io --test browser_fixtures`"
    );
}

#[test]
fn committed_colormaps_are_up_to_date() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/web/src/generated/colormaps.json");
    let committed = std::fs::read_to_string(path)
        .unwrap_or_default()
        .replace("\r\n", "\n");
    assert!(
        cfd_io::image::colormaps_json() == committed,
        "apps/web/src/generated/colormaps.json is outdated; run `cargo run -p cfd-cli -- colormaps apps/web/src/generated/colormaps.json`"
    );
}
