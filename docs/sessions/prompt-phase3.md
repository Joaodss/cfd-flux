Let's do **Phase 3 — server and jobs** of the project in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the session log `docs/sessions/2026-10-13-phase2c-compare.md`, the design proposal `docs/sessions/phase3-design-proposal.md` (written at the end of Phase 2c, **not yet approved**), ADR-006, ADR-010, ADR-011, ADR-017, ADR-020, ADR-021 and ADR-023 in `docs/07-decisions-and-questions.md`, the "Phase 3" section of `docs/05-implementation-plan.md`, `docs/02-architecture.md`, `docs/03-data-model.md` §6–7 and `docs/development.md`. Phase 2 is complete: three backends behind the `Solver` trait (CPU reference, wgpu, CUDA bit-identical to the CPU), `cfd-cli run/render/bench/verify/compare`, the comparison report `validation/compare/` (13/13).

Session goals:
1. Go through the open questions (Q1–Q8) of the design proposal with me, update the proposal, and wait for my OK before coding.
2. Then implement in the proposal's order (§11), each step tested and committed before the next: extract `cfd-engines` and `cfd-job` from the CLI with no behaviour change; the command loop; the server skeleton; the job manager with per-device semaphores and SQLite; WebSocket streaming with backpressure and the `cfd-cli submit` acceptance client (submit the cylinder, receive frames in real time, cancel in < 1 s).
3. If there is time: the on-device sampling/f16 kernels and the `skip_macro` kernel variant, with parity cases.
4. Docs (02, 03 §6–7, ADRs), plan checkboxes, session checklist and final check.

How to work with me: as before — I have a lot of CFD experience but little Rust/GPU experience (I come from C#): explain Rust idioms as they come up (async/tokio vs threads, channels, `Arc`/`Mutex`, axum extractors and state, error handling), with C#/.NET parallels; propose designs before coding; keep the CPU backend as the numerical reference; work on a branch (`feat/phase3-server`), small commits, everything in the repository in English; stop after step 2 if the session gets long, with everything tested and committed.
