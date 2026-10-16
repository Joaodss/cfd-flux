# Session checklist — Phase 3, frame codec and kernel items (2026-10-16)

Request: see [prompt-phase3-kernels.md](prompt-phase3-kernels.md). Branch `feat/phase3-kernels`.
Design: [phase3-design-proposal.md](phase3-design-proposal.md) §6–7. Stop after goal 2 if the session gets long,
with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Designs for the codec, on-device sampling and `skip_macro` proposed and approved | ✅ as proposed |
| 1 | LFFR v2 (`codec` field), byte shuffle + `lz4_flex`, v1 still read; measurements; ADR-007, 03 §6.1, decoder plan | ✅ |
| 2 | On-device sampling (WGSL + CUDA), `Solver::request_samples`, parity cases, wired into `cfd-job` and the stream | ✅ ADR-028 |
| 3 | `skip_macro` kernel variant (CPU, WGSL, CUDA), parity case, benchmark | ✅ ADR-029 |
| 4 | Docs (04, 02, ADRs), plan checkboxes, final check | ✅ ADR-007 revised, ADR-028, ADR-029 |

## Decisions taken with the author

- Codec, sampling and `skip_macro` designs approved as proposed (2026-10-16), including one deviation from the
  design proposal: `Solver::request_samples(&[SampleSpec])` takes several specs at once (the stored frame and one
  per stream format), so the CPU default copies the lattice fields once for all of them.

## Found along the way

- **Frame codec:** a 1024² f32 frame encodes in 13.6 ms to 64% of its size (`ruzstd`: 328 ms, 91%); the server
  streams the cylinder at 160–190 frames/s on CUDA (was 37.5). The blocked byte shuffle is 2.9× faster than the
  scalar loop; baseline x86-64 has no byte-shuffle instruction, so the rest is memory bandwidth.
- **On-device sampling:** CUDA's samples are bit-identical to the CPU conversion on the first run (f64 in the
  CPU's order, `--fmad=false`, `cvt.rn.f16.f32`); wgpu within 1e-5 and one f16 ulp. `pack2x16float` rounds to
  nearest-even on the RTX 4060 (Vulkan): a probe test of exact ties, subnormal ties and the overflow threshold
  found no difference from `half` (the WGSL spec allows either neighbour; the test only asserts one ulp). The
  output thread's work for a 1024² frame (stored f32 + an f16 stream format) falls from 43 to 13 ms.
- **Not obvious:** the samples are fused (fields computed inside the box filter, one thread per output word), so
  no full-resolution scratch buffer is needed (it would be 400 MB at 4096²); f16 values are written two per
  32-bit word on both GPU backends (WGSL has no 16-bit storage), padded at an odd count.
- **`skip_macro`:** +16.3–16.8% at ≥ 1024² isothermal on both GPU engines (84/72 B), +14.5–14.8% thermal (130/114 B), +62–68% at
  512² (the lattice then fits the L2 cache). Storing u only where a flag set inside the pull loop said a
  pressure outlet had read it made wgpu's 256² kernel 20% slower than storing every step; storing it on every
  cell with a boundary link (`mask != 0`, known before the loop) is faster everywhere. A mutation (no u store on
  those cells) makes the parity case fail on all three backends.

## Small deviations from the design proposal

- `request_samples` takes several specs (approved), and its handle returns a `SampleSet` that is converted where
  it is collected: the CPU default never converts on the solver thread.
- Stream formats that appear between a frame's request and its publication get no frame for that output event;
  the snapshot that subscribing already triggers fills the gap.
- Colour ranges come from the samples (the stored full-resolution frame when frames are stored, else the
  downsampled stream samples).
- `skip_macro` stores u on every cell with a boundary link, a superset of the pressure-outlet cells of §7 (faster;
  see above). It is an `LbmConfig` field (on by default, `--no-skip-macro`) so the parity case can build both.

## Final check

- `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` (with CUDA, the default) and
  `cargo clippy -p cfd-cli -p cfd-cuda -p cfd-engines -p cfd-job -p cfd-server --all-targets --no-default-features
  -D warnings`: clean.
- `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **199 tests pass** (was 180): 6 new frame-format
  tests (codecs, v1 compatibility, raw fields, shuffle, corrupt lz4), 3 `cfd_core::sample`, 2 `abi` layout tests, a
  WGSL layout test, the `pack2x16float` probe test, 3 parity cases × 2 GPU backends (sampling, thermal sampling,
  `skip_macro`) and the CPU `skip_macro` case; the stream tests were rewritten for samples. Without the `cuda`
  feature the CLI, server, job and engine tests pass (60).
- Measurements: frame codec and server stream, on-device sampling, `skip_macro` (all in
  [benchmarks](../benchmarks.md)).
- Not done / follow-ups: CI has not run on this branch (not pushed); the WGSL `pack2x16float` rounding was only
  checked on the RTX 4060 (Vulkan) — other drivers stay held to one ulp; the run loop's sampling every 6 steps now
  limits small-grid streaming (on-device probe sampling would help, as noted in the Phase 3 server session).

## After the PR was opened

- **CI (Windows, wgpu on DX12) failed:** FXC rejected `sample.wgsl` ("not all control paths return a value" for a
  `switch` with a `return` in every case, after naga's translation to HLSL). Rewritten with one return. Locally
  reproduced and checked with `WGPU_BACKEND=dx12`, which also showed:
  - `pack2x16float` rounds **toward zero** on DX12 (124 of 334 probes one ulp lower than RNE), RNE on Vulkan: the
    one-ulp bound of the wgpu sampling parity is needed.
  - The two `skip_macro` pipelines round slightly differently on DX12 (≤ 8e-7 of the run's peak speed; Vulkan is
    exact), so the parity case holds wgpu to the 04 §7 tolerance, measured against the run's peak speed (the
    periodic box's vortex decays to ~1e-8, where a relative metric only measures round-off).
- Benchmarks re-run on a quiet desktop: `skip_macro` +16.3–16.8% isothermal, +14.5–14.8% thermal at ≥ 1024²
  (2,874 → 3,343 MLUPS on CUDA); sampling, codec and stream results unchanged within a few percent (CUDA stream
  214–215 frames/s).
- Next session: [prompt-phase4.md](prompt-phase4.md) (housekeeping, then the Phase 4 editor design proposal).
