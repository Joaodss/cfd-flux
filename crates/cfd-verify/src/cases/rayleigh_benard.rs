//! Onset of Rayleigh-Bénard convection between rigid plates (type A: linear stability theory,
//! Ra_c = 1707.76 at k_c H = 3.117).
//!
//! A layer one critical wavelength long starts from the conduction profile plus a small
//! perturbation. After the faster modes have decayed, the amplitude of the critical mode grows or
//! decays exponentially with rate σ(Ra); σ changes sign at Ra_c, which is found by fitting σ(Ra)
//! linearly through runs on both sides of the threshold.

use std::f64::consts::PI;

use anyhow::Result;
use cfd_core::domain::flags;
use cfd_core::solver::LatticeFields;
use cfd_core::{Domain, DomainOptions};

use super::{Case, Outcome};
use crate::harness::Ctx;
use crate::scenes::{self, T_COLD, T_HOT};
use crate::{Level, Metric, Plot, Series, Table};

/// Critical Rayleigh number for rigid-rigid plates (Chandrasekhar 1961, Table III).
pub const RA_CRITICAL: f64 = 1707.76;
const PRANDTL: f64 = 0.71;
/// Lattice value of the buoyancy velocity √(gβΔT H).
const U_B_LB: f64 = 0.08;
/// Rayleigh numbers around the threshold (±9%), where σ(Ra) is close to linear.
const RAYLEIGH: [f64; 4] = [1550.0, 1650.0, 1750.0, 1850.0];
/// Initial temperature perturbation, as a fraction of ΔT.
const PERTURBATION: f64 = 1e-3;
/// The growth rate is fitted over t ∈ [FIT_START, RUN_LENGTH] diffusive times H²/α.
const FIT_START: f64 = 0.5;
const RUN_LENGTH: f64 = 2.0;
/// Samples of the amplitude per diffusive time.
const SAMPLES: usize = 40;

pub fn case() -> Case {
    Case::new(
        "rayleigh-benard",
        "Rayleigh-Bénard onset of convection",
        "Linear stability theory: Ra_c = 1707.76 for rigid plates (Chandrasekhar 1961)",
        true,
        run,
    )
}

fn run(ctx: &Ctx) -> Result<Outcome> {
    let (sizes, limit): (&[u32], f64) = match ctx.level {
        Level::Quick => (&[16], 0.05),
        Level::Full => (&[16, 32, 64], 0.02),
    };
    let mut out = Outcome {
        description: format!(
            "Layer of height H = 1 m between a hot bottom and a cold top plate (fixed temperatures, \
             ΔT = 1 K), periodic along x over one critical wavelength 2π/k_c (k_c H = {}), Pr = \
             {PRANDTL}, Boussinesq buoyancy with √(gβΔTH) mapped to lattice velocity {U_B_LB}. \
             Initial state: conduction profile in hydrostatic balance plus a perturbation \
             {PERTURBATION:e} ΔT sin(kx) sin(πy/H). The amplitude A of the critical mode (the \
             x-Fourier coefficient of v at wavenumber k, summed over rows) is fitted to e^(σt) over \
             t = {FIT_START}–{RUN_LENGTH} H²/α, at Ra = {}. Ra_c is the zero of a linear fit of \
             σ(Ra). Pass/fail: Ra_c within {:.0}% of {RA_CRITICAL} at the finest resolution.",
            scenes::RB_CRITICAL_WAVENUMBER,
            RAYLEIGH.map(|r| r.to_string()).join(", "),
            limit * 100.0
        ),
        ..Default::default()
    };
    let mut rows = Vec::new();
    let mut ra_c = f64::NAN;
    for &h in sizes {
        let mut sigmas = Vec::new();
        let mut curves = Vec::new();
        for &ra in &RAYLEIGH {
            let opts = DomainOptions {
                lattice_velocity: U_B_LB,
                characteristic_velocity: Some(1.0),
            };
            let d = ctx.domain(&scenes::rayleigh_benard(h, ra, PRANDTL), opts)?;
            let alpha = d.physics.alpha.expect("thermal domain");
            let t_diff = (h as f64).powi(2) / alpha;
            let every = (t_diff / SAMPLES as f64).round().max(1.0) as u64;
            let initial = initial_fields(&d);
            let mut run = ctx.run(format!("H = {h}, Ra = {ra}"), d)?;
            run.solver.set_equilibrium(&initial)?;
            let mut curve = Vec::new();
            let total = (RUN_LENGTH * SAMPLES as f64).round() as usize;
            for _ in 0..total {
                run.step(every)?;
                let t = run.stat.steps as f64 / t_diff;
                let fields = run.fields();
                curve.push((t, mode_amplitude(run.domain(), &fields).ln()));
            }
            let fit: Vec<(f64, f64)> = curve
                .iter()
                .copied()
                .filter(|&(t, _)| t >= FIT_START)
                .collect();
            sigmas.push(slope(&fit).0);
            curves.push((ra, curve));
            out.runs.push(run.stat);
        }
        let (a, b) = slope(
            &RAYLEIGH
                .iter()
                .copied()
                .zip(sigmas.iter().copied())
                .collect::<Vec<_>>(),
        );
        ra_c = -b / a;
        let mut row = vec![
            h.to_string(),
            format!("{ra_c:.1}"),
            format!("{:+.2}%", 100.0 * (ra_c / RA_CRITICAL - 1.0)),
        ];
        row.extend(sigmas.iter().map(|s| format!("{s:+.4}")));
        rows.push(row);
        if h == *sizes.last().expect("sizes") {
            out.plots.push(Plot {
                file: "rayleigh-benard-amplitude.svg".into(),
                title: format!("Rayleigh-Bénard, H = {h}: amplitude of the critical mode"),
                x_label: "t (H²/α)".into(),
                y_label: "ln A".into(),
                log_x: false,
                log_y: false,
                series: curves
                    .into_iter()
                    .map(|(ra, c)| Series::line(format!("Ra = {ra}"), c))
                    .collect(),
            });
            let line = |ra: f64| (ra, a * ra + b);
            out.plots.push(Plot {
                file: "rayleigh-benard-growth.svg".into(),
                title: format!("Rayleigh-Bénard, H = {h}: growth rate of the critical mode"),
                x_label: "Ra".into(),
                y_label: "σ (α/H²)".into(),
                log_x: false,
                log_y: false,
                series: vec![
                    Series::markers(
                        "measured",
                        RAYLEIGH
                            .iter()
                            .copied()
                            .zip(sigmas.iter().copied())
                            .collect(),
                    ),
                    Series::dashed("linear fit", vec![line(RAYLEIGH[0]), line(RAYLEIGH[3])]),
                    Series::dashed(
                        format!("Ra_c = {RA_CRITICAL}"),
                        vec![(RA_CRITICAL, sigmas[0]), (RA_CRITICAL, sigmas[3])],
                    ),
                ],
            });
        }
    }
    let finest = sizes[sizes.len() - 1];
    out.metrics.push(Metric::relative(
        format!("Ra_c at H = {finest}"),
        ra_c,
        RA_CRITICAL,
        limit,
    ));
    let mut headers = vec!["H (cells)".into(), "Ra_c".into(), "error".into()];
    headers.extend(RAYLEIGH.iter().map(|ra| format!("σ at Ra = {ra}")));
    out.tables.push(Table {
        title: "Critical Rayleigh number by resolution (σ in units of α/H²)".into(),
        headers,
        rows,
    });
    Ok(out)
}

