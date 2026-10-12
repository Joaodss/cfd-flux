Let's do **Phase 2c — engine comparison** (and, if there is time, the **design** of **Phase 3 — server and jobs**) of the project in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the session log `docs/sessions/2026-10-12-phase2b-cuda.md`, ADR-017..022 in `docs/07-decisions-and-questions.md`, the "Phase 2" (2c) and "Phase 3" sections of `docs/05-implementation-plan.md`, `docs/04-numerical-methods.md` §3.5 and §7, `docs/benchmarks.md` and `docs/development.md`. Phase 2b is complete: three backends behind the `Solver` trait — `CpuLbm` (`cfd-lbm`, the numerical reference), `WgpuLbm` (`cfd-gpu`) and `CudaLbm` (`cfd-cuda`, `cuda` feature, on by default in `cfd-cli`; bit-identical to the CPU with `--fmad=false`). Kernel constants and the parameter block come from `cfd_lbm::abi`; the CPU ↔ GPU parity cases and the difference metric live in `cfd_verify::parity` (`differences`, `max_diff`, `max_abs`) — reuse them, do not copy them. `cfd-cli run/bench/verify --backend auto|cpu|wgpu|cuda` work (`crates/cfd-cli/src/backend.rs` resolves the backend; `SolverChoice::make` builds a solver). The validation suite passes 13/13 on all three backends (`validation/report*/`). On the RTX 4060 (driver 617.42 = CUDA 13.4) both GPU backends run at ≈ 2,850–2,950 MLUPS at 1024² on a quiet desktop; the Windows compositor can cost up to 20% when the display is busy (benchmark with nothing animating, display at 50 Hz).

Session goals (in this order, each step tested before moving on):
1. `cfd-cli compare scene.json --backends cpu,wgpu,cuda --steps N` (default: every available backend): runs the same scene on each backend from the same initial state and reports, against the first backend (the CPU when present) as the reference:
   - per field (ρ, ux, uy, θ and the physical output fields): max absolute difference, relative L2 difference, and the metric of 04 §7 (ρ absolute, u and θ relative to the field maximum) with pass/fail against 1e-5 — reuse `cfd_verify::parity`;
   - "bit-identical" when the fields are equal, as CUDA ↔ CPU should be;
   - a difference map per field as PNG (reuse `cfd-io`'s image code and the colour maps of `render`), written to `--out`;
   - diagnostics (mass, kinetic energy, max |u|), forces per element, and MLUPS per backend (timed with `finish`);
   - a Markdown report (`README.md`) plus `compare.json`, in the style of the `verify` reports; a non-zero exit code when a backend is outside the tolerance.
   For chaotic runs (e.g. the Re 100 cylinder over many periods), document that instantaneous fields diverge and compare statistics instead (St, mean/max C_D from the force series), as 04 §7 says.
2. `cfd-cli compare --validation` (or a documented loop): a comparison report for the validation scenes (`cfd_verify::scenes`), committed to `validation/compare/`.
3. `cfd-cli bench --backends wgpu,cuda`: one interleaved run (backend A, backend B, repeat) producing a single MLUPS table per resolution with a column per backend, in Markdown and `--json`; update `docs/benchmarks.md` with a quiet-desktop run.
4. Tests: `compare` end to end on a small scene (CPU vs CPU must be bit-identical; GPU backends when available, skipped otherwise as in the existing tests), the report and the exit code; `bench --backends` parsing.
5. Docs: plan checkboxes (2c), `docs/development.md` commands, README, session checklist and final check. Mark the Phase 2 acceptance as fully met if it is.
6. (If there is time) **Phase 3 design only**, no code: propose the `cfd-server` architecture (axum routes, job manager and per-device semaphore, the worker thread with its command loop from ADR-011, WebSocket frames with backpressure, the GPU sampling/f16 kernel moved from Phase 2a, "macroscopic fields only when needed" from the new Phase 3 item, `ResultStore`/SQLite, `/api/capabilities`) and wait for my OK in the next session.

Known points from Phase 2b to keep in mind:
- `SolverArgs::resolve` (cfd-cli) also configures rayon's global thread pool, which can be done only once per process: resolving several backends needs that part split out.
- `Solver` is `Send` but not `Sync`; GPU solvers are asynchronous (`step` returns before the work is done; call `finish` before reading the clock; `request_fields` overlaps copies with stepping).
- CUDA tests skip without a GPU unless `CFD_REQUIRE_CUDA=1`; wgpu tests use `CFD_REQUIRE_GPU=1`; CI has no NVIDIA GPU (the `cuda-kernels` job only compiles the kernels with NVRTC).
- Shell tip from the last session: avoid Perl/sed replacements with backslashes in Markdown (they turned `\v`, `\b` into control characters); use the Edit tool for such text.

How to work with me:
- I have a lot of CFD experience but little Rust/GPU experience (I come from C#): explain Rust idioms as they come up (e.g. traits and generics, error handling with `anyhow`/`thiserror`, iterators), with C#/.NET parallels where they help; no need to explain the physics.
- Before coding, propose the design and wait for my OK: the `compare` command's options and output layout, what is compared and how (reusing `cfd_verify::parity`), how backends are created side by side (one `SolverChoice` per backend), the report format, and the `bench --backends` output.
- Keep the CPU backend as the numerical reference.
- Create a session checklist in `docs/sessions/` and verify at the end that everything was done.
- Work on a branch (`feat/phase2c-compare`), small commits, and tick the plan checkboxes when done. Everything in the repository is in English.
- If the session gets long, stop after step 3 or 4 with everything tested and committed.
