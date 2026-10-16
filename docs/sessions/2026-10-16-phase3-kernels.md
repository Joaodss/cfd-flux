# Session checklist — Phase 3, frame codec and kernel items (2026-10-16)

Request: see [prompt-phase3-kernels.md](prompt-phase3-kernels.md). Branch `feat/phase3-kernels`.
Design: [phase3-design-proposal.md](phase3-design-proposal.md) §6–7. Stop after goal 2 if the session gets long,
with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Designs for the codec, on-device sampling and `skip_macro` proposed and approved | ✅ as proposed |
| 1 | LFFR v2 (`codec` field), byte shuffle + `lz4_flex`, v1 still read; measurements; ADR-007, 03 §6.1, decoder plan | ✅ |
| 2 | On-device sampling (WGSL + CUDA), `Solver::request_samples`, parity cases, wired into `cfd-job` and the stream | ⏳ |
| 3 | `skip_macro` kernel variant (CPU, WGSL, CUDA), parity case, benchmark | ⏳ |
| 4 | Docs (04, 02, ADRs), plan checkboxes, final check | ⏳ |

## Decisions taken with the author

- Codec, sampling and `skip_macro` designs approved as proposed (2026-10-16), including one deviation from the
  design proposal: `Solver::request_samples(&[SampleSpec])` takes several specs at once (the stored frame and one
  per stream format), so the CPU default copies the lattice fields once for all of them.

## Found along the way

- **Frame codec:** a 1024² f32 frame encodes in 13.6 ms to 64% of its size (`ruzstd`: 328 ms, 91%); the server
  streams the cylinder at 160–190 frames/s on CUDA (was 37.5). The blocked byte shuffle is 2.9× faster than the
  scalar loop; baseline x86-64 has no byte-shuffle instruction, so the rest is memory bandwidth.
