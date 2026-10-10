//! Cases with analytical solutions (type A in the validation guide): Taylor-Green, Poiseuille,
//! Couette. Convergence studies use diffusive scaling: τ fixed, `u_lb ∝ 1/N`.

use std::f64::consts::PI;

use anyhow::Result;
use cfd_core::solver::LatticeFields;
use cfd_core::DomainOptions;

use super::{within, Case, Outcome};
use crate::harness::{fitted_order, order, rel_l2, Ctx};
use crate::plot::{Plot, Series};
use crate::scenes::{self, ids};
use crate::{fmt_num, Level, Metric, Table};

/// Log-log convergence plot of errors against N, with a slope −2 guide.
fn convergence_plot(file: &str, title: &str, n: &[f64], e: &[f64], label: &str) -> Plot {
    let guide = e[0] * (n[0] / n[n.len() - 1]).powi(2);
    Plot {
        file: file.into(),
        title: title.into(),
        x_label: "cells per characteristic length N".into(),
        y_label: label.into(),
        log_x: true,
        log_y: true,
        series: vec![
            Series::markers(
                "simulation",
                n.iter().copied().zip(e.iter().copied()).collect(),
            ),
            Series::dashed("slope −2", vec![(n[0], e[0]), (n[n.len() - 1], guide)]),
        ],
    }
}

/// Note for resolutions finer than the convergence fit.
const FLOOR_NOTE: &str = "f32 round-off floor (ADR-015), not fitted";

/// Number of leading resolutions up to `max_n`: those where the discretisation error is above
/// the f32 round-off floor and that enter the order fit.
fn fitted_levels(sizes: &[f64], max_n: f64) -> usize {
    sizes.iter().take_while(|&&s| s <= max_n).count()
}

/// Pairwise orders as table cells ("—" for the first resolution).
fn order_cells(n: &[f64], e: &[f64]) -> Vec<String> {
    (0..n.len())
        .map(|i| {
            if i == 0 {
                "—".into()
            } else {
                format!("{:.2}", order(n[i - 1], e[i - 1], n[i], e[i]))
            }
        })
        .collect()
}

// ---- Taylor-Green ------------------------------------------------------------------------

const TG_NU: f64 = 0.01;
const TG_U0: f64 = 0.128;
/// Finest resolution in the order fit: beyond it (u_lb = 0.0025 and ~17 000 steps at N = 256)
/// accumulated f32 rounding (~2e-4) dominates the discretisation error.
const TG_FIT_MAX: f64 = 128.0;

/// Exact Taylor-Green fields in lattice units on an `n²` periodic box of side 1 m, at physical
/// time `t`, for lattice velocity amplitude `u_lb` at t = 0 (pressure → lattice density).
fn taylor_green_fields(n: usize, u_lb: f64, t: f64) -> LatticeFields {
    let k = 2.0 * PI;
    let u = u_lb * (-2.0 * TG_NU * k * k * t).exp();
    let mut f = LatticeFields {
        density: vec![0.0; n * n],
        ux: vec![0.0; n * n],
        uy: vec![0.0; n * n],
        theta: None,
    };
    for y in 0..n {
        for x in 0..n {
            let (kx, ky) = (
                k * (x as f64 + 0.5) / n as f64,
                k * (y as f64 + 0.5) / n as f64,
            );
            let p = -u * u / 4.0 * ((2.0 * kx).cos() + (2.0 * ky).cos());
            let i = y * n + x;
            f.density[i] = (1.0 + 3.0 * p) as f32;
            f.ux[i] = (-u * kx.cos() * ky.sin()) as f32;
            f.uy[i] = (u * kx.sin() * ky.cos()) as f32;
        }
    }
    f
}

