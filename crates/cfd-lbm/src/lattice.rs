//! Velocity sets. The direction order comes from [`cfd_core::domain::LINKS`] so that the
//! per-cell link masks and every backend agree on it.

use cfd_core::domain::{LINKS, OPPOSITE};

/// A lattice velocity set, as compile-time constants (≈ C# `static abstract` interface members).
pub trait Lattice {
    /// Number of directions.
    const Q: usize;
    /// Directions `c_i`.
    const C: &'static [[i32; 2]];
    /// Weights `w_i`.
    const W: &'static [f32];
    /// `OPP[i]` is the direction opposite to `i`.
    const OPP: &'static [usize];
}

/// D2Q9 for the flow (`c_s² = 1/3`).
pub struct D2Q9;

/// D2Q5 for the temperature, with `w₀ = 1/3`, `wᵢ = 1/6` ⇒ `c_s² = 1/3`, so `τ_g = 3α + ½`.
pub struct D2Q5;

pub const W9: [f32; 9] = [
    4.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 9.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
    1.0 / 36.0,
];

pub const W5: [f32; 5] = [1.0 / 3.0, 1.0 / 6.0, 1.0 / 6.0, 1.0 / 6.0, 1.0 / 6.0];

impl Lattice for D2Q9 {
    const Q: usize = 9;
    const C: &'static [[i32; 2]] = &LINKS;
    const W: &'static [f32] = &W9;
    const OPP: &'static [usize] = &OPPOSITE;
}

impl Lattice for D2Q5 {
    const Q: usize = 5;
    const C: &'static [[i32; 2]] = LINKS.split_at(5).0;
    const W: &'static [f32] = &W5;
    const OPP: &'static [usize] = OPPOSITE.split_at(5).0;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Checks the moments `Σ w = 1`, `Σ w c = 0`, `Σ w c_a c_b = c_s² δ_ab`.
    fn check_isotropy<L: Lattice>() {
        let (mut m0, mut m1, mut m2) = (0.0f64, [0.0f64; 2], [[0.0f64; 2]; 2]);
        for i in 0..L::Q {
            let (w, c) = (L::W[i] as f64, L::C[i].map(f64::from));
            m0 += w;
            for a in 0..2 {
                m1[a] += w * c[a];
                for b in 0..2 {
                    m2[a][b] += w * c[a] * c[b];
                }
            }
            assert_eq!(L::C[L::OPP[i]], L::C[i].map(|v| -v));
        }
        assert!((m0 - 1.0).abs() < 1e-7);
        assert!(m1.iter().all(|v| v.abs() < 1e-7));
        assert!((m2[0][0] - 1.0 / 3.0).abs() < 1e-7 && (m2[1][1] - 1.0 / 3.0).abs() < 1e-7);
        assert!(m2[0][1].abs() < 1e-7);
    }

    #[test]
    fn velocity_sets_are_isotropic() {
        check_isotropy::<D2Q9>();
        check_isotropy::<D2Q5>();
    }
}
