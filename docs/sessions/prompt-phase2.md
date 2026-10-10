Let's start **Phase 2a — wgpu/WGSL backend** of the project in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the session logs `docs/sessions/2026-10-10-phase1.md` and `docs/sessions/2026-10-10-phase1-cli-verify.md`, ADR-002 and ADR-014..018 in `docs/07-decisions-and-questions.md`, the "Phase 2" section of `docs/05-implementation-plan.md`, `docs/04-numerical-methods.md` §3.5 (implementation notes shared by every backend) and §7 (parity tolerance), `docs/benchmarks.md` and `docs/development.md`. Phase 1 is complete: the CPU reference solver `CpuLbm` in `crates/cfd-lbm` (memory layout documented in `kernel.rs`), the `Solver` trait in `cfd-core`, `cfd-cli run/render/bench/verify`, and the backend-agnostic validation suite `cfd-verify` (12/12 cases pass on the CPU; report in `validation/report/`). `crates/cfd-gpu` is an empty skeleton.

Session goals (in this order, each step tested before moving on):
0. Carried over from Phase 1: the Rayleigh-Bénard onset case (Ra_c ≈ 1708, 04 §7) in `cfd-verify`, passing on the CPU backend.
1. `cfd-gpu`: adapter discovery and selection (prefer a discrete GPU, accept software adapters such as WARP/lavapipe for tests), device limits (`max_storage_buffer_binding_size`, workgroup limits), a clear error or CPU fallback when there is no adapter or not enough memory.
2. `WgpuLbm` implementing `Solver`: the same SoA layout as the CPU (`f[i·n + cell]`, shifted populations, rest population as the remainder, ping-pong), the domain's flags / `bc_slot` / `BcParams` table uploaded unchanged (decide how to handle the `u16` flags, since WGSL has no 16-bit storage type), and a D2Q9 TRT fused pull stream-collide WGSL kernel with Guo forcing. Parity test against `CpuLbm` on Taylor-Green and Poiseuille.
3. Link-wise boundaries in WGSL (bounce-back with wall velocity, anti-bounce-back pressure outlet, zero-gradient outlet, periodic edges); parity on the cavity and the cylinder.
4. Thermal D2Q5 + Boussinesq in WGSL; parity on the heated cavity.
5. GPU reductions for `diagnostics()` and `forces()` (momentum exchange), field readback for `lattice_fields()` / sampling, and asynchronous readback with double-buffered staging so stepping does not wait for copies.
6. Wire `--backend wgpu` (and `auto`) into `cfd-cli run/bench/verify`: `cfd-cli verify --full --backend wgpu` passes with the same tolerances as the CPU; `cfd-cli bench --backend wgpu` results recorded in `docs/benchmarks.md`.

Acceptance for this session (from the Phase 2 plan, wgpu part): CPU ↔ wgpu parity within the tolerance of 04 §7 (max relative difference < 1e-5 in the macroscopic fields after N steps; statistics such as St and mean C_D for the chaotic cylinder), the full validation suite passes on wgpu, and ≥ 1,000 MLUPS at 1024² on the RTX 4060. CUDA (2b) and `cfd-cli compare` (2c) are for the next session(s).

How to work with me:
- I have a lot of CFD experience but little Rust/GPU experience (I come from C#): explain Rust, wgpu and WGSL idioms as they come up (adapters/devices/queues, buffers and bind groups, workgroups and dispatch, async mapping, the GPU memory model), with C#/.NET parallels where they help; no need to explain the physics.
- Before coding, propose the design and wait for my OK: crate structure, buffer layout and bind groups, how the `u16` flags and the `#[repr(C)]` `BcParams` are uploaded, workgroup size and dispatch, ping-pong without copies, how `step(n)` avoids CPU round-trips, how reductions and readback work, how the WGSL kernel is kept in sync with `kernel.rs` (shared constants, a single source of truth where possible), and how parity tests run in CI (skip cleanly when there is no adapter).
- Keep the CPU backend as the numerical reference: every GPU kernel needs a parity test, and the layout must stay identical so CUDA (2b) can reuse it.
- Create a session checklist in `docs/sessions/` and verify at the end that everything was done.
- Work on a branch (`feat/phase2a-wgpu`), small commits, and tick the plan checkboxes when done. Everything in the repository is in English.
- If the session gets long, stop after step 3 or 4 with everything tested and committed.