pub fn taylor_green() -> Case {
    Case::new(
        "taylor-green",
        "Taylor-Green vortex decay",
        "Analytical: u = U e^(−2νk²t) (−cos kx sin ky, sin kx cos ky)",
        true,
        |ctx: &Ctx| -> Result<Outcome> {
            let sizes: &[usize] = match ctx.level {
                Level::Quick => &[16, 32, 64],
                Level::Full => &[16, 32, 64, 128, 256],
            };
            let k = 2.0 * PI;
            let t_visc = 1.0 / (2.0 * TG_NU * k * k);
            let mut out = Outcome {
                description: format!(
                    "Periodic 1 m box, ν = {TG_NU} m²/s, U = {TG_U0} m/s (Re = UL/ν = {:.1}), run for one \
                     viscous time 1/(2νk²) = {t_visc:.3} s. Diffusive scaling: u_lb = 0.64/N (τ = 0.65). \
                     Error: relative L2 of the velocity field; decay rate from the kinetic energy.",
                    TG_U0 / TG_NU
                ),
                ..Default::default()
            };
            let (mut e, mut decay) = (Vec::new(), Vec::new());
            for &n in sizes {
                let u_lb = 0.64 / n as f64;
                let opts = DomainOptions {
                    lattice_velocity: u_lb,
                    characteristic_velocity: Some(TG_U0),
                };
                let d = ctx.domain(&scenes::periodic_box(n as u32, TG_NU), opts)?;
                let steps = (t_visc / d.units.dt).round() as u64;
                let t = steps as f64 * d.units.dt;
                let mut run = ctx.run(format!("N = {n}"), d)?;
                run.solver
                    .set_equilibrium(&taylor_green_fields(n, u_lb, 0.0))?;
                let e0 = run.solver.diagnostics().kinetic_energy;
                run.step(steps)?;
                let e1 = run.solver.diagnostics().kinetic_energy;
                let sim = run.fields();
                let exact = taylor_green_fields(n, u_lb, t);
                let cat = |f: &LatticeFields| -> Vec<f64> {
                    f.ux.iter().chain(&f.uy).map(|&v| v as f64).collect()
                };
                e.push(rel_l2(&cat(&sim), &cat(&exact)));
                let rate_sim = -(e1 / e0).ln() / t;
                let rate = 4.0 * TG_NU * k * k;
                decay.push((rate_sim / rate - 1.0).abs());
                out.runs.push(run.stat);
            }
            let n: Vec<f64> = sizes.iter().map(|&v| v as f64).collect();
            let orders = order_cells(&n, &e);
            let fit = fitted_levels(&n, TG_FIT_MAX);
            out.tables.push(Table {
                title: "Convergence".into(),
                headers: vec![
                    "N".into(),
                    "velocity L2 error".into(),
                    "order".into(),
                    "decay-rate error".into(),
                    "note".into(),
                ],
                rows: (0..n.len())
                    .map(|i| {
                        vec![
                            sizes[i].to_string(),
                            format!("{:.3e}", e[i]),
                            orders[i].clone(),
                            format!("{:.3e}", decay[i]),
                            if i < fit { "" } else { FLOOR_NOTE }.into(),
                        ]
                    })
                    .collect(),
            });
            let i32 = sizes.iter().position(|&v| v == 32).expect("N = 32 is run");
            out.metrics.push(Metric::check(
                "velocity L2 error at N = 32",
                e[i32],
                "0 (limit 1%)",
                e[i32],
                0.01,
            ));
            out.metrics.push(within(
                format!("convergence order (fit, N ≤ {TG_FIT_MAX})"),
                fitted_order(&n[..fit], &e[..fit]),
                1.8,
                2.3,
            ));
            let last = decay.len() - 1;
            out.metrics.push(Metric::check(
                format!("energy decay-rate error at N = {}", sizes[last]),
                decay[last],
                "0 (limit 1%)",
                decay[last],
                0.01,
            ));
            out.plots.push(convergence_plot(
                "taylor-green-convergence.svg",
                "Taylor-Green: velocity error vs resolution",
                &n,
                &e,
                "relative L2 error",
            ));
            Ok(out)
        },
    )
}

// ---- Plane channels ------------------------------------------------------------------------

/// Channel parameters: H = 1 m, ν = 1 m²/s, characteristic velocity 1 m/s (Re = 1);
/// u_lb = 1/(6N) keeps τ = 1.
const CH_NU: f64 = 1.0;
/// Finest channel resolution in the order fit / exactness check (f32 round-off grows with N).
const CH_FIT_MAX: f64 = 32.0;

