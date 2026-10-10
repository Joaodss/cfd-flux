//! Differentially heated square cavity at Ra 1e3–1e6, Pr = 0.71, against de Vahl Davis (1983)
//! (type B/C).

use anyhow::{anyhow, Result};
use cfd_core::DomainOptions;
use cfd_io::image::Colormap;

use super::{Case, Outcome};
use crate::harness::{refined_max, snapshot, Ctx};
use crate::references::{self, DE_VAHL_DAVIS_CITATION};
use crate::scenes::{self, T_COLD, T_HOT};
use crate::{fmt_num, Level, Metric, Table};

const PRANDTL: f64 = 0.71;
/// Lattice value of the buoyancy velocity √(gβΔT L).
const U_B_LB: f64 = 0.08;
/// Steady when u and θ change by less than this (relative) over half a buoyancy time L/u_b.
const STEADY_TOL: f64 = 1e-6;

pub fn cases() -> Vec<Case> {
    [(1e3, "1e3"), (1e4, "1e4"), (1e5, "1e5"), (1e6, "1e6")]
        .into_iter()
        .map(|(ra, name)| {
            Case::new(
                format!("heated-cavity-ra{name}"),
                format!("Differentially heated cavity, Ra = {name}"),
                DE_VAHL_DAVIS_CITATION,
                ra < 1e5,
                move |ctx: &Ctx| run(ctx, ra),
            )
        })
        .collect()
}

/// Quantities measured on a steady solution (de Vahl Davis' definitions; velocities in α/L).
#[derive(Debug, Clone, Copy)]
struct Measured {
    nu_hot: f64,
    nu_mean: f64,
    u_max: f64,
    y_u_max: f64,
    v_max: f64,
    x_v_max: f64,
}

