# 08 — Validation guide: how accuracy is measured

This guide explains, step by step, how we know the simulator is **right** — not just pretty. It complements the table of cases in [04 §7](04-numerical-methods.md#7-validation).

## 1. The idea in three sentences

1. Pick a problem whose answer is **already known** (from an exact formula or from published results accepted by the community).
2. Simulate it and **measure the same quantity** the reference reports.
3. Compare using a **number** (the error) and a **threshold** (the tolerance). If the error exceeds the threshold, the test fails — automatically, in CI.

There are three kinds of reference, from easiest to hardest to use:

| Type | Example | What is compared |
|------|---------|------------------|
| **A. Analytical solution** (exact formula) | Poiseuille, Couette, Taylor-Green | The whole field, point by point |
| **B. Tabulated reference data** | Cavity (Ghia 1982), heated cavity (de Vahl Davis 1983) | Values at specific points/lines |
| **C. Integral quantities** | Cylinder (Schäfer & Turek 1996), dam break | One or a few numbers: C_D, St, Nu, front position |

## 2. Error measures

Let `s` be the simulated value and `r` the reference value, at `N` points:

- **Relative L2 error** (the main one):
  `E_L2 = sqrt( Σ (s − r)² ) / sqrt( Σ r² )` → one number; 0.01 = 1% error.
- **Maximum error** (L∞): `max |s − r|` — catches localised errors (e.g. near walls).
- **Relative error of a quantity**: `|s − r| / |r|` — for C_D, St, Nu.

## 3. Convergence order (the most important test)

A small error at a single resolution can be luck. The strong evidence is: **as the grid gets finer, the error drops at the expected rate.**

1. Run the same case with N = 32, 64, 128, 256 cells along the characteristic direction.
2. Compute the L2 error for each.
3. The order is `p = log(E_N / E_2N) / log(2)`.
4. LBM is a **second-order** method: expect `p ≈ 2` (the error drops to ~1/4 when the resolution doubles).

On a log-log plot (error vs. resolution) the points should form a line with slope −2. If `p ≈ 1`, there is almost always a bug in the boundary conditions.

> **LBM note:** when refining, keep the Reynolds number fixed and use *diffusive scaling* (`u_lb ∝ 1/N`); otherwise the compressibility (Mach) error does not shrink and the apparent order is wrong.

## 4. The cases, explained

### 4.1 Poiseuille (channel flow) — type A
- **Setup:** channel between two parallel walls, flow driven by a constant body force (or a pressure difference), periodic boundaries in the flow direction.
- **Reference:** parabolic profile `u(y) = (G / 2ν) · y · (H − y)`.
- **Measure:** once the flow is steady (change between steps < 1e-10), compare `u(y)` along a column with the formula → E_L2. Repeat at 4 resolutions → order.
- **Target:** E_L2 < 1% at N = 32; p ≈ 2.

### 4.2 Taylor-Green (decaying vortices) — type A
- **Setup:** periodic domain with a known initial vortex pattern, no walls.
- **Reference:** the amplitude decays as `exp(−2 ν k² t)`.
- **Measure:** total kinetic energy over time vs. the formula; velocity field at a fixed time → E_L2 and order. It is the best test of the solver **core** (no boundaries involved).

### 4.3 Lid-driven cavity — type B
- **Setup:** closed square, top wall moving at velocity U. Re = U·L/ν = 100, 400, 1000.
- **Reference:** tables from Ghia et al. (1982): `u` along the vertical centreline and `v` along the horizontal centreline (17 points each).
- **Measure:** at steady state, interpolate the simulated field at those points → max and L2 error.
- **Target:** deviation < 1–2% of the lid velocity.

### 4.4 Cylinder in a channel (Schäfer & Turek 2D-1 and 2D-2) — type C
- **Setup:** 2.2 m × 0.41 m channel, cylinder of diameter 0.1 m slightly off-centre, parabolic inlet profile.
- **Measure:**
  - **Force on the cylinder** (momentum-exchange method in LBM) → coefficients `C_D = 2F_x / (ρ U² D)`, `C_L` likewise with `F_y`.
  - **Re 20** (steady): final C_D. Target: 5.57–5.59.
  - **Re 100** (periodic): C_L time series → dominant frequency `f` via FFT or zero crossings → **Strouhal** `St = f·D/U`. Target: 0.295–0.305; max C_D 3.22–3.24.
- **Caveat:** a cylinder drawn in pixels has "staircase" walls; it needs ~20+ cells across the diameter to stay within tolerance. A good case for comparing resolutions and, later, *interpolated bounce-back*.

### 4.5 Differentially heated cavity (de Vahl Davis 1983) — type B/C
- **Setup:** closed square, hot left wall, cold right wall, adiabatic top/bottom, gravity. Rayleigh number Ra = 10³ … 10⁶.
- **Measure:** **mean Nusselt number** on the hot wall: `Nu = (L / ΔT) · mean of (−∂T/∂x)` at the wall. Also maximum velocities along the centrelines.
- **Target:** Nu ≈ 1.118 / 2.243 / 4.519 / 8.800 (Ra 10³ / 10⁴ / 10⁵ / 10⁶), error < 1–2%.

### 4.6 Dam break (Martin & Moyce 1952) — type C, liquids
- **Setup:** water column of width `a` resting against a wall, released at time 0.
- **Measure:** water front position `x(t)` in dimensionless time `t·sqrt(2g/a)`, compared with the experimental points. Also liquid **mass conservation** (must stay < 0.1%).
- **Target:** curve within the experimental scatter (experimental data, not exact — looser tolerance, ~5%).

## 5. Checks that need no reference

These run on **every** simulation, not just in tests:
- Total mass constant (closed systems) — relative drift < 1e-6.
- No NaN/Inf.
- Symmetry: a symmetric case must give a symmetric result (until a physical instability breaks it).
- Maximum Mach < ~0.17 (LBM validity limit).

## 6. How it looks in the code

```
validation/
  references/            # reference data as CSV (Ghia, de Vahl Davis, Schäfer-Turek, ...) with the source cited
  report/                # generated by `verify --full`: README.md, SVG plots, PNG snapshots, results.json
crates/cfd-verify/
  src/scenes.rs          # validation geometries as regular scenes, parametrised by resolution
  src/cases/             # one module per case family; each case only sees `Box<dyn Solver>`
  src/harness.rs         # steady state, timing, error norms, convergence orders
```

- **Cases are Rust code, not JSON files:** they need a resolution parameter, body forces and
  non-uniform initial fields that the scene format does not express. Each case still builds a
  regular `Scene` (in physical units) and goes through `Scene → Domain`, so the unit conversion is
  validated too. The reference CSVs are embedded in the binary at compile time.
- **Backend-agnostic:** the cases receive a factory `Fn(Domain) -> Box<dyn Solver>`; the CLI
  chooses the backend, so Phase 2 reuses the suite unchanged for wgpu and CUDA.
- `cfd-cli verify --quick` → low resolutions, runs in CI on every commit (about a minute); the report
  is uploaded as a CI artifact.
- `cfd-cli verify --full` → all resolutions; run locally before releases and commit `validation/report/`.
- `cfd-cli verify --list`, `--cases cavity,cylinder-re20`, `--collision bgk` (BGK shows a genuine
  order 2 for Poiseuille, where TRT is exact).
- (`cfd-cli validate` is the existing command that checks scene *files* for format/consistency errors.)
- The report is published with the documentation: **a great portfolio piece** ("this simulator reproduces Ghia 1982 within 1%").

## 7. Recommended order

1. Taylor-Green (tests only the core) → 2. Poiseuille (walls and forcing) → 3. Cavity (moving walls) → 4. Cylinder (inlet/outlet, forces) → 5. Heated cavity (thermal) → 6. Dam break (free surface).

Each step isolates one new part of the code: if it fails, you know where to look.
