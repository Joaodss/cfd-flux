//! Colour-mapped images of scalar fields, encoded as PNG.
//!
//! Field values are row-major from the bottom-left corner (as in frames and scenes); images are
//! stored top-down (as in PNG), so rendering flips the rows.

/// Perceptual colour maps, as piecewise-linear interpolation between 9 samples.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Colormap {
    /// Sequential (matplotlib's viridis): velocity magnitude.
    Viridis,
    /// Sequential (matplotlib's inferno): temperature.
    Inferno,
    /// Diverging (Moreland's cool-warm), white-ish at the centre: vorticity.
    Coolwarm,
}

const VIRIDIS: [[u8; 3]; 9] = [
    [68, 1, 84],
    [72, 40, 120],
    [62, 73, 137],
    [49, 104, 142],
    [38, 130, 142],
    [31, 158, 137],
    [53, 183, 121],
    [110, 206, 88],
    [253, 231, 37],
];

const INFERNO: [[u8; 3]; 9] = [
    [0, 0, 4],
    [31, 12, 72],
    [85, 15, 109],
    [136, 34, 106],
    [186, 54, 85],
    [227, 89, 51],
    [249, 142, 9],
    [249, 203, 53],
    [252, 255, 164],
];

const COOLWARM: [[u8; 3]; 9] = [
    [59, 76, 192],
    [98, 130, 234],
    [141, 176, 254],
    [184, 208, 249],
    [221, 221, 221],
    [245, 196, 173],
    [244, 154, 123],
    [222, 96, 77],
    [180, 4, 38],
];

/// Colour of cells outside the fluid (solids, inlets, outlets).
pub const MASK_COLOUR: [u8; 3] = [96, 96, 96];

impl Colormap {
    pub fn name(self) -> &'static str {
        match self {
            Colormap::Viridis => "viridis",
            Colormap::Inferno => "inferno",
            Colormap::Coolwarm => "coolwarm",
        }
    }

    fn samples(self) -> &'static [[u8; 3]; 9] {
        match self {
            Colormap::Viridis => &VIRIDIS,
            Colormap::Inferno => &INFERNO,
            Colormap::Coolwarm => &COOLWARM,
        }
    }

    /// Colour at `t ∈ [0, 1]` (clamped; NaN maps to 0).
    pub fn rgb(self, t: f32) -> [u8; 3] {
        let s = self.samples();
        let t = if t.is_nan() { 0.0 } else { t.clamp(0.0, 1.0) };
        let x = t * (s.len() - 1) as f32;
        let i = (x as usize).min(s.len() - 2);
        let f = x - i as f32;
        std::array::from_fn(|c| {
            let (a, b) = (s[i][c] as f32, s[i + 1][c] as f32);
            (a + (b - a) * f).round() as u8
        })
    }
}

/// An 8-bit RGB image, rows top-down.
#[derive(Debug, Clone, PartialEq)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    #[error("field has {found} values, expected {expected}")]
    Length { expected: usize, found: usize },
    #[error("PNG encoding failed: {0}")]
    Png(#[from] png::EncodingError),
}

/// How to map a scalar field to colours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColourScale {
    pub colormap: Colormap,
    pub min: f32,
    pub max: f32,
}

impl ColourScale {
    fn t(&self, v: f32) -> f32 {
        let span = self.max - self.min;
        if span > 0.0 {
            (v - self.min) / span
        } else {
            0.5
        }
    }
}

/// Renders a scalar field of `width × height` values (rows bottom-up) into an image with
/// `scale × scale` pixels per cell. Cells where `mask` is `false` are drawn in [`MASK_COLOUR`].
pub fn render(
    values: &[f32],
    width: u32,
    height: u32,
    colours: &ColourScale,
    mask: Option<&[bool]>,
    scale: u32,
) -> Result<Image, ImageError> {
    let (w, h, s) = (width as usize, height as usize, scale.max(1) as usize);
    for len in std::iter::once(values.len()).chain(mask.map(<[bool]>::len)) {
        if len != w * h {
            return Err(ImageError::Length {
                expected: w * h,
                found: len,
            });
        }
    }
    let mut rgb = Vec::with_capacity(w * h * s * s * 3);
    for y in (0..h).rev() {
        let row: Vec<u8> = (0..w)
            .flat_map(|x| {
                let i = y * w + x;
                let c = match mask {
                    Some(m) if !m[i] => MASK_COLOUR,
                    _ => colours.colormap.rgb(colours.t(values[i])),
                };
                std::iter::repeat_n(c, s).flatten()
            })
            .collect();
        for _ in 0..s {
            rgb.extend_from_slice(&row);
        }
    }
    Ok(Image {
        width: (w * s) as u32,
        height: (h * s) as u32,
        rgb,
    })
}

