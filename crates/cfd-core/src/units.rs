//! Physical ↔ lattice unit conversion and LBM stability checks.
//!
//! The scene is in SI units. The solver works in lattice units, where `dx = dt = 1` and the
//! reference density `ρ₀ = 1`. Temperatures are made dimensionless as `θ = (T − T_ref) / ΔT`.
//!
//! Given the cell size `dx`, a characteristic velocity `U` and a target lattice velocity
//! `u_lb`, the time step is `dt = u_lb · dx / U` (see `docs/04-numerical-methods.md` §3.3).

use crate::validate::Report;

/// Standard acceleration of gravity (m/s²), the default for Earth scenes.
pub const STANDARD_GRAVITY: f64 = 9.80665;

/// Lattice speed of sound squared for D2Q9 (and for the chosen D2Q5 weights).
pub const CS2: f64 = 1.0 / 3.0;

/// Default target lattice velocity for the characteristic velocity (Ma ≈ 0.087).
pub const DEFAULT_LATTICE_VELOCITY: f64 = 0.05;

/// Relaxation times below this are rejected (TRT tolerates values close to 0.5, BGK less so).
pub const TAU_MIN: f64 = 0.505;
/// Relaxation times below this produce a warning.
pub const TAU_WARN_LOW: f64 = 0.51;
/// Relaxation times above this produce a warning (excessive numerical diffusion / wall slip).
pub const TAU_WARN_HIGH: f64 = 2.0;
/// Mach number above which compressibility errors become noticeable.
pub const MACH_WARN: f64 = 0.17;
/// Mach number above which the simulation is rejected.
pub const MACH_MAX: f64 = 0.5;
/// Grid Reynolds number (`U·dx/ν`) above which under-resolution is likely.
pub const GRID_RE_WARN: f64 = 30.0;

/// Conversion factors between physical (SI) and lattice units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UnitSystem {
    /// Cell size (m).
    pub dx: f64,
    /// Time step (s).
    pub dt: f64,
    /// Reference density (kg/m³), maps to lattice density 1.
    pub rho0: f64,
    /// Reference temperature (K), maps to θ = 0.
    pub t_ref: f64,
    /// Temperature scale (K), maps to θ = 1.
    pub delta_t: f64,
}

impl UnitSystem {
    /// Lattice velocity per physical velocity: `dt / dx`.
    fn velocity_factor(&self) -> f64 {
        self.dt / self.dx
    }

    pub fn velocity_to_lattice(&self, u: f64) -> f64 {
        u * self.velocity_factor()
    }

    pub fn velocity_to_physical(&self, u_lb: f64) -> f64 {
        u_lb / self.velocity_factor()
    }

    /// Kinematic viscosity or thermal diffusivity (m²/s) → lattice.
    pub fn diffusivity_to_lattice(&self, d: f64) -> f64 {
        d * self.dt / (self.dx * self.dx)
    }

    /// Acceleration (m/s²) → lattice.
    pub fn acceleration_to_lattice(&self, a: f64) -> f64 {
        a * self.dt * self.dt / self.dx
    }

    /// Gauge pressure (Pa) → lattice density, using `p = c_s² ρ`.
    pub fn gauge_pressure_to_density(&self, p: f64) -> f64 {
        1.0 + p / (self.rho0 * CS2) * self.velocity_factor().powi(2)
    }

    /// Lattice density → gauge pressure (Pa).
    pub fn density_to_gauge_pressure(&self, rho_lb: f64) -> f64 {
        (rho_lb - 1.0) * CS2 * self.rho0 / self.velocity_factor().powi(2)
    }

    pub fn temperature_to_lattice(&self, t: f64) -> f64 {
        (t - self.t_ref) / self.delta_t
    }

    pub fn temperature_to_physical(&self, theta: f64) -> f64 {
        self.t_ref + theta * self.delta_t
    }

    /// Heat flux (W/m²) → lattice θ-flux, for a fluid with density `rho` and specific heat `cp`.
    pub fn heat_flux_to_lattice(&self, q: f64, rho: f64, cp: f64) -> f64 {
        q / (rho * cp * self.delta_t) * self.velocity_factor()
    }

    /// Lattice force per unit depth → N/m (2D: force per metre of depth).
    pub fn force_to_physical(&self, f_lb: f64) -> f64 {
        f_lb * self.rho0 * self.dx * self.dx / (self.dt * self.dt)
    }

    /// Physical time (s) → number of steps, rounded to the nearest step (at least 1).
    pub fn time_to_steps(&self, t: f64) -> u64 {
        ((t / self.dt).round() as u64).max(1)
    }

    pub fn steps_to_time(&self, steps: u64) -> f64 {
        steps as f64 * self.dt
    }
}

/// Lattice relaxation time for a diffusivity in lattice units (`c_s² = 1/3`).
pub fn relaxation_time(diffusivity_lb: f64) -> f64 {
    diffusivity_lb / CS2 + 0.5
}

/// Values the stability checks look at, all in lattice units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StabilityInput {
    /// Kinematic viscosity.
    pub nu: f64,
    /// Thermal diffusivity, when the thermal lattice is used.
    pub alpha: Option<f64>,
    /// Largest velocity expected from the boundary/initial conditions.
    pub u_max: f64,
}

