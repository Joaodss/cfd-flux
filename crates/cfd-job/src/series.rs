//! The three CSV time series written every sample interval: `probes.csv`, `forces.csv` and
//! `diagnostics.csv`.

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::Result;
use cfd_core::domain::Domain;
use cfd_core::solver::{Diagnostics, Solver};
use cfd_core::Scene;

use crate::store::ResultDir;

/// Column-safe version of a name: letters, digits, `-` and `_`.
fn column_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub(crate) struct Series {
    probes: BufWriter<File>,
    forces: BufWriter<File>,
    diagnostics: BufWriter<File>,
    /// Element id → column name, in column order.
    force_columns: Vec<(u16, String)>,
    thermal: bool,
}

impl Series {
    pub fn create(dir: &ResultDir, scene: &Scene, domain: &Domain) -> Result<Self> {
        let thermal = domain.physics.thermal();
        let mut probes = dir.create_file("probes.csv")?;
        write!(probes, "step,time_s")?;
        for p in &domain.probes {
            let n = column_name(&p.name);
            write!(probes, ",{n}.ux_m_s,{n}.uy_m_s,{n}.p_Pa")?;
            if thermal {
                write!(probes, ",{n}.T_K")?;
            }
        }
        writeln!(probes)?;

        // Every element that owns a bounce-back slot can receive a force; 0 = domain edges.
        let mut ids: Vec<u16> = domain.slot_element.clone();
        ids.sort_unstable();
        ids.dedup();
        let force_columns: Vec<(u16, String)> = ids
            .into_iter()
            .map(|id| {
                let name = match scene.elements.iter().find(|e| e.id == id) {
                    _ if id == 0 => "edges".to_string(),
                    Some(e) => e.name.clone().unwrap_or_else(|| format!("element{id}")),
                    None => format!("element{id}"),
                };
                (id, column_name(&name))
            })
            .collect();
        let mut forces = dir.create_file("forces.csv")?;
        write!(forces, "step,time_s")?;
        for (_, n) in &force_columns {
            write!(forces, ",{n}.fx_N_m,{n}.fy_N_m")?;
        }
        writeln!(forces)?;

        let mut diagnostics = dir.create_file("diagnostics.csv")?;
        write!(
            diagnostics,
            "step,time_s,mass_lu,kinetic_energy_lu,max_velocity_m_s,max_mach"
        )?;
        if thermal {
            write!(diagnostics, ",thermal_energy_lu")?;
        }
        writeln!(diagnostics)?;
        Ok(Self {
            probes,
            forces,
            diagnostics,
            force_columns,
            thermal,
        })
    }

    pub fn sample(&mut self, solver: &mut dyn Solver, diag: &Diagnostics) -> Result<()> {
        let domain = solver.domain();
        let (step, time) = (diag.step, domain.units.steps_to_time(diag.step));
        let units = domain.units;

        write!(
            self.diagnostics,
            "{step},{time},{},{},{},{}",
            diag.mass,
            diag.kinetic_energy,
            units.velocity_to_physical(diag.max_velocity),
            diag.max_mach()
        )?;
        if let Some(e) = diag.thermal_energy {
            write!(self.diagnostics, ",{e}")?;
        }
        writeln!(self.diagnostics)?;

        let forces = solver.forces();
        write!(self.forces, "{step},{time}")?;
        for (id, _) in &self.force_columns {
            let f = forces
                .iter()
                .find(|f| f.element == *id)
                .map_or([0.0; 2], |f| f.physical(&units));
            write!(self.forces, ",{},{}", f[0], f[1])?;
        }
        writeln!(self.forces)?;

        let probes = solver.probes();
        write!(self.probes, "{step},{time}")?;
        for p in &probes {
            write!(
                self.probes,
                ",{},{},{}",
                p.velocity[0], p.velocity[1], p.pressure
            )?;
            if self.thermal {
                write!(self.probes, ",{}", p.temperature.unwrap_or(f64::NAN))?;
            }
        }
        writeln!(self.probes)?;
        Ok(())
    }

    pub fn flush(&mut self) -> Result<()> {
        self.probes.flush()?;
        self.forces.flush()?;
        self.diagnostics.flush()?;
        Ok(())
    }
}
