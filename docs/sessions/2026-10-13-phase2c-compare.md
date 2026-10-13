# Session checklist — Phase 2c, engine comparison (2026-10-13)

Request: see [prompt-phase2c.md](prompt-phase2c.md). Branch `feat/phase2c-compare`.
Stop after step 3 or 4 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Design proposal (options and output layout of `compare`, what is compared and how, backends side by side, report format, `bench --backends` output) approved by the author | ✅ |
| 1 | Refactors: `EngineArgs` split out of `SolverArgs` (threads configured once, `open` per backend); named parity tolerances; `upward_crossings` shared in `harness` | ✅ |
| 2 | `cfd_verify::compare` (metrics, statistics, verdict, report) and `cfd-cli compare SCENE`: fields, bit-identity, difference maps, diagnostics, forces, MLUPS, `README.md` + `compare.json`, exit code | ✅ |
| 3 | `cfd-cli compare --validation`: report for the validation scenes committed to `validation/compare/`; chaotic cylinder compared by statistics | ✅ 13/13 |
| 4 | `cfd-cli bench --backends`: interleaved runs, one table with a column per backend, `--json`; quiet-desktop run in `docs/benchmarks.md` | ✅ |
| 5 | Tests: compare end to end (CPU vs CPU bit-identical; GPU backends when available), report and exit code; `bench --backends` parsing | ✅ |
| 6 | Docs: plan checkboxes (2c), `docs/development.md`, README, 04 §7 / 08 notes, ADR-023, Phase 2 acceptance, final check | ✅ |
| 7 | (If there is time) Phase 3 design proposal, no code — to be approved in the next session | ✅ [proposal](phase3-design-proposal.md), [prompt](prompt-phase3.md); awaiting the author's OK |

## Decisions taken with the author

- Design approved as proposed: comparison engine in `cfd_verify::compare`, CLI glue in `cfd-cli/src/compare.rs`.
- `--steps 1000` by default (not the scene's `endTime`, which is chaotic for most example scenes).
- Statistics tolerance 1e-3 (relative) for chaotic runs (`--statistics`).
- Backends run one after another in `compare` (memory, no GPU contention); `bench --backends` interleaves.
- The failure path (exit code 1) is tested at the library level; no hidden perturbation flag (a CPU
  backend with BGK against one with TRT serves as a backend that really differs).
- After the first validation comparison: **the verdict is the 04 §7 field metric** (plus the mass of
  closed domains); kinetic energy, max |u| and forces are reported for information (ADR-023).
- Exit codes as `diff`/`cmp`: 1 = outside the tolerance, 2 = could not compare.

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
  Decided with the author: they are information in `compare` (ADR-023); the parity tests keep them.
- On the Re = 100 cylinder the wgpu fields drift by 1.6e-4 after 20 s (phase drift of the periodic
  street) while St = 0.2962 (verify at D = 20: 0.2963) and the force statistics agree within 5.2e-6.
- **Interleaved benchmark on a quiet desktop:** from 1024² on CUDA and wgpu are within ±0.3%
  (2,873 / 2,877 MLUPS isothermal, 1,880 / 1,884 thermal at 1024²); CUDA leads only on cache-resident
  grids (+16% at 512² isothermal, +18% at 256² thermal). The portability cost of wgpu is nil at
  production sizes.
- `compare` first wrote its output directory before opening the backends, so a failed attempt left a
  non-empty directory behind; it now touches the disk only after everything has opened.
- Long series made 1.3 MB SVG plots; the energy plot is thinned to ~1000 points and the force plot
  shows the last 1000 samples.

## Final check

- `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` (with CUDA, the default) and
  `cargo clippy -p cfd-cli -p cfd-cuda --all-targets --no-default-features -D warnings`: clean.
- `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **136 tests pass** (was 120; new: 8 library
  tests of `cfd_verify::compare` — metrics, bit-identity, statistics, CPU vs CPU, a differing backend
  (TRT vs BGK) failing with maps and a report, divergence, statistics mode, closed-domain mass — 4 end-to-end
  `compare` tests (CPU vs CPU bit-identical with `--force` handling, CUDA bit-identical at `--tolerance 0`,
  wgpu within 1e-5 with `--maps all`, error exit codes and argument conflicts), 3 `bench --backends`
  parsing/table tests, the label test, the zero-crossing test moved to `harness`). Without the `cuda`
  feature the CLI tests pass with CUDA reported unavailable.
- `cfd-cli compare --validation`: **13/13** (report `validation/compare/`, code `ef53348`, 64 s).
- `cfd-cli bench --backends cuda,wgpu` on a quiet desktop: section in `docs/benchmarks.md`.
- Plan: both 2c CLI items ticked; Phase 2 acceptance marked fully met. The third 2c item (backend in the job
  metadata, side-by-side comparison in the UI) belongs to Phases 3–5 and stays open.
- Phase 3 design proposal written (no code): crates `cfd-engines`/`cfd-job` shared with the CLI, axum routes,
  job states, per-physical-device semaphores, worker command loop with ≤ 50 ms batches, output thread,
  WebSocket with `watch` (latest frame) / `broadcast` (events), on-device sampling/f16, `skip_macro`
  (the pressure outlet is the only in-kernel reader of the macroscopic planes), capabilities, ResultStore and
  SQLite, tests, order, and eight open questions.
- Not done / follow-ups: the author's review of the Phase 3 proposal; CI has not run on this branch (not
  pushed).
