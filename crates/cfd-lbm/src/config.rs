//! Solver settings that are not part of the scene (the scene describes physics, not algorithms).

/// TRT "magic" parameter `Λ = (τ⁺ − ½)(τ⁻ − ½)` that puts half-way bounce-back walls exactly
/// half-way for Poiseuille flow, independently of the viscosity.
pub const MAGIC_WALL: f64 = 3.0 / 16.0;

/// Collision operator of both lattices.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Collision {
    /// Single relaxation time (the textbook reference).
    Bgk,
    /// Two relaxation times: the symmetric part relaxes with `τ` (viscosity / free for the
    /// thermal lattice), the antisymmetric one with the rate fixed by `magic`.
    Trt { magic: f64 },
}

impl Default for Collision {
    fn default() -> Self {
        Collision::Trt { magic: MAGIC_WALL }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct LbmConfig {
    pub collision: Collision,
}

/// Relaxation rates `(ω⁺, ω⁻)` for the flow lattice, where `τ` sets the viscosity
/// (symmetric moments).
pub fn flow_rates(collision: Collision, tau: f64) -> [f64; 2] {
    match collision {
        Collision::Bgk => [1.0 / tau, 1.0 / tau],
        Collision::Trt { magic } => [1.0 / tau, 1.0 / (0.5 + magic / (tau - 0.5))],
    }
}

/// Relaxation rates `(ω⁺, ω⁻)` for the thermal lattice, where `τ` sets the diffusivity
/// (antisymmetric moments).
pub fn thermal_rates(collision: Collision, tau: f64) -> [f64; 2] {
    match collision {
        Collision::Bgk => [1.0 / tau, 1.0 / tau],
        Collision::Trt { magic } => [1.0 / (0.5 + magic / (tau - 0.5)), 1.0 / tau],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trt_rates_satisfy_the_magic_parameter() {
        let tau = 0.53;
        let [wp, wm] = flow_rates(Collision::default(), tau);
        let magic = (1.0 / wp - 0.5) * (1.0 / wm - 0.5);
        assert!((magic - MAGIC_WALL).abs() < 1e-12);
        let [wp, wm] = thermal_rates(Collision::default(), tau);
        assert!((1.0 / wm - tau).abs() < 1e-12);
        assert!(((1.0 / wp - 0.5) * (1.0 / wm - 0.5) - MAGIC_WALL).abs() < 1e-12);
        assert_eq!(flow_rates(Collision::Bgk, 0.8), [1.25, 1.25]);
    }
}
