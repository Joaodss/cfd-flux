//! The three CSV time series written every sample interval: `probes.csv`, `forces.csv` and
//! `diagnostics.csv`. Each sample is also handed to the observer as numbers ([`SeriesRow`]), the
//! same values as the CSV line, so a viewer can plot them live (`series` messages).

use std::fs::File;
use std::io::{BufWriter, Write};

use anyhow::Result;
use cfd_core::domain::Domain;
use cfd_core::heat::HeatWalls;
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

/// The column names of the three series (their CSV headers).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct SeriesColumns {
    pub forces: Vec<String>,
    pub probes: Vec<String>,
    pub diagnostics: Vec<String>,
    /// `heat.csv` (thermal runs with stored frames); empty otherwise.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub heat: Vec<String>,
}

/// One sample of every series: `[step, time, …]` in the order of [`SeriesColumns`]. A missing
/// value (a probe's temperature outside the fluid) is `NaN`, written as `NaN` in the CSV.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct SeriesRow {
    pub forces: Vec<f64>,
    pub probes: Vec<f64>,
    pub diagnostics: Vec<f64>,
}

pub(crate) struct Series {
    probes: BufWriter<File>,
    forces: BufWriter<File>,
    diagnostics: BufWriter<File>,
    /// Element id → column name, in column order.
    force_columns: Vec<(u16, String)>,
    thermal: bool,
    columns: SeriesColumns,
    heat: Option<HeatSeries>,
}

/// The name of an element in the columns of the series: its name, or `element{id}`; 0 is the
/// implicit wall at the domain edges.
fn element_column(scene: &Scene, id: u16) -> String {
    let name = match scene.elements.iter().find(|e| e.id == id) {
        _ if id == 0 => "edges".to_string(),
        Some(e) => e.name.clone().unwrap_or_else(|| format!("element{id}")),
        None => format!("element{id}"),
    };
    column_name(&name)
}

/// Writes `values` as one CSV line. `f64`'s `Display` is the shortest text that reads back as
/// the same number, and whole numbers have no `.0` (the step column stays an integer).
fn write_row(w: &mut impl Write, values: &[f64]) -> Result<()> {
    for (i, v) in values.iter().enumerate() {
        if i > 0 {
            w.write_all(b",")?;
        }
        write!(w, "{v}")?;
    }
    writeln!(w)?;
    Ok(())
}

fn write_header(w: &mut impl Write, columns: &[String]) -> Result<()> {
    writeln!(w, "{}", columns.join(","))?;
    Ok(())
}

/// `heat.csv`: the heat through every thermal wall element at each stored frame (design proposal
/// §7.3). Unlike the three series it is written by the *output thread*, from the stored frame's
/// temperature, so it is engine-independent; the thread flushes each line before handing it to
/// the observer, so every row an observer has been given is on disk.
pub(crate) struct HeatSeries {
    walls: HeatWalls,
    file: BufWriter<File>,
}

impl HeatSeries {
    /// Creates `heat.csv` (header only) if the scene has thermal walls; the columns are
    /// `step, time_s, {element}.q_W_m, {element}.length_m` for each, in element order.
    fn create(dir: &ResultDir, scene: &Scene) -> Result<Option<(Self, Vec<String>)>> {
        let Some(walls) = HeatWalls::new(scene) else {
            return Ok(None);
        };
        let mut columns = vec!["step".to_string(), "time_s".to_string()];
        for id in walls.element_ids() {
            let name = element_column(scene, id);
            columns.push(format!("{name}.q_W_m"));
            columns.push(format!("{name}.length_m"));
        }
        let mut file = dir.create_file("heat.csv")?;
        write_header(&mut file, &columns)?;
        file.flush()?;
        Ok(Some((Self { walls, file }, columns)))
    }

    /// Measures the walls on a temperature field, writes (and flushes) the line, and returns it:
    /// `[step, time, q₁, length₁, q₂, length₂, …]`.
    pub fn row(&mut self, step: u64, time: f64, temperature: &[f32]) -> Result<Vec<f64>> {
        let mut row = vec![step as f64, time];
        for h in self.walls.measure(temperature) {
            row.extend([h.q_w_per_m, h.length_m]);
        }
        write_row(&mut self.file, &row)?;
        self.file.flush()?;
        Ok(row)
    }
}

