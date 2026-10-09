# 07 — Decisions (ADRs) and open questions

## Decision log

Short format: context → decision → consequences. Status: **Proposed** (to be confirmed by the project owner) or **Accepted**.

### ADR-001 — Rust for the server and solver · *Proposed*
- **Context:** performance close to C/C++, memory safety, a good web (axum) and GPU (wgpu) ecosystem.
- **Decision:** Rust for the whole backend.
- **Consequences:** fewer ready-made CFD libraries than in C++/Python, offset by writing the kernels from scratch. Python only for AI training and analysis scripts.

### ADR-002 — Two GPU backends from the start: wgpu/WGSL and CUDA · *Accepted (2026-10-09)*
- **Context:** we want to support any GPU and possibly run in the browser, but the author's development GPU is NVIDIA and comparing engines is a goal in itself.
- **Decision:** implement **both** GPU backends in Phase 2: `wgpu` + WGSL (portable) and CUDA via `cudarc` (CUDA C kernels compiled at runtime with NVRTC — preferably to `sm_89` cubin, see [development.md](development.md) — or precompiled). CUDA sits behind a Cargo `cuda` feature so the project builds without the CUDA Toolkit.
- **Consequences:** three implementations of the same kernel (CPU, WGSL, CUDA) → more maintenance, offset by cross-backend parity tests and a comparison tool (`cfd-cli compare`). It allows measuring the real cost of portability (wgpu vs CUDA MLUPS) and using CUDA features (shared memory, warp shuffles) where worthwhile.

### ADR-003 — LBM as the first method · *Proposed*
- **Context:** pixel grid, arbitrary hand-drawn boundaries, GPU.
- **Decision:** LBM D2Q9 (TRT) + D2Q5 thermal in the MVP.
- **Consequences:** limited to low-Mach flows in the MVP; high-speed compressible gases wait for Phase 7.

### ADR-004 — Complete 2D first; 3D as the project's second stage · *Accepted (2026-10-09)*
- **Context:** 3D is a real medium-term goal (Q8), but it changes the UI substantially (voxel/slice editing, volumetric visualisation).
- **Decision:** all of 2D (including validation) is finished and well tested before 3D starts. Still, from now on: `grid` can gain an optional `depth`, layers are N-dimensional arrays, and the solver code is dimension-generic where it costs nothing (e.g. parameterised `DxQy` velocity tables, indexing through an `idx()` function), so 3D does not require a rewrite.
- **Consequences:** 3D moves from "maybe" to **Stage 2** of the roadmap (see the plan).

