# Session checklist — Phase 1 step 6, CLI and validation suite (2026-10-10)

Request: see [prompt-phase1-step6.md](prompt-phase1-step6.md). Branch `feat/phase1-cli-verify`.
Stop after step 2 or 3 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| 0 | Design proposal (CLI options, output layout, backend-agnostic cases) approved by the author | ✅ |
| 1a | `cfd-io::image`: colour maps (viridis, inferno, coolwarm) and PNG encoding | ⏳ |
| 1b | `cfd-cli run`: frames, `probes.csv`, `forces.csv`, `diagnostics.csv`, `meta.json`, clean abort on NaN | ⏳ |
| 1c | `cfd-cli render`: PNG sequences with a fixed range per field (also called at the end of `run`) | ⏳ |
| 2 | `cfd-cli bench`: MLUPS isothermal/thermal at several sizes; results in `docs/benchmarks.md` | ⏳ |
| 3a | `cfd-verify` crate: harness (`SolverFactory`, steady state, error norms, orders), references with sources | ⏳ |
| 3b | Cases: Taylor-Green, Poiseuille, Couette | ⏳ |
| 3c | Cases: lid-driven cavity Re 100/400/1000 (Ghia 1982) | ⏳ |
| 3d | Cases: cylinder Schäfer-Turek 2D-1 (Re 20) and 2D-2 (Re 100) | ⏳ |
| 3e | Cases: heated cavity Ra 1e3–1e6 (de Vahl Davis 1983) | ⏳ |
| 3f | `cfd-cli verify [--quick|--full]`, Markdown report with SVG plots in `validation/report/`, CI step | ⏳ |
| 4 | Phase 1 acceptance: tolerances of 04 §7, ≥ 50 MLUPS, von Kármán street PNG sequence | ⏳ |
| 5 | Tick plan checkboxes, update docs (02/03/04/08 where the design differs), final verification | ⏳ |

## Decisions taken with the author

- Validation cases are Rust code in a backend-agnostic crate `cfd-verify` that receives a
  `SolverFactory` (`&dyn Fn(Domain) -> Result<Box<dyn Solver>>`); the CLI wires the backends.
- Staircase cylinder: pass/fail at the finest resolution with a 2% tolerance on C_D and St; the
  Schäfer-Turek bands are reported alongside. Interpolated bounce-back is a possible follow-up.
- With TRT (Λ = 3/16) Poiseuille and Couette are exact to round-off; the convergence-order check
  is replaced by "all errors < 1e-5" in that case.
- Tolerances are fixed before looking at results; failures are reported, not hidden.