fn channel_options(n: u32, moving_wall: bool) -> DomainOptions {
    DomainOptions {
        lattice_velocity: 1.0 / (6.0 * n as f64),
        characteristic_velocity: (!moving_wall).then_some(1.0),
    }
}

/// Velocity profile `u(y)` (lattice units) of a plane channel, fluid rows 1..=n.
fn profile(f: &LatticeFields, n: u32) -> Vec<f64> {
    (1..=n as usize).map(|r| f.ux[r * 4 + 1] as f64).collect()
}

/// Force (N/m) on element `id` in the x direction.
fn force_x(run: &mut crate::harness::Run, id: u16) -> f64 {
    let units = run.domain().units;
    run.solver
        .forces()
        .iter()
        .find(|f| f.element == id)
        .map_or(0.0, |f| f.physical(&units)[0])
}

/// Max steps and check interval for a channel of height `n` (diffusive time 6N² with τ = 1).
fn channel_steps(n: u32) -> (u64, u64) {
    let t = 6 * n as u64 * n as u64;
    ((t / 10).max(50), 60 * t)
}

pub fn poiseuille() -> Case {
    Case::new(
        "poiseuille",
        "Plane Poiseuille flow (body force)",
        "Analytical: u(y) = G y (H − y) / (2ν)",
        true,
        |ctx: &Ctx| -> Result<Outcome> {
            let sizes: &[u32] = match ctx.level {
                Level::Quick => &[8, 16, 32],
                Level::Full => &[8, 16, 32, 64],
            };
            let g = 8.0 * CH_NU; // u_max = G H² / (8ν) = 1 m/s
            let mut out = Outcome {
                description: format!(
                    "Channel of height H = 1 m between no-slip walls, periodic in x, driven by a body \
                     acceleration G = {g} m/s² (u_max = 1 m/s, Re = 1, τ = 1). Run to steady state \
                     (relative change per 0.1 diffusive time < 1e-6). Error: relative L2 of u(y); wall shear stress from \
                     momentum exchange against ρGH/2."
                ),
                ..Default::default()
            };
            let (mut e, mut shear) = (Vec::new(), Vec::new());
            for &n in sizes {
                let mut d = ctx.domain(
                    &scenes::plane_channel(n, CH_NU, 0.0),
                    channel_options(n, false),
                )?;
                let g_lb = d.units.acceleration_to_lattice(g);
                d.physics.body_force = [g_lb, 0.0];
                let nu_lb = d.physics.nu;
                let width = 4.0 * d.units.dx;
                let mut run = ctx.run(format!("N = {n}"), d)?;
                let (every, max) = channel_steps(n);
                run.to_steady(every, 1e-6, max, false)?;
                let f = run.fields();
                let exact: Vec<f64> = (0..n)
                    .map(|j| {
                        let y = j as f64 + 0.5;
                        g_lb / (2.0 * nu_lb) * y * (n as f64 - y)
                    })
                    .collect();
                e.push(rel_l2(&profile(&f, n), &exact));
                // τ_w = ρ G H / 2 on each wall (ρ = 1 kg/m³).
                let tau_w = g / 2.0;
                let fx = force_x(&mut run, ids::WALL);
                shear.push((fx / width / tau_w - 1.0).abs());
                out.runs.push(run.stat);
            }
            let n: Vec<f64> = sizes.iter().map(|&v| v as f64).collect();
            let orders = order_cells(&n, &e);
            let fit = fitted_levels(&n, CH_FIT_MAX);
            out.tables.push(Table {
                title: "Convergence".into(),
                headers: vec![
                    "N".into(),
                    "u(y) L2 error".into(),
                    "order".into(),
                    "wall-shear error".into(),
                    "note".into(),
                ],
                rows: (0..n.len())
                    .map(|i| {
                        vec![
                            sizes[i].to_string(),
                            format!("{:.3e}", e[i]),
                            orders[i].clone(),
                            format!("{:.3e}", shear[i]),
                            if i < fit { "" } else { FLOOR_NOTE }.into(),
                        ]
                    })
                    .collect(),
            });
            let i32 = sizes.iter().position(|&v| v == 32).expect("N = 32 is run");
            out.metrics.push(Metric::check(
                "u(y) L2 error at N = 32",
                e[i32],
                "0 (limit 1%)",
                e[i32],
                0.01,
            ));
            let max_e = e[..fit].iter().copied().fold(0.0, f64::max);
            if max_e < 1e-5 {
                // TRT with Λ = 3/16 puts bounce-back walls exactly half-way: no discretisation
                // error is left to converge.
                out.metrics.push(Metric::check(
                    format!("largest L2 error, N ≤ {CH_FIT_MAX} (exact scheme)"),
                    max_e,
                    "< 1e-5",
                    max_e,
                    1e-5,
                ));
            } else {
                out.metrics.push(within(
                    format!("convergence order (fit, N ≤ {CH_FIT_MAX})"),
                    fitted_order(&n[..fit], &e[..fit]),
                    1.8,
                    2.3,
                ));
                out.plots.push(convergence_plot(
                    "poiseuille-convergence.svg",
                    "Poiseuille: profile error vs resolution",
                    &n,
                    &e,
                    "relative L2 error",
                ));
            }
            let last = shear.len() - 1;
            out.metrics.push(Metric::check(
                format!("wall shear stress error at N = {}", sizes[last]),
                shear[last],
                "0 (limit 1%)",
                shear[last],
                0.01,
            ));
            Ok(out)
        },
    )
}

