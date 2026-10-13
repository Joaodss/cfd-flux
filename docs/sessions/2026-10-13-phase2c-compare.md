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

## Final check
