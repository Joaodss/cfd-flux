# Session checklist — Phase 2a, wgpu/WGSL backend (2026-10-10)

Request: see [prompt-phase2.md](prompt-phase2.md). Branch `feat/phase2a-wgpu`.
Stop after step 3 or 4 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Design proposal (crate layout, buffers/bind groups, flags, dispatch, readback, CI) approved by the author | ✅ |
| 0 | Rayleigh-Bénard onset case in `cfd-verify`, passing on the CPU backend | ✅ |
| 1 | `cfd-gpu`: adapter discovery/ranking, device limits, clear errors; WGSL prelude generated from Rust; naga validation test | ⏳ |
| 2 | `WgpuLbm`: SoA buffers, packed flags, ping-pong bind groups, D2Q9 TRT + Guo kernel; parity on Taylor-Green and Poiseuille | ⏳ |
| 3 | Link-wise boundaries in WGSL; parity on the cavity, the cylinder and outlet channels | ⏳ |
| 4 | Thermal D2Q5 + Boussinesq in WGSL; parity on the heated cavity | ⏳ |
| 5 | GPU reductions (diagnostics, forces), non-blocking `step`, `finish`, async double-buffered readback | ⏳ |
| 6 | `--backend wgpu`/`auto` in `run`/`bench`/`verify`; `verify --full --backend wgpu`; benchmarks recorded | ⏳ |
| 7 | CI (lavapipe/WARP), plan checkboxes, docs (02/03/04/07, development, benchmarks), final verification | ⏳ |

## Decisions taken with the author

- `u16` flags are uploaded as the same bytes, two cells per WGSL `u32` word, unpacked with a shift.
- `step()` is non-blocking (≤ 2 batches in flight); a divergence may be reported one call late.
  New `Solver` methods with synchronous defaults: `finish()`, `request_fields()`, `take_fields()`.
- Parity metric: per-field maximum difference, ρ absolute, u and θ normalised by the field maximum,
  < 1e-5.
- `--backend auto` ignores software adapters (WARP, lavapipe) and falls back to the CPU.
- The f16 sampling kernel is deferred to Phase 3 (frame streaming).
- Rayleigh-Bénard: growth rates of the critical mode at several Ra, Ra_c from a linear fit of σ(Ra).

## Found along the way

## Final check
