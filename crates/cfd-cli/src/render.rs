//! `cfd-cli render`: PNG sequences from the frames of a run directory.
//!
//! Each field gets one colour range for the whole sequence (so animations do not flicker),
//! taken from robust quantiles over every frame unless given with `--range`.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use cfd_core::scene::CellType;
use cfd_core::Scene;
use cfd_io::frame::Frame;
use cfd_io::image::{self, Colormap, ColourScale};
use clap::{Args, ValueEnum};
use rayon::prelude::*;
use serde_json::json;

/// Scalar fields that can be rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ValueEnum)]
pub enum PngField {
    /// Velocity magnitude.
    Velocity,
    Vorticity,
    Temperature,
    Pressure,
}

impl PngField {
    pub fn name(self) -> &'static str {
        match self {
            PngField::Velocity => "velocity",
            PngField::Vorticity => "vorticity",
            PngField::Temperature => "temperature",
            PngField::Pressure => "pressure",
        }
    }

    /// Name of the frame field it is computed from.
    pub fn source(self) -> &'static str {
        self.name()
    }

    fn colormap(self) -> Colormap {
        match self {
            PngField::Velocity => Colormap::Viridis,
            PngField::Vorticity => Colormap::Coolwarm,
            PngField::Temperature => Colormap::Inferno,
            PngField::Pressure => Colormap::Coolwarm,
        }
    }

    fn unit(self) -> &'static str {
        match self {
            PngField::Velocity => "m/s",
            PngField::Vorticity => "1/s",
            PngField::Temperature => "K",
            PngField::Pressure => "Pa",
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct RenderOpts {
    /// Fields to render (default: velocity, vorticity, and temperature when present).
    #[arg(long, value_enum, value_delimiter = ',')]
    pub png: Option<Vec<PngField>>,
    /// Do not write PNG images.
    #[arg(long)]
    pub no_png: bool,
    /// Pixels per cell (default: enough for ~800 px on the long side, at most 16).
    #[arg(long)]
    pub scale: Option<u32>,
    /// Fixed colour range for a field, e.g. `vorticity=-50:50` (repeatable).
    #[arg(long, value_parser = parse_range, value_name = "FIELD=MIN:MAX")]
    pub range: Vec<(PngField, f32, f32)>,
}

fn parse_range(s: &str) -> Result<(PngField, f32, f32), String> {
    let (field, rest) = s.split_once('=').ok_or("expected FIELD=MIN:MAX")?;
    let (lo, hi) = rest.split_once(':').ok_or("expected FIELD=MIN:MAX")?;
    let field = PngField::from_str(field, true)?;
    let lo: f32 = lo.parse().map_err(|e| format!("{e}"))?;
    let hi: f32 = hi.parse().map_err(|e| format!("{e}"))?;
    if lo.is_nan() || hi.is_nan() || hi <= lo {
        return Err("MAX must be greater than MIN".into());
    }
    Ok((field, lo, hi))
}

impl RenderOpts {
    /// The fields to render, given whether the run is thermal.
    pub fn fields(&self, thermal: bool) -> Vec<PngField> {
        if self.no_png {
            return Vec::new();
        }
        match &self.png {
            Some(f) => f.clone(),
            None if thermal => vec![
                PngField::Velocity,
                PngField::Vorticity,
                PngField::Temperature,
            ],
            None => vec![PngField::Velocity, PngField::Vorticity],
        }
    }
}

/// Scalar values of `field` in a frame (velocity → magnitude); `None` if the frame lacks it.
fn scalar(frame: &Frame, field: PngField) -> Option<Vec<f32>> {
    let f = frame
        .fields
        .iter()
        .find(|f| f.desc.name == field.source())?;
    Some(match f.desc.components {
        1 => f.values.clone(),
        _ => f
            .values
            .chunks_exact(f.desc.components as usize)
            .map(|c| c.iter().map(|v| v * v).sum::<f32>().sqrt())
            .collect(),
    })
}

fn read_frame(path: &Path) -> Result<Frame> {
    let bytes = std::fs::read(path).with_context(|| format!("cannot read {}", path.display()))?;
    Frame::decode(&bytes).with_context(|| format!("invalid frame {}", path.display()))
}

/// Colour range from quantiles of the fluid values of every frame.
fn auto_range(field: PngField, samples: &[f32]) -> (f32, f32) {
    const MAX_SORT: usize = 4_000_000;
    let range = match field {
        PngField::Velocity => {
            image::quantile_range(samples, 0.0, 0.995, MAX_SORT).map(|r| (0.0, r.1))
        }
        PngField::Vorticity => {
            let abs: Vec<f32> = samples.iter().map(|v| v.abs()).collect();
            image::quantile_range(&abs, 0.0, 0.99, MAX_SORT).map(|r| (-r.1, r.1))
        }
        PngField::Temperature => image::quantile_range(samples, 0.0, 1.0, MAX_SORT),
        PngField::Pressure => image::quantile_range(samples, 0.005, 0.995, MAX_SORT),
    };
    match range {
        Some((lo, hi)) if hi > lo => (lo, hi),
        Some((lo, _)) => (lo - 0.5, lo + 0.5),
        None => (0.0, 1.0),
    }
}

/// Renders the frames in `dir/frames` to `dir/png/<field>/NNNNNN.png` and records the colour
/// ranges in `dir/png/ranges.json`. Returns the number of images written.
pub fn render_dir(dir: &Path, opts: &RenderOpts) -> Result<usize> {
    let scene_path = dir.join("scene.json");
    let scene = Scene::from_json(
        &std::fs::read_to_string(&scene_path)
            .with_context(|| format!("cannot read {}", scene_path.display()))?,
    )
    .with_context(|| format!("invalid scene {}", scene_path.display()))?;
    let cells = scene.grid.cell_count();
    let mask: Vec<bool> = scene
        .layers
        .cell_type
        .decode_u8(cells)
        .context("invalid cellType layer")?
        .into_iter()
        .map(|v| v == CellType::Fluid as u8)
        .collect();

    let mut frames: Vec<PathBuf> = std::fs::read_dir(dir.join("frames"))
        .with_context(|| format!("no frames in {}", dir.display()))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "bin"))
        .collect();
    frames.sort();
    if frames.is_empty() {
        bail!("no frames in {}", dir.join("frames").display());
    }

    let first = read_frame(&frames[0])?;
    if (first.width, first.height) != (scene.grid.width, scene.grid.height) {
        bail!("frames and scene.json have different grid sizes");
    }
    let fields: Vec<PngField> = opts
        .fields(scene.physics.thermal)
        .into_iter()
        .filter(|&f| scalar(&first, f).is_some())
        .collect();
    if fields.is_empty() {
        return Ok(0);
    }
    let scale = opts.scale.unwrap_or_else(|| {
        let long = first.width.max(first.height).max(1);
        800u32.div_ceil(long).clamp(1, 16)
    });

    // Pass 1: subsample the fluid values of every frame, field by field.
    let fluid: Vec<usize> = (0..cells).filter(|&i| mask[i]).collect();
    let stride = (fluid.len() * frames.len()).div_ceil(8_000_000).max(1);
    let per_frame: Vec<Vec<Vec<f32>>> = frames
        .par_iter()
        .map(|p| -> Result<Vec<Vec<f32>>> {
            let frame = read_frame(p)?;
            Ok(fields
                .iter()
                .map(|&f| {
                    let v = scalar(&frame, f).unwrap_or_default();
                    fluid
                        .iter()
                        .step_by(stride)
                        .filter_map(|&i| v.get(i).copied())
                        .collect()
                })
                .collect())
        })
        .collect::<Result<_>>()?;
    let scales: Vec<ColourScale> = fields
        .iter()
        .enumerate()
        .map(|(k, &f)| {
            let (min, max) = match opts.range.iter().find(|r| r.0 == f) {
                Some(&(_, lo, hi)) => (lo, hi),
                None => {
                    let all: Vec<f32> = per_frame
                        .iter()
                        .flat_map(|v| v[k].iter().copied())
                        .collect();
                    auto_range(f, &all)
                }
            };
            ColourScale {
                colormap: f.colormap(),
                min,
                max,
            }
        })
        .collect();

    // Pass 2: render.
    for f in &fields {
        std::fs::create_dir_all(dir.join("png").join(f.name()))?;
    }
    frames.par_iter().try_for_each(|p| -> Result<()> {
        let frame = read_frame(p)?;
        let stem = p.file_stem().expect("frame file name");
        for (f, cs) in fields.iter().zip(&scales) {
            let Some(values) = scalar(&frame, *f) else {
                continue;
            };
            let img = image::render(&values, frame.width, frame.height, cs, Some(&mask), scale)?;
            let out = dir
                .join("png")
                .join(f.name())
                .join(stem)
                .with_extension("png");
            std::fs::write(&out, img.encode_png()?)
                .with_context(|| format!("cannot write {}", out.display()))?;
        }
        Ok(())
    })?;

    // Record the colour ranges (the run's meta.json is left untouched).
    let ranges: serde_json::Map<_, _> = fields
        .iter()
        .zip(&scales)
        .map(|(f, cs)| {
            (
                f.name().to_string(),
                json!({
                    "colormap": cs.colormap.name(),
                    "min": cs.min,
                    "max": cs.max,
                    "unit": f.unit(),
                }),
            )
        })
        .collect();
    let info = json!({ "pixelsPerCell": scale, "images": frames.len(), "fields": ranges });
    std::fs::write(
        dir.join("png").join("ranges.json"),
        serde_json::to_string_pretty(&info)? + "\n",
    )?;
    for (f, cs) in fields.iter().zip(&scales) {
        eprintln!(
            "png/{}: {} images, {} range [{:.4e}, {:.4e}] {}",
            f.name(),
            frames.len(),
            cs.colormap.name(),
            cs.min,
            cs.max,
            f.unit()
        );
    }
    Ok(frames.len() * fields.len())
}