impl Series {
    /// With `heat`, also creates `heat.csv` if the scene has thermal walls; its writer is for
    /// [`take_heat`](Self::take_heat).
    pub fn create(dir: &ResultDir, scene: &Scene, domain: &Domain, heat: bool) -> Result<Self> {
        let thermal = domain.physics.thermal();
        let start = || vec!["step".to_string(), "time_s".to_string()];
        let mut probe_columns = start();
        for p in &domain.probes {
            let n = column_name(&p.name);
            for q in ["ux_m_s", "uy_m_s", "p_Pa"] {
                probe_columns.push(format!("{n}.{q}"));
            }
            if thermal {
                probe_columns.push(format!("{n}.T_K"));
            }
        }

        // Every element that owns a bounce-back slot can receive a force; 0 = domain edges.
        let mut ids: Vec<u16> = domain.slot_element.clone();
        ids.sort_unstable();
        ids.dedup();
        let force_columns: Vec<(u16, String)> = ids
            .into_iter()
            .map(|id| (id, element_column(scene, id)))
            .collect();
        let mut forces_columns = start();
        for (_, n) in &force_columns {
            forces_columns.push(format!("{n}.fx_N_m"));
            forces_columns.push(format!("{n}.fy_N_m"));
        }
        let mut diagnostics_columns = start();
        for c in [
            "mass_lu",
            "kinetic_energy_lu",
            "max_velocity_m_s",
            "max_mach",
        ] {
            diagnostics_columns.push(c.into());
        }
        if thermal {
            diagnostics_columns.push("thermal_energy_lu".into());
        }
        let (heat, heat_columns) = match heat {
            true => HeatSeries::create(dir, scene)?.unzip(),
            false => (None, None),
        };
        let columns = SeriesColumns {
            forces: forces_columns,
            probes: probe_columns,
            diagnostics: diagnostics_columns,
            heat: heat_columns.unwrap_or_default(),
        };

        let mut probes = dir.create_file("probes.csv")?;
        write_header(&mut probes, &columns.probes)?;
        let mut forces = dir.create_file("forces.csv")?;
        write_header(&mut forces, &columns.forces)?;
        let mut diagnostics = dir.create_file("diagnostics.csv")?;
        write_header(&mut diagnostics, &columns.diagnostics)?;
        let mut series = Self {
            probes,
            forces,
            diagnostics,
            force_columns,
            thermal,
            columns,
            heat,
        };
        // The headers are on disk from the start, for viewers that read the files.
        series.flush()?;
        Ok(series)
    }

    pub fn columns(&self) -> &SeriesColumns {
        &self.columns
    }

    /// The writer of `heat.csv`, for the output thread (which owns it from then on).
    pub fn take_heat(&mut self) -> Option<HeatSeries> {
        self.heat.take()
    }

    /// Writes one sample of every series and returns it.
    pub fn sample(&mut self, solver: &mut dyn Solver, diag: &Diagnostics) -> Result<SeriesRow> {
        let domain = solver.domain();
        let (step, time) = (diag.step as f64, domain.units.steps_to_time(diag.step));
        let units = domain.units;

        let mut diagnostics = vec![
            step,
            time,
            diag.mass,
            diag.kinetic_energy,
            units.velocity_to_physical(diag.max_velocity),
            diag.max_mach(),
        ];
        diagnostics.extend(diag.thermal_energy);

        let all = solver.forces();
        let mut forces = vec![step, time];
        for (id, _) in &self.force_columns {
            let f = all
                .iter()
                .find(|f| f.element == *id)
                .map_or([0.0; 2], |f| f.physical(&units));
            forces.extend(f);
        }

        let mut probes = vec![step, time];
        for p in &solver.probes() {
            probes.extend([p.velocity[0], p.velocity[1], p.pressure]);
            if self.thermal {
                probes.push(p.temperature.unwrap_or(f64::NAN));
            }
        }

        write_row(&mut self.diagnostics, &diagnostics)?;
        write_row(&mut self.forces, &forces)?;
        write_row(&mut self.probes, &probes)?;
        Ok(SeriesRow {
            forces,
            probes,
            diagnostics,
        })
    }

    /// Puts every complete line on disk (a viewer that joins late reads the files, then appends
    /// the rows it is sent after them).
    pub fn flush(&mut self) -> Result<()> {
        self.probes.flush()?;
        self.forces.flush()?;
        self.diagnostics.flush()?;
        Ok(())
    }
}
