# CLAUDE.md

live-fluids — web CFD simulator: pixel editor in the browser → local Rust server → GPU solver (CUDA and wgpu) with CPU fallback. Status: **Phase 0 complete, Phase 1 next** (scene format, validation, frames, CLI, web skeleton); no solver yet.

- Documentation lives in `docs/` (index in `README.md`). Read `docs/05-implementation-plan.md` before starting any phase; commands and environment are in `docs/development.md`.
- Language: **everything in the repository is in English** — code, comments, docs, UI text, commit messages (the project is part of the author's English CV/portfolio). The author may chat in Portuguese; reply in their language, but write repository content in English.
- Decisions and open questions: `docs/07-decisions-and-questions.md` — update it when the author answers or a decision changes.
- Scene format: source of truth is `crates/cfd-core/src/scene.rs`. `schema/`, `scenes/` and `apps/web/src/generated/` are generated (`cfd-cli schema`, `cfd-cli example --all scenes`, `npm run gen:types`); tests fail if they are out of date.
- Author: strong CFD background (don't explain basic physics), little Rust/GPU experience, comes from C# → explain Rust/wgpu/CUDA idioms when relevant, with C# parallels. The prototypes in `sandbox/` are for the author to learn with: don't write them unless asked.
- Local environment (Windows): Rust MSVC toolchain (native build/test OK), VS Build Tools 2022, CUDA Toolkit 13.4 (driver 610.62 = CUDA 13.3; prefer `sm_89` cubin over PTX), RTX 4060 8 GB. Details in `docs/development.md`. Repository: github.com/Joaodss/cfd-flux, branch `master`.
- Working rules:
  - Three backends: CPU (numerical reference), wgpu/WGSL and CUDA (`cfd-cuda`, `cuda` feature). Every GPU kernel needs a parity test; keep the same memory layout in all three.
  - Every new solver must pass the validation cases in `docs/04-numerical-methods.md` §7 (method in `docs/08-validation-guide.md`).
  - Physical units in the scene; conversion to lattice units only in the solver.
- When finishing plan tasks, tick their checkboxes in `docs/05-implementation-plan.md`.