### ADR-005 — Frontend in TypeScript + Vite + React · *Accepted (2026-10-09)*
- **Context:** the author knows React; the canvas editor and WebGL visualisation are imperative, framework-independent code, so the framework barely affects performance.
- **Decision:** React + TypeScript + Vite, with **Zustand** (state, easy to use outside React components — important for the canvas), **Radix UI / shadcn/ui + Tailwind** (panels and forms), **uPlot** (fast time-series plots), **Vitest** + **Playwright** (tests). For 3D (Stage 2): **Three.js via react-three-fiber** — another reason to stay with React. Linting with oxlint (Vite's current default) + Prettier.
- **Rule:** pixels never go through React state; the canvas reads/writes `TypedArray`s directly and React only renders panels and tools.
- **Alternatives considered:** Svelte/SolidJS (less overhead, but no real gain here and an extra learning curve).

### ADR-006 — Server and worker in the same process in the MVP · *Proposed*
- **Decision:** in-memory queue; separate processes only when there are several GPUs/users.
- **Consequences:** trivial deployment; the queue trait allows swapping in Redis/NATS without touching the rest.

### ADR-007 — f16 + zstd results for streaming · *Accepted (implemented in `cfd-io`)*
- **Decision:** streaming frames in compressed f16; downloads in f32.

### ADR-008 — Free-surface liquids in the MVP · *Accepted (2026-10-09)*
- **Context:** the author wants gases and liquids from the first stage (Q5).
- **Decision:** the MVP includes **free-surface LBM** (water/air as liquid + "empty"), which reuses the D2Q9 kernel and the three backends. Phase-field (two real fluids), advanced surface tension and FLIP are post-MVP.
- **Consequences:** the MVP grows (~2–3 extra weeks of work); the free surface is harder to parallelise on the GPU (interface ↔ fluid ↔ gas cell conversion), so it comes only after single-phase LBM is validated on all three engines.

### ADR-009 — Open source and example-driven · *Accepted (2026-10-09)*
- **Context:** portfolio, public, educational use (Q1).
- **Decision:** public repository from Phase 0; a gallery of quick examples (precomputed or low resolution, < 10 s) as a first-class feature; user documentation and a README with GIFs.
- **Consequences:** CI must work without a GPU/CUDA; watch resource limits if a public demo is ever hosted.

### ADR-010 — Local-first, easy self-hosting, no hosted server · *Accepted (2026-10-09)*
- **Context:** no budget for a server; runs on the development machine; others may want to host it (Q3).
- **Decision:** `cfd-server` is **a single binary** that also serves the static frontend (`cfd-server --open` opens the browser). No mandatory external dependencies (SQLite + disk). Optional `Dockerfile`/`docker-compose` for anyone who wants to host it. Authentication/quotas are off by default and only enabled through configuration.
- **Suggestion (proposed, undecided):** a free public demo on **GitHub Pages** with the precomputed example gallery and, later, the solver compiled to WebGPU running in the browser at low resolution — zero server cost.
- **Consequences:** the plan's "Production" section became "Self-hosting" and lost priority.

### ADR-011 — Architecture ready for interactive mode from the start · *Accepted (2026-10-09)*
- **Context:** interactive mode (changing geometry/parameters while the simulation runs) is wanted but is not MVP (Q6). Since everything runs locally (ADR-010), there is no network latency — interactive mode is feasible and very appealing for a portfolio.
- **Decision:** the MVP has **no** interactive UI, but from Phase 3 the worker loop is a *command loop* (`Step`, `Pause`, `Resume`, `Cancel`, `PatchCells`, `SetParam`) and the `Solver` trait has `update_boundaries`. MVP pause/resume already uses that mechanism, and interactive mode (Phase 5b) is just UI + implementing `PatchCells` in the kernels.
- **Consequences:** near-zero cost now; avoids rewriting the worker later.

### ADR-012 — MPM as the candidate for phase changes involving solids/granular media · *Proposed*
- **Context:** we want all phase changes, including granular → liquid (e.g. crushed ice melting) (Q9).
- **Proposed decision:** solids/granular media via **MPM** (Material Point Method) coupled to the grid, with temperature and latent heat per particle; melting transfers mass from the particle to the liquid (free-surface LBM or the MPM grid). Melting/solidification of continuous volumes with the enthalpy method; evaporation/condensation with thermal multiphase LBM.
- **Starting reference:** Stomakhin et al., "Augmented MPM for phase-change and varied materials", SIGGRAPH 2014.
- **To be decided in Phase 9b** after a prototype (alternative: DEM + LBM).

### ADR-013 — English as the project language · *Accepted (2026-10-09)*
- **Context:** the project is part of the author's English CV/portfolio and is open source.
- **Decision:** everything in the repository — code, comments, documentation, UI text, commit messages, session logs — is written in English. The original Portuguese planning documents were translated on 2026-10-09.
- **Consequences:** a single language for readers and contributors; conversations with the assistant may still happen in Portuguese.

## Open questions

Answers to these questions can change priorities. Record the answer and the date here.

| # | Question | Answer / default assumed until answered |
|---|----------|-----------------------------------------|
| Q1 | Who is the main audience: personal/educational use, or a public multi-user service? | **Answered (2026-10-09):** portfolio and exploration project; **open source and public**; useful for education and quick simulation examples. Multi-user production remains far off |
| Q2 | Which GPU(s) are available for development and for the server (vendor, VRAM)? | **Answered (2026-10-09):** **NVIDIA GeForce RTX 4060, 8 GB** (driver 610.62 = CUDA 13.3); CUDA Toolkit 13.4 installed and verified |
| Q3 | Where will the server run: local machine, own server, cloud? | **Answered (2026-10-09):** only on the development machine; no budget for a server. It should be easy for others to self-host (see ADR-010) |
| Q4 | Is engineering-grade accuracy (quantitative validation) needed, or is the focus visual/interactive? | **Answered (2026-10-09):** yes, quantitative accuracy validation; method explained in [08 — Validation guide](08-validation-guide.md) |
| Q5 | Priority between gases (aerodynamics, ventilation, cooling) and liquids (free surface)? | **Answered (2026-10-09):** **gases and liquids in the first stage** → free-surface liquids are in the MVP (see ADR-008) |
| Q6 | Is interactive mode (changing geometry while the simulation runs) important early on? | **Answered (2026-10-09):** wanted, not in the MVP → architecture ready from Phase 3 (ADR-011), UI in Phase 5b |
| Q7 | Frontend framework preference (React, Svelte, other)? | **Answered (2026-10-09):** React (ADR-005, with recommended tooling) |
| Q8 | Is 3D a real medium-term goal or just a "maybe"? | **Answered (2026-10-09):** a real goal; **Stage 2**, after 2D is complete and tested (ADR-004) |
| Q9 | "Other states" — what do you have in mind: plasma, granular, deformable solids, phase change? | **Answered (2026-10-09):** post-MVP: granular media, plasma and common phase changes, including granular → liquid (crushed ice melting) (ADR-012). Rare cases (solid ↔ gas: sublimation/deposition, and others) in a later phase (Phase 9c) |
| Q10 | Project license — open source confirmed; which license? | **Answered (2026-10-09):** **MIT OR Apache-2.0** |
| Q11 | Prior experience with Rust, GPUs and CFD (to calibrate documentation detail and pace)? | **Answered (2026-10-09):** strong CFD experience; little Rust and GPU experience (learns fast); C# background. → Phase 0 includes learning prototypes (see the plan) |
| Q12 | Project name: the repository is `cfd-flux`, the docs and packages say `live-fluids`. Which one is final? | *Open* |

### License options (Q10)

| License | Type | What it allows / requires | Good fit if… |
|---------|------|---------------------------|--------------|
| **MIT** | Permissive | Any use, including commercial and closed source; only requires keeping the copyright notice | You want maximum adoption, no strings attached |
| **Apache-2.0** | Permissive | Like MIT + an explicit patent grant and rules for contributions | Same, with patent protection |
| **MIT OR Apache-2.0** | Permissive (dual) | Users pick one; the Rust ecosystem convention | A Rust project that wants frictionless integration |
| **MPL-2.0** | Weak copyleft (per file) | Modified files must stay open; can be combined with closed code | Middle ground |
| **GPL-3.0** | Strong copyleft | Anyone **distributing** modified versions must open the whole code under the GPL | You want distributed derivatives to stay open |
| **AGPL-3.0** | Strong copyleft + network | Like the GPL, but also anyone **offering a modified version as a web service** must open the code | You want to stop someone closing the project and selling it as SaaS |
