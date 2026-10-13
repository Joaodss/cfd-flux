# Session checklist — Phase 3, server and jobs (2026-10-13)

Request: see [prompt-phase3.md](prompt-phase3.md). Branch `feat/phase3-server`.
Design: [phase3-design-proposal.md](phase3-design-proposal.md). Stop after step 2 if the session gets long,
with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| D | Open questions Q1–Q8 of the design proposal answered; proposal approved by the author | ✅ all eight as proposed |
| 1 | `cfd-engines` and `cfd-job` extracted from the CLI, no behaviour change | ⏳ |
| 2 | Command loop with time-capped batches, `JobObserver`, output thread; `cfd-cli run` on top of it | ⏳ |
| 3 | `cfd-server` skeleton: config, tracing, `/api/capabilities`, `/api/scenes/validate`, limits, CORS | ⏳ |
| 4 | `JobManager`, per-device semaphores, submit/status/pause/resume/cancel, SQLite, `LocalStore` | ⏳ |
| 5 | WebSocket streaming (`watch`/`broadcast`), `cfd-cli submit`, integration tests, acceptance | ⏳ |
| 6 | (If there is time) on-device sampling + f16 kernels, parity cases | ⏳ |
| 7 | (If there is time) `skip_macro` kernel variant, parity case, benchmark | ⏳ |
| 8 | Docs (02, 03 §6–7, ADRs), plan checkboxes, final check | ⏳ |

## Decisions taken with the author

- Q1–Q8 of the proposal: every recommendation accepted (new crates `cfd-engines`/`cfd-job`; one job slot per
  physical GPU and one CPU job at a time, `--cpu-jobs`; `sqlx` with runtime queries; f32 stored frames with a
  per-job size limit; `interrupted` on restart, no checkpoints yet; `zstd+base64` layers and a 64 MB body
  limit with a clear `413`; `127.0.0.1:7878` with `--listen`; server first, kernels after).