/// Checks relaxation times, Mach number and grid Reynolds number; adds issues to `report`.
pub fn check_stability(input: &StabilityInput, report: &mut Report) {
    const HINT: &str =
        "increase the resolution (smaller cellSize) or lower the lattice velocity (more steps)";
    check_tau(
        relaxation_time(input.nu),
        "stability.tau",
        "viscous",
        HINT,
        report,
    );
    if let Some(alpha) = input.alpha {
        check_tau(
            relaxation_time(alpha),
            "stability.tauThermal",
            "thermal",
            HINT,
            report,
        );
    }

    let mach = input.u_max * CS2.sqrt().recip();
    if mach > MACH_MAX {
        report.error(
            "stability.mach",
            format!("lattice Mach number {mach:.3} exceeds {MACH_MAX}: lower the lattice velocity"),
        );
    } else if mach > MACH_WARN {
        report.warning(
            "stability.mach",
            format!(
                "lattice Mach number {mach:.3} exceeds {MACH_WARN}: expect compressibility errors"
            ),
        );
    }

    let grid_re = input.u_max / input.nu;
    if grid_re > GRID_RE_WARN {
        report.warning(
            "stability.gridRe",
            format!("grid Reynolds number U·dx/ν = {grid_re:.1} is high: {HINT}"),
        );
    }
}

fn check_tau(tau: f64, code: &'static str, what: &str, hint: &str, report: &mut Report) {
    if tau.is_nan() || tau < TAU_MIN {
        report.error(
            code,
            format!("{what} relaxation time τ = {tau:.4} is below {TAU_MIN}: {hint}"),
        );
    } else if tau < TAU_WARN_LOW {
        report.warning(
            code,
            format!("{what} relaxation time τ = {tau:.4} is close to 0.5: the run may be unstable"),
        );
    } else if tau > TAU_WARN_HIGH {
        report.warning(
            code,
            format!(
                "{what} relaxation time τ = {tau:.3} is large: accuracy drops; use a coarser grid or a higher lattice velocity"
            ),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::Severity;

    fn units() -> UnitSystem {
        UnitSystem {
            dx: 0.005,
            dt: 1.0e-4,
            rho0: 1.2,
            t_ref: 300.0,
            delta_t: 10.0,
        }
    }

    #[test]
    fn conversions_round_trip() {
        let u = units();
        let close = |a: f64, b: f64| (a - b).abs() <= 1e-12 * a.abs().max(1.0);
        assert!(close(
            u.velocity_to_physical(u.velocity_to_lattice(1.5)),
            1.5
        ));
        assert!(close(u.velocity_to_lattice(1.0), 0.02));
        assert!(close(
            u.density_to_gauge_pressure(u.gauge_pressure_to_density(12.5)),
            12.5
        ));
        assert!(close(
            u.temperature_to_physical(u.temperature_to_lattice(305.0)),
            305.0
        ));
        assert!(close(u.temperature_to_lattice(305.0), 0.5));
        assert!(close(
            u.diffusivity_to_lattice(1.0e-3),
            1.0e-3 * 1.0e-4 / 2.5e-5
        ));
        assert_eq!(u.time_to_steps(1.0), 10_000);
        assert_eq!(u.time_to_steps(0.0), 1);
    }

    #[test]
    fn relaxation_time_is_three_nu_plus_half() {
        assert!((relaxation_time(0.1) - 0.8).abs() < 1e-12);
    }

    fn codes(input: StabilityInput) -> Vec<(Severity, &'static str)> {
        let mut r = Report::default();
        check_stability(&input, &mut r);
        r.issues.iter().map(|i| (i.severity, i.code)).collect()
    }

    #[test]
    fn stable_parameters_give_no_issues() {
        let ok = StabilityInput {
            nu: 0.02,
            alpha: Some(0.03),
            u_max: 0.05,
        };
        assert!(codes(ok).is_empty());
    }

    #[test]
    fn unstable_parameters_are_reported() {
        let low_tau = StabilityInput {
            nu: 1e-4,
            alpha: None,
            u_max: 0.001,
        };
        assert_eq!(codes(low_tau), vec![(Severity::Error, "stability.tau")]);

        let near_half = StabilityInput {
            nu: 0.002,
            alpha: Some(1e-5),
            u_max: 0.05,
        };
        assert_eq!(
            codes(near_half),
            vec![
                (Severity::Warning, "stability.tau"),
                (Severity::Error, "stability.tauThermal"),
            ]
        );

        let fast = StabilityInput {
            nu: 0.1,
            alpha: None,
            u_max: 0.2,
        };
        assert_eq!(codes(fast), vec![(Severity::Warning, "stability.mach")]);

        // With Ma < 0.17, a high grid Reynolds number implies τ close to 0.5.
        let coarse = StabilityInput {
            nu: 0.002,
            alpha: None,
            u_max: 0.08,
        };
        assert_eq!(
            codes(coarse),
            vec![
                (Severity::Warning, "stability.tau"),
                (Severity::Warning, "stability.gridRe"),
            ]
        );
    }
}
