//! The validation cases, in the order of `docs/08-validation-guide.md` §7: each one isolates
//! one more part of the solver.
//!
//! A [`Case`] is data plus a closure (in C# terms: a record holding a `Func<Ctx, Outcome>`), so
//! parametrised cases such as the cavity at three Reynolds numbers are just three entries.

mod analytical;
mod cavity;
mod cylinder;
mod heated_cavity;

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::time::Instant;

use anyhow::Result;

use crate::harness::Ctx;
use crate::{CaseResult, Level, Metric, Plot, RunStat, Snapshot, Table};

/// What a case returns; [`run`] adds the identification and timing.
#[derive(Default)]
pub struct Outcome {
    pub description: String,
    pub metrics: Vec<Metric>,
    pub tables: Vec<Table>,
    pub plots: Vec<Plot>,
    pub snapshots: Vec<Snapshot>,
    pub runs: Vec<RunStat>,
}

type CaseFn = dyn Fn(&Ctx) -> Result<Outcome> + Send + Sync;

pub struct Case {
    pub id: String,
    pub title: String,
    pub reference: String,
    /// Part of `--quick`.
    pub quick: bool,
    run: Box<CaseFn>,
}

impl Case {
    fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        reference: impl Into<String>,
        quick: bool,
        run: impl Fn(&Ctx) -> Result<Outcome> + Send + Sync + 'static,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            reference: reference.into(),
            quick,
            run: Box::new(run),
        }
    }

    pub fn in_level(&self, level: Level) -> bool {
        level == Level::Full || self.quick
    }
}

/// Every case, in the recommended order.
pub fn all() -> Vec<Case> {
    let mut cases = vec![
        analytical::taylor_green(),
        analytical::poiseuille(),
        analytical::couette(),
    ];
    cases.extend(cavity::cases());
    cases.extend(cylinder::cases());
    cases.extend(heated_cavity::cases());
    cases
}

/// Runs one case, turning errors and panics into a failed result.
pub fn run(case: &Case, ctx: &Ctx) -> CaseResult {
    let t = Instant::now();
    ctx.log(&format!("{} — {}", case.id, case.title));
    let outcome = catch_unwind(AssertUnwindSafe(|| (case.run)(ctx)));
    let (outcome, failure) = match outcome {
        Ok(Ok(o)) => (o, None),
        Ok(Err(e)) => (Outcome::default(), Some(format!("{e:#}"))),
        Err(p) => {
            let msg = p
                .downcast_ref::<String>()
                .cloned()
                .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
                .unwrap_or_else(|| "panic".into());
            (Outcome::default(), Some(format!("panic: {msg}")))
        }
    };
    let result = CaseResult {
        id: case.id.clone(),
        title: case.title.clone(),
        reference: case.reference.clone(),
        description: outcome.description,
        metrics: outcome.metrics,
        tables: outcome.tables,
        plots: outcome.plots,
        snapshots: outcome.snapshots,
        runs: outcome.runs,
        seconds: t.elapsed().as_secs_f64(),
        failure,
    };
    for m in &result.metrics {
        let status = match m.passed() {
            Some(true) => "pass",
            Some(false) => "FAIL",
            None => "info",
        };
        ctx.log(&format!(
            "    [{status}] {} = {} (ref. {}){}",
            m.name,
            crate::fmt_num(m.value),
            m.reference,
            m.error
                .map(|e| format!(", error {}", crate::fmt_num(e)))
                .unwrap_or_default()
        ));
    }
    if let Some(f) = &result.failure {
        ctx.log(&format!("    FAILED: {f}"));
    }
    result
}

/// Pass when `value ∈ [lo, hi]` (error = distance outside the interval).
pub(crate) fn within(name: impl Into<String>, value: f64, lo: f64, hi: f64) -> Metric {
    let error = (lo - value).max(value - hi).max(0.0);
    let error = if value.is_finite() { error } else { f64::NAN };
    Metric::check(
        name,
        value,
        format!("{} – {}", crate::fmt_num(lo), crate::fmt_num(hi)),
        error,
        0.0,
    )
}
