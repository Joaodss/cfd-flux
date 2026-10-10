//! Running solvers inside cases: construction through the factory, timed stepping, steady-state
//! detection, error norms, convergence orders, interpolation and field snapshots.

use std::time::Instant;

use anyhow::{anyhow, Result};
use cfd_core::domain::flags;
use cfd_core::solver::LatticeFields;
use cfd_core::{Domain, DomainOptions, Scene, Solver};
use cfd_io::image::{self, Colormap, ColourScale};

use crate::{Level, RunStat, Snapshot, SolverFactory};

/// What a case gets to work with.
pub struct Ctx<'a> {
    pub factory: SolverFactory<'a>,
    pub level: Level,
    log: &'a (dyn Fn(&str) + Sync),
}

impl<'a> Ctx<'a> {
    pub fn new(factory: SolverFactory<'a>, level: Level, log: &'a (dyn Fn(&str) + Sync)) -> Self {
        Self {
            factory,
            level,
            log,
        }
    }

    pub fn log(&self, msg: &str) {
        (self.log)(msg)
    }

    /// `Scene → Domain`, turning a validation report into an error.
    pub fn domain(&self, scene: &Scene, opts: DomainOptions) -> Result<Domain> {
        Domain::from_scene(scene, &opts).map_err(|r| {
            let msgs: Vec<String> = r
                .errors()
                .map(|i| format!("[{}] {}", i.code, i.message))
                .collect();
            anyhow!("{}: {}", scene.name, msgs.join("; "))
        })
    }

    /// Creates a timed solver run for `domain`.
    pub fn run(&self, label: impl Into<String>, domain: Domain) -> Result<Run> {
        let fluid_cells = domain.fluid_cell_count();
        let label = label.into();
        self.log(&format!(
            "  {label}: {}×{} cells",
            domain.width, domain.height
        ));
        Ok(Run {
            solver: (self.factory)(domain)?,
            stat: RunStat {
                label,
                fluid_cells,
                steps: 0,
                seconds: 0.0,
                converged: None,
            },
        })
    }
}

/// A solver plus the bookkeeping of how long it ran.
pub struct Run {
    pub solver: Box<dyn Solver>,
    pub stat: RunStat,
}

impl Run {
    pub fn domain(&self) -> &Domain {
        self.solver.domain()
    }

    /// Advances `n` steps (timed).
    pub fn step(&mut self, n: u64) -> Result<()> {
        let t = Instant::now();
        let mut left = n;
        while left > 0 {
            let chunk = left.min(1 << 20);
            self.solver
                .step(chunk as u32)
                .map_err(|e| anyhow!("{}: {e}", self.stat.label))?;
            left -= chunk;
        }
        self.stat.steps += n;
        self.stat.seconds += t.elapsed().as_secs_f64();
        Ok(())
    }

    pub fn fields(&mut self) -> LatticeFields {
        self.solver.lattice_fields()
    }

    /// Steps until the largest change of u (and θ when `theta`) over `every` steps, relative
    /// to its maximum magnitude, is below `tol`, or `max_steps` is reached. Records the outcome
    /// in `stat.converged` and returns it.
    pub fn to_steady(&mut self, every: u64, tol: f64, max_steps: u64, theta: bool) -> Result<bool> {
        let mut prev = self.fields();
        let start = self.stat.steps;
        let converged = loop {
            if self.stat.steps - start >= max_steps {
                break false;
            }
            self.step(every)?;
            let now = self.fields();
            let speed = max_abs_value(&now.ux).max(max_abs_value(&now.uy));
            let mut change = max_diff(&now.ux, &prev.ux).max(max_diff(&now.uy, &prev.uy)) / speed;
            if theta {
                if let (Some(a), Some(b)) = (&now.theta, &prev.theta) {
                    change = change.max(max_diff(a, b) / max_abs_value(a));
                }
            }
            prev = now;
            if change < tol {
                break true;
            }
        };
        self.stat.converged = Some(converged);
        Ok(converged)
    }
}

/// `max |a|` (at least 1e-30, so it can divide).
fn max_abs_value(a: &[f32]) -> f64 {
    a.iter().map(|v| v.abs() as f64).fold(1e-30, f64::max)
}

/// `max |a − b|`.
fn max_diff(a: &[f32], b: &[f32]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(x, y)| (x - y).abs() as f64)
        .fold(0.0, f64::max)
}

/// Relative L2 error `‖s − r‖ / ‖r‖`.
pub fn rel_l2(sim: &[f64], reference: &[f64]) -> f64 {
    let num: f64 = sim
        .iter()
        .zip(reference)
        .map(|(s, r)| (s - r).powi(2))
        .sum();
    let den: f64 = reference.iter().map(|r| r * r).sum();
    (num / den).sqrt()
}

/// `max |s − r|`.
pub fn max_abs(sim: &[f64], reference: &[f64]) -> f64 {
    sim.iter()
        .zip(reference)
        .map(|(s, r)| (s - r).abs())
        .fold(0.0, f64::max)
}

/// Observed order between two resolutions `n1 < n2`.
pub fn order(n1: f64, e1: f64, n2: f64, e2: f64) -> f64 {
    (e1 / e2).ln() / (n2 / n1).ln()
}

