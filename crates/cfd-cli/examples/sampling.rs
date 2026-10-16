//! Cost of one output frame with on-device sampling vs the CPU conversion (design proposal §6).
//!
//! ```bash
//! cargo run --release -p cfd-cli --example sampling -- [cuda,wgpu,cpu] [1024,2048]
//! ```
//!
//! A lid-driven cavity is stepped a little on each engine; then, per frame, the two samples a
//! server job takes — the stored frame (velocity, pressure, vorticity; f32, full resolution) and
//! one stream format (the same fields, f16, box-downsampled to at most 1024 cells per side) — are
//! produced twice: by `Solver::request_samples` (on the device for the GPU engines) and by the
//! CPU path (`request_fields`, then `FieldSet::from_lattice`, the box filter and the f16
//! conversion). Times are medians of 7 frames: waiting for the copy (`wait`), converting
//! (`into_samples`, the output thread's work) and encoding both frames (LFFR v2).

use std::time::{Duration, Instant};

use anyhow::Result;
use cfd_core::sample::{LatticeSamples, PendingSamples, Precision, SampleSpec};
use cfd_core::scene::OutputField;
use cfd_core::{Domain, DomainOptions};
use cfd_engines::{EngineKind, EngineOptions};

fn ms(d: Duration) -> String {
    format!("{:.2} ms", d.as_secs_f64() * 1e3)
}

fn median(mut v: Vec<Duration>) -> Duration {
    v.sort();
    v[v.len() / 2]
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let kinds: Vec<EngineKind> = args
        .first()
        .map_or("cuda,wgpu,cpu", String::as_str)
        .split(',')
        .map(|s| {
            EngineKind::BY_PREFERENCE
                .into_iter()
                .find(|k| k.name() == s)
                .ok_or_else(|| anyhow::anyhow!("unknown engine {s}"))
        })
        .collect::<Result<_>>()?;
    let sizes: Vec<u32> = args
        .get(1)
        .map_or("1024,2048", String::as_str)
        .split(',')
        .map(|s| s.parse())
        .collect::<Result<_, _>>()?;
    let opts = EngineOptions::default();
    let engines = cfd_engines::open_all(&kinds, &opts)?;
    let fields = vec![
        OutputField::Velocity,
        OutputField::Pressure,
        OutputField::Vorticity,
    ];
    println!("| Engine | Grid | Path | Copy (wait) | Conversion | Encoding (2 frames) | Total |");
    println!("|---|---|---|---:|---:|---:|---:|");
    for &n in &sizes {
        let scene = cfd_verify::scenes::lid_driven_cavity(n, 100.0);
        let domain = Domain::from_scene(&scene, &DomainOptions::default())
            .map_err(|r| anyhow::anyhow!("{:?}", r.issues))?;
        let specs = [
            SampleSpec::full(fields.clone(), Precision::F32),
            SampleSpec {
                fields: fields.clone(),
                factor: domain.width.max(domain.height).div_ceil(1024),
                precision: Precision::F16,
            },
        ];
        for (label, choice) in &engines {
            let mut solver = choice.make(domain.clone())?;
            solver.step(100)?;
            solver.finish()?;
            for device in [true, false] {
                if !device && solver.backend() == cfd_core::solver::BackendKind::Cpu {
                    continue; // the same path twice
                }
                let (mut wait, mut convert, mut encode) = (vec![], vec![], vec![]);
                for _ in 0..7 {
                    solver.step(10)?;
                    solver.finish()?;
                    let t0 = Instant::now();
                    let pending: Box<dyn PendingSamples> = if device {
                        solver.request_samples(&specs)
                    } else {
                        Box::new(LatticeSamples {
                            pending: solver.request_fields(),
                            specs: specs.to_vec(),
                        })
                    };
                    let set = pending.wait();
                    let t1 = Instant::now();
                    let samples = set.into_samples(solver.domain());
                    let t2 = Instant::now();
                    for s in &samples {
                        cfd_job::encode_sample("bench", 0, s)?;
                    }
                    let t3 = Instant::now();
                    wait.push(t1 - t0);
                    convert.push(t2 - t1);
                    encode.push(t3 - t2);
                }
                let (w, c, e) = (median(wait), median(convert), median(encode));
                println!(
                    "| {label} | {}×{} | {} | {} | {} | {} | {} |",
                    domain.width,
                    domain.height,
                    if device {
                        "request_samples"
                    } else {
                        "CPU conversion"
                    },
                    ms(w),
                    ms(c),
                    ms(e),
                    ms(w + c + e)
                );
            }
        }
    }
    Ok(())
}
