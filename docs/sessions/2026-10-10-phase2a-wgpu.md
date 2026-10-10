# Session checklist — Phase 2a, wgpu/WGSL backend (2026-10-10)

Request: see [prompt-phase2.md](prompt-phase2.md). Branch `feat/phase2a-wgpu`.
Stop after step 3 or 4 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Design proposal (crate layout, buffers/bind groups, flags, dispatch, readback, CI) approved by the author | ✅ |
| 0 | Rayleigh-Bénard onset case in `cfd-verify`, passing on the CPU backend | ✅ |
| 1 | `cfd-gpu`: adapter discovery/ranking, device limits, clear errors; WGSL prelude generated from Rust; naga validation test | ✅ |
| 2 | `WgpuLbm`: SoA buffers, packed flags, ping-pong bind groups, D2Q9 TRT + Guo kernel; parity on Taylor-Green and Poiseuille | ✅ |
| 3 | Link-wise boundaries in WGSL; parity on the cavity, the cylinder and outlet channels | ✅ |
| 4 | Thermal D2Q5 + Boussinesq in WGSL; parity on the heated cavity | ✅ |
| 5 | GPU reductions (diagnostics, forces), non-blocking `step`, `finish`, async double-buffered readback | ✅ |
| 6 | `--backend wgpu`/`auto` in `run`/`bench`/`verify`; `verify --full --backend wgpu`; benchmarks recorded | ✅ |
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

- **GPU compilers fold NaN checks on computed values.** A bit test `(bits & 0x7f800000) == 0x7f800000`
  on ρ never fired on the RTX 4060 (Vulkan): LLVM recognises it as an Inf/NaN class test and, since
  Vulkan does not require NaN/Inf preservation, folds it to `false` for arithmetic results. Testing the
  *loaded* rest population instead works (divergence flagged one step later); ADR-019.
- **Parity metric:** normalising each velocity component by its own maximum fails in a channel, where
  uy is zero up to round-off (relative "differences" of 2); both components now share the scale. With
  velocities ≪ the lattice velocity (the first steps of Rayleigh-Bénard) f32 round-off of the
  populations (~1e-7) is ~2e-5 of max |u|, so parity cases must develop a real flow.
- wgpu 30 needs Rust 1.87 (workspace MSRV raised from 1.85), which also enabled clippy's
  `manual_is_multiple_of`.
- The kernel is bandwidth-bound from 1024² on (≈ 90% of 272 GB/s); workgroup size (32–512) and Vulkan
  vs DX12 change nothing measurable. 256²–512² grids run from the 24 MB L2 cache (7,000–9,000 MLUPS).
- The full validation suite runs in ~80 s on the GPU (1,351 s on 16 CPU threads); the metric values
  agree with the CPU to 3–4 digits, the cylinder statistics (St, C_D,max) to 4.
- `cfd-cli verify`/`bench`/`run` default to `auto`, which is now the GPU on a machine with one:
  the docs and CI name `--backend cpu` explicitly where the CPU is meant.

## Final check
