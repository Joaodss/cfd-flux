//! Contracts with the web viewer (`apps/web`): its generated reference data is current, and its
//! Strouhal number is `harness::oscillation_frequency`'s on the same force history.
//!
//! The force history is a committed, trimmed `forces.csv` of the "Cylinder Re=100" example
//! (8.5–10 s of a 12 s CUDA run, the cylinder's columns). This test writes the frequency Rust
//! finds into `cylinder-statistics.json` when `UPDATE_FIXTURES=1` and otherwise fails if the file
//! differs; the viewer's test (`apps/web/tests/statistics.test.ts`) must find the same St to 1e-12.

use std::path::PathBuf;

use cfd_verify::harness::oscillation_frequency;
use serde_json::json;

fn web(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../apps/web")
        .join(path)
}

/// The committed text of a file (line endings as written by Rust, whatever git did to them).
fn committed(path: &str) -> String {
    std::fs::read_to_string(web(path))
        .unwrap_or_default()
        .replace("\r\n", "\n")
}

#[test]
fn committed_references_are_up_to_date() {
    assert!(
        cfd_verify::web::references_json() == committed("src/generated/references.json"),
        "apps/web/src/generated/references.json is outdated; run `cargo run -p cfd-cli -- references apps/web/src/generated/references.json`"
    );
}

/// The example's reference scales: ρ = 1 kg/m³, Ū = 1 m/s, D = 0.1 m.
const DENSITY: f64 = 1.0;
const VELOCITY: f64 = 1.0;
const LENGTH: f64 = 0.1;

#[test]
fn the_viewers_strouhal_fixture_is_up_to_date() {
    let csv = committed("tests/fixtures/series/cylinder-forces.csv");
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let fy = header
        .iter()
        .position(|c| *c == "Cylinder.fy_N_m")
        .expect("the cylinder's lift column");
    // C_L = k F_y as the viewer computes it: the factor first, then the product.
    let k = 2.0 / (DENSITY * VELOCITY * VELOCITY * LENGTH);
    let (mut t, mut cl) = (Vec::new(), Vec::new());
    for line in lines {
        let v: Vec<f64> = line.split(',').map(|s| s.parse().unwrap()).collect();
        t.push(v[1]);
        cl.push(k * v[fy]);
    }
    let f = oscillation_frequency(&t, &cl).expect("several periods");
    let expected = serde_json::to_string_pretty(&json!({
        "reference": { "density": DENSITY, "velocity": VELOCITY, "length": LENGTH },
        "samples": t.len(),
        "frequency": f,
        "strouhal": f * LENGTH / VELOCITY,
    }))
    .unwrap()
        + "\n";
    let path = "tests/fixtures/series/cylinder-statistics.json";
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(web(path), &expected).unwrap();
    }
    assert!(
        committed(path) == expected,
        "{path} is out of date: run `UPDATE_FIXTURES=1 cargo test -p cfd-verify --test web_contract`"
    );
    // A real shedding frequency: St ≈ 0.297 at D = 20 cells.
    assert!((f * LENGTH / VELOCITY - 0.297).abs() < 0.005);
}

/// The quick heated cavity's reference scales as `meta.json` reports them: the cavity side
/// L = 0.1 m, the air's conductivity and the temperature difference that makes Ra = 1e5.
const CAVITY_LENGTH: f64 = 0.1;
const CAVITY_CONDUCTIVITY: f64 = 0.0257;
const CAVITY_DELTA_T: f64 = 0.9572000000000003;

/// The Nusselt numbers of a `heat.csv`: per wall element, the mean over all its rows (and the
/// extremes), as the viewer computes them for a window. `cfd_core::heat::nusselt` is the formula.
#[test]
fn the_viewers_nusselt_fixture_is_up_to_date() {
    let csv = committed("tests/fixtures/series/cavity-heat.csv");
    let mut lines = csv.lines();
    let header: Vec<&str> = lines.next().expect("header").split(',').collect();
    let rows: Vec<Vec<f64>> = lines
        .map(|l| l.split(',').map(|s| s.parse().unwrap()).collect())
        .collect();
    let elements: Vec<serde_json::Value> = header
        .iter()
        .enumerate()
        .filter_map(|(c, name)| name.strip_suffix(".q_W_m").map(|e| (c, e)))
        .map(|(c, element)| {
            assert_eq!(header[c + 1], format!("{element}.length_m"));
            let nu: Vec<f64> = rows
                .iter()
                .map(|r| {
                    cfd_core::heat::nusselt(
                        r[c],
                        r[c + 1],
                        CAVITY_LENGTH,
                        CAVITY_CONDUCTIVITY,
                        CAVITY_DELTA_T,
                    )
                })
                .collect();
            // A sequential sum from 0, like the viewer's loop.
            let mut sum = 0.0;
            for v in &nu {
                sum += v;
            }
            let mean = sum / nu.len() as f64;
            let min = nu.iter().copied().fold(f64::INFINITY, f64::min);
            let max = nu.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            json!({
                "element": element,
                "samples": nu.len(),
                "mean": mean,
                "min": min,
                "max": max,
                "spread": (max - min) / mean.abs(),
            })
        })
        .collect();
    let expected = serde_json::to_string_pretty(&json!({
        "reference": {
            "length": CAVITY_LENGTH,
            "conductivity": CAVITY_CONDUCTIVITY,
            "deltaT": CAVITY_DELTA_T,
        },
        "elements": elements,
    }))
    .unwrap()
        + "\n";
    let path = "tests/fixtures/series/cavity-nusselt.json";
    if std::env::var_os("UPDATE_FIXTURES").is_some() {
        std::fs::write(web(path), &expected).unwrap();
    }
    assert!(
        committed(path) == expected,
        "{path} is out of date: run `UPDATE_FIXTURES=1 cargo test -p cfd-verify --test web_contract`"
    );
    // A real cavity: the hot wall's Nu is de Vahl Davis' 4.509 within the suite's 2 %.
    let hot = &elements[0];
    assert!((hot["mean"].as_f64().unwrap() / 4.509 - 1.0).abs() < 0.02);
}
