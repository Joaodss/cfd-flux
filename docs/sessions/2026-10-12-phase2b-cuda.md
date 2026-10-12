# Session checklist — Phase 2b, CUDA backend (2026-10-12)

Request: see [prompt-phase2b.md](prompt-phase2b.md). Branch `feat/phase2b-cuda`.
Stop after step 3 or 4 if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Design proposal (crates/features, compilation and caching, constant generator, layout and ping-pong, launches, async stepping, reductions/readback, parity harness, `unsafe` boundary, CI) approved by the author | ✅ |
| 1 | `cfd-cuda` with `cudarc` (`cuda` feature, dynamic loading): device discovery, compute capability, memory checks, NVRTC → `sm_XY` cubin (PTX only when the driver allows it), clear errors without a GPU/NVRTC | ✅ |
| 2 | Shared constants: `cfd_lbm::abi` (`KernelParams`, WGSL prelude, CUDA header with `static_assert` layout checks); wgpu moved onto it | ✅ |
| 3 | `CudaLbm`: SoA layout, pointer ping-pong, fused kernel translated from `lbm.wgsl`; shared parity harness in `cfd_verify::parity`; FMA policy decided from parity numbers | ⬜ |
| 4 | Warp-shuffle diagnostics, link gather, non-blocking `step` with events, `finish`, `request_fields` via a copy stream and pinned buffers | ⬜ |
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

## Final check