impl Image {
    pub fn encode_png(&self) -> Result<Vec<u8>, ImageError> {
        let mut out = Vec::new();
        let mut encoder = png::Encoder::new(&mut out, self.width, self.height);
        encoder.set_color(png::ColorType::Rgb);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_compression(png::Compression::Fast);
        encoder.write_header()?.write_image_data(&self.rgb)?;
        Ok(out)
    }
}

/// Values at the `lo` and `hi` quantiles (0…1) of the finite values, using a sort of at most
/// `max_samples` evenly strided values (enough for a colour range).
pub fn quantile_range(values: &[f32], lo: f64, hi: f64, max_samples: usize) -> Option<(f32, f32)> {
    let stride = values.len().div_ceil(max_samples.max(1)).max(1);
    let mut v: Vec<f32> = values
        .iter()
        .step_by(stride)
        .copied()
        .filter(|x| x.is_finite())
        .collect();
    if v.is_empty() {
        return None;
    }
    v.sort_by(f32::total_cmp);
    let at = |q: f64| v[((v.len() - 1) as f64 * q.clamp(0.0, 1.0)).round() as usize];
    Some((at(lo), at(hi)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colormaps_hit_their_end_samples() {
        assert_eq!(Colormap::Viridis.rgb(0.0), VIRIDIS[0]);
        assert_eq!(Colormap::Viridis.rgb(1.0), VIRIDIS[8]);
        assert_eq!(Colormap::Coolwarm.rgb(0.5), COOLWARM[4]);
        assert_eq!(Colormap::Inferno.rgb(2.0), INFERNO[8]);
        assert_eq!(Colormap::Inferno.rgb(f32::NAN), INFERNO[0]);
        // Half-way between two samples.
        let mid = Colormap::Viridis.rgb(1.0 / 16.0);
        assert_eq!(mid, [70, 21, 102]);
    }

    #[test]
    fn render_flips_rows_scales_and_masks() {
        // 2 × 2 field: bottom row (0, 1), top row (2, 3).
        let values = [0.0, 1.0, 2.0, 3.0];
        let mask = [true, true, false, true];
        let cs = ColourScale {
            colormap: Colormap::Viridis,
            min: 0.0,
            max: 3.0,
        };
        let img = render(&values, 2, 2, &cs, Some(&mask), 2).unwrap();
        assert_eq!((img.width, img.height), (4, 4));
        let px = |x: usize, y: usize| &img.rgb[(y * 4 + x) * 3..(y * 4 + x) * 3 + 3];
        assert_eq!(px(0, 0), MASK_COLOUR); // top-left = cell (0, 1), masked
        assert_eq!(px(3, 1), VIRIDIS[8]); // top-right = cell (1, 1) = max
        assert_eq!(px(1, 3), VIRIDIS[0]); // bottom-left = cell (0, 0) = min
        let png = img.encode_png().unwrap();
        assert_eq!(&png[1..4], b"PNG");
        assert!(render(&values, 3, 2, &cs, None, 1).is_err());
    }

    #[test]
    fn quantiles_ignore_non_finite_values() {
        let mut v: Vec<f32> = (0..=100).map(|i| i as f32).collect();
        v.push(f32::NAN);
        assert_eq!(quantile_range(&v, 0.0, 1.0, 1000), Some((0.0, 100.0)));
        assert_eq!(quantile_range(&v, 0.1, 0.9, 1000), Some((10.0, 90.0)));
        assert_eq!(quantile_range(&[f32::NAN], 0.0, 1.0, 10), None);
    }
}