/// Conduction profile with the perturbation, in hydrostatic balance (`c_s² ∂ρ/∂y = F_y`, so the
/// start is free of acoustic waves). Grid: plate rows 0 and h + 1, fluid rows 1…h with the plates
/// half-way between rows.
fn initial_fields(d: &Domain) -> LatticeFields {
    let (w, h) = (d.width as usize, d.height as usize - 2);
    let theta_hot = d.units.temperature_to_lattice(T_HOT);
    let theta_cold = d.units.temperature_to_lattice(T_COLD);
    let span = theta_hot - theta_cold;
    let conduction = |row: usize| {
        let s = (row as f64 - 0.5) / h as f64;
        theta_hot - span * s
    };
    let mut rho_row = vec![1.0; h + 2];
    for row in 2..=h {
        let f = d.physics.buoyancy[1] * 0.5 * (conduction(row - 1) + conduction(row));
        rho_row[row] = rho_row[row - 1] + 3.0 * f;
    }
    let mean = rho_row[1..=h].iter().sum::<f64>() / h as f64;
    let mut lf = LatticeFields::initial(d);
    let theta = lf.theta.as_mut().expect("thermal domain");
    for (row, rho) in rho_row.iter().enumerate().take(h + 1).skip(1) {
        let s = (row as f64 - 0.5) / h as f64;
        for x in 0..w {
            let kx = 2.0 * PI * (x as f64 + 0.5) / w as f64;
            let c = row * w + x;
            theta[c] = (conduction(row) + PERTURBATION * span * kx.sin() * (PI * s).sin()) as f32;
            lf.density[c] = (rho - mean + 1.0) as f32;
        }
    }
    lf
}

/// `A = sqrt(Σ_rows |Σ_x v e^(−ikx)|²)` over the fluid cells: the x-independent acoustic mode
/// and the other wavenumbers do not contribute.
fn mode_amplitude(d: &Domain, f: &LatticeFields) -> f64 {
    let w = d.width as usize;
    (0..d.height as usize)
        .map(|y| {
            let (mut re, mut im) = (0.0, 0.0);
            for x in 0..w {
                let c = y * w + x;
                if flags::flow(d.flags[c]) != flags::FLUID {
                    continue;
                }
                let kx = 2.0 * PI * (x as f64 + 0.5) / w as f64;
                re += f.uy[c] as f64 * kx.cos();
                im -= f.uy[c] as f64 * kx.sin();
            }
            re * re + im * im
        })
        .sum::<f64>()
        .sqrt()
}

/// Least-squares line `y = a x + b`; returns `(a, b)`.
fn slope(p: &[(f64, f64)]) -> (f64, f64) {
    let k = p.len() as f64;
    let mx = p.iter().map(|q| q.0).sum::<f64>() / k;
    let my = p.iter().map(|q| q.1).sum::<f64>() / k;
    let sxy: f64 = p.iter().map(|q| (q.0 - mx) * (q.1 - my)).sum();
    let sxx: f64 = p.iter().map(|q| (q.0 - mx).powi(2)).sum();
    let a = sxy / sxx;
    (a, my - a * mx)
}

#[cfg(test)]
mod tests {
    use super::slope;

    #[test]
    fn least_squares_line() {
        let p: Vec<(f64, f64)> = (0..5).map(|i| (i as f64, 3.0 * i as f64 - 2.0)).collect();
        let (a, b) = slope(&p);
        assert!((a - 3.0).abs() < 1e-12 && (b + 2.0).abs() < 1e-12);
    }
}
