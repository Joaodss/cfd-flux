# 04 — Numerical methods

This document describes the methods to implement, in order, and why. All of them sit behind the `Solver` trait (see [02](02-architecture.md#24-simulation-core--cfd-core-and-solvers)).

## 1. Overview and order

| Order | Method | Fluids / phenomena | Phase |
|-------|--------|--------------------|-------|
| 1 | **LBM D2Q9** (BGK → TRT/MRT) | Single-phase incompressible gas/liquid (low Ma) | 1–2 |
| 2 | **Thermal LBM D2Q5** (double distribution) + Boussinesq | Forced and natural convection | 1–2 |
| 3 | **Free-surface LBM (VOF-LBM)** — in the MVP | Liquids with a free surface (falling water, waves) | 6a |
| 4 | Multiphase LBM (Shan-Chen / phase-field) | Bubbles, droplets, two immiscible fluids | 6b |
| 5 | Projection (Chorin) on a MAC grid + multigrid | "Classic" incompressible; comparison with LBM | 7 |
| 6 | Compressible finite volumes (Godunov, HLLC, MUSCL) | High-speed gases, shock waves | 7 |
| 7 | SPH | Meshless alternative for liquids | 10 |

## 2. Why start with LBM

- **Uniform Cartesian grid** = pixels. Complex hand-drawn boundaries are handled with bounce-back, without explicit geometry.
- **Fully local:** each cell only reads its immediate neighbours → ideal for GPUs (memory bandwidth is the only limit). Reference implementations reach thousands of MLUPS on one GPU.
- **No global Poisson equation** in the base case (pressure is `p = c_s² ρ`).
- Well-studied extensions for heat transfer, multiphase, free surfaces and turbulence (LES).

**Limitations** to keep in mind: only weakly compressible (Ma ≲ 0.1–0.3), stability sensitive to τ close to 0.5 (high grid Reynolds number), time step tied to resolution.

## 3. LBM D2Q9 — details

### 3.1 Algorithm per step
1. **Collision:** `f_i* = f_i − (f_i − f_i^eq)/τ + F_i` (BGK + Guo forcing for gravity/buoyancy).
2. **Streaming:** `f_i(x + c_i, t+1) = f_i*(x, t)`.
3. **Boundaries:** link-wise, half-way rules applied during the pull ([ADR-014](07-decisions-and-questions.md)): bounce-back with the wall velocity (walls, moving walls, velocity inlets), anti-bounce-back (pressure outlets), copy of the interior neighbour (zero-gradient outlets), periodic indexing.
4. **Macroscopic quantities:** `ρ = Σ f_i`, `ρu = Σ c_i f_i + F/2`.

Implemented as **a single fused kernel** (stream-collide, "pull") with two buffers (ping-pong). Later optimization: **AA pattern** or **Esoteric Pull** to use a single buffer (half the memory).

### 3.2 Collision operators (increasing robustness)
1. BGK — simple, the reference.
2. **TRT** (two-relaxation-time) — viscosity-independent wall location; good cost/benefit. *Recommended default.*
3. MRT / regularized / cumulant — more stable at high Re.
4. LES Smagorinsky (local effective τ) for turbulence.

### 3.3 Unit conversion (solver side)
Given `dx` (= `cellSize`), a characteristic physical velocity `U` and viscosity `ν` (implemented in `cfd-core::units` and `domain`; `U` is the largest of the inlet peaks, moving-wall speeds, initial velocity and, with Boussinesq, `√(gβΔT·L)`, unless overridden):
- Choose the lattice velocity `u_lb` ≤ 0.1 (default 0.05) → `dt = u_lb · dx / U`.
- `ν_lb = ν · dt / dx²` → `τ = 3 ν_lb + 0.5`.
- Checks: `τ > 0.5 + ε` (e.g. 0.505 with TRT); `Ma = u_lb·√3 < 0.17`; warning if the grid Reynolds number `U·dx/ν` is high.
- If the combination is unstable, the server **suggests** to the user: increase resolution, lower `u_lb` (more steps) or enable LES.

### 3.4 Memory (f32)
`9 × 4 B × 2 buffers + flags + macroscopic fields ≈ 85 B/cell`.
4096² ≈ 16.8 M cells → ~1.4 GB. With the AA pattern and f16 storage of `f_i` (FluidX3D technique) → ~0.5 GB.

**As implemented (CPU, Phase 1):** `f` (9 × 2 buffers) + `g` (5 × 2, thermal only) + ρ, u, θ + flags and BC slot ≈ 100 B/cell isothermal, 140 B/cell thermal. Measured ≈ 120 MLUPS isothermal and ≈ 75 MLUPS thermal on 16 threads (no explicit SIMD yet).

**As implemented (wgpu, Phase 2a):** the same arrays plus one 4-plane buffer for ρ, ux, uy, θ: ≈ 100 B/cell isothermal, ≈ 140 B/cell thermal (4000² thermal ≈ 2.2 GB). An update moves ≈ 86 B (isothermal) / 130 B (thermal); measured 2,850 / 1,850 MLUPS on an RTX 4060, ≈ 90% of its memory bandwidth ([benchmarks](benchmarks.md)).

**As implemented (CUDA, Phase 2b):** the same buffers (the `u16` flags read directly, a 1-value dummy instead of `g` without the thermal lattice), plus one device snapshot of the four macroscopic planes (16 B/cell) per field copy in flight ([ADR-021](07-decisions-and-questions.md)). Throughput: see [benchmarks](benchmarks.md).

### 3.5 Implementation notes (shared by every backend)
- SoA populations `f[i·n + cell]`, pull scheme, ping-pong buffers; one fused kernel per step also writes ρ, u (with the `F/2` correction) and θ.
- **Shifted populations** `f̃ᵢ = fᵢ − wᵢ` in f32, and rest populations computed as the remainder (`f̃₀ = Δρ − Σ`, `g₀ = θ − Σ`), which conserves mass and heat by construction ([ADR-015](07-decisions-and-questions.md)).
- TRT with Guo forcing split into symmetric/antisymmetric parts, each scaled by its own `(1 − ω±/2)`; BGK is the special case `ω⁺ = ω⁻`.
- Only Boussinesq buoyancy is applied as a force; the hydrostatic pressure is added at sampling time ([ADR-016](07-decisions-and-questions.md)).
- Forces on elements: momentum exchange over bounce-back links, `F = Σ c_ī (2 f̃*_ī + 6 wᵢ ρ₀ cᵢ·u_w)` with the shifted population `f̃ = f − w`: this removes the reference pressure ρ₀c_s², so forces are gauge like every other output (it cancels on closed bodies anyway).
- The initial populations, relaxation rates and the list of bounce-back links come from shared code (`cfd_lbm::shared`). The GPU gathers the outgoing population of every link and the CPU sums the forces in f64 in the CPU backend's order; diagnostics are reduced per workgroup on the GPU (Σ(ρ − 1) rather than Σρ, to keep f32 partial sums accurate) and finished in f64.
- Divergence (NaN/Inf) checks on the GPU test values *loaded* from memory (the populations): compilers may assume arithmetic results are finite and fold a check on them away ([ADR-019](07-decisions-and-questions.md)).
- Parity with the CPU (§7) is measured on ρ in absolute terms and on u and θ relative to the field maximum; both velocity components share the scale (a component that is zero up to round-off must not set it). Measured on the RTX 4060: wgpu ≤ 3e-6 (its driver fuses multiply-adds); CUDA, compiled without multiply-add contraction (`--fmad=false`), is **bit-identical** to the CPU on every parity case and in every validation result. The cases live in `cfd_verify::parity` and are shared by both GPU backends; `cfd_verify::compare` uses the same metric on whole scenes and on the validation geometries (§7).
- **Output sampling** ([ADR-028](07-decisions-and-questions.md)): the physical output fields (velocity, gauge pressure with the hydrostatic part, density, temperature, vorticity with central differences between fluid neighbours and one-sided ones at boundaries) are computed on the device, optionally averaged over `k × k` blocks (partial blocks at the edges average what they have; an f32 sum in row order) and stored as f32 or f16 rounded to nearest-even. The CPU conversion (`Sample::from_lattice`) is the reference: CUDA reproduces it bit for bit (its conversions run in f64, in the same order), wgpu within 1e-5 of the field maximum and one f16 ulp (f32 arithmetic). Parity cases in `cfd_verify::parity` compare each backend's samples with the CPU conversion of its own lattice fields, so they test the sampling kernel alone.

## 4. Heat transfer

- **Double distribution:** a second lattice `g_i` (D2Q5) for the temperature advection-diffusion equation, with `τ_g = α_lb / c_s,g² + 0.5`. Implemented weights: `w₀ = 1/3`, `wᵢ = 1/6` ⇒ `c_s,g² = 1/3` ⇒ `τ_g = 3 α_lb + 0.5`, equilibrium `gᵢ = wᵢ θ (1 + 3 cᵢ·u)`. With TRT, `τ_g` sets the antisymmetric rate (diffusivity) and Λ = 3/16 sets the symmetric one.
- **Coupling:** Boussinesq — force `F = −ρ₀ β (T − T_ref) g`, applied via Guo forcing.
- **Thermal boundaries:** fixed temperature (anti-bounce-back), flux, adiabatic (bounce-back of `g`).
- **Conjugate (solid-fluid):** later phase — the solid also solves diffusion with its own `α_s`, with flux continuity at the interface.

## 5. Liquids and multiphase (Phase 6)

| Approach | Pros | Cons |
|----------|------|------|
| **Free-surface LBM** (Körner et al. 2005; mass per cell, interface cells) | Reuses the LBM kernel, great on GPUs | Single phase (the gas is "empty"), surface tension needs curvature |
| **FLIP/APIC** (particles + MAC grid) | Excellent visuals, low dissipation | Needs a Poisson solve (multigrid); GPU particles are work |
| **Shan-Chen** (pseudo-potential LBM) | Simple, spontaneous phase separation | Spurious currents, limited density ratios |
| **Phase-field LBM** (conservative Allen-Cahn) | High density ratios (water/air ≈ 1000) | More complex |

**Recommendation:** start with free-surface LBM (water in air), then phase-field for two real fluids.

## 6. Other methods (Phase 7+)

- **Projection (Chorin/Stam) on a MAC grid:** semi-Lagrangian/BFECC or MacCormack advection, Poisson via PCG with a multigrid preconditioner; the basis for FLIP and for comparisons with LBM.
- **Compressible finite volumes:** compressible Euler/Navier-Stokes, MUSCL/WENO reconstruction, HLLC flux, SSP Runge-Kutta; enables gases at Ma > 0.3 and shocks.
- **Phase change:** enthalpy method (solidification/melting), evaporation later (see the plan, Phases 9b/9c).
- **Other states:** granular (DEM, MPM or continuum μ(I)), non-Newtonian (shear-rate-dependent viscosity — easy in LBM with a local τ).

## 7. Validation

How each case is measured, step by step: see [08 — Validation guide](08-validation-guide.md). Every solver must pass these cases (automated in `validation/`, run in CI on the CPU backend at reduced resolution and on the GPU nightly / locally):

| Case | Reference | Target metric |
|------|-----------|---------------|
| 2D Poiseuille | Analytical solution | Profile L2 error < 1%; convergence order ≈ 2 |
| Couette | Analytical | L2 error < 1% |
| Taylor-Green (decay) | Analytical | Energy decay rate; order ≈ 2 |
| Lid-driven cavity Re 100/400/1000 | Ghia, Ghia & Shin (1982) | Centreline profiles u(y), v(x) |
| Cylinder in a channel Re 20 / Re 100 | Schäfer & Turek (1996) | Re 20: C_D ≈ 5.58; Re 100: St ≈ 0.30, C_D,max ≈ 3.23 |
| Differentially heated cavity Ra 10³–10⁶ | de Vahl Davis (1983) | Mean Nu ≈ 1.118 / 2.243 / 4.519 / 8.800 |
| Rayleigh-Bénard | Linear theory | Onset of convection at Ra_c ≈ 1708 |
| Dam break | Martin & Moyce (1952) | Front position vs time |
| Sod shock tube | Exact solution | ρ, u, p profiles (compressible only) |

**Parity between backends (CPU, wgpu, CUDA):** same scene, N steps, maximum relative difference < 1e-5 (f32) in the macroscopic fields; the CPU backend is the reference. For long/chaotic runs (e.g. the von Kármán street) statistics are compared (St, mean C_D) instead of instantaneous fields, because tiny rounding differences diverge over time. Tool: `cfd-cli compare` (ADR-023): the verdict is this field metric (and the mass of closed domains); `--statistics` judges the force statistics (frequency, mean/min/max) and the mean kinetic energy over a window instead (relative limit 1e-3); kinetic energy, max |u| and forces are reported for information, since they amplify field differences in their own ways (energy is quadratic in u; forces on pressure-loaded walls measure ρ − 1 relatively). `cfd-cli compare --validation` writes [`validation/compare/`](../validation/compare/README.md): CUDA bit-identical to the CPU on all 13 validation geometries (the Re = 100 cylinder over 75,000 steps included), wgpu ≤ 7.9e-6 on the field cases; on the cylinder wgpu's fields drift by 1.6e-4 after 20 s while St and the force statistics agree within 5.2e-6.

## 8. Runtime diagnostics

Computed periodically (GPU reductions): total mass, kinetic energy, maximum velocity (Mach warning), NaN/Inf detection (aborts the job with a useful message and the last valid frame).

## 9. Key references

- Krüger et al., *The Lattice Boltzmann Method: Principles and Practice*, Springer, 2017.
- Guo, Zheng & Shi, forcing in LBM, Phys. Rev. E 65, 2002.
- Ginzburg, TRT models, 2008.
- Körner et al., free-surface LBM, J. Comput. Phys., 2005.
- Lehmann, FluidX3D (memory/f16 optimizations for GPU LBM), 2022.
- Bridson, *Fluid Simulation for Computer Graphics*, 2nd ed., 2015 (projection, FLIP).
- Toro, *Riemann Solvers and Numerical Methods for Fluid Dynamics*, 2009 (compressible).
