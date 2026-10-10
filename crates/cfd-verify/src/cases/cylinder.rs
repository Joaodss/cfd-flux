//! Flow around a cylinder in a channel: Schäfer & Turek (1996) benchmarks 2D-1 (steady, Re 20)
//! and 2D-2 (periodic, Re 100) (type C: integral quantities).
//!
//! The cylinder is a staircase of cells (simple bounce-back), so the accuracy is limited by the
//! geometric representation; the cases report the published reference intervals next to the
//! pass/fail tolerance agreed for this geometry (2% at the finest level, 5% in `--quick`).

use anyhow::Result;
use cfd_core::scene::OutputField;
use cfd_core::solver::SampleRequest;
use cfd_core::DomainOptions;
use cfd_io::image::Colormap;

use super::{within, Case, Outcome};
use crate::harness::{snapshot, Ctx, Run};
use crate::plot::{Plot, Series};
use crate::references::{self, SCHAFER_TUREK_CITATION};
use crate::scenes::{self, ids, ST_DIAMETER};
use crate::{fmt_num, Level, Metric, Table};

/// Lattice velocity of the inlet peak (Ma ≈ 0.14).
const U_PEAK_LB: f64 = 0.08;

pub fn cases() -> Vec<Case> {
    vec![
        Case::new(
            "cylinder-re20",
            "Cylinder in a channel, Re = 20 (Schäfer-Turek 2D-1)",
            SCHAFER_TUREK_CITATION,
            true,
            steady_re20,
        ),
        Case::new(
            "cylinder-re100",
            "Cylinder in a channel, Re = 100 (Schäfer-Turek 2D-2)",
            SCHAFER_TUREK_CITATION,
            false,
            periodic_re100,
        ),
    ]
}

fn sizes(level: Level) -> &'static [u32] {
    match level {
        Level::Quick => &[20],
        Level::Full => &[20, 40],
    }
}

/// Resolutions of the periodic case: D = 80 in `--full` shows how slowly C_D,max converges.
fn sizes_periodic(level: Level) -> &'static [u32] {
    match level {
        Level::Quick => &[20],
        Level::Full => &[20, 40, 80],
    }
}

fn limit(level: Level) -> f64 {
    match level {
        Level::Quick => 0.05,
        Level::Full => 0.02,
    }
}

/// Drag and lift coefficients `2F/(ρ Ū² D)` from the force on the cylinder (ρ = 1).
fn coefficients(run: &mut Run, u_mean: f64) -> [f64; 2] {
    let units = run.domain().units;
    let f = run
        .solver
        .forces()
        .iter()
        .find(|f| f.element == ids::CYLINDER)
        .map_or([0.0; 2], |f| f.physical(&units));
    f.map(|c| 2.0 * c / (u_mean * u_mean * ST_DIAMETER))
}

fn options() -> DomainOptions {
    DomainOptions {
        lattice_velocity: U_PEAK_LB,
        characteristic_velocity: None,
    }
}

fn band(case: &str, q: &str) -> Result<[f64; 2]> {
    references::schafer_turek(case, q)
        .ok_or_else(|| anyhow::anyhow!("no Schäfer-Turek reference for {case} {q}"))
}

fn band_text(b: [f64; 2]) -> String {
    format!("{} – {}", fmt_num(b[0]), fmt_num(b[1]))
}

fn vorticity_snapshot(run: &mut Run, file: &str, caption: String) -> Result<crate::Snapshot> {
    let fs = run.solver.sample(&SampleRequest {
        fields: vec![OutputField::Vorticity],
    });
    snapshot(
        file,
        caption,
        run.domain(),
        &fs.fields[0].values,
        Colormap::Coolwarm,
        true,
    )
}

