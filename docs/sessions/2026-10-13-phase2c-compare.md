# Session checklist — Phase 2c, engine comparison (2026-10-13)

Request: see [prompt-phase2c.md](prompt-phase2c.md). Branch `feat/phase2c-compare`.
Stop after step 3 or 4 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Design proposal (options and output layout of `compare`, what is compared and how, backends side by side, report format, `bench --backends` output) approved by the author | ✅ |
| 1 | Refactors: `EngineArgs` split out of `SolverArgs` (threads configured once, `open` per backend); named parity tolerances; `upward_crossings` shared in `harness` | ✅ |
| 2 | `cfd_verify::compare` (metrics, statistics, verdict, report) and `cfd-cli compare SCENE`: fields, bit-identity, difference maps, diagnostics, forces, MLUPS, `README.md` + `compare.json`, exit code | ⬜ |
| 3 | `cfd-cli compare --validation`: report for the validation scenes committed to `validation/compare/`; chaotic cylinder compared by statistics | ⬜ |
| 4 | `cfd-cli bench --backends`: interleaved runs, one table with a column per backend, `--json`; quiet-desktop run in `docs/benchmarks.md` | ⬜ |
| 5 | Tests: compare end to end (CPU vs CPU bit-identical; GPU backends when available), report and exit code; `bench --backends` parsing | ⬜ |
| 6 | Docs: plan checkboxes (2c), `docs/development.md`, README, 04 §7 / 08 notes, Phase 2 acceptance, final check | ⬜ |
| 7 | (If there is time) Phase 3 design proposal, no code — to be approved in the next session | ⬜ |

## Decisions taken with the author

- Design approved as proposed: comparison engine in `cfd_verify::compare`, CLI glue in `cfd-cli/src/compare.rs`.
- `--steps 1000` by default (not the scene's `endTime`, which is chaotic for most example scenes).
- Statistics tolerance 1e-3 (relative) for chaotic runs (`--statistics`).
- Backends run one after another in `compare` (memory, no GPU contention); `bench --backends` interleaves.
- The failure path (exit code 1) is tested at the library level; no hidden perturbation flag.

## Found along the way

- **CUDA is bit-identical to the CPU on all 13 validation geometries**, including the chaotic Re = 100
  cylinder over 75,000 steps (20 s, ≈ 60 shedding periods) — the strongest parity result so far.
- wgpu: every field within 1e-5 (largest 7.9e-6, Taylor-Green after one viscous time); on the Re = 100
  cylinder its fields diverge (as expected) while St, the force statistics and the mean kinetic energy
  agree to 5e-6.
- Mass is conserved by construction only in closed domains: with inlets/outlets Σρ follows the flow and
  its relative difference is the mean density difference (1.8e-7 on the channel example for wgpu), so the
  parity tests' 1e-7 mass limit applies to closed domains only; open ones use the field tolerance. Sums are
  never held below round-off (backends reduce in different orders even with bit-identical fields).
- Two parity-test limits are stricter than the 04 §7 field metric on longer runs: kinetic energy (quadratic
  in u: δE/E ≈ 2 δu/u; Taylor-Green 1.64e-5 vs 1e-5) and forces on pressure-dominated walls (they measure
  ρ − 1 relatively; Rayleigh-Bénard plates 3.2e-4 vs 1e-4, while ρ agrees to 1.2e-7 absolute).

## Final check
