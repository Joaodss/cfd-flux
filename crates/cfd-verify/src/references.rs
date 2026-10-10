//! Published reference data, embedded at compile time from `validation/references/*.csv`
//! (`include_str!` is the Rust counterpart of a C# embedded resource). Each file cites its source
//! in `#` comment lines.

const GHIA_U: &str =
    include_str!("../../../validation/references/ghia1982-u-vertical-centreline.csv");
const GHIA_V: &str =
    include_str!("../../../validation/references/ghia1982-v-horizontal-centreline.csv");
const DE_VAHL_DAVIS: &str = include_str!("../../../validation/references/de-vahl-davis1983.csv");
const SCHAFER_TUREK: &str = include_str!("../../../validation/references/schafer-turek1996.csv");

/// Citations, for reports.
pub const GHIA_CITATION: &str =
    "Ghia, Ghia & Shin (1982), J. Comput. Phys. 48, 387–411, Tables I–II";
pub const DE_VAHL_DAVIS_CITATION: &str =
    "de Vahl Davis (1983), Int. J. Numer. Meth. Fluids 3, 249–264";
pub const SCHAFER_TUREK_CITATION: &str =
    "Schäfer & Turek (1996), Notes Numer. Fluid Mech. 52, 547–566";

/// A CSV file: header names and numeric rows (the first column may be text).
struct Csv {
    header: Vec<String>,
    rows: Vec<Vec<String>>,
}

fn parse(text: &str) -> Csv {
    let mut lines = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'));
    let split = |l: &str| {
        l.split(',')
            .map(|s| s.trim().to_string())
            .collect::<Vec<_>>()
    };
    let header = split(lines.next().expect("CSV header"));
    let rows = lines.map(split).collect();
    Csv { header, rows }
}

impl Csv {
    fn col(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h == name)
    }

    fn num(&self, row: usize, col: usize) -> f64 {
        self.rows[row][col]
            .parse()
            .unwrap_or_else(|_| panic!("bad number '{}' in reference data", self.rows[row][col]))
    }
}

/// Values along a line, sorted by increasing coordinate.
#[derive(Debug, Clone, PartialEq)]
pub struct Profile {
    pub coord: Vec<f64>,
    pub value: Vec<f64>,
}

fn ghia(text: &str, re: u32) -> Option<Profile> {
    let csv = parse(text);
    let col = csv.col(&format!("re{re}"))?;
    let mut pts: Vec<(f64, f64)> = (0..csv.rows.len())
        .map(|r| (csv.num(r, 0), csv.num(r, col)))
        .collect();
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));
    Some(Profile {
        coord: pts.iter().map(|p| p.0).collect(),
        value: pts.iter().map(|p| p.1).collect(),
    })
}

/// `u / U_lid` along the vertical centreline, against `y`.
pub fn ghia_u(re: u32) -> Option<Profile> {
    ghia(GHIA_U, re)
}

/// `v / U_lid` along the horizontal centreline, against `x`.
pub fn ghia_v(re: u32) -> Option<Profile> {
    ghia(GHIA_V, re)
}

/// Ghia points excluded from error norms: `(re, x)` of a known misprint in Table II.
pub const GHIA_V_EXCLUDED: [(u32, f64); 1] = [(400, 0.9063)];

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeVahlDavis {
    pub ra: f64,
    /// Average Nusselt number over the cavity.
    pub nu_mean: f64,
    /// Average Nusselt number on the hot wall.
    pub nu_hot: f64,
    /// Maximum `u` on the vertical mid-plane (scaled by α/L) and its height.
    pub u_max: f64,
    pub y_u_max: f64,
    /// Maximum `v` on the horizontal mid-plane (scaled by α/L) and its position.
    pub v_max: f64,
    pub x_v_max: f64,
}

pub fn de_vahl_davis(ra: f64) -> Option<DeVahlDavis> {
    let csv = parse(DE_VAHL_DAVIS);
    let c = |n: &str| csv.col(n).expect("de Vahl Davis column");
    (0..csv.rows.len())
        .find(|&r| (csv.num(r, c("ra")) / ra - 1.0).abs() < 1e-9)
        .map(|r| DeVahlDavis {
            ra,
            nu_mean: csv.num(r, c("nu_mean")),
            nu_hot: csv.num(r, c("nu_hot")),
            u_max: csv.num(r, c("u_max")),
            y_u_max: csv.num(r, c("y_u_max")),
            v_max: csv.num(r, c("v_max")),
            x_v_max: csv.num(r, c("x_v_max")),
        })
}

/// Reference interval `[lower, upper]` of a Schäfer-Turek quantity, e.g. `("2D-1", "c_d")`.
pub fn schafer_turek(case: &str, quantity: &str) -> Option<[f64; 2]> {
    let csv = parse(SCHAFER_TUREK);
    (0..csv.rows.len())
        .find(|&r| csv.rows[r][0] == case && csv.rows[r][1] == quantity)
        .map(|r| [csv.num(r, 2), csv.num(r, 3)])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_tables_load() {
        let u = ghia_u(100).unwrap();
        assert_eq!(u.coord.len(), 17);
        assert_eq!((u.coord[0], u.coord[16]), (0.0, 1.0));
        assert_eq!(u.value[16], 1.0);
        assert_eq!(ghia_u(1000).unwrap().value[15], 0.65928);
        let v = ghia_v(400).unwrap();
        assert_eq!(v.coord[7], 0.2344);
        assert_eq!(v.value[7], 0.30174);
        assert!(ghia_u(3200).is_none());

        let d = de_vahl_davis(1e6).unwrap();
        assert_eq!((d.nu_mean, d.nu_hot, d.v_max), (8.8, 8.817, 219.36));
        assert!(de_vahl_davis(1e7).is_none());

        assert_eq!(schafer_turek("2D-1", "c_d"), Some([5.57, 5.59]));
        assert_eq!(schafer_turek("2D-2", "st"), Some([0.295, 0.305]));
    }
}
