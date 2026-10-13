# Session checklist — Phase 3, server and jobs (2026-10-13)

Request: see [prompt-phase3.md](prompt-phase3.md). Branch `feat/phase3-server`.
Design: [phase3-design-proposal.md](phase3-design-proposal.md). Stop after step 2 if the session gets long,
with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Open questions Q1–Q8 of the design proposal answered; proposal approved by the author | ✅ all eight as proposed |
| 1 | `cfd-engines` and `cfd-job` extracted from the CLI, no behaviour change | ✅ old/new `cfd-cli` byte-identical on CPU and CUDA |
| 2 | Command loop with time-capped batches, `JobObserver`, output thread; `cfd-cli run` on top of it | ✅ results independent of the batch size |
| 3 | `cfd-server` skeleton: config, tracing, `/api/capabilities`, `/api/scenes/validate`, limits, CORS | ✅ |
| 4 | `JobManager`, per-device semaphores, submit/status/pause/resume/cancel, SQLite, `LocalStore` | ✅ |
| 5 | WebSocket streaming (`watch`/`broadcast`), `cfd-cli submit`, integration tests, acceptance | ✅ acceptance met on CUDA, wgpu and CPU |
| 6 | (If there is time) on-device sampling + f16 kernels, parity cases | ⏳ |
| 7 | (If there is time) `skip_macro` kernel variant, parity case, benchmark | ⏳ |
| 8 | Docs (02, 03 §6–7, ADRs), plan checkboxes, final check | ✅ for steps 1–5 (ADR-024–027, Q13) |

## Decisions taken with the author

- Q1–Q8 of the proposal: every recommendation accepted (new crates `cfd-engines`/`cfd-job`; one job slot per
  physical GPU and one CPU job at a time, `--cpu-jobs`; `sqlx` with runtime queries; f32 stored frames with a
  per-job size limit; `interrupted` on restart, no checkpoints yet; `zstd+base64` layers and a 64 MB body
  limit with a clear `413`; `127.0.0.1:7878` with `--listen`; server first, kernels after).

## Small deviations from the proposal (for the author's review)

- `ResultStore` is a struct (`cfd_job::LocalStore`, local disk) rather than a trait: one implementation so
  far; it becomes a trait when an object store arrives.
- No `Step` command: stepping is the loop itself. A `Snapshot` command was added (a frame for a viewer that
  joins a paused job or asks for a new stream format).
- Engine choice reads its memory from CUDA (also for wgpu on the same GPU) and checks 90% of it; the CPU has no
  memory check (the grid limit, 4096², applies).
- Validation issues do not yet carry `params` (numbers as data for localised messages): that comes with the
  editor (Phase 4). Time estimates use the mean MLUPS of earlier completed jobs per engine.
- `meta.json` gains `run.framesStored` and the status `cancelled`; `cfd-cli run` output is otherwise unchanged.

## Found along the way

- **The run loop is free:** 2,878 MLUPS through `cfd-cli run --no-frames` on a 1024² cavity (CUDA) vs 2,885
  in `bench`; one-step and 50 ms batches give byte-identical results.
- **Cancel latency** over the API: 30–46 ms on CUDA, wgpu and the CPU (release). In the first debug-build run it
  was 0.8–0.9 s on the GPUs: the solver was blocked on a full output queue and a cancel waited for queued frames
  to be encoded. Fixed: a cancel drops queued frames and the loop reads commands while it waits for room
  (0.25–0.34 s in debug now).
- **A lagging viewer could miss the final frame** (the `lagged` path sent `done` without flushing it). Fixed:
  one exit path flushes the latest frame first. A viewer that stopped reading got 24 of 151 frames, the job
  stored all 151 and ran at full speed.
- **Frame encoding is the bottleneck of the server at high frame rates:** `ruzstd` takes 12.6 ms per f32
  frame of the 442 × 84 cylinder and 7.2 ms per f16 stream frame, and compresses these fields by only 3–4%
  (the low mantissa bits are noise). With a frame every 60 steps the job runs at ≈ 37 frames/s, paced by
  the output thread. Recorded as open question Q13 (no compression for stream frames, byte-shuffle before
  zstd, the C `zstd` crate, or f16 storage).
- CUDA and wgpu report the same PCI location (`pci:0000:26:00.0`) for the RTX 4060, so they share one slot.
- Sampling diagnostics, forces and probes every 6 steps (the default) halves the solver's throughput on
  the cylinder (batches end every 6 steps; ≈ 0.26 ms per sample, the probes copying every field).
  On-device probe sampling would help; not planned yet.

## Final check

- `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` (with CUDA, the default) and
  `cargo clippy -p cfd-cli -p cfd-cuda -p cfd-engines -p cfd-job -p cfd-server --all-targets --no-default-features
  -D warnings`: clean.
- `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **180 tests pass** (was 136; now 8 `cfd-engines`, 3 of them moved from the CLI
  (auto choice, inventory with fake devices, memory checks, software adapters), 3 + 6 `cfd-job` (batch sizing,
  job ids, cancel latency, pause/resume/snapshot/unsupported commands, orphaned pause, frames to observers,
  batch-size independence, storage limit), 4 + 23 `cfd-server` (downsampling, stream frames, hub; API, jobs,
  WebSocket, acceptance on CPU, wgpu and CUDA), 3 `cfd-cli submit`). Without the `cuda` feature the CLI and
  server tests pass with CUDA reported unavailable (44 tests).
- Acceptance with release builds: `cfd-cli submit scenes/cylinder-re100.json --cancel-after 3` on CUDA, wgpu and
  the CPU — frames in real time, cancel in 30–46 ms (table in [benchmarks](../benchmarks.md)).
- Not done / follow-ups: steps 6–7 (on-device sampling/f16, `skip_macro`) — prompt for the next session in
  [prompt-phase3-kernels.md](prompt-phase3-kernels.md); open question Q13 (frame encoding); CI has not run on
  this branch (not pushed).
