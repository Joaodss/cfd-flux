//! Validation cases for live-fluids solvers (`docs/08-validation-guide.md`).
//!
//! The cases only see the [`cfd_core::Solver`] trait: the caller supplies a [`SolverFactory`]
//! that builds a solver of any backend for a [`Domain`], so the same suite validates the CPU
//! reference now and the GPU backends in Phase 2.
//!
//! - [`scenes`]: the validation geometries as regular scenes (also used by `cfd-cli bench`).
//! - [`references`]: published reference data, embedded from `validation/references/`.
//! - [`harness`]: running solvers to steady state, timing, error norms and convergence orders.
//! - [`cases`]: the cases themselves and the [`cases::all`] registry.
//! - [`report`]: the Markdown report with SVG plots and PNG snapshots.

pub mod cases;
pub mod harness;
mod plot;
pub mod references;
pub mod report;
pub mod scenes;

use cfd_core::{Domain, Solver};
use serde::Serialize;

pub use plot::{Plot, Series};

/// Builds a solver for a domain on some backend (in C#: a `Func<Domain, ISolver>`).
pub type SolverFactory<'a> = &'a (dyn Fn(Domain) -> anyhow::Result<Box<dyn Solver>> + Sync);

/// How thorough a verification run is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    /// Low resolutions, a subset of cases: fast enough for CI.
    Quick,
    /// Every case at the resolutions needed for the published tolerances.
    Full,
}

/// One measured quantity, optionally compared with a reference under a tolerance.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Metric {
    pub name: String,
    pub value: f64,
    /// Reference as text (a value, an interval or a formula).
    pub reference: String,
    /// Error used for the check (relative unless the name says otherwise).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<f64>,
    /// The check passes when `error <= limit`; `None` = reported only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<f64>,
}

impl Metric {
    /// A quantity reported for information only.
    pub fn info(name: impl Into<String>, value: f64, reference: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value,
            reference: reference.into(),
            error: None,
            limit: None,
        }
    }

    /// A checked quantity: passes when `error <= limit`.
    pub fn check(
        name: impl Into<String>,
        value: f64,
        reference: impl Into<String>,
        error: f64,
        limit: f64,
    ) -> Self {
        Self {
            name: name.into(),
            value,
            reference: reference.into(),
            error: Some(error),
            limit: Some(limit),
        }
    }

    /// Relative error against a reference value.
    pub fn relative(name: impl Into<String>, value: f64, reference: f64, limit: f64) -> Self {
        let error = ((value - reference) / reference).abs();
        Self::check(name, value, fmt_num(reference), error, limit)
    }

    /// `None` when reported only.
    pub fn passed(&self) -> Option<bool> {
        match (self.error, self.limit) {
            (Some(e), Some(l)) => Some(e <= l && e.is_finite()),
            _ => None,
        }
    }
}

/// A table for the report (convergence studies, profiles).
#[derive(Debug, Clone, Serialize)]
pub struct Table {
    pub title: String,
    pub headers: Vec<String>,
    pub rows: Vec<Vec<String>>,
}

/// Cost of one solver run inside a case.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunStat {
    pub label: String,
    pub fluid_cells: usize,
    pub steps: u64,
    pub seconds: f64,
    /// Whether the run met its steady-state or periodicity criterion (when it has one).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub converged: Option<bool>,
}

impl RunStat {
    pub fn mlups(&self) -> f64 {
        if self.seconds > 0.0 {
            self.fluid_cells as f64 * self.steps as f64 / self.seconds / 1e6
        } else {
            0.0
        }
    }
}

/// A snapshot image of a field, written as PNG next to the report.
#[derive(Debug, Clone)]
pub struct Snapshot {
    pub file: String,
    pub caption: String,
    pub image: cfd_io::image::Image,
}

/// Everything a case produced.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseResult {
    pub id: String,
    pub title: String,
    pub reference: String,
    /// What was set up and measured (Markdown).
    pub description: String,
    pub metrics: Vec<Metric>,
    pub tables: Vec<Table>,
    #[serde(skip)]
    pub plots: Vec<Plot>,
    #[serde(skip)]
    pub snapshots: Vec<Snapshot>,
    pub runs: Vec<RunStat>,
    pub seconds: f64,
    /// Set when the case could not complete (divergence, setup error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<String>,
}

impl CaseResult {
    pub fn passed(&self) -> bool {
        self.failure.is_none() && self.metrics.iter().all(|m| m.passed() != Some(false))
    }
}

/// Compact number formatting for tables: 4 significant digits.
pub fn fmt_num(v: f64) -> String {
    if v == 0.0 || !v.is_finite() {
        return format!("{v}");
    }
    let mag = v.abs().log10().floor() as i32;
    if (-3..5).contains(&mag) {
        let decimals = (3 - mag).max(0) as usize;
        format!("{v:.decimals$}")
    } else {
        format!("{v:.3e}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metrics_pass_or_fail_on_their_limit() {
        assert_eq!(Metric::relative("a", 1.01, 1.0, 0.02).passed(), Some(true));
        assert_eq!(Metric::relative("a", 1.03, 1.0, 0.02).passed(), Some(false));
        assert_eq!(
            Metric::relative("a", f64::NAN, 1.0, 0.02).passed(),
            Some(false)
        );
        assert_eq!(Metric::info("a", 1.0, "").passed(), None);
        assert_eq!(fmt_num(5.5795), "5.580");
        assert_eq!(fmt_num(0.0106), "0.01060");
        assert_eq!(fmt_num(219.36), "219.4");
        assert_eq!(fmt_num(1.5e-6), "1.500e-6");
    }
}
