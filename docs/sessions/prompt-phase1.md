Let's start **Phase 1 — CPU solver core (LBM)** of the project in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the "Phase 1" section of `docs/05-implementation-plan.md`, `docs/04-numerical-methods.md` (§3 LBM, §4 thermal, §7 validation) and `docs/08-validation-guide.md`. Phase 0 is complete: `Scene` format in `crates/cfd-core`, frames in `crates/cfd-io`, CLI in `crates/cfd-cli`, 4 scenes in `scenes/`, CI green. `crates/cfd-lbm` is empty.

Session goals (in this order, each step tested before moving on):
1. `cfd-core`: `Domain` (precomputed per-cell flags), `Scene → Domain` conversion and a physical ↔ lattice units module with stability checks (τ, Mach) and warnings.
2. `cfd-lbm`: D2Q9 with a fused stream-collide kernel (pull, ping-pong), **SoA** layout designed to be identical in the future wgpu/CUDA backends; BGK and TRT (default); parallelised with `rayon`.
3. Boundaries: bounce-back, moving wall, velocity inlet (uniform/parabolic), pressure/zero-gradient outlet, periodic; Guo forcing.
4. Thermal D2Q5 + Boussinesq; fixed/adiabatic/flux thermal BCs.
5. Diagnostics (mass, energy, max velocity, NaN), momentum-exchange forces, probes.
6. `cfd-cli run` (PNG + frames) and `cfd-cli bench` (MLUPS); `cfd-cli verify` with the cases Taylor-Green → Poiseuille → cavity (Ghia) → cylinder (Schäfer-Turek) → heated cavity (de Vahl Davis), including convergence order.

Acceptance criteria: those of Phase 1 in the plan (validation within tolerance, ≥ 50 MLUPS on the CPU, von Kármán street visible for the Re 100 cylinder).

How to work with me:
- I have a lot of CFD experience but little Rust/GPU experience (I come from C#): explain Rust idioms as they come up, with C# parallels; no need to explain the physics.
- Before coding, propose the type design (`Domain`, `Lattice`, `Solver`/`Backend` traits) and wait for my OK.
- Create a session checklist in `docs/sessions/` and verify at the end that everything was done.
- Work on a branch (`feat/phase1-lbm-cpu`), small commits, and tick the plan checkboxes when done. Everything in the repository is in English.
- If the session gets long, stop after step 3 or 4 with everything tested and committed.
