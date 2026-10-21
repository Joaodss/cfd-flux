//! The reference data of the web viewer's statistics (`apps/web/src/generated/references.json`,
//! written by `cfd-cli references`): each benchmark's reference intervals and the relative
//! tolerance the validation suite checks them with at each resolution, so the viewer marks a run
//! the way `cfd-cli verify` would. A test keeps the committed file current.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::cases::cylinder_tolerances;
use crate::references::{schafer_turek, SCHAFER_TUREK_CITATION};

/// One benchmark. Quantities are named like the viewer's statistics (`strouhal`, `cdMax`, …).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Benchmark {
    title: &'static str,
    citation: &'static str,
    /// Reference intervals `[lower, upper]`; the suite compares with their centre.
    quantities: BTreeMap<&'static str, [f64; 2]>,
    /// The quantities the suite passes or fails on (the others are shown for information).
    checked: Vec<&'static str>,
    /// Relative tolerance by resolution (cells per reference length), coarsest first: a run is
    /// checked with the last entry at or below its resolution.
    tolerances: Vec<Tolerance>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tolerance {
    cells: u32,
    relative: f64,
}

fn interval(case: &str, quantity: &str) -> [f64; 2] {
    schafer_turek(case, quantity)
        .unwrap_or_else(|| panic!("Schäfer-Turek {case} {quantity} is in the reference table"))
}

fn tolerances(periodic: bool) -> Vec<Tolerance> {
    cylinder_tolerances(periodic)
        .into_iter()
        .map(|(cells, relative)| Tolerance { cells, relative })
        .collect()
}

/// The JSON of `apps/web/src/generated/references.json` (pretty, with a final newline).
pub fn references_json() -> String {
    let benchmarks = BTreeMap::from([
        (
            "schaferTurek2D1",
            Benchmark {
                title: "Schäfer–Turek 2D-1 (cylinder, Re = 20, steady)",
                citation: SCHAFER_TUREK_CITATION,
                quantities: BTreeMap::from([
                    ("cdMean", interval("2D-1", "c_d")),
                    ("clMean", interval("2D-1", "c_l")),
                ]),
                checked: vec!["cdMean"],
                tolerances: tolerances(false),
            },
        ),
        (
            "schaferTurek2D2",
            Benchmark {
                title: "Schäfer–Turek 2D-2 (cylinder, Re = 100, periodic)",
                citation: SCHAFER_TUREK_CITATION,
                quantities: BTreeMap::from([
                    ("strouhal", interval("2D-2", "st")),
                    ("cdMax", interval("2D-2", "c_d_max")),
                    ("clMax", interval("2D-2", "c_l_max")),
                ]),
                checked: vec!["strouhal", "cdMax"],
                tolerances: tolerances(true),
            },
        ),
    ]);
    serde_json::to_string_pretty(&benchmarks).expect("serialisable") + "\n"
}