fn steady_re20(ctx: &Ctx) -> Result<Outcome> {
    let (u_max, u_mean) = (0.3, 0.2);
    let [cd_lo, cd_hi] = band("2D-1", "c_d")?;
    let cl_band = band("2D-1", "c_l")?;
    let dp_band = band("2D-1", "dp")?;
    let cd_ref = 0.5 * (cd_lo + cd_hi);
    let mut out = Outcome {
        description: format!(
            "Channel 2.2 m × 0.41 m, cylinder D = 0.1 m at (0.2, 0.2) m drawn as cells, parabolic \
             inlet with U_max = {u_max} m/s (Ū = {u_mean} m/s, Re = ŪD/ν = 20), zero-pressure outlet; \
             inlet peak lattice velocity {U_PEAK_LB}. Run until the relative change of u over 0.25 s \
             is below 1e-6. C_D, C_L from momentum exchange on the cylinder; Δp between the fluid \
             cells next to the front and back points. Pass/fail: C_D within {:.0}% of the centre of \
             the reference interval at the finest resolution.",
            100.0 * limit(ctx.level)
        ),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut last = [0.0; 3];
    for &d_cells in sizes(ctx.level) {
        let d = ctx.domain(&scenes::schafer_turek(d_cells, u_max), options())?;
        let dt = d.units.dt;
        let mut run = ctx.run(format!("D = {d_cells} cells"), d)?;
        // Steady when C_D changes by less than 5e-5 (relative) over 2 s. C_D approaches its
        // limit slowly (time constant ≈ 8 s), and C_L carries ~3e-4 of acoustic noise.
        let every = (2.0 / dt).round() as u64;
        let mut prev = [f64::INFINITY; 2];
        let mut steady = false;
        while run.stat.steps as f64 * dt < 90.0 {
            run.step(every)?;
            let c = coefficients(&mut run, u_mean);
            if ((c[0] - prev[0]) / c[0]).abs() < 5e-5 {
                steady = true;
                break;
            }
            prev = c;
        }
        run.stat.converged = Some(steady);
        let [cd, cl] = coefficients(&mut run, u_mean);
        let probes = run.solver.probes();
        let dp = probes[0].pressure - probes[1].pressure;
        rows.push(vec![
            d_cells.to_string(),
            format!("{cd:.4}"),
            format!("{:+.2}%", 100.0 * (cd / cd_ref - 1.0)),
            format!("{cl:.4}"),
            format!("{dp:.4}"),
        ]);
        last = [cd, cl, dp];
        if d_cells == *sizes(ctx.level).last().expect("sizes") {
            out.snapshots.push(vorticity_snapshot(
                &mut run,
                "cylinder-re20-vorticity.png",
                format!("Vorticity at Re = 20, D = {d_cells} cells (steady)"),
            )?);
        }
        out.runs.push(run.stat);
    }
    let finest = sizes(ctx.level).last().expect("sizes");
    out.metrics.push(Metric::check(
        format!("C_D at D = {finest} cells"),
        last[0],
        format!(
            "{} (interval {})",
            fmt_num(cd_ref),
            band_text([cd_lo, cd_hi])
        ),
        (last[0] / cd_ref - 1.0).abs(),
        limit(ctx.level),
    ));
    out.metrics.push(Metric::info(
        format!("C_L at D = {finest} cells"),
        last[1],
        band_text(cl_band),
    ));
    out.metrics.push(Metric::info(
        format!("Δp (Pa) at D = {finest} cells"),
        last[2],
        band_text(dp_band),
    ));
    out.tables.push(Table {
        title: "Results by resolution".into(),
        headers: vec![
            "D (cells)".into(),
            "C_D".into(),
            "C_D error".into(),
            "C_L".into(),
            "Δp (Pa)".into(),
        ],
        rows,
    });
    Ok(out)
}

/// Upward zero crossings of `y − mean(y)`, linearly interpolated in time.
fn upward_crossings(t: &[f64], y: &[f64]) -> Vec<f64> {
    let mean = y.iter().sum::<f64>() / y.len() as f64;
    (1..y.len())
        .filter(|&i| y[i - 1] - mean < 0.0 && y[i] - mean >= 0.0)
        .map(|i| {
            let (a, b) = (y[i - 1] - mean, y[i] - mean);
            t[i - 1] + (t[i] - t[i - 1]) * (-a / (b - a))
        })
        .collect()
}

/// Force history of a window: `(t, C_D, C_L)`.
type History = Vec<(f64, f64, f64)>;

fn record(run: &mut Run, seconds: f64, every: u64, u_mean: f64) -> Result<History> {
    let dt = run.domain().units.dt;
    let samples = (seconds / (every as f64 * dt)).round() as usize;
    let mut h = Vec::with_capacity(samples);
    for _ in 0..samples {
        run.step(every)?;
        let [cd, cl] = coefficients(run, u_mean);
        h.push((run.stat.steps as f64 * dt, cd, cl));
    }
    Ok(h)
}

fn amplitude(h: &History) -> f64 {
    let (lo, hi) = h
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), s| {
            (lo.min(s.2), hi.max(s.2))
        });
    0.5 * (hi - lo)
}