/// Least-squares order `p` of `e ≈ C N^{−p}` over every resolution.
pub fn fitted_order(n: &[f64], e: &[f64]) -> f64 {
    let x: Vec<f64> = n.iter().map(|v| v.ln()).collect();
    let y: Vec<f64> = e.iter().map(|v| v.ln()).collect();
    let k = x.len() as f64;
    let (mx, my) = (x.iter().sum::<f64>() / k, y.iter().sum::<f64>() / k);
    let sxy: f64 = x.iter().zip(&y).map(|(a, b)| (a - mx) * (b - my)).sum();
    let sxx: f64 = x.iter().map(|a| (a - mx).powi(2)).sum();
    -sxy / sxx
}

/// Linear interpolation in a table with increasing `xs` (clamped at the ends).
pub fn interp(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    match xs.iter().position(|&v| v >= x) {
        None => ys[ys.len() - 1],
        Some(0) => ys[0],
        Some(i) => {
            let t = (x - xs[i - 1]) / (xs[i] - xs[i - 1]);
            ys[i - 1] + t * (ys[i] - ys[i - 1])
        }
    }
}

/// Maximum of samples `(x, y)` refined with a parabola through the largest one and its
/// neighbours (uniform spacing). Returns `(x_max, y_max)`.
pub fn refined_max(x: &[f64], y: &[f64]) -> (f64, f64) {
    let i = (0..y.len())
        .max_by(|&a, &b| y[a].total_cmp(&y[b]))
        .expect("samples");
    if i == 0 || i + 1 == y.len() {
        return (x[i], y[i]);
    }
    let (a, b, c) = (y[i - 1], y[i], y[i + 1]);
    let denom = a - 2.0 * b + c;
    if denom == 0.0 {
        return (x[i], y[i]);
    }
    let d = 0.5 * (a - c) / denom; // offset in samples, |d| ≤ 0.5
    let h = x[i + 1] - x[i];
    (x[i] + d * h, b - 0.25 * (a - c) * d)
}

/// Snapshot of a scalar field (one value per cell) with non-fluid cells masked, about 480 px
/// on the long side. `symmetric` centres the colour range on zero.
pub fn snapshot(
    file: &str,
    caption: impl Into<String>,
    domain: &Domain,
    values: &[f32],
    colormap: Colormap,
    symmetric: bool,
) -> Result<Snapshot> {
    let mask: Vec<bool> = domain
        .flags
        .iter()
        .map(|&f| flags::flow(f) == flags::FLUID)
        .collect();
    let fluid: Vec<f32> = values
        .iter()
        .zip(&mask)
        .filter_map(|(v, m)| m.then_some(*v))
        .collect();
    let (mut min, mut max) = if symmetric {
        let abs: Vec<f32> = fluid.iter().map(|v| v.abs()).collect();
        let m = image::quantile_range(&abs, 0.0, 0.99, 1 << 22).map_or(1.0, |r| r.1);
        (-m, m)
    } else {
        image::quantile_range(&fluid, 0.0, 1.0, 1 << 22).unwrap_or((0.0, 1.0))
    };
    if max <= min {
        (min, max) = (min - 0.5, min + 0.5);
    }
    let long = domain.width.max(domain.height).max(1);
    let scale = 480u32.div_ceil(long).clamp(1, 8);
    let img = image::render(
        values,
        domain.width,
        domain.height,
        &ColourScale { colormap, min, max },
        Some(&mask),
        scale,
    )?;
    Ok(Snapshot {
        file: file.into(),
        caption: caption.into(),
        image: img,
    })
}

/// Velocity magnitude of lattice fields.
pub fn speed(f: &LatticeFields) -> Vec<f32> {
    f.ux.iter()
        .zip(&f.uy)
        .map(|(u, v)| (u * u + v * v).sqrt())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert!((order(16.0, 4e-3, 32.0, 1e-3) - 2.0).abs() < 1e-12);
        let n = [8.0, 16.0, 32.0, 64.0];
        let e: Vec<f64> = n.iter().map(|v: &f64| 3.0 * v.powf(-2.0)).collect();
        assert!((fitted_order(&n, &e) - 2.0).abs() < 1e-12);
        let xs = [0.0, 1.0, 3.0];
        let ys = [0.0, 2.0, 4.0];
        assert_eq!(interp(&xs, &ys, 2.0), 3.0);
        assert_eq!(interp(&xs, &ys, -1.0), 0.0);
        assert_eq!(interp(&xs, &ys, 9.0), 4.0);
        // Parabola peaking at x = 0.3.
        let x: Vec<f64> = (0..10).map(|i| i as f64 * 0.1).collect();
        let y: Vec<f64> = x.iter().map(|v| 1.0 - (v - 0.33).powi(2)).collect();
        let (xm, ym) = refined_max(&x, &y);
        assert!((xm - 0.33).abs() < 1e-9 && (ym - 1.0).abs() < 1e-9);
        assert!((rel_l2(&[1.0, 2.0], &[1.0, 2.0])).abs() < 1e-15);
        assert_eq!(max_abs(&[1.0, 2.5], &[1.0, 2.0]), 0.5);
    }
}
