//! Minimal SVG line plots for the validation report (no plotting dependency).

use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Style {
    Line,
    Dashed,
    Markers,
}

#[derive(Debug, Clone)]
pub struct Series {
    pub label: String,
    pub points: Vec<(f64, f64)>,
    pub style: Style,
}

impl Series {
    pub fn line(label: impl Into<String>, points: Vec<(f64, f64)>) -> Self {
        Self {
            label: label.into(),
            points,
            style: Style::Line,
        }
    }

    pub fn dashed(label: impl Into<String>, points: Vec<(f64, f64)>) -> Self {
        Self {
            label: label.into(),
            points,
            style: Style::Dashed,
        }
    }

    pub fn markers(label: impl Into<String>, points: Vec<(f64, f64)>) -> Self {
        Self {
            label: label.into(),
            points,
            style: Style::Markers,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Plot {
    /// File name of the SVG next to the report.
    pub file: String,
    pub title: String,
    pub x_label: String,
    pub y_label: String,
    pub log_x: bool,
    pub log_y: bool,
    pub series: Vec<Series>,
}

const W: f64 = 640.0;
const H: f64 = 420.0;
const LEFT: f64 = 72.0;
const RIGHT: f64 = 20.0;
const TOP: f64 = 40.0;
const BOTTOM: f64 = 56.0;
const COLOURS: [&str; 6] = [
    "#1f77b4", "#d62728", "#2ca02c", "#ff7f0e", "#9467bd", "#8c564b",
];

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// One axis: data range (in transformed coordinates when logarithmic) and ticks.
struct Axis {
    log: bool,
    lo: f64,
    hi: f64,
}

impl Axis {
    fn new(values: impl Iterator<Item = f64>, log: bool) -> Self {
        let (mut lo, mut hi) = (f64::INFINITY, f64::NEG_INFINITY);
        for v in values.filter(|v| v.is_finite() && (!log || *v > 0.0)) {
            let t = if log { v.log10() } else { v };
            lo = lo.min(t);
            hi = hi.max(t);
        }
        if !lo.is_finite() {
            (lo, hi) = (0.0, 1.0);
        }
        if hi - lo < 1e-12 {
            let pad = if lo == 0.0 { 1.0 } else { 0.1 * lo.abs() };
            (lo, hi) = (lo - pad, hi + pad);
        }
        if log {
            (lo, hi) = (lo.floor(), hi.ceil());
        } else {
            let step = nice_step((hi - lo) / 5.0);
            (lo, hi) = ((lo / step).floor() * step, (hi / step).ceil() * step);
        }
        Self { log, lo, hi }
    }

    fn t(&self, v: f64) -> f64 {
        if self.log {
            v.log10()
        } else {
            v
        }
    }

    /// Position in [0, 1].
    fn frac(&self, v: f64) -> f64 {
        (self.t(v) - self.lo) / (self.hi - self.lo)
    }

    /// Tick values (untransformed) with labels.
    fn ticks(&self) -> Vec<(f64, String)> {
        if self.log {
            let decades = (self.hi - self.lo).round() as i32;
            let mults: &[f64] = if decades <= 1 {
                &[1.0, 2.0, 5.0]
            } else {
                &[1.0]
            };
            let mut out = Vec::new();
            for e in self.lo as i32..=self.hi as i32 {
                for m in mults {
                    let v = m * 10f64.powi(e);
                    if v.log10() <= self.hi + 1e-9 {
                        out.push((v, label(v)));
                    }
                }
            }
            out
        } else {
            let step = nice_step((self.hi - self.lo) / 5.0);
            let n = ((self.hi - self.lo) / step).round() as i64;
            (0..=n)
                .map(|k| {
                    let v = self.lo + k as f64 * step;
                    let v = if v.abs() < step * 1e-9 { 0.0 } else { v };
                    (v, label(v))
                })
                .collect()
        }
    }
}

fn nice_step(raw: f64) -> f64 {
    let p = 10f64.powf(raw.log10().floor());
    let m = raw / p;
    let n = if m <= 1.0 {
        1.0
    } else if m <= 2.0 {
        2.0
    } else if m <= 5.0 {
        5.0
    } else {
        10.0
    };
    n * p
}

fn label(v: f64) -> String {
    if v == 0.0 {
        return "0".into();
    }
    let a = v.abs();
    if !(1e-3..1e5).contains(&a) {
        let e = a.log10().floor() as i32;
        let m = v / 10f64.powi(e);
        if (m.abs() - 1.0).abs() < 1e-9 {
            format!("{}1e{e}", if v < 0.0 { "-" } else { "" })
        } else {
            format!("{m:.0}e{e}")
        }
    } else {
        let s = format!("{v:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

impl Plot {
    pub fn to_svg(&self) -> String {
        let all = || self.series.iter().flat_map(|s| s.points.iter());
        let xa = Axis::new(all().map(|p| p.0), self.log_x);
        let ya = Axis::new(all().map(|p| p.1), self.log_y);
        let (pw, ph) = (W - LEFT - RIGHT, H - TOP - BOTTOM);
        let px = |x: f64| LEFT + xa.frac(x) * pw;
        let py = |y: f64| TOP + (1.0 - ya.frac(y)) * ph;

        let mut s = String::new();
        let _ = write!(
            s,
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" viewBox="0 0 {W} {H}" font-family="Helvetica, Arial, sans-serif" font-size="12">"#
        );
        let _ = write!(s, r#"<rect width="{W}" height="{H}" fill="white"/>"#);
        let _ = write!(
            s,
            r#"<text x="{}" y="22" text-anchor="middle" font-size="14" font-weight="bold">{}</text>"#,
            W / 2.0,
            escape(&self.title)
        );
        // Grid and ticks.
        for (v, l) in xa.ticks() {
            let x = px(v);
            let _ = write!(
                s,
                r##"<line x1="{x:.1}" y1="{TOP}" x2="{x:.1}" y2="{:.1}" stroke="#e5e5e5"/><text x="{x:.1}" y="{:.1}" text-anchor="middle">{l}</text>"##,
                TOP + ph,
                TOP + ph + 16.0
            );
        }
        for (v, l) in ya.ticks() {
            let y = py(v);
            let _ = write!(
                s,
                r##"<line x1="{LEFT}" y1="{y:.1}" x2="{:.1}" y2="{y:.1}" stroke="#e5e5e5"/><text x="{:.1}" y="{:.1}" text-anchor="end">{l}</text>"##,
                LEFT + pw,
                LEFT - 6.0,
                y + 4.0
            );
        }
        let _ = write!(
            s,
            r##"<rect x="{LEFT}" y="{TOP}" width="{pw}" height="{ph}" fill="none" stroke="#333"/>"##
        );
        let _ = write!(
            s,
            r#"<text x="{:.1}" y="{:.1}" text-anchor="middle">{}</text>"#,
            LEFT + pw / 2.0,
            H - 14.0,
            escape(&self.x_label)
        );
        let _ = write!(
            s,
            r#"<text transform="translate(18 {:.1}) rotate(-90)" text-anchor="middle">{}</text>"#,
            TOP + ph / 2.0,
            escape(&self.y_label)
        );

        // Series, clipped to the plot area.
        let _ = write!(
            s,
            r#"<clipPath id="area"><rect x="{LEFT}" y="{TOP}" width="{pw}" height="{ph}"/></clipPath><g clip-path="url(#area)">"#
        );
        let ok = |p: &&(f64, f64)| {
            p.0.is_finite()
                && p.1.is_finite()
                && (!self.log_x || p.0 > 0.0)
                && (!self.log_y || p.1 > 0.0)
        };
        for (k, ser) in self.series.iter().enumerate() {
            let c = COLOURS[k % COLOURS.len()];
            match ser.style {
                Style::Markers => {
                    for p in ser.points.iter().filter(ok) {
                        let _ = write!(
                            s,
                            r#"<circle cx="{:.1}" cy="{:.1}" r="3.5" fill="none" stroke="{c}" stroke-width="1.5"/>"#,
                            px(p.0),
                            py(p.1)
                        );
                    }
                }
                Style::Line | Style::Dashed => {
                    let pts: Vec<String> = ser
                        .points
                        .iter()
                        .filter(ok)
                        .map(|p| format!("{:.1},{:.1}", px(p.0), py(p.1)))
                        .collect();
                    let dash = if ser.style == Style::Dashed {
                        r#" stroke-dasharray="6 4""#
                    } else {
                        ""
                    };
                    let _ = write!(
                        s,
                        r#"<polyline points="{}" fill="none" stroke="{c}" stroke-width="1.6"{dash}/>"#,
                        pts.join(" ")
                    );
                }
            }
        }
        s.push_str("</g>");

        // Legend in the corner that hides the fewest data points.
        let lw = 12.0
            + 7.0
                * self
                    .series
                    .iter()
                    .map(|x| x.label.chars().count())
                    .max()
                    .unwrap_or(0) as f64
            + 30.0;
        let lh = 8.0 + 16.0 * self.series.len() as f64;
        let corners = [
            (LEFT + pw - lw - 8.0, TOP + 8.0),
            (LEFT + 8.0, TOP + 8.0),
            (LEFT + pw - lw - 8.0, TOP + ph - lh - 8.0),
            (LEFT + 8.0, TOP + ph - lh - 8.0),
        ];
        let hidden = |(cx, cy): (f64, f64)| {
            all()
                .filter(|p| ok(p))
                .filter(|p| {
                    let (x, y) = (px(p.0), py(p.1));
                    x >= cx && x <= cx + lw && y >= cy && y <= cy + lh
                })
                .count()
        };
        let (lx, ly) = corners
            .into_iter()
            .min_by_key(|&c| hidden(c))
            .expect("four corners");
        let _ = write!(
            s,
            r##"<rect x="{lx:.1}" y="{ly:.1}" width="{lw:.1}" height="{:.1}" fill="white" fill-opacity="0.85" stroke="#999"/>"##,
            8.0 + 16.0 * self.series.len() as f64
        );
        for (k, ser) in self.series.iter().enumerate() {
            let c = COLOURS[k % COLOURS.len()];
            let y = ly + 16.0 + 16.0 * k as f64;
            match ser.style {
                Style::Markers => {
                    let _ = write!(
                        s,
                        r#"<circle cx="{:.1}" cy="{:.1}" r="3.5" fill="none" stroke="{c}" stroke-width="1.5"/>"#,
                        lx + 18.0,
                        y - 4.0
                    );
                }
                _ => {
                    let dash = if ser.style == Style::Dashed {
                        r#" stroke-dasharray="6 4""#
                    } else {
                        ""
                    };
                    let _ = write!(
                        s,
                        r#"<line x1="{:.1}" y1="{:.1}" x2="{:.1}" y2="{:.1}" stroke="{c}" stroke-width="1.6"{dash}/>"#,
                        lx + 6.0,
                        y - 4.0,
                        lx + 30.0,
                        y - 4.0
                    );
                }
            }
            let _ = write!(
                s,
                r#"<text x="{:.1}" y="{y:.1}">{}</text>"#,
                lx + 36.0,
                escape(&ser.label)
            );
        }
        s.push_str("</svg>\n");
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn svg_contains_series_and_labels() {
        let p = Plot {
            file: "t.svg".into(),
            title: "Convergence <test>".into(),
            x_label: "N".into(),
            y_label: "error".into(),
            log_x: true,
            log_y: true,
            series: vec![
                Series::markers("sim", vec![(16.0, 1e-2), (32.0, 2.5e-3), (64.0, 6e-4)]),
                Series::dashed("slope −2", vec![(16.0, 1e-2), (64.0, 6.25e-4)]),
            ],
        };
        let svg = p.to_svg();
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>\n"));
        assert!(svg.contains("Convergence &lt;test&gt;"));
        assert_eq!(svg.matches("<circle").count(), 4); // 3 points + legend
        assert!(svg.contains("stroke-dasharray"));
        assert!(svg.contains(">1e-3<") || svg.contains(">0.001<"));
        assert_eq!(label(0.5), "0.5");
        assert_eq!(label(2e-5), "2e-5");
        assert_eq!(nice_step(0.23), 0.5);
    }
}