fn run(ctx: &Ctx, ra: f64) -> Result<Outcome> {
    let r = references::de_vahl_davis(ra).ok_or_else(|| anyhow!("no reference for Ra {ra}"))?;
    let sizes: &[u32] = match (ctx.level, ra >= 1e6) {
        (Level::Quick, _) => &[64],
        (Level::Full, false) => &[64, 128],
        (Level::Full, true) => &[128, 256],
    };
    let mut out = Outcome {
        description: format!(
            "Square cavity of side L = 1 m, N² fluid cells; hot left wall, cold right wall (ΔT = 1 K), \
             adiabatic top and bottom; Pr = {PRANDTL}, Boussinesq buoyancy. The buoyancy velocity \
             √(gβΔTL) maps to lattice velocity {U_B_LB}. Run until u and θ change by less than \
             {STEADY_TOL:e} (relative) over half a buoyancy time. Nu on the hot wall from a \
             second-order one-sided temperature gradient at the half-way wall; Nū as the cavity \
             average 1 + L⟨uθ⟩/(αΔT); velocity maxima on the mid-planes refined with a parabola, \
             in units of α/L. Pass/fail: both Nusselt numbers within 2% at the finest resolution."
        ),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut last: Option<Measured> = None;
    for &n in sizes {
        let opts = DomainOptions {
            lattice_velocity: U_B_LB,
            characteristic_velocity: None,
        };
        let d = ctx.domain(&scenes::heated_cavity(n, ra, PRANDTL), opts)?;
        let alpha = d.physics.alpha.expect("thermal domain");
        let theta_hot = d.units.temperature_to_lattice(T_HOT);
        let theta_span = theta_hot - d.units.temperature_to_lattice(T_COLD);
        let mut run = ctx.run(format!("N = {n}"), d)?;
        let t_b = (n as f64 / U_B_LB) as u64;
        let t_diff = (n as f64 * n as f64 / alpha) as u64;
        run.to_steady(t_b / 2, STEADY_TOL, t_diff.max(200 * t_b), true)?;
        let f = run.fields();
        let theta = f.theta.as_ref().expect("thermal fields");
        let (nu_, w) = (n as usize, n as usize + 2);
        let l = n as f64;

        // Hot wall half-way between columns 0 (wall) and 1 (first fluid column).
        let nu_hot = (0..nu_)
            .map(|y| {
                let (t1, t2) = (theta[y * w + 1] as f64, theta[y * w + 2] as f64);
                let grad = (-8.0 * theta_hot + 9.0 * t1 - t2) / 3.0;
                -grad * l / theta_span
            })
            .sum::<f64>()
            / l;
        let flux: f64 = (0..nu_)
            .flat_map(|y| (1..=nu_).map(move |x| y * w + x))
            .map(|i| (f.ux[i] * theta[i]) as f64)
            .sum();
        let nu_mean = 1.0 + l * (flux / (l * l)) / (alpha * theta_span);

        // Mid-planes: x = 0.5 between fluid columns n/2 − 1 and n/2 (grid columns n/2, n/2 + 1);
        // y = 0.5 between rows n/2 − 1 and n/2.
        let scale = l / alpha;
        let m = nu_ / 2;
        let coord: Vec<f64> = (0..nu_).map(|i| (i as f64 + 0.5) / l).collect();
        let u_mid: Vec<f64> = (0..nu_)
            .map(|y| 0.5 * (f.ux[y * w + m] + f.ux[y * w + m + 1]) as f64 * scale)
            .collect();
        let v_mid: Vec<f64> = (0..nu_)
            .map(|x| 0.5 * (f.uy[(m - 1) * w + x + 1] + f.uy[m * w + x + 1]) as f64 * scale)
            .collect();
        let (y_u_max, u_max) = refined_max(&coord, &u_mid);
        let (x_v_max, v_max) = refined_max(&coord, &v_mid);
        let meas = Measured {
            nu_hot,
            nu_mean,
            u_max,
            y_u_max,
            v_max,
            x_v_max,
        };
        rows.push(vec![
            n.to_string(),
            format!("{nu_hot:.4}"),
            format!("{:+.2}%", 100.0 * (nu_hot / r.nu_hot - 1.0)),
            format!("{nu_mean:.4}"),
            format!("{:+.2}%", 100.0 * (nu_mean / r.nu_mean - 1.0)),
            format!("{u_max:.3} @ {y_u_max:.3}"),
            format!("{v_max:.3} @ {x_v_max:.3}"),
        ]);
        last = Some(meas);
        if n == *sizes.last().expect("sizes") {
            out.snapshots.push(snapshot(
                &format!("heated-cavity-ra{}-temperature.png", ra.log10() as i32),
                format!("Temperature (hot left, cold right) at Ra = {ra:.0e}, N = {n}"),
                run.domain(),
                theta,
                Colormap::Inferno,
                false,
            )?);
        }
        out.runs.push(run.stat);
    }
    let m = last.expect("at least one resolution");
    let finest = sizes[sizes.len() - 1];
    out.metrics.push(Metric::relative(
        format!("Nu on the hot wall at N = {finest}"),
        m.nu_hot,
        r.nu_hot,
        0.02,
    ));
    out.metrics.push(Metric::relative(
        format!("Nū (cavity average) at N = {finest}"),
        m.nu_mean,
        r.nu_mean,
        0.02,
    ));
    out.metrics.push(Metric::info(
        "u_max on x = 0.5 (α/L)",
        m.u_max,
        format!("{} at y = {}", fmt_num(r.u_max), r.y_u_max),
    ));
    out.metrics
        .push(Metric::info("  at y", m.y_u_max, fmt_num(r.y_u_max)));
    out.metrics.push(Metric::info(
        "v_max on y = 0.5 (α/L)",
        m.v_max,
        format!("{} at x = {}", fmt_num(r.v_max), r.x_v_max),
    ));
    out.metrics
        .push(Metric::info("  at x", m.x_v_max, fmt_num(r.x_v_max)));
    out.tables.push(Table {
        title: format!(
            "Results by resolution (reference: Nu_hot = {}, Nū = {}, u_max = {} @ {}, v_max = {} @ {})",
            r.nu_hot, r.nu_mean, r.u_max, r.y_u_max, r.v_max, r.x_v_max
        ),
        headers: vec![
            "N".into(),
            "Nu hot wall".into(),
            "error".into(),
            "Nū".into(),
            "error".into(),
            "u_max @ y".into(),
            "v_max @ x".into(),
        ],
        rows,
    });
    Ok(out)
}
