//! Frame codec benchmark (open question Q13, `docs/benchmarks.md`, "Frame encoding").
//!
//! ```bash
//! # A 1024² lid-driven cavity scene (Re = 100, one frame every 0.5 s):
//! cargo run --release -p cfd-cli --example frame_codec -- cavity-scene cavity-1024.json
//! # Encode and decode times and sizes of stored frames, for every codec, f32 and f16:
//! cargo run --release -p cfd-cli --example frame_codec -- bench run-dir/frames/000004.bin …
//! ```

use std::time::{Duration, Instant};

use anyhow::{bail, Context, Result};
use cfd_io::frame::{shuffle, shuffle_into, Codec, Dtype, Frame};

/// Median wall time of `f` over enough repetitions to fill ≈ 1 s (at least 3).
fn time<T>(mut f: impl FnMut() -> T) -> (Duration, T) {
    let t = Instant::now();
    let mut out = f();
    let once = t.elapsed();
    let reps = (Duration::from_secs(1).as_secs_f64() / once.as_secs_f64().max(1e-6))
        .clamp(3.0, 200.0) as usize;
    let mut times = vec![once];
    for _ in 0..reps {
        let t = Instant::now();
        out = f();
        times.push(t.elapsed());
    }
    times.sort();
    (times[times.len() / 2], out)
}

/// The byte shuffle as a plain indexed loop (the Q13 measurement).
fn scalar_shuffle(input: &[u8], width: usize) -> Vec<u8> {
    let m = input.len() / width;
    let mut out = vec![0u8; input.len()];
    for i in 0..m {
        for b in 0..width {
            out[b * m + i] = input[i * width + b];
        }
    }
    out
}

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

fn bench(path: &str) -> Result<()> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {path}"))?;
    let stored = Frame::decode(&bytes)?;
    let values: usize = stored.fields.iter().map(|f| f.values.len()).sum();
    println!(
        "\n{path}: {} × {}, {} fields, {values} values; stored as {:?}, {} bytes",
        stored.width,
        stored.height,
        stored.fields.len(),
        Frame::decode_header(&bytes)?.codec,
        bytes.len()
    );
    let single = rayon::ThreadPoolBuilder::new().num_threads(1).build()?;
    for dtype in [Dtype::F32, Dtype::F16] {
        let mut frame = stored.clone();
        frame.fields.iter_mut().for_each(|f| f.desc.dtype = dtype);
        let raw = frame.encode_with(Codec::None)?.len();
        let (pack, _) = time(|| frame.encode_with(Codec::None).unwrap());
        println!("  {dtype:?}: raw {raw} bytes, packing {}", ms(pack));
        for codec in [Codec::Zstd, Codec::ShuffleLz4] {
            let (enc, out) = time(|| frame.encode_with(codec).unwrap());
            let (enc1, _) = single.install(|| time(|| frame.encode_with(codec).unwrap()));
            let (dec, _) = time(|| Frame::decode(&out).unwrap());
            println!(
                "    {codec:?}: {:.1}% of raw, encode {} ({} on one thread), decode {}",
                100.0 * out.len() as f64 / raw as f64,
                ms(enc),
                ms(enc1),
                ms(dec)
            );
        }
        let w = dtype.size();
        let field: Vec<u8> = frame
            .fields
            .iter()
            .flat_map(|f| match dtype {
                Dtype::F32 => f
                    .values
                    .iter()
                    .flat_map(|v| v.to_le_bytes())
                    .collect::<Vec<_>>(),
                Dtype::F16 => f
                    .values
                    .iter()
                    .flat_map(|v| f16_from_f32(*v).to_le_bytes())
                    .collect(),
            })
            .collect();
        let (scalar, a) = time(|| scalar_shuffle(&field, w));
        let (blocked, b) = time(|| shuffle(&field, w));
        let mut reused = vec![0u8; field.len()];
        let (into, _) = time(|| shuffle_into(&field, w, &mut reused));
        assert_eq!(a, b);
        println!(
            "    shuffle of every field: scalar loop {}, blocked {}, blocked into a reused buffer {}",
            ms(scalar),
            ms(blocked),
            ms(into)
        );
    }
    Ok(())
}

/// Round-to-nearest-even f32 → f16 bits, flushing subnormals (enough for a benchmark input).
fn f16_from_f32(v: f32) -> u16 {
    let x = v.to_bits();
    let sign = ((x >> 16) & 0x8000) as u16;
    let exp = ((x >> 23) & 0xff) as i32 - 127 + 15;
    if exp >= 31 {
        return sign | 0x7c00;
    }
    if exp <= 0 {
        return sign;
    }
    let mant = x & 0x7f_ffff;
    let mut h = ((exp as u32) << 10) | (mant >> 13);
    let rest = mant & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && h & 1 == 1) {
        h += 1;
    }
    sign | h as u16
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("cavity-scene") => {
            let out = args.get(1).context("cavity-scene <out.json>")?;
            let mut scene = cfd_verify::scenes::lid_driven_cavity(1024, 100.0);
            scene.run.end_time = 2.0;
            scene.run.output_interval = 0.5;
            std::fs::write(out, serde_json::to_string_pretty(&scene)?)?;
            Ok(())
        }
        Some("bench") if args.len() > 1 => args[1..].iter().try_for_each(|p| bench(p)),
        _ => bail!("usage: frame_codec cavity-scene <out.json> | bench <frame.bin>…"),
    }
}