fn periodic_re100(ctx: &Ctx) -> Result<Outcome> {
    let (u_max, u_mean) = (1.5, 1.0);
    let st_band = band("2D-2", "st")?;
    let cd_band = band("2D-2", "c_d_max")?;
    let cl_band = band("2D-2", "c_l_max")?;
    let (st_ref, cd_ref) = (
        0.5 * (st_band[0] + st_band[1]),
        0.5 * (cd_band[0] + cd_band[1]),
    );
    let mut out = Outcome {
        description: format!(
            "Same channel and cylinder as 2D-1 with U_max = {u_max} m/s (Ū = {u_mean} m/s, Re = 100); \
             inlet peak lattice velocity {U_PEAK_LB}. The run continues in 1 s windows until the C_L \
             amplitude changes by less than 1% between windows, then records 3 s (≈ 9 shedding \
             periods). St = fD/Ū from the upward zero crossings of C_L; C_D,max and C_L,max over the \
             record. Pass/fail: St and C_D,max within {:.0}% of the centre of the reference \
             intervals at the finest resolution.",
            100.0 * limit(ctx.level)
        ),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut last = (0.0, 0.0, 0.0);
    let mut last_history = History::new();
    for &d_cells in sizes_periodic(ctx.level) {
        let d = ctx.domain(&scenes::schafer_turek(d_cells, u_max), options())?;
        let dt = d.units.dt;
        let mut run = ctx.run(format!("D = {d_cells} cells"), d)?;
        // About 2000 force samples per second.
        let every = ((1.0 / dt) / 2000.0).round().max(1.0) as u64;
        let mut prev_amp = 0.0;
        let mut periodic = false;
        while run.stat.steps as f64 * dt < 40.0 {
            let h = record(&mut run, 1.0, every, u_mean)?;
            let amp = amplitude(&h);
            if amp > 0.1 && ((amp - prev_amp) / amp).abs() < 0.01 {
                periodic = true;
                break;
            }
            prev_amp = amp;
        }
        run.stat.converged = Some(periodic);
        let h = record(&mut run, 3.0, every, u_mean)?;
        let t: Vec<f64> = h.iter().map(|s| s.0).collect();
        let cl: Vec<f64> = h.iter().map(|s| s.2).collect();
        let crossings = upward_crossings(&t, &cl);
        let f = if crossings.len() >= 2 {
            (crossings.len() - 1) as f64 / (crossings[crossings.len() - 1] - crossings[0])
        } else {
            f64::NAN
        };
        let st = f * ST_DIAMETER / u_mean;
        let cd_max = h.iter().map(|s| s.1).fold(f64::NEG_INFINITY, f64::max);
        let cl_max = h.iter().map(|s| s.2).fold(f64::NEG_INFINITY, f64::max);
        rows.push(vec![
            d_cells.to_string(),
            format!("{st:.4}"),
            format!("{:+.2}%", 100.0 * (st / st_ref - 1.0)),
            format!("{cd_max:.4}"),
            format!("{:+.2}%", 100.0 * (cd_max / cd_ref - 1.0)),
            format!("{cl_max:.4}"),
            format!("{:.1}", t[0]),
        ]);
        last = (st, cd_max, cl_max);
        if d_cells == *sizes_periodic(ctx.level).last().expect("sizes") {
            let t_now = run.stat.steps as f64 * dt;
            out.snapshots.push(vorticity_snapshot(
                &mut run,
                "cylinder-re100-vorticity.png",
                format!("Vorticity at Re = 100, D = {d_cells} cells, t = {t_now:.2} s: von Kármán vortex street"),
            )?);
            last_history = h;
        }
        out.runs.push(run.stat);
    }
    let finest = sizes_periodic(ctx.level).last().expect("sizes");
    out.metrics.push(Metric::check(
        format!("St at D = {finest} cells"),
        last.0,
        format!("{} (interval {})", fmt_num(st_ref), band_text(st_band)),
        (last.0 / st_ref - 1.0).abs(),
        limit(ctx.level),
    ));
    out.metrics.push(Metric::check(
        format!("C_D,max at D = {finest} cells"),
        last.1,
        format!("{} (interval {})", fmt_num(cd_ref), band_text(cd_band)),
        (last.1 / cd_ref - 1.0).abs(),
        limit(ctx.level),
    ));
    out.metrics.push(Metric::info(
        format!("C_L,max at D = {finest} cells"),
        last.2,
        band_text(cl_band),
    ));
    out.tables.push(Table {
        title: "Results by resolution".into(),
        headers: vec![
            "D (cells)".into(),
            "St".into(),
            "St error".into(),
            "C_D,max".into(),
            "C_D,max error".into(),
            "C_L,max".into(),
            "record starts at t (s)".into(),
        ],
        rows,
    });
    // The last 1 s of the record.
    let t_end = last_history.last().map_or(0.0, |s| s.0);
    let tail: Vec<_> = last_history.iter().filter(|s| s.0 >= t_end - 1.0).collect();
    out.plots.push(Plot {
        file: "cylinder-re100-forces.svg".into(),
        title: format!("Cylinder Re = 100, D = {finest} cells: force coefficients"),
        x_label: "t (s)".into(),
        y_label: "coefficient".into(),
        log_x: false,
        log_y: false,
        series: vec![
            Series::line("C_D", tail.iter().map(|s| (s.0, s.1)).collect()),
            Series::line("C_L", tail.iter().map(|s| (s.0, s.2)).collect()),
        ],
    });
    // Not a pass/fail quantity, but a sanity check of the periodic state.
    if !out.runs.iter().all(|r| r.converged == Some(true)) {
        out.metrics
            .push(within("periodic state reached (1 = yes)", 0.0, 1.0, 1.0));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zero_crossings_give_the_frequency() {
        let f = 3.0;
        let t: Vec<f64> = (0..3000).map(|i| i as f64 * 1e-3).collect();
        let y: Vec<f64> = t
            .iter()
            .map(|t| 0.2 + (2.0 * std::f64::consts::PI * f * t + 0.3).sin())
            .collect();
        let c = upward_crossings(&t, &y);
        let measured = (c.len() - 1) as f64 / (c[c.len() - 1] - c[0]);
        assert!((measured / f - 1.0).abs() < 1e-3, "{measured}");
    }
}
