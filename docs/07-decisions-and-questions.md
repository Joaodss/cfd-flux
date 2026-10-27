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

### ADR-006 — Server and worker in the same process in the MVP · *Accepted (2026-10-13, implemented in Phase 3)*
- **Decision:** in-memory queue; separate processes only when there are several GPUs/users.
- **Consequences:** trivial deployment. As implemented (ADR-025), each job runs on its own threads inside `cfd-server` and the queue is one semaphore per physical device; a distributed queue (Redis/NATS) would replace the semaphores and the `JobManager`, not the job runner (`cfd-job`), which already talks to the server only through channels.

### ADR-007 — f16 + compressed results for streaming · *Accepted (implemented in `cfd-io`; codec revised 2026-10-16)*
- **Decision:** streaming frames in compressed f16; downloads in f32.
- **Revision (2026-10-13, Q13; implemented 2026-10-16):** the compression is a byte shuffle + `lz4_flex` (still pure Rust) instead of `ruzstd`: LFFR v2 frames carry a `codec` field (`shuffle+lz4` written; `zstd` for v1 frames, still read; `none`), one lz4 block per field, compressed in parallel ([03 §6.1](03-data-model.md#61-frame)). A 1024² f32 frame encodes in 13.6 ms to 64% (`ruzstd`: 328 ms, 91%); the cylinder streams at 160–190 frames/s on CUDA instead of 37 ([benchmarks](benchmarks.md#frame-codec-lffr-v2--amd-ryzen-7-3700x-2026-10-16)). The browser decodes LZ4 blocks and unshuffles in TypeScript (Phase 5a).

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
- **Consequences:** near-zero cost now; avoids rewriting the worker later. Implemented in Phase 3 (`cfd_job::run`, ADR-025): pause, resume, cancel and snapshots go through the command loop; `PatchCells`/`SetParam` exist and are answered "not supported" until Phase 5b, when `update_boundaries` joins the `Solver` trait.

### ADR-012 — MPM as the candidate for phase changes involving solids/granular media · *Proposed*
- **Context:** we want all phase changes, including granular → liquid (e.g. crushed ice melting) (Q9).
- **Proposed decision:** solids/granular media via **MPM** (Material Point Method) coupled to the grid, with temperature and latent heat per particle; melting transfers mass from the particle to the liquid (free-surface LBM or the MPM grid). Melting/solidification of continuous volumes with the enthalpy method; evaporation/condensation with thermal multiphase LBM.
- **Starting reference:** Stomakhin et al., "Augmented MPM for phase-change and varied materials", SIGGRAPH 2014.
- **To be decided in Phase 9b** after a prototype (alternative: DEM + LBM).

### ADR-013 — English as the project language · *Accepted (2026-10-09)*
- **Context:** the project is part of the author's English CV/portfolio and is open source.
- **Decision:** everything in the repository — code, comments, documentation, UI text, commit messages, session logs — is written in English. The original Portuguese planning documents were translated on 2026-10-09.
- **Consequences:** a single language for readers and contributors; conversations with the assistant may still happen in Portuguese.

### ADR-014 — Link-wise boundary conditions driven by precomputed flags · *Accepted (2026-10-10)*
- **Context:** boundaries are hand-drawn pixels: there is no reliable normal, and corners are everywhere. Node-based schemes such as Zou-He need a normal and special corner treatment, and are awkward to keep identical in three backends.
- **Decision:** every boundary is a rule applied to a *link* between a fluid cell and a non-fluid neighbour, half-way between them; only fluid cells are updated. Walls, moving walls and **velocity inlets** use bounce-back with the wall velocity (`f_i = f*_ī + 6 w_i ρ₀ c_i·u_w`, with `ρ₀ = 1`); pressure outlets use anti-bounce-back; zero-gradient outlets copy the interior neighbour along a precomputed inward axis. `Scene → Domain` precomputes per-cell `u16` flags (kind, thermal kind, inward axis, 8-bit link mask) and a `#[repr(C)]` parameter table, uploaded unchanged to every backend (see [03 §5](03-data-model.md#5-internal-domain-domain-solver-side)).
- **Consequences:** with TRT (Λ = 3/16) walls sit exactly half-way for any viscosity. Velocity bounce-back with `ρ₀` imposes the mass flux `ρ₀u`; using the local density instead was tried and makes inlets unstable with open outlets (the inflow grows with the density). A zero-gradient outlet cannot carry a pressure gradient, so velocity inlets without a pressure outlet produce the warning `domain.noPressureReference`.

### ADR-015 — f32 storage with shifted populations · *Accepted (2026-10-10)*
- **Context:** WebGPU has no f64 and consumer GPUs run f64 at 1/64 speed; the CPU backend is the parity reference for the GPU ones.
- **Decision:** all backends store `f32`. The flow populations are stored shifted (`f̃ᵢ = fᵢ − wᵢ`, FluidX3D's trick) and rest populations are computed as the remainder of the sum (`f̃₀ = Δρ − Σ`, `g₀ = θ − Σ`), so mass and heat are conserved by construction. Temperatures are centred on the initial temperature (`θ = (T − T_ref)/ΔT`). A scene asking for `precision: "f64"` runs in f32 with the warning `support.precision`.
- **Consequences:** second-order convergence is visible down to errors of ~1e-3 at 64² (Taylor-Green, Poiseuille); if a validation case ever needs more, the CPU kernel can be made generic over the float type.

### ADR-016 — Gravity: hydrostatic part in the output, buoyancy in the solver · *Accepted (2026-10-10)*
- **Context:** in a single-phase flow without a free surface, uniform gravity is balanced by a hydrostatic pressure gradient and does not change the velocity; applying it in a weakly compressible LBM only adds a density stratification (compressibility error).
- **Decision:** gravity stays a scene property (`physics.gravity`, with `units::STANDARD_GRAVITY` as the Earth default). The solver applies only the Boussinesq force `−ρ₀β(T − T_ref)g`; the hydrostatic pressure `ρ₀ g·(x − x_ref)` is added to the output pressure (reference: centroid of the pressure outlets, else of the fluid). The free surface (Phase 6a) will apply full gravity.
- **Consequences:** sampled pressure is the physical gauge pressure; the solver field is the dynamic part.

### ADR-017 — Each backend implements `Solver` directly · *Accepted (2026-10-10)*
- **Context:** the architecture listed both a `Solver` and a `Backend` trait.
- **Decision:** a single `Solver` trait in `cfd-core`, implemented by each backend type (`CpuLbm` now; `WgpuLbm`, `CudaLbm` in Phase 2). Construction is backend-specific; code that chooses at runtime holds a `Box<dyn Solver>`. Backends expose raw lattice fields; conversion to physical fields, probes and units is shared code. Interactive updates (`update_boundaries`/`PatchCells`, ADR-011) will be added as trait methods in Phase 5b.
- **Consequences:** less abstraction now; a device-level `Backend` trait is only worth it if several methods share the GPU backends (Phase 7).

### ADR-018 — Validation cases are backend-agnostic Rust code (`cfd-verify`) · *Accepted (2026-10-10)*
- **Context:** the validation guide planned `validation/cases/*.json` scenes, but the cases need a resolution parameter (convergence studies), body forces and non-uniform initial fields that the scene format does not express, and Phase 2 must run the same cases on wgpu and CUDA.
- **Decision:** a library crate `cfd-verify` holds the cases as Rust code. Each case builds regular `Scene`s in physical units (so `Scene → Domain` and the unit conversion are validated too) and receives a factory `Fn(Domain) -> Box<dyn Solver>`; it never names a backend. Reference data live in `validation/references/*.csv` with their sources and are embedded at compile time. `cfd-cli verify --quick` runs in CI; `--full` writes the committed report `validation/report/`. Tolerances are set per case before looking at results; a staircase cylinder is checked at 2% of the Schäfer-Turek interval centre (5% in `--quick`), with the published interval shown alongside. Steady states are judged on the quantity being measured (C_D, Nu) when field changes are dominated by f32 noise or slowly decaying acoustic waves. Convergence orders are fitted only over resolutions above the f32 round-off floor (Taylor-Green N ≤ 128, channels N ≤ 32; decided with the author after the first full run); finer levels stay in the report, marked as the floor.
- **Consequences:** the suite found two bugs the unit tests (all at `dx = 1`) could not: forces were scaled by `ρ₀dx²/dt²` instead of `ρ₀dx³/dt²`, and momentum exchange included the absolute reference pressure (forces are now gauge). Interpolated bounce-back remains the way to reach the Schäfer-Turek intervals at moderate resolution (D = 80 is needed now for C_D,max).

### ADR-019 — GPU memory layout: the CPU layout, uploaded unchanged · *Accepted (2026-10-10)*
- **Context:** three backends must give the same numbers (parity < 1e-5) and stay maintainable; WGSL has no 16-bit storage type and WebGPU guarantees only 8 storage buffers per shader stage.
- **Decision:** the GPU keeps the CPU's SoA layout (`f[i·n + cell]`, shifted populations, rest population as the remainder, ping-pong by swapping two prebuilt bind groups). The `u16` flags are uploaded as the same bytes, two cells per `u32` word, and unpacked with a shift; `BcParams` (32 bytes) and `bc_slot` are uploaded as they are. ρ, ux, uy and θ share one buffer of four planes, followed by a status word, so the kernel binds exactly 8 storage buffers. Lattice constants and flag bits are generated into a WGSL prelude from the Rust constants (never typed twice); naga validates the shaders and checks the struct layouts in tests that need no GPU. The initial populations, relaxation rates and the bounce-back link list come from shared `cfd-lbm` code, so a parity difference can only come from a kernel.
- **Consequences:** 2 B/cell of flags and no per-backend conversion code; CUDA (Phase 2b) can read the same bytes as `uint16_t`. GPU compilers may assume floating-point results are never NaN/Inf (Vulkan does not require preserving them) and fold a divergence check on computed values away, so the kernel tests the populations it *loads*: divergence is flagged one step after it appears.

### ADR-020 — Asynchronous GPU stepping and readback · *Accepted (2026-10-10)*
- **Context:** a GPU step at 1024² takes ~0.4 ms; waiting for the GPU after every `step` call, or copying fields while it idles, wastes a large part of that.
- **Decision:** `WgpuLbm::step(n)` records up to 1000 steps per command buffer, submits and returns; it waits only when more than 2 batches are in flight. Each batch ends with a copy of the status word, checked when the batch is retired, so a divergence is reported at the latest on the next `step`/`finish` call. The `Solver` trait gains `finish()` (wait for queued steps; timed regions call it) and `request_fields()`, which returns a `PendingFields` handle (≈ `Task<LatticeFields>`) backed by a pool of staging buffers; both have synchronous defaults, so the CPU backend is unchanged. `diagnostics()` and `forces()` reduce or gather on the GPU and read back only per-workgroup partials or per-link values, summed in f64 on the CPU in the CPU backend's order.
- **Consequences:** `cfd-cli run` requests a frame before submitting the next steps and writes it while they run. `--backend auto` picks a hardware GPU when there is one and never a software adapter, which is slower than the CPU backend.
  - *Addendum (2026-10-18):* a blocking readback cannot rely on `device.poll(Wait)` alone. A poll collects the map callbacks of every finished submission and runs them after it returns, on the thread that polled; when another thread polls the same device (solvers sharing one `GpuContext` on several threads, as the parallel parity tests do), our `Wait` can return before our callback has run. This was the intermittent `status readback failed` of the CI parity tests. `Mapping::wait` now keeps waiting (yielding) until the callback has run, bounded by 10 s, and returns the poll error or the timeout, which `retire` adds to the message (`tests/concurrent_poll.rs` reproduces the race).

### ADR-021 — CUDA backend: runtime-loaded `cudarc`, cubins, no FMA, one `unsafe` module · *Accepted (2026-10-12)*
- **Context:** ADR-002 asks for a CUDA backend that the workspace can build without the CUDA Toolkit; the driver on the development machine (13.3) is older than the toolkit (13.4), so NVRTC's PTX may be rejected; the workspace denies `unsafe_code`, but kernel launches through `cudarc` are `unsafe`; parity with the CPU reference must stay < 1e-5.
- **Decision:**
  - `cudarc` 0.19 with **dynamic loading** and the CUDA 13.0 bindings: the libraries are opened at runtime, so every build (and CI) compiles the backend without the toolkit; running it needs a driver ≥ 13.0 and the CUDA 13 NVRTC. `cfd-cli` enables the `cuda` feature by default. Missing driver, device or NVRTC give a clear error (NVRTC and its builtins library are also found through `CUDA_PATH` when not on `PATH`); `--backend auto` then falls back to wgpu, then to the CPU.
  - Kernels (`crates/cfd-cuda/kernels/*.cu`, line-by-line translations of the WGSL ones) are compiled once per process by NVRTC **to a cubin for the device's architecture** (`sm_89`), or an older cubin of the same major version; PTX only when NVRTC does not know the device and the driver is at least as new as NVRTC.
  - Constants and the parameter block come from `cfd_lbm::abi`, which generates both the WGSL prelude and the CUDA header; the CUDA structs are followed by generated `static_assert`s with the Rust sizes and offsets, so NVRTC rejects any layout drift (a CI job compiles the kernels with NVRTC only, no GPU).
  - **`--fmad=false`**: no multiply-add contraction, IEEE division and square root. CUDA is then **bit-identical** to the CPU backend on every parity case; with `--fmad=true` the differences are 1e-7–8e-7 (like wgpu) and throughput is the same, the kernel being bandwidth-bound (`--fmad` in `cfd-cli` for comparisons).
  - All `unsafe` code is in `cfd-cuda/src/raw.rs`, with a `// SAFETY:` comment per block (enforced by `clippy::undocumented_unsafe_blocks`). Kernels are launched through one typed wrapper each; their safety argument rests on invariants that `Lattice::new` checks once (slots < number of BC records, link cells < n) and on every launch of a solver being ordered on its own stream. `cudarc`'s per-buffer event tracking is disabled (it costs several driver calls per launch).
  - Stepping and readback follow ADR-020: batches of ≤ 1000 steps, ≤ 2 in flight, a status word copied to pinned memory per batch. Steps are replayed from a **CUDA graph** of 50 steps (captured once, and again when a parameter changes): a kernel launch costs ≈ 8 µs of CPU time on Windows (WDDM), more than a step on small grids, while a graph replay costs ≈ 0.6 µs per step (`--graph-steps 0` launches every step on its own). `request_fields` snapshots the fields on the device and copies the snapshot to pinned host memory on a second stream, so the PCIe transfer overlaps with the next steps.
- **Consequences:** CUDA serves as a second, exact implementation of the CPU kernel (any CPU ↔ CUDA difference is a bug, not round-off), while wgpu shows the effect of the driver's FMA contraction. The kernels were checked with `compute-sanitizer` (memcheck clean, no shared-memory races). CUDA 12 drivers are not supported by these bindings.

### ADR-022 — Memory-lean storage after the MVP, before 3D · *Proposed (2026-10-13)*
- **Context:** the GPU kernels are bandwidth-bound (CUDA at 94% of the RTX 4060's DRAM bandwidth, exactly the ideal 84 B per cell), so speed and the largest grid both depend only on the bytes stored and moved per cell. Cards with less memory or bandwidth hit this first, and 3D (D3Q19) needs ≈ 175 B/cell with two f32 buffers (≈ 360³ on 8 GB). Each storage change rewrites the core of three kernels (CPU, WGSL, CUDA).
- **Proposed decision:** (1) skip the ρ/u stores on steps whose fields are not read (Phase 3, parity unchanged); (2) after the MVP, once 6a and 5b have settled the kernel and before 3D, add an **f16/FP16C storage option** (compute in f32, f32 stays the default) implemented on all three backends including the CPU, and **Esoteric Pull** (one population buffer), as Phase 8a. Parity is compared within one storage mode (still exact); f32 vs f16 is judged by the validation suite with its own tolerances; Esoteric Pull must be bit-identical to the ping-pong kernel.
- **Consequences:** ≈ 1.7× speed and ≈ 2.3× larger grids in 2D, ≈ 510³ in 3D on 8 GB; a scene-format change (`run.storage`); twice the configurations to test; the main risks (boundary rules in both step parities, f16 accuracy at low Mach and small temperature differences, WGSL 16-bit support) and their mitigations are listed in the plan, Phase 8a. To be confirmed when Phase 8a starts.

### ADR-023 — Engine comparison: the verdict is the 04 §7 field metric · *Accepted (2026-10-13)*
- **Context:** `cfd-cli compare` runs one scene on several backends from the same initial state and reports everything that differs. The parity tests also hold diagnostics and forces to fixed limits (kinetic energy and max |u| 1e-5, mass 1e-7, forces 1e-4 of the largest), chosen for their short runs (≤ 6,000 steps). On longer runs the first validation comparison showed those limits are stricter than the field metric they derive from: wgpu's Taylor-Green kinetic energy differed by 1.64e-5 (E is quadratic in u, so δE/E ≈ 2δu/u, with u within 7.9e-6), and the Rayleigh-Bénard plate forces by 3.2e-4 of the largest force (pressure-loaded walls measure ρ − 1 relatively; ρ agreed to 1.2e-7 absolute). With inlets and outlets Σρ is not conserved, and its relative difference is the mean density difference.
- **Decision:** the comparison logic lives in `cfd_verify::compare` (backend-agnostic, reusing `cfd_verify::parity`); `cfd-cli compare` opens each backend side by side and runs them one after another. A backend passes when the 04 §7 metric of every lattice field (ρ absolute, ux and uy relative to the largest reference component, θ relative to its maximum) is within the tolerance (default 1e-5; 0 requires bit-identical fields) and, in a **closed** domain, the mass agrees to 1e-7. Kinetic energy, max |u|, thermal energy, the mass of an open domain and the forces are reported with their differences for information; the parity tests keep their limits on the reduction and force kernels. Chaotic or periodic flows over many periods use `--statistics`: the verdict moves to the oscillation frequency, the mean/min/max of each force component and the mean kinetic energy over a window (relative limit 1e-3), with the instantaneous fields shown for information. Exit code 1 outside the tolerance, 2 when the comparison cannot run.
- **Consequences:** `validation/compare/` (13 validation geometries) passes 13/13: CUDA bit-identical everywhere, wgpu ≤ 7.9e-6 and, on the Re 100 cylinder, St and force statistics within 5.2e-6 while its fields drift by 1.6e-4. A backend whose reduction or force kernel is wrong but whose fields are right passes `compare` and fails the parity tests, which run in CI.

### ADR-024 — Engine selection and the job runner are crates shared by the CLI and the server · *Accepted (2026-10-13)*
- **Context:** before Phase 3 the CLI owned backend selection and the run loop that writes frames, CSV series and `meta.json`; the server needs both, and two copies would give two result layouts that drift (design proposal Q1).
- **Decision:** `cfd-engines` holds engine kinds, opening engines (`auto` = CUDA > hardware wgpu > CPU), `SolverChoice::make` (the only code naming the concrete solver types) and the `Inventory` a long-running process opens once (devices, memory, physical-device keys; an optional `clap` feature derives `ValueEnum`). `cfd-job` holds `prepare` (`Scene → Domain` with the run's overrides), `run` (the command loop of ADR-025), `meta.json`, the CSV series and the result directory (`ResultDir`, `LocalStore`), reporting through a `JobObserver` trait. `cfd-cli run` is a console observer over `cfd-job`; the server's observer feeds its channels.
- **Consequences:** the extraction changed nothing: the old and new `cfd-cli` wrote byte-identical frames, CSVs and PNGs on CPU and CUDA (`meta.json` differs only in timings), and a server job writes the same CSVs and frame fields as a direct run (tested). The result store is a struct for local disk; an object store (S3/MinIO) would make it a trait.

### ADR-025 — Jobs: a command loop with time-capped batches, one slot per physical device, and an output thread that never drops stored frames · *Accepted (2026-10-13)*
- **Context:** the server must pause and cancel within a second (Phase 3 acceptance), never block its async runtime, keep GPU contexts on their thread, not oversubscribe a GPU that two APIs (CUDA, wgpu) can open, and keep every stored frame even when the disk is slow (design proposal Q2, Q4, Q5).
- **Decision:**
  - The job runs on a dedicated OS thread (`cfd_job::run`), which makes the solver; only messages cross threads: commands through a `std::sync::mpsc` channel drained between batches (blocking `recv` while paused), events through the observer.
  - Each batch runs up to the next sample/output event and at most ≈ 50 ms of measured work: its size is the smoothed measured step rate × 50 ms (the first batch is one step), so a command waits at most one batch plus the GPU work in flight.
  - Frames go to an output thread through a bounded channel (2): conversion to physical fields, encoding and writing overlap with the solver, and a slow disk or encoder makes the solver wait. A cancel drops the frames still queued, and the loop keeps reading commands while it waits for room. A per-job stored-bytes limit (`maxStoredBytes`, 10 GiB) stops storing, not the job.
  - The server keeps one `tokio::sync::Semaphore` per physical device (CUDA and wgpu on the same GPU share it, matched by PCI location; `--cpu-jobs`, default 1, for the CPU). A job's task waits for a permit (FIFO), starts the job's thread and moves the permit into it. Jobs active when the server stops are cancelled and recorded as `failed/interrupted`; on start-up, rows still active become `interrupted` (no checkpoints yet).
- **Consequences:** cancel latency 30–46 ms on the CPU, CUDA and wgpu for the cylinder (release build; 0.25–0.34 s in a debug build, where encoding a frame takes longer); results do not depend on the batch size (one-step and 50 ms batches give byte-identical files); the run loop costs ≈ 0.2% against `bench` at 1024² on CUDA (2,878 vs 2,885 MLUPS). With frames every 60 steps on the 442 × 84 cylinder, the output thread (≈ 14 ms per stored f32 frame, 12.6 ms of it zstd compression that saves only 3%, plus 7 ms per f16 stream frame) limits the job to ≈ 37 frames/s: by design the solver waits rather than drop a stored frame (open question Q13). *Since LFFR v2 (2026-10-16) the stored and streamed frames of this case encode in ≈ 0.8 ms and the job streams 160–190 frames/s on CUDA ([benchmarks](benchmarks.md)).*

### ADR-026 — Streaming: the latest frame per format in a `watch` channel, events in a `broadcast` channel · *Accepted (2026-10-13)*
- **Context:** viewers differ in speed and in what they want to see (fields, size); a slow viewer must never slow the job, and stored frames must never be dropped (they are not: the output thread writes them before streaming).
- **Decision:** per job and per stream format (fields, `maxSize`) a `tokio::sync::watch` channel holds only the latest encoded frame (f16, box-downsampled by an integer factor), encoded on the output thread for the formats that have viewers. Each viewer's task sends the latest frame and awaits the socket, so frames that arrive meanwhile replace each other; the final frame is always flushed before `done`. Events (`status`, `progress` at most 10 Hz, `log`, `done`) go through a `tokio::sync::broadcast` channel (64 per viewer); a viewer that lags past it is resynchronised from the job's view. A viewer joining a paused job, or asking for a new format, triggers a `Snapshot` command. Control stays on REST; client messages are reserved for Phase 5b.
- **Consequences:** a viewer that stopped reading received 24 of 151 frames (the rest skipped) while the job ran at full speed and stored all 151; viewers with different formats share nothing but the job. Encoding costs scale with the number of distinct formats being watched (usually one). The f16 conversion and downsampling run on the device since ADR-028 (2026-10-16): each format is one more sample of the frame.

### ADR-027 — Server persistence and API errors: SQLite through `sqlx` with runtime queries; one error enum with stable codes · *Accepted (2026-10-13)*
- **Context:** the server must run with no external services (ADR-010) and stay portable to Postgres if it is ever hosted; the editor needs machine-readable errors it can translate (design proposal Q3, Q6, Q7).
- **Decision:** `sqlx` with SQLite (bundled, `runtime-tokio`), queries checked at runtime (no database needed to build), migrations embedded with `sqlx::migrate!`; tables `jobs` (status, reason, backend, device, options, steps, frames, MLUPS, times, result directory) and `scenes`. Frames and series stay on disk. Errors are one `ApiError` enum turned into a status and `{ "error": { code, message, issues? } }`; validation issues keep the stable codes of `cfd_core::validate`. Bodies are limited to 64 MB, and a `413` names the `zstd+base64` layer encoding. The server binds `127.0.0.1:7878` (`--listen` to change it).
- **Consequences:** time estimates use the mean MLUPS of earlier completed jobs per engine (none until one has run). Issues carry their values as data (`params`) and the cell they are about (`at`) since Phase 4, and `/validate` returns the estimate (with Re, Ma, τ, …) also when a limit or the engine choice fails (03 §7).

### ADR-028 — Output sampling on the device: several specs per request, CUDA in f64, the CPU conversion as the reference · *Accepted (2026-10-16)*
- **Context:** a frame copied ρ, ux, uy, θ (16 B/cell) and converted them on the job's output thread (`FieldSet::from_lattice`: pressure with the hydrostatic part, vorticity, temperature), then box-downsampled and converted to f16 for every stream format: 32 ms per 1024² frame, 121 ms at 2048², more than a frame interval at interactive rates (design proposal §6). The CPU conversion computes in f64; WGSL has no portable f64.
- **Decision:**
  - `Solver::request_samples(&[SampleSpec]) -> Box<dyn PendingSamples>` (`cfd_core::sample`): a spec is the fields, an integer box-filter factor and the precision (f32 or round-to-nearest-even f16). One request carries every spec of a frame — the stored frame and each stream format — so the CPU default copies the lattice fields once. Its handle's `wait` returns a `SampleSet`: samples computed on the device, or lattice fields that `into_samples` converts where they are collected (the output thread), never on the solver thread. `Sample::from_lattice` (`from_lattice` → the box filter → f16) is the reference.
  - CUDA (`sample.cu`): one thread per output word, the conversions in **f64** in the CPU's order (the 4060's FP64 rate is irrelevant for a once-per-frame kernel), the box sums in f32 in the CPU's row order, `cvt.rn.f16.f32`; with `--fmad=false` every sample is **bit-identical** to the CPU's. WGSL (`sample.wgsl`): the same structure in f32 with coefficients precomputed in f64 (`SampleParams`), two f16 per word through `pack2x16float`.
  - The output layout and parameters are `cfd_lbm::abi::SampleParams`/`SampleLayout` (generated constants, layout checks against both languages). Each field is packed in 32-bit words; samples are copied with the backends' asynchronous readback (pinned memory and the copy stream on CUDA, a staging pool on wgpu).
  - Jobs ask the observer for its stream specs at every frame (`JobObserver::stream_specs`); `FrameEvent` carries the samples and `cfd_job::encode_sample` writes their values into LFFR frames as they are (f16 bits included).
- **Consequences:** a 1024² frame (stored f32 + one f16 stream format) costs the output thread 13 ms instead of 43, 46 instead of 159 ms at 2048² (CUDA; the conversion disappears from the CPU, the copy grows by 1.5× the bytes and the kernel). Parity cases (`cfd_verify::parity`, full resolution and two box factors, f32 and f16, thermal, periodic, gravity): CUDA bit-identical; wgpu within 1e-5 of the field maximum and one f16 ulp, and its f16 is exactly RNE of its own f32 — a probe test of ties, subnormals and overflow shows `pack2x16float` rounds to nearest-even on the RTX 4060 with Vulkan but **toward zero with DX12** (124 of 334 probes one ulp lower; the WGSL spec allows either neighbour), so wgpu stays held to one f16 ulp. The sampling shader must have a single return per function: DX12's FXC rejected a `switch` with a `return` in every case (found by the Windows CI job, which runs wgpu on DX12). Stored wgpu frames now come from the GPU's f32 arithmetic (within ~1e-7 of the CPU conversion); colour ranges come from the samples. A stream format added between a request and its frame waits for the next frame (subscribing triggers a snapshot).

### ADR-029 — `skip_macro`: macroscopic stores only on the last step of each `step` call · *Accepted (2026-10-16)*
- **Context:** the fused stream-collide kernel stored ρ, ux, uy (and θ) every step — 12 (16) of the 84 (130) bytes
  each update moves on the bandwidth-bound GPUs — although they are read only by frames, probes and diagnostics
  between `step` calls, and inside the kernel only by pressure outlets, which read their own cell's velocity of the
  previous step (design proposal §7).
- **Decision:** every backend's kernel takes a `write_macro` switch (CPU: an argument; CUDA: a template argument,
  four entry points; WGSL: a pipeline-overridable constant, two pipelines). `step(n)` runs n−1 steps without the
  stores and one with them, so every read between calls sees fresh fields and the `Solver` trait does not change.
  Skipping steps still store u on every cell with a boundary link (`mask != 0`, a superset of the pressure-outlet
  cells, known before the pull loop) and the divergence status word. CUDA captures one graph per variant (the
  final writing step is launched alone). `LbmConfig::skip_macro` (on by default; `--no-skip-macro` in the CLI)
  turns it off.
- **Consequences:** results are bit-identical to storing every step — a parity case compares fields, diagnostics
  and forces with `==` after calls of 1–1203 steps on the CPU, CUDA and wgpu with Vulkan, and fails on all three if
  the boundary-cell u store is removed. With DX12, FXC compiles the two pipeline variants with slightly different
  rounding (≤ 8e-7 of the run's peak speed), so wgpu is held to the 04 §7 tolerance there. `bench --backends cuda,wgpu`: +16.3% at ≥ 1024² isothermal (2,874 → 3,343 MLUPS
  on CUDA, as 84/72 B predicts), +14.5% thermal (130/114 B), +62–68% at 512² (the lattice fits in the 32 MB L2), +10–13% at 256².
  A first version stored u where a flag set inside the unrolled pull loop marked a pressure-outlet read: wgpu's
  256² kernel became 20% slower than storing every step; the mask-based rule is faster everywhere. A `step(1)` loop
  gains nothing (every step is a last step): the run loop's batches are what benefit.

### ADR-030 — Web editor: pixels outside React, WebGL2 cells, tile-swap history, server-side validation · *Accepted (2026-10-18)*
- **Context:** Phase 4 needs an editor for grids up to 4096² (16 M cells) with undo, live shape previews, files and
  validation, written by an author new to TypeScript/React, with the Rust types as the source of truth
  ([design proposal](sessions/phase4-design-proposal.md), every answer of its §14 accepted).
- **Decision:** the layers are typed arrays owned by plain TypeScript objects, never React state; the scene table,
  view and UI state live in a Zustand store (with `immer`). Cells are drawn by **WebGL2** from integer textures
  (`R8UI`, `R16UI`, dirty 64×64 tiles uploaded with `texSubImage2D`) and a palette texture; a Canvas2D overlay draws
  selections, probes and issue markers. Tools write through a revertible `Stroke` (shapes are redrawn live); the
  **history** keeps changed 64×64 tiles and swaps them on undo/redo (uniform tiles as one value; 256 MB / 200
  entries), with table changes and whole-grid replacements in the same entries. Files are the `Scene` JSON with
  **`deflate+base64`** layers (native `CompressionStream`; a new encoding read by `cfd-core` through `miniz_oxide`).
  **Validation stays on the server** (`/api/scenes/validate`, debounced, aborted when outdated), whose issues carry
  `params` and a cell so the editor words and marks them; the preset library lives in `cfd_core::library` and is
  generated into the web app.
- **Consequences:** a 4096² grid uploads and draws in ≈ 80 ms and a tile in ≈ 3 ms; a half-domain fill takes 0.17 s
  and its undo entry 128 kB. Nothing in the editor duplicates physics or validation (one implementation, in Rust);
  the editor needs the local server for checks and estimates — compiling `cfd-core` to WASM stays the option for a
  serverless demo (ADR-010). Contract tests keep the two sides in step: an editor-written scene file read by
  `cfd-core`, a sample of the browser's deflate decoded in Rust, every issue code worded by the editor, generated
  types and library checked as up to date. The acceptance scenario is a Playwright test in CI.

### ADR-031 — Phase 5 viewer: decoding in a worker, one viewer format, series from the server, statistics in TypeScript, the app served by `cfd-server` · *Accepted (2026-10-22)*
- **Context:** Phase 5 turns the job view into the viewer of the MVP (FR-20–FR-24): live and stored frames, a timeline,
  plots, force and Nusselt statistics, overlays, export and a gallery that opens with results
  ([design proposal](sessions/phase5-design-proposal.md), every answer of its §16 accepted on 2026-10-19; the
  cylinder example changed on 2026-10-21, below). The rule of ADR-005/ADR-030 stays: pixels never go through React
  state, and nothing is computed in two languages without a contract test.
- **Frames.** LFFR v2 is decoded by **our own TypeScript decoder** (an LZ4 block decoder with bounds checks, the byte
  unshuffle and a float16 table; no `lz4js`) in **one Web Worker** that also fetches stored frames and computes each
  field's min/max/histogram, so a 1024² frame never costs the page's thread an animation frame. Live and stored frames
  share **one viewer format**: every frame field, box-downsampled to `maxSize` (1024 by default), f16.
  `WS …/stream?maxSize=` and `GET …/frames/{n}?fields=&maxSize=` give the same bytes (a Rust test on the CPU engine
  compares them), so one cache serves both. The cache is **memory only** — an LRU of decoded frames (512 MiB by
  default, farthest from the playhead evicted first, prefetch in the play direction); the server, which already holds
  every stored frame, is the store, and there is no IndexedDB. Contract tests: frames written by `cfd-io`
  (`tests/fixtures/frames/`, rewritten by `UPDATE_FIXTURES=1`, failing when out of date) are decoded bit for bit in
  Node, and the f16 table is checked against `DataView.getFloat16` on all 65,536 values. *Amended 2026-10-27 (#24):*
  one worker did ≈ 3.5 frames/s at 1024²; frames now go to a **pool of 1–4 decode workers** (by the machine's
  threads, the next to the least busy; frames may come back out of order, the viewer looks at their index), the
  streamlines to a worker of their own, and playback prefetches one frame more than there are workers
  ([benchmarks](benchmarks.md), "After the fixes for #24 and #25"). *Amended 2026-10-27 (#17, #18):* the
  viewer format's `maxSize` (256/512/1024/2048/full) and the cache budget (128 MiB–4 GiB) are **settings**
  remembered per browser in `localStorage`; another `maxSize` reconnects the stream, empties the cache and reloads
  the playhead's frame (frames of the old format still in a worker or a fetch are dropped), a smaller budget evicts
  at once. **Sharpen:** a paused frame zoomed in past the viewer format (a block wider than one device pixel) is
  fetched once at full resolution — `frames/{n}` without `maxSize`, whose values are the f16 rounding of the
  stored f32 (a Rust test) — and drawn with a per-cell fluid fraction; not a second cache: one frame, dropped when
  playing, scrubbing or zooming out, one request at a time, skipped past the cache budget.
- **Rendering.** WebGL2 only (no WebGPU path in Phase 5): `R16F`/`RG16F` textures divided by each block's fluid
  fraction, colour maps **generated from Rust's `Colormap`** (the CLI's PNGs and the viewer agree), automatic,
  symmetric (±p99) or manual ranges, instanced arrows read from the velocity texture, streamlines (RK4 on the unit
  direction field, in the decode worker), particles in an `RG32F` ping-pong texture and dye in `RGBA16F` (both need
  `EXT_color_buffer_float`; the checkboxes are disabled without it). Export draws the same scene at `cells × scale`
  into a PNG, and a video with `VideoEncoder` + `mp4-muxer` (H.264, else VP9/AV1; a `MediaRecorder` WebM without
  WebCodecs). New dependencies: `uPlot`, `mp4-muxer`; in Rust `rust-embed`, `webbrowser`.
- **Series.** The solver's samples reach the viewer as `series` WebSocket messages (rows of `forces.csv`,
  `probes.csv`, `diagnostics.csv`, and `heat.csv` for thermal runs), batched by the server right before a (throttled)
  `progress` and before `done`. `cfd-job` hands each sample to `JobObserver::sample` as the numbers of its CSV line —
  **one formatting path** — and flushes the files before every `progress`, so every row a viewer was sent is on disk;
  a viewer that joins late reads the CSVs and appends the stream's rows past their last step. `heat.csv`
  (`cfd_core::heat`: the heat through each fixed-temperature or flux wall, from the stored frame's temperature) is
  written by the **output thread**, so it is the same for every engine; adiabatic walls, inlets and outlets are not
  counted. `Scene.reference` (optional length, velocity, ΔT) gives Re, Ra and the force coefficients their scales
  without changing the time step. *Amended 2026-10-27 (#19):* the Statistics tab overrides them **for the session**
  (not saved; reset with the job, an "overridden" marker and Reset) through a modified copy of `meta.json`'s
  reference, so C_D, C_L, St, Nu, the plots, the CSV exports and the tolerance's cells per length follow together.
- **Statistics in TypeScript, references from Rust.** St, C_D and C_L statistics and the Nusselt averages are computed
  in the browser over a window (default: the second half of the run so far; draggable on a plot). Rust fixtures keep
  the two implementations together: a trimmed real `forces.csv` and the frequency `cfd-verify` finds in it (St agrees
  to 1e-12), a real cavity `heat.csv` and its means (1e-12; only arithmetic is involved, so Linux CI computes the same
  numbers). The benchmark references and the validation suite's tolerance by resolution (5 % at D = 20 cells, 2 % at
  the finest size, 2 % for Nu at N ≥ 64) are generated by `cfd-cli references` from `cfd_verify::references` and
  checked as up to date.
- **The app and the gallery.** `cfd-server` serves the web app from the same origin as the API: `/api/*` is the API
  (an unknown `/api/…` path stays the JSON 404), every other path is a static file (`index.html` for `/`; hashed
  `assets/` are `immutable`, the rest `no-cache`). `--web-dir DIR` serves a directory (`tower-http`); the Cargo feature
  `web` (default off, so `cargo build` and the Rust CI need no Node) embeds `apps/web/dist` with `rust-embed`;
  `--open` opens the browser once the socket is bound. The gallery's **Open with results** calls
  `POST /api/simulations?reuse=completed`: the latest completed job with the same `scene_hash` (SHA-256 of the
  validated scene's canonical JSON plus every option that changes the result but the engine) whose results are still
  on disk, else a new job. On a CPU-only server the gallery offers **quick variants** (`cfd_core::examples::quick`,
  `scenes/*-quick.json`): channel 200 × 50 cells, 10 s (0.7 s on the CPU); heated cavity 64², 80 s (5.9 s); the dam
  break has none. **The cylinder's half resolution diverges** (D = 10 cells, τ = 0.5100, at step 1,290 of 30,000 at
  the default lattice velocity; a higher one breaks the Mach limit), so the variant uses a **coarser grid of 58 cells
  across the channel** (D ≈ 14 cells, τ = 0.5141): stable, 10 s in 6.1 s. The cylinder's St/C_D acceptance applies to
  the full example only.
- **Acceptance.** The MVP criterion of the proposal's §14 is the Playwright test tagged `@full`
  (`npm run test:e2e:full`, needs a GPU server, not run in CI; `npm run test:e2e` leaves it out): from the editor's
  gallery open **Cylinder Re=100**, run it on the GPU, watch the street form live, scrub to the start and play again,
  read the statistics, export a PNG, the force CSV and a video. The cylinder example runs **20 s with a frame every
  0.02 s** (the approved 12 s does not work: the shedding's overshoot decays until ≈ 10 s, so the second half of a
  12 s run reads C_D,max 3.419, +5.8 %; over 10–20 s the run is periodic). Measured (2026-10-22, RTX 4060, CUDA,
  120,000 steps in ≈ 2 s, default window 10–20 s, D = 20 cells, 28 periods): **St 0.2973 (−0.9 %) ✓, C_D,max 3.375
  (+4.5 %) ✓** (limit ±5 %), C_L,max 1.040 (+4.0 %, information). The test saw 860 distinct playhead positions before
  the job completed, 11 vorticity sign changes along the wake's centre line at the end (at most one at frame 0), a
  PNG of 120 kB at 2 px per cell, a force CSV of 10,001 rows whose C_D,max over 10–20 s equals the Statistics tab's,
  and an MP4 of 51 frames (154 kB); the whole flow takes ≈ 45 s.
- **Consequences:** the viewer costs the server one encoding per frame, and the browser's memory is bounded by the
  cache (a 1024² run plays from the server rather than from memory; a 4096² run at a few frames/s — a downsampled
  preview stored next to each frame would fix it). The stream's rows and the CSVs cannot disagree. The statistics
  have two implementations (Rust for the suite, TypeScript for the viewer), tied by fixtures instead of by WASM;
  compiling `cfd-core` to WASM stays the option for a serverless demo (ADR-010). Streamlines run on the CPU,
  particles and dye on the GPU, and neither is part of any acceptance number. Not done: a Nusselt plot in the Plots
  tab; the MediaRecorder fallback and the video's image were not looked at by eye; open question Q14 (the editor's
  discoverability, Phase 4's hand-timed run) is unchanged.

## Open questions

Answers to these questions can change priorities. Record the answer and the date here.

| # | Question | Answer / default assumed until answered |
|---|----------|-----------------------------------------|
| Q1 | Who is the main audience: personal/educational use, or a public multi-user service? | **Answered (2026-10-09):** portfolio and exploration project; **open source and public**; useful for education and quick simulation examples. Multi-user production remains far off |
| Q2 | Which GPU(s) are available for development and for the server (vendor, VRAM)? | **Answered (2026-10-09):** **NVIDIA GeForce RTX 4060, 8 GB** (driver 610.62 = CUDA 13.3, updated to 617.42 = CUDA 13.4 on 2026-10-12); CUDA Toolkit 13.4 installed and verified; GPU performance counters enabled for Nsight Compute |
| Q3 | Where will the server run: local machine, own server, cloud? | **Answered (2026-10-09):** only on the development machine; no budget for a server. It should be easy for others to self-host (see ADR-010) |
| Q4 | Is engineering-grade accuracy (quantitative validation) needed, or is the focus visual/interactive? | **Answered (2026-10-09):** yes, quantitative accuracy validation; method explained in [08 — Validation guide](08-validation-guide.md) |
| Q5 | Priority between gases (aerodynamics, ventilation, cooling) and liquids (free surface)? | **Answered (2026-10-09):** **gases and liquids in the first stage** → free-surface liquids are in the MVP (see ADR-008) |
| Q6 | Is interactive mode (changing geometry while the simulation runs) important early on? | **Answered (2026-10-09):** wanted, not in the MVP → architecture ready from Phase 3 (ADR-011), UI in Phase 5b |
| Q7 | Frontend framework preference (React, Svelte, other)? | **Answered (2026-10-09):** React (ADR-005, with recommended tooling) |
| Q8 | Is 3D a real medium-term goal or just a "maybe"? | **Answered (2026-10-09):** a real goal; **Stage 2**, after 2D is complete and tested (ADR-004) |
| Q9 | "Other states" — what do you have in mind: plasma, granular, deformable solids, phase change? | **Answered (2026-10-09):** post-MVP: granular media, plasma and common phase changes, including granular → liquid (crushed ice melting) (ADR-012). Rare cases (solid ↔ gas: sublimation/deposition, and others) in a later phase (Phase 9c) |
| Q10 | Project license — open source confirmed; which license? | **Answered (2026-10-09):** **MIT OR Apache-2.0** |
| Q11 | Prior experience with Rust, GPUs and CFD (to calibrate documentation detail and pace)? | **Answered (2026-10-09):** strong CFD experience; little Rust and GPU experience (learns fast); C# background. → Phase 0 includes learning prototypes (see the plan) |
| Q12 | Project name: the repository is `cfd-flux`, the docs and packages say `live-fluids`. Which one is final? | **Answered (2026-10-13):** **`cfd-flux`** (the repository name); **renamed 2026-10-18** in the docs, crate and package descriptions, the web app, the license notice and the server's database file (`<data>/cfd-flux.db`); session logs keep the old name as written, and the local folder is the author's to rename |
| Q13 | Frame encoding: the pure-Rust `ruzstd` encoder (ADR-007) takes 12.6 ms per stored f32 frame of the 442 × 84 cylinder (4 values per cell) and 7.2 ms per f16 stream frame, for a gain of only 3–4% (594 → 574 kB, 297 → 286 kB: the low mantissa bits of physical fields are noise). This caps the server at ≈ 37 frames/s there (ADR-025) and would cost ≈ 0.35 s per stored 1024² frame. Options: no compression for stream frames, a byte-shuffle filter before zstd (as HDF5/Blosc do for floats), the C `zstd` crate, f16 storage by default. | **Answered (2026-10-13): byte shuffle + `lz4_flex`** (pure Rust) for stored and streamed frames, with a `codec` field in a v2 frame header (v1 zstd frames stay readable); **implemented 2026-10-16** (LFFR v2, ADR-007). Measured on 2026-10-13 ([benchmarks](benchmarks.md#frame-encoding-open-question-q13--amd-ryzen-7-3700x-2026-10-13)): a byte shuffle makes every codec 25–35% smaller; shuffle + `lz4_flex` (pure Rust) encodes a 1024² f32 frame in 25 ms to 64% (now: 305 ms, 88%), shuffle + C `zstd` level 1 in 34 ms to 60%. |
| Q14 | Editor discoverability (the Phase 4 hand-timed acceptance took 9 min 30 s against < 2 min): the author drew the channel's walls, inlet and outlet by hand before finding out that the domain edges are walls already and that an edge can be painted as an inlet/outlet in one action. Options: **A** — a "Channel" starting template in *New scene* next to the empty one (edges walls, a parabolic inlet on the left edge, a pressure outlet on the right, the elements created); **B** — visible edge presets: the canvas draws the four domain edges with their current kind (wall/periodic), and the Scene tab gets per-edge buttons "Make inlet" / "Make outlet" that paint the edge column and create the element in one undo entry; **C** — both, plus a one-line hint in the status bar ("edges are walls; drag along an edge to make it an inlet"). | **Answered (2026-10-21): B**, visible edge presets — the canvas shows each domain edge's kind, and the Scene tab gets per-edge "Make inlet" / "Make outlet" actions that paint the edge column and create the element in one undo entry (it fixes the cause, the edges' invisible state, for every scene). A only if a re-timed run is still slow. To implement after the Phase 5 acceptance, then the author re-times the Phase 4 run. |

### License options (Q10)

| License | Type | What it allows / requires | Good fit if… |
|---------|------|---------------------------|--------------|
| **MIT** | Permissive | Any use, including commercial and closed source; only requires keeping the copyright notice | You want maximum adoption, no strings attached |
| **Apache-2.0** | Permissive | Like MIT + an explicit patent grant and rules for contributions | Same, with patent protection |
| **MIT OR Apache-2.0** | Permissive (dual) | Users pick one; the Rust ecosystem convention | A Rust project that wants frictionless integration |
| **MPL-2.0** | Weak copyleft (per file) | Modified files must stay open; can be combined with closed code | Middle ground |
| **GPL-3.0** | Strong copyleft | Anyone **distributing** modified versions must open the whole code under the GPL | You want distributed derivatives to stay open |
| **AGPL-3.0** | Strong copyleft + network | Like the GPL, but also anyone **offering a modified version as a web service** must open the code | You want to stop someone closing the project and selling it as SaaS |
