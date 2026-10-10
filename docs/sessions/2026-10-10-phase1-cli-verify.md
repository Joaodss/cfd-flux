# Session checklist — Phase 1 step 6, CLI and validation suite (2026-10-10)

Request: see [prompt-phase1-step6.md](prompt-phase1-step6.md). Branch `feat/phase1-cli-verify`.
Stop after step 2 or 3 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| 0 | Design proposal (CLI options, output layout, backend-agnostic cases) approved by the author | ✅ |
| 1a | `cfd-io::image`: colour maps (viridis, inferno, coolwarm) and PNG encoding | ✅ |
| 1b | `cfd-cli run`: frames, `probes.csv`, `forces.csv`, `diagnostics.csv`, `meta.json`, clean abort on NaN | ✅ |
| 1c | `cfd-cli render`: PNG sequences with a fixed range per field (also called at the end of `run`) | ✅ |
| 2 | `cfd-cli bench`: MLUPS isothermal/thermal at several sizes; results in `docs/benchmarks.md` | ✅ |
| 3a | `cfd-verify` crate: harness (`SolverFactory`, steady state, error norms, orders), references with sources | ✅ |
| 3b | Cases: Taylor-Green, Poiseuille, Couette | ✅ |
| 3c | Cases: lid-driven cavity Re 100/400/1000 (Ghia 1982) | ✅ |
| 3d | Cases: cylinder Schäfer-Turek 2D-1 (Re 20) and 2D-2 (Re 100) | ✅ |
| 3e | Cases: heated cavity Ra 1e3–1e6 (de Vahl Davis 1983) | ✅ |
| 3f | `cfd-cli verify --quick/--full`, Markdown report with SVG plots in `validation/report/`, CI step | ✅ |
| 4 | Phase 1 acceptance: tolerances of 04 §7, ≥ 50 MLUPS, von Kármán street PNG sequence | ✅ |
| 5 | Tick plan checkboxes, update docs (02/03/04/07/08, README, CLAUDE.md), final verification | ✅ |

## Decisions taken with the author

- Validation cases are Rust code in a backend-agnostic crate `cfd-verify` that receives a
  `SolverFactory` (`&dyn Fn(Domain) -> Result<Box<dyn Solver>>`); the CLI wires the backends (ADR-018).
- Staircase cylinder: pass/fail at the finest resolution with a 2% tolerance (5% in `--quick`) on
  C_D, St and C_D,max; the Schäfer-Turek intervals are reported alongside. Interpolated
  bounce-back is a possible follow-up.
- With TRT (Λ = 3/16) Poiseuille and Couette are exact to round-off; the order check becomes
  "largest error < 1e-5".
- After the first full run (two order checks failed below the f32 round-off floor): orders are
  fitted over Taylor-Green N ≤ 128 and channels N ≤ 32; finer levels stay in the report, marked.

## Found along the way

- **Force scaling bug** (`cfd-core::units::force_to_physical`): `ρ₀dx²/dt²` (a pressure) instead of
  `ρ₀dx³/dt²` (N/m) — forces were 1/dx too large. Invisible to the unit tests (all at dx = 1); caught by
  the wall-shear checks of Poiseuille/Couette.
- **Forces are now gauge**: momentum exchange with shifted populations drops the reference pressure
  ρ₀c_s², which showed up as −45 N/m on an inlet at rest (it cancels on closed bodies).
- Steady-state detection must normalise both velocity components by the speed (normalising uy by
  max |uy| ≈ round-off never converged), and some cases are better judged on the measured quantity:
  the cylinder's C_D drifts with an ~8 s time constant while C_L carries acoustic noise; the heated
  cavity's fields never get below 1e-6 at low τ, but Nu converges cleanly.
- Halving the cylinder's lattice velocity (Ma 0.14 → 0.07) changes C_D,max by only 0.6%: the
  staircase geometry, not compressibility, limits accuracy; D = 80 cells brings C_D,max within 0.9%.
- Ghia's Table II Re 400 value at x = 0.9063 is a known misprint (excluded); Dixit & Babu (2006)
  misprint de Vahl Davis' u_max at Ra 1e3 as 3.469 (the reference is 3.649).

## Final check

- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`: clean.
- `cargo test --workspace`: 68 tests pass (was 55; new: `cfd-io` image, `cfd-cli` end-to-end run tests,
  `cfd-verify` unit tests, the force-scaling test in `cfd-core`).
- `cfd-cli verify --full`: **12/12 cases pass** in 1351 s on 16 threads; report committed in
  [`validation/report/`](../../validation/report/README.md) (its commit `727020d-dirty`: the code was
  727020d; only documentation was being edited during the run). Key numbers:
  - Taylor-Green order 2.16 (N ≤ 128); BGK Poiseuille order 2.00 (`--collision bgk`).
  - Ghia, max deviation / U_lid at 256²: Re 100 0.9%, Re 400 0.5%, Re 1000 1.6%.
  - Schäfer-Turek: 2D-1 C_D = 5.600 (+0.36%, D = 40); 2D-2 St = 0.3000, C_D,max = 3.258 (+0.87%),
    C_L,max = 0.9905 (D = 80).
  - de Vahl Davis: Nu within 0.4% for Ra 1e3–1e6; u_max/v_max and positions to 3–4 digits.
- `cfd-cli verify --quick` (CI): 8/8 cases pass in 18 s locally.
- `cfd-cli bench` ([benchmarks.md](../benchmarks.md)): 65–137 MLUPS on 8 threads, 85–180 on 16.
- Von Kármán street: `cfd-cli run scenes/cylinder-re100.json --out out/cyl --end-time 12
  --output-interval 0.05` writes 241 vorticity/velocity PNGs; the street is visible from t ≈ 6 s.
- Not done / follow-ups: interpolated bounce-back for curved walls; Rayleigh-Bénard onset and the
  dam break (later phases); `cfd-cli compare` (Phase 2).
