//! Lid-driven cavity at Re 100, 400, 1000 against Ghia, Ghia & Shin (1982) (type B).

use anyhow::{anyhow, Result};
use cfd_core::DomainOptions;
use cfd_io::image::Colormap;

use super::{Case, Outcome};
use crate::harness::{interp, max_abs, rel_l2, snapshot, speed, Ctx};
use crate::plot::{Plot, Series};
use crate::references::{self, Profile, GHIA_CITATION, GHIA_V_EXCLUDED};
use crate::scenes;
use crate::{Level, Metric, Table};

/// Lattice velocity of the lid (Ma ≈ 0.14).
const U_LID_LB: f64 = 0.08;
/// Steady when the relative change of u over half a lid transit is below this.
const STEADY_TOL: f64 = 1e-5;

pub fn cases() -> Vec<Case> {
    [100u32, 400, 1000]
        .into_iter()
        .map(|re| {
            Case::new(
                format!("cavity-re{re}"),
                format!("Lid-driven cavity, Re = {re}"),
                GHIA_CITATION,
                re != 1000,
                move |ctx: &Ctx| run(ctx, re),
            )
        })
        .collect()
}

/// Simulated profile along a centreline, with the wall values at both ends.
/// `cells[i]` are the values at cell centres `(i + ½)/n`.
fn with_walls(cells: &[f64], at_0: f64, at_1: f64) -> (Vec<f64>, Vec<f64>) {
    let n = cells.len() as f64;
    let mut c = vec![0.0];
    let mut v = vec![at_0];
    for (i, &x) in cells.iter().enumerate() {
        c.push((i as f64 + 0.5) / n);
        v.push(x);
    }
    c.push(1.0);
    v.push(at_1);
    (c, v)
}

/// Errors of a simulated profile at the reference points, excluding the walls and the
/// `excluded` coordinates. Returns `(max |s − r|, relative L2)`.
fn profile_errors(coord: &[f64], value: &[f64], r: &Profile, excluded: &[f64]) -> (f64, f64) {
    let (mut s, mut rv) = (Vec::new(), Vec::new());
    for (&c, &v) in r.coord.iter().zip(&r.value) {
        if c <= 0.0 || c >= 1.0 || excluded.iter().any(|&e| (e - c).abs() < 1e-6) {
            continue;
        }
        s.push(interp(coord, value, c));
        rv.push(v);
    }
    (max_abs(&s, &rv), rel_l2(&s, &rv))
}

