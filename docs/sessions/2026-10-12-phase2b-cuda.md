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
| 5 | `--backend cuda`, `auto` = CUDA > wgpu > CPU (fallback tested); `verify --full --backend cuda`; benchmarks and wgpu vs CUDA discussion | ✅ |
| 6 | (2c) `cfd-cli compare`, `bench --backends` | ⬜ not started: the session stopped after step 5 (see below) |
| 7 | CI (`--features cuda` without a GPU, NVRTC-only kernel compile), plan checkboxes, docs, ADR-021, final verification | ✅ (CI not run yet: branch not pushed) |

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
  (when `bin\x64` is not on `PATH`) must preload the builtins library as well.
- `compute-sanitizer`: memcheck finds no errors on the parity suite; racecheck finds no hazards, but
  terminates the process silently when ~1000 launches of the 37k-cell cylinder kernel are queued at once
  (a resource limit of the tool: 500 per call is clean, and the 4k-cell cases are clean at 1000).
- **Launch overhead:** a CUDA kernel launch costs ≈ 8 µs of CPU time on Windows (WDDM), more than a 256²
  step, so the GPU waited for the CPU (6,000 vs wgpu's 9,300 MLUPS). Replaying 50 captured steps per CUDA
  graph brings it to ≈ 0.6 µs per step and 256² to wgpu's rate (ADR-021).
- **The remaining 3–8% gap to wgpu at ≥ 1024² is the desktop compositor, not the kernel:** Nsight Systems
  shows 83% of the 1024² CUDA steps at 360–380 µs (wgpu's rate), and ~8% stretched by 200–600 µs once
  every ~6.9 ms — one refresh of the 144 Hz display driven by the same GPU.
- FMA (`--fmad`) changes neither parity tolerance compliance nor throughput (bandwidth-bound kernel).
- `cudarc`'s defaults are tuned for safety, not for a solver: per-buffer event tracking (several driver
  calls per launch) and write-combined pinned memory (slow for the CPU to read) are both turned off here.
- Nsight Compute needs the GPU performance counters to be enabled for all users (administrator setting);
  Vulkan tracing in Nsight Systems needs administrator rights as well, so the wgpu side was not traced.

## Final check

- `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` (with CUDA, the default) and
  `cargo clippy -p cfd-cli -p cfd-cuda --all-targets --no-default-features -D warnings`: clean.
- `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **120 tests pass** (was 89; new: 12 CPU ↔ CUDA
  parity tests on the shared harness, device/NVRTC/layout tests, graph equivalence, CLI `run` on CUDA, the
  auto-fallback chain end to end, `abi` generator tests). Without the `cuda` feature the CUDA tests skip.
- Parity on the RTX 4060: CUDA **bit-identical** to the CPU (ρ, u, θ) on all ten field cases; wgpu unchanged
  (≤ 2.7e-6) after moving its parity tests onto the shared harness and its constants onto `cfd_lbm::abi`.
- `compute-sanitizer --tool memcheck` on the parity and graph tests: 0 errors.
- `cfd-cli verify --full --backend cuda`: **13/13 in 66 s** (report `validation/report-cuda/`, code `b6a5d72`):
  every metric equal to the CPU report and all 19 plots/snapshots byte-identical. Poiseuille's tightest check
  6.606e-6 (CPU 6.606e-6, wgpu 8.6e-6); Re 100 cylinder St 0.3000, C_D,max 3.258, C_L,max 0.9905.
- Benchmarks (RTX 4060): 2,600–2,750 MLUPS isothermal and 1,690–1,820 thermal at ≥ 1024², 9,300–9,700 at 256²
  → acceptance (≥ 1,000 MLUPS at 1024², aim ≥ 2,000) met.
- Not done / follow-ups: 2c (`cfd-cli compare`, `bench --backends`); CI with the new `cuda-kernels` job has
  not run yet (needs a push); the optional AA pattern / Esoteric Pull; the f16 sampling kernel (Phase 3).
