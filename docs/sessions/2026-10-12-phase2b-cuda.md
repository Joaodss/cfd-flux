# Session checklist — Phase 2b, CUDA backend (2026-10-12)

Request: see [prompt-phase2b.md](prompt-phase2b.md). Branch `feat/phase2b-cuda`.
Stop after step 3 or 4 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Design proposal (crates/features, compilation and caching, constant generator, layout and ping-pong, launches, async stepping, reductions/readback, parity harness, `unsafe` boundary, CI) approved by the author | ✅ |
| 1 | `cfd-cuda` with `cudarc` (`cuda` feature, dynamic loading): device discovery, compute capability, memory checks, NVRTC → `sm_XY` cubin (PTX only when the driver allows it), clear errors without a GPU/NVRTC | ✅ |
| 2 | Shared constants: `cfd_lbm::abi` (`KernelParams`, WGSL prelude, CUDA header with `static_assert` layout checks); wgpu moved onto it | ✅ |
| 3 | `CudaLbm`: SoA layout, pointer ping-pong, fused kernel translated from `lbm.wgsl`; shared parity harness in `cfd_verify::parity`; FMA policy decided from parity numbers | ✅ |
| 4 | Warp-shuffle diagnostics, link gather, non-blocking `step` with events, `finish`, `request_fields` via a copy stream and pinned buffers | ✅ |
| 5 | `--backend cuda`, `auto` = CUDA > wgpu > CPU (fallback tested); `verify --full --backend cuda`; benchmarks and wgpu vs CUDA discussion | ⬜ |
| 6 | (2c) `cfd-cli compare`, `bench --backends` | ⬜ |
| 7 | CI (`--features cuda` without a GPU, NVRTC-only kernel compile), plan checkboxes, docs, ADR-021, final verification | ⬜ |

## Decisions taken with the author

- `cudarc` 0.19 with dynamic loading and the CUDA 13.0 bindings: builds without the toolkit; needs a
  driver ≥ 13.0 and a CUDA 13 NVRTC at runtime.
- `cuda` feature on by default in `cfd-cli` (off in the library crates); CI builds and lints with it.
- All `unsafe` in `cfd-cuda/src/raw.rs`, one typed safe wrapper per kernel with a `// SAFETY:` argument.
- Parity harness shared by both GPU backends in `cfd_verify::parity`.
- CI gains an NVRTC-only install (no GPU) so the kernels and their layout `static_assert`s compile.
- The f16 sampling kernel stays in Phase 3.

## Found along the way

- **CUDA with `--fmad=false` is bit-identical to the CPU reference** on all ten parity field cases (300–6000
  steps, every boundary kind, thermal): same operations in the same order, IEEE division and square root,
  no contraction. With `--fmad=true` the differences are 1e-7–8e-7, like wgpu (whose driver contracts too).
- NVRTC loads `nvrtc-builtins64_134.dll` by name when compiling, so preloading NVRTC from `CUDA_PATH`
  (when `bind` is not on `PATH`) must preload the builtins library as well.
- `compute-sanitizer`: memcheck finds no errors on the parity suite; racecheck finds no hazards, but
  terminates the process silently when ~1000 launches of the 37k-cell cylinder kernel are queued at once
  (a resource limit of the tool: 500 per call is clean, and the 4k-cell cases are clean at 1000).

## Final check
