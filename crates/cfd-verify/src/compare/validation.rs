//! The validation geometries of `cfd_verify::scenes`, set up for `cfd-cli compare --validation`:
//! the scene, lattice velocity and initial state of each validation case, at a moderate
//! resolution and for a fixed number of steps (the comparison judges the backends against each
//! other, not the physics, which `cfd-cli verify` does).

use anyhow::{anyhow, Result};
use cfd_core::{Domain, DomainOptions, Scene};

use super::{element_names, Mode, Plan, Setup};
use crate::parity;
use crate::scenes::{self, ST_DIAMETER};

fn domain(scene: &Scene, lattice_velocity: f64, characteristic: Option<f64>) -> Result<Domain> {
    let opts = DomainOptions {
        lattice_velocity,
        characteristic_velocity: characteristic,
    };
    Domain::from_scene(scene, &opts).map_err(|r| {
        let msgs: Vec<String> = r
            .errors()
            .map(|i| format!("[{}] {}", i.code, i.message))
            .collect();
        anyhow!("{}: {}", scene.name, msgs.join("; "))
    })
}

fn setup(
    id: &str,
    title: &str,
    description: String,
    scene: &Scene,
    domain: Domain,
    steps: u64,
) -> Setup {
    let plan = Plan::new(&domain, steps);
    Setup {
        id: id.into(),
        title: title.into(),
        description,
        domain,
        initial: None,
        plan,
        element_names: element_names(scene),
    }
}

/// Every validation geometry, in the order of the validation report.
pub fn validation_set() -> Result<Vec<Setup>> {
    let mut out = Vec::new();

    // Taylor-Green vortex: periodic box, analytical initial fields, one viscous time.
    {
        let (n, nu, u0) = (64, 0.01, 0.128);
        let u_lb = 0.64 / n as f64;
        let s = scenes::periodic_box(n, nu);
        let d = domain(&s, u_lb, Some(u0))?;
        let t_visc = 1.0 / (2.0 * nu * (2.0 * std::f64::consts::PI).powi(2));
        let steps = (t_visc / d.units.dt).round() as u64;
        let mut c = setup(
            "taylor-green",
            "Taylor-Green vortex",
            format!(
                "Periodic {n}² box, Re = {:.1}, analytical initial fields, one viscous time.",
                u0 / nu
            ),
            &s,
            d,
            steps,
        );
        c.initial = Some(parity::taylor_green(n, u_lb as f32));
        out.push(c);
    }

    // Plane channels: Poiseuille (body force) and Couette (moving wall), one diffusive time.
    let n = 32;
    {
        let s = scenes::plane_channel(n, 1.0, 0.0);
        let mut d = domain(&s, 1.0 / (6.0 * n as f64), Some(1.0))?;
        d.physics.body_force = [d.units.acceleration_to_lattice(8.0), 0.0];
        out.push(setup(
            "poiseuille",
            "Plane Poiseuille flow (body force)",
            format!("Channel of {n} cells, body force, τ = 1, one diffusive time 6N²."),
            &s,
            d,
            6 * (n * n) as u64,
        ));
    }
    {
        let s = scenes::plane_channel(n, 1.0, 1.0);
        let d = domain(&s, 1.0 / (6.0 * n as f64), None)?;
        out.push(setup(
            "couette",
            "Plane Couette flow (moving wall)",
            format!("Channel of {n} cells, moving top wall, τ = 1, one diffusive time 6N²."),
            &s,
            d,
            6 * (n * n) as u64,
        ));
    }

    // Lid-driven cavity.
    for re in [100, 400, 1000] {
        let n = 128;
        let s = scenes::lid_driven_cavity(n, re as f64);
        let d = domain(&s, 0.08, None)?;
        out.push(setup(
            &format!("cavity-re{re}"),
            &format!("Lid-driven cavity, Re = {re}"),
            format!("{n}² cavity, lid lattice velocity 0.08, 20000 steps (the flow is still developing)."),
            &s,
            d,
            20_000,
        ));
    }

    // Cylinder in a channel (Schäfer-Turek), D = 20 cells.
    {
        let (u_max, u_mean) = (0.3, 0.2);
        let s = scenes::schafer_turek(20, u_max);
        let d = domain(&s, 0.08, None)?;
        let mut c = setup(
            "cylinder-re20",
            "Cylinder in a channel, Re = 20",
            "Schäfer-Turek 2D-1 geometry, D = 20 cells, 20000 steps (approaching the steady state). \
             Coefficients with L = D and U = Ū."
                .into(),
            &s,
            d,
            20_000,
        );
        (c.plan.ref_length, c.plan.ref_velocity) = (ST_DIAMETER, u_mean);
        out.push(c);
    }
    {
        let (u_max, u_mean) = (1.5, 1.0);
        let s = scenes::schafer_turek(20, u_max);
        let d = domain(&s, 0.08, None)?;
        let dt = d.units.dt;
        let steps = (20.0 / dt).round() as u64;
        let mut c = setup(
            "cylinder-re100",
            "Cylinder in a channel, Re = 100 (statistics)",
            "Schäfer-Turek 2D-2 geometry, D = 20 cells, 20 s of simulated time (≈ 60 shedding \
             periods). The vortex street is periodic from t ≈ 7 s; by then round-off differences \
             have grown enough for the instantaneous fields of backends that are not bit-identical \
             to differ, so the verdict uses the force statistics from t = 10 s (≈ 30 periods, \
             ≈ 1000 samples per second): St = fD/Ū and the mean, minimum and maximum of the drag \
             and lift."
                .into(),
            &s,
            d,
            steps,
        );
        c.plan.mode = Mode::Statistics;
        c.plan.stats_from = (10.0 / dt).round() as u64;
        c.plan.sample_every = ((1.0 / dt) / 1000.0).round().max(1.0) as u64;
        (c.plan.ref_length, c.plan.ref_velocity) = (ST_DIAMETER, u_mean);
        out.push(c);
    }

    // Differentially heated cavity.
    for exp in 3..=6 {
        let (n, ra) = (64, 10f64.powi(exp));
        let s = scenes::heated_cavity(n, ra, 0.71);
        let d = domain(&s, 0.08, None)?;
        out.push(setup(
            &format!("heated-cavity-ra1e{exp}"),
            &format!("Differentially heated cavity, Ra = 1e{exp}"),
            format!("{n}² cavity, Pr = 0.71, buoyancy lattice velocity 0.08, 20000 steps."),
            &s,
            d,
            20_000,
        ));
    }

    // Rayleigh-Bénard: a perturbed conduction profile above the onset.
    {
        let (h, ra) = (24, 2e4);
        let s = scenes::rayleigh_benard(h, ra, 0.71);
        let d = domain(&s, 0.08, None)?;
        let mut init = cfd_lbm::shared::initial_fields(&d);
        let w = d.width as usize;
        if let Some(theta) = init.theta.as_mut() {
            for (i, t) in theta.iter_mut().enumerate() {
                let (x, y) = (i % w, i / w);
                *t += 0.1 * ((x as f32 * 0.3).sin() + (y as f32 * 0.2).cos());
            }
        }
        let mut c = setup(
            "rayleigh-benard",
            "Rayleigh-Bénard convection",
            format!(
                "Layer of height {h} cells heated from below, Ra = {ra:.0}, Pr = 0.71, perturbed \
                 conduction profile, 10000 steps (convection rolls developing)."
            ),
            &s,
            d,
            10_000,
        );
        c.initial = Some(init);
        out.push(c);
    }
    Ok(out)
}