fn run(ctx: &Ctx, re: u32) -> Result<Outcome> {
    let sizes: &[u32] = match ctx.level {
        Level::Quick => &[64],
        Level::Full => &[64, 128, 256],
    };
    let ghia_u = references::ghia_u(re).ok_or_else(|| anyhow!("no Ghia u data for Re {re}"))?;
    let ghia_v = references::ghia_v(re).ok_or_else(|| anyhow!("no Ghia v data for Re {re}"))?;
    let excluded_v: Vec<f64> = GHIA_V_EXCLUDED
        .iter()
        .filter(|e| e.0 == re)
        .map(|e| e.1)
        .collect();

    let mut out = Outcome {
        description: format!(
            "Square cavity of side 1 m, lid at 1 m/s, ν = {:.4} m²/s; N² fluid cells, lid lattice \
             velocity {U_LID_LB} (Ma ≈ {:.2}). Run until the relative change of u over half a lid \
             transit is below {STEADY_TOL:e}. The centreline profiles are averaged over the two \
             middle columns/rows and linearly interpolated (walls half-way) at Ghia's points; the \
             15 interior points of each profile are compared{}. Errors are relative to the lid \
             velocity.",
            1.0 / re as f64,
            U_LID_LB * 3f64.sqrt(),
            if excluded_v.is_empty() {
                String::new()
            } else {
                " (the v point at x = 0.9063 is excluded: a known misprint in Ghia's Table II)"
                    .into()
            }
        ),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut u_series = Vec::new();
    let mut v_series = Vec::new();
    let mut last_errors = (0.0, 0.0);
    for &n in sizes {
        let opts = DomainOptions {
            lattice_velocity: U_LID_LB,
            characteristic_velocity: None,
        };
        let d = ctx.domain(&scenes::lid_driven_cavity(n, re as f64), opts)?;
        let mut run = ctx.run(format!("N = {n}"), d)?;
        let transit = (n as f64 / U_LID_LB) as u64;
        run.to_steady(transit / 2, STEADY_TOL, 400 * transit, false)?;
        let f = run.fields();
        let (n_us, w) = (n as usize, n as usize);
        let m = n_us / 2;
        // u along x = 0.5 (columns m−1, m) and v along y = 0.5 (rows m−1, m), over U_lid.
        let u_col: Vec<f64> = (0..n_us)
            .map(|y| 0.5 * (f.ux[y * w + m - 1] + f.ux[y * w + m]) as f64 / U_LID_LB)
            .collect();
        let v_row: Vec<f64> = (0..n_us)
            .map(|x| 0.5 * (f.uy[(m - 1) * w + x] + f.uy[m * w + x]) as f64 / U_LID_LB)
            .collect();
        let (yc, uc) = with_walls(&u_col, 0.0, 1.0);
        let (xc, vc) = with_walls(&v_row, 0.0, 0.0);
        let (u_max, u_l2) = profile_errors(&yc, &uc, &ghia_u, &[]);
        let (v_max, v_l2) = profile_errors(&xc, &vc, &ghia_v, &excluded_v);
        rows.push(vec![
            n.to_string(),
            format!("{u_max:.4}"),
            format!("{v_max:.4}"),
            format!("{u_l2:.3e}"),
            format!("{v_l2:.3e}"),
        ]);
        last_errors = (u_max, v_max);
        u_series.push(Series::line(
            format!("N = {n}"),
            uc.iter().copied().zip(yc.iter().copied()).collect(),
        ));
        v_series.push(Series::line(
            format!("N = {n}"),
            xc.iter().copied().zip(vc.iter().copied()).collect(),
        ));
        if n == *sizes.last().expect("sizes") {
            let sp: Vec<f32> = speed(&f).iter().map(|v| v / U_LID_LB as f32).collect();
            out.snapshots.push(snapshot(
                &format!("cavity-re{re}-speed.png"),
                format!("Velocity magnitude |u|/U at Re = {re}, N = {n}"),
                run.domain(),
                &sp,
                Colormap::Viridis,
                false,
            )?);
        }
        out.runs.push(run.stat);
    }
    let finest = sizes[sizes.len() - 1];
    out.metrics.push(Metric::check(
        format!("max |u − u_Ghia| / U at N = {finest}"),
        last_errors.0,
        "0 (limit 2%)",
        last_errors.0,
        0.02,
    ));
    out.metrics.push(Metric::check(
        format!("max |v − v_Ghia| / U at N = {finest}"),
        last_errors.1,
        "0 (limit 2%)",
        last_errors.1,
        0.02,
    ));
    out.tables.push(Table {
        title: "Deviation from Ghia et al. (1982) by resolution".into(),
        headers: vec![
            "N".into(),
            "max |Δu|/U".into(),
            "max |Δv|/U".into(),
            "u relative L2".into(),
            "v relative L2".into(),
        ],
        rows,
    });
    u_series.push(Series::markers(
        "Ghia et al. (1982)",
        ghia_u
            .value
            .iter()
            .copied()
            .zip(ghia_u.coord.iter().copied())
            .collect(),
    ));
    v_series.push(Series::markers(
        "Ghia et al. (1982)",
        ghia_v
            .coord
            .iter()
            .copied()
            .zip(ghia_v.value.iter().copied())
            .collect(),
    ));
    out.plots.push(Plot {
        file: format!("cavity-re{re}-u.svg"),
        title: format!("Cavity Re = {re}: u on the vertical centreline"),
        x_label: "u / U".into(),
        y_label: "y / L".into(),
        log_x: false,
        log_y: false,
        series: u_series,
    });
    out.plots.push(Plot {
        file: format!("cavity-re{re}-v.svg"),
        title: format!("Cavity Re = {re}: v on the horizontal centreline"),
        x_label: "x / L".into(),
        y_label: "v / U".into(),
        log_x: false,
        log_y: false,
        series: v_series,
    });
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_errors_skip_walls_and_excluded_points() {
        let r = Profile {
            coord: vec![0.0, 0.25, 0.5, 1.0],
            value: vec![0.0, 1.0, 2.0, 0.0],
        };
        // Simulated profile = reference + 0.1 at 0.25 and exact at 0.5.
        let (c, v) = (vec![0.0, 0.25, 0.5, 1.0], vec![5.0, 1.1, 2.0, 5.0]);
        let (m, _) = profile_errors(&c, &v, &r, &[]);
        assert!((m - 0.1).abs() < 1e-12);
        let (m, _) = profile_errors(&c, &v, &r, &[0.25]);
        assert_eq!(m, 0.0);
        let (c, v) = with_walls(&[1.0, 3.0], 0.0, 1.0);
        assert_eq!(c, [0.0, 0.25, 0.75, 1.0]);
        assert_eq!(v, [0.0, 1.0, 3.0, 1.0]);
    }
}