pub fn couette() -> Case {
    Case::new(
        "couette",
        "Plane Couette flow (moving wall)",
        "Analytical: u(y) = U y / H",
        true,
        |ctx: &Ctx| -> Result<Outcome> {
            let sizes: &[u32] = match ctx.level {
                Level::Quick => &[16],
                Level::Full => &[16, 32],
            };
            let mut out = Outcome {
                description:
                    "Channel of height H = 1 m, top wall moving at U = 1 m/s, periodic in x \
                              (Re = 1, τ = 1). Run to steady state. Error: relative L2 of u(y); \
                              shear stress ρνU/H on both walls from momentum exchange."
                        .into(),
                ..Default::default()
            };
            let mut rows = Vec::new();
            for &n in sizes {
                let d = ctx.domain(
                    &scenes::plane_channel(n, CH_NU, 1.0),
                    channel_options(n, true),
                )?;
                let u_lb = d.units.velocity_to_lattice(1.0);
                let width = 4.0 * d.units.dx;
                let mut run = ctx.run(format!("N = {n}"), d)?;
                let (every, max) = channel_steps(n);
                run.to_steady(every, 1e-6, max, false)?;
                let f = run.fields();
                let exact: Vec<f64> = (0..n).map(|j| u_lb * (j as f64 + 0.5) / n as f64).collect();
                let e = rel_l2(&profile(&f, n), &exact);
                let tau = CH_NU * 1.0 / 1.0; // ρνU/H with ρ = 1
                let bottom = force_x(&mut run, ids::WALL) / width;
                let top = force_x(&mut run, ids::DRIVE) / width;
                let shear_err = (bottom / tau - 1.0).abs().max((top / -tau - 1.0).abs());
                out.metrics.push(Metric::check(
                    format!("u(y) L2 error at N = {n}"),
                    e,
                    "0 (limit 1%)",
                    e,
                    0.01,
                ));
                rows.push(vec![
                    n.to_string(),
                    format!("{e:.3e}"),
                    fmt_num(bottom),
                    fmt_num(top),
                ]);
                if n == *sizes.last().expect("sizes") {
                    out.metrics.push(Metric::check(
                        format!("wall shear stress error at N = {n}"),
                        shear_err,
                        "0 (limit 1%)",
                        shear_err,
                        0.01,
                    ));
                }
                out.runs.push(run.stat);
            }
            out.tables.push(Table {
                title: "Results (exact shear stress ±1 Pa)".into(),
                headers: vec![
                    "N".into(),
                    "u(y) L2 error".into(),
                    "bottom wall τ (Pa)".into(),
                    "top wall τ (Pa)".into(),
                ],
                rows,
            });
            Ok(out)
        },
    )
}
