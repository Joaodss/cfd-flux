Let's continue **Phase 1 — step 6 (CLI and validation suite)** of the project in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the session log `docs/sessions/2026-10-10-phase1.md` (what was done, decisions, open work), ADR-014..017 in `docs/07-decisions-and-questions.md`, the "Phase 1" section of `docs/05-implementation-plan.md`, `docs/04-numerical-methods.md` §3.5 and §7, and `docs/08-validation-guide.md`. Steps 1–5 are merged: `Scene → Domain` and units in `cfd-core`, the `Solver` trait, and the CPU LBM solver `CpuLbm` in `cfd-lbm` (D2Q9 + D2Q5, TRT/BGK, link-wise BCs, Guo/Boussinesq, diagnostics, forces, probes), with 55 tests.

Session goals (in this order, each step tested before moving on):
1. `cfd-cli run scene.json --out DIR`: runs a scene on the CPU backend, writes binary frames (`cfd-io`) every `outputInterval`, PNG snapshots with a colour map (velocity magnitude, vorticity, temperature), `probes.csv`, `forces.csv`, `meta.json` (units, Re/Ma/Pr/Ra, warnings, timings, diagnostics); aborts cleanly on NaN.
2. `cfd-cli bench`: MLUPS at several resolutions (isothermal and thermal), reproducible; record results in `docs/benchmarks.md`.
3. `cfd-cli verify [--quick|--full]` with reference data in `validation/references/` (sources cited): Taylor-Green → Poiseuille → Couette → lid-driven cavity (Ghia 1982, Re 100/400/1000) → cylinder (Schäfer-Turek 2D-1 Re 20: C_D; 2D-2 Re 100: St, C_D,max) → heated cavity (de Vahl Davis, Ra 1e3–1e6: mean Nu), including convergence orders; Markdown report with errors and plots in `validation/report/`. `--quick` must be fast enough to run in CI.
4. Phase 1 acceptance: all applicable cases within the tolerances of `docs/04-numerical-methods.md` §7, ≥ 50 MLUPS on the CPU, von Kármán street visible for the Re 100 cylinder (PNG sequence from `cfd-cli run`).

How to work with me:
- I have a lot of CFD experience but little Rust/GPU experience (I come from C#): explain Rust idioms as they come up, with C# parallels; no need to explain the physics.
- Before coding, propose the design (CLI options, output layout, how validation cases are defined and where they live — backend-agnostic via `Box<dyn Solver>` so Phase 2 can reuse them) and wait for my OK.
- Create a session checklist in `docs/sessions/` and verify at the end that everything was done.
- Work on a branch (`feat/phase1-cli-verify`), small commits, and tick the plan checkboxes when done. Everything in the repository is in English.
- If the session gets long, stop after step 2 or 3 with everything tested and committed.
