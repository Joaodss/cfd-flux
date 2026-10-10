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
| 7 | CI (lavapipe/WARP), plan checkboxes, docs (02/03/04/07, development, benchmarks), final verification | ✅ |

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
- `cargo fmt --check`, `cargo clippy --workspace --all-targets -D warnings`: clean.
- `CFD_REQUIRE_GPU=1 cargo test --workspace`: **89 tests pass** (was 68; new: 12 CPU ↔ wgpu parity tests,
  naga shader/layout tests, end-to-end `run` on both backends, Rayleigh-Bénard and helper tests). The parity
  tests also pass on WARP (`CFD_WGPU_ADAPTER="basic render"`, 33 s).
- Parity on the RTX 4060: max difference ρ ≤ 2.4e-7, u ≤ 2.3e-6, θ ≤ 9.1e-7 (tolerance 1e-5) on Taylor-Green
  (TRT, BGK), Poiseuille, Couette, cavity, cylinder, a channel with every boundary kind (incl. zero-gradient
  and heat-flux), heated cavity and Rayleigh-Bénard; diagnostics and forces agree as well.
- `cfd-cli verify --full`: **13/13 on both backends** — CPU 1369 s, wgpu 77 s (reports in
  `validation/report/` and `validation/report-wgpu/`; both from code `a5c7055`, the `-dirty` suffix comes
  from documentation/report files being edited during the runs). Rayleigh-Bénard: Ra_c = 1707.0 at H = 64
  (−0.04%). Re 100 cylinder: St 0.3000 / C_D,max 3.258 on both, C_L,max 0.9905 (CPU) vs 0.9904 (wgpu).
- `verify --quick --backend wgpu --allow-software` on WARP: 9/9 in 178 s (the CI step uses lavapipe). The
  tightest check is Poiseuille's "largest L2 error < 1e-5" (exact scheme, f32 floor): 6.6e-6 CPU,
  8.6e-6 RTX 4060, 6.7e-6 WARP — if lavapipe ever exceeds it, that limit (not the solver) needs revisiting.
- Benchmarks (RTX 4060): 2,834 MLUPS isothermal / 1,812 thermal at 1024², ≈ 2,850 / 1,890 up to 4000²
  → acceptance (≥ 1,000 MLUPS at 1024²) met; 22× the CPU backend.
- `cfd-cli run scenes/cylinder-re100.json --backend wgpu --end-time 12 --output-interval 0.05`: 241 frames,
  vortex street as on the CPU, 1,817 MLUPS including output.
- Not done / follow-ups: CUDA (2b), `cfd-cli compare` (2c), the optional AA pattern / Esoteric Pull, the GPU
  f16 sampling kernel (moved to Phase 3). CI with lavapipe has not run yet (first push of this branch).
