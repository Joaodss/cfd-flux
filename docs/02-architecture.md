# 02 — Architecture

## 1. Overview

```
┌──────────────────────────────── Browser ──────────────────────────────────┐
│  Editor (Canvas2D)   Properties panel     Viewer (WebGL2/WebGPU)          │
│        │                   │                       ▲                      │
│        └── Scene (TS state)┘                       │ binary frames        │
└───────────────┬────────────────────────────────────┼──────────────────────┘
                │ POST /api/simulations (JSON+layers) │ WS /api/simulations/{id}/stream
                ▼                                     │
┌──────────────────────────── cfd-server (Rust, axum) ┴─────────────────────┐
│  Validation ─▶ Job manager ─▶ Queue (in-process → Redis/NATS)             │
│  Metadata (SQLite → Postgres)   Results (disk → S3/MinIO)                 │
└───────────────────────────────┬───────────────────────────────────────────┘
                                ▼
┌──────────────────────────── cfd-worker (Rust) ────────────────────────────┐
│  Scene → Domain (grid, masks, materials, lattice units)                   │
│  trait Solver ──┬── LbmSolver ──┬── GPU backend (wgpu + WGSL)             │
│                 │               ├── GPU backend (CUDA via cudarc)         │
│                 │               └── CPU backend (rayon + SIMD)            │
│                 ├── ProjectionSolver (Phase 7)                            │
│                 └── ... (FLIP, compressible FV, AI surrogate)             │
│  Output: field sampling → encoding (f16, shuffle + lz4) → stream/disk     │
└───────────────────────────────────────────────────────────────────────────┘
```

**Server and worker run in the same process** (ADR-006; implemented in Phase 3): axum on the tokio runtime handles I/O, and each running job has two dedicated OS threads, one for the solver (its command loop) and one for its output. Splitting into separate processes/machines only happens when needed (several GPUs, several users).

```
                         ┌──────────────── cfd-server process ─────────────────────────────────┐
 browser / test client   │ tokio runtime (async)                       dedicated threads       │
 ─────────────────────   │ ┌──────────────────────┐                   ┌──────────────────────┐ │
 REST (JSON) ───────────▶│ │ axum routes          │  Command (mpsc)   │ job thread           │ │
                         │ │  /api/simulations …  │──────────────────▶│  Box<dyn Solver>     │ │
 WS /stream ◀───────────▶│ │ JobManager (State)   │                   │  cfd_job::run        │ │
   text: events (JSON)   │ │  slot per device     │  Event (broadcast)│  command loop        │ │
   binary: LFFR frames   │ │  (tokio Semaphore)   │◀──────────────────│                      │ │
                         │ │ one task per viewer  │  latest frame     ├──────────────────────┤ │
                         │ │                      │◀───── (watch) ────│ output thread        │ │
                         │ └──────────┬───────────┘                   │  convert, encode,    │ │
                         │            │ sqlx                          │  write frames/       │ │
                         │       SQLite (jobs, scenes)        results/{id}/ (cfd-job layout) │ │
                         └─────────────────────────────────────────────────────────────────────┘
```

`cfd-cli run` and a server job share the same code (`cfd-engines` to open engines, `cfd-job` for the run loop and the result layout; ADR-024), so they write the same result directories.

## 2. Components

### 2.1 Frontend — `apps/web`
- **Stack:** TypeScript + Vite + React, Zustand (+ immer), Radix/shadcn + Tailwind, lucide icons, uPlot; Vitest (+ Testing Library on happy-dom) + Playwright (the Phase 4 acceptance scenario and the viewer tests, in CI; the Phase 5 acceptance run on the GPU, tagged `@full`, locally); Three.js/react-three-fiber for the 3D stage (see [ADR-005](07-decisions-and-questions.md)).
- **Editor:** layers stored as `Uint8Array`/`Uint16Array` (one per logical layer, outside React), rendered with **WebGL2**: the layers are uploaded unchanged into integer textures (`R8UI` cell type, `R16UI` element id; changed 64×64 tiles with `texSubImage2D`), and a fragment shader maps each screen pixel to its cell, colours it from a palette texture and draws grid lines from 8 px/cell; a Canvas2D overlay draws tool previews and the domain edges (a band per stretch, coloured and named: the scene's wall/periodic kind, or the inlet/outlet drawn on the edge cells; the Scene tab's "Make wall/inlet/outlet" paints an edge in one undo entry, Q14). Zoom 1/8–32 px/cell on a √2 ladder, pan, fit. Drawing with Bresenham / scanline fill. A 4096² domain uploads and draws in ≈ 80 ms, a changed tile in ≈ 3 ms (design: [Phase 4 proposal](sessions/phase4-design-proposal.md) §3).
- **State:** a Zustand store (with `immer`) for the scene table, view and UI state; the pixel layers are only referenced from it. Tools write through a revertible `Stroke` that copies each 64×64 tile on its first write (shapes and moved selections are redrawn live on every pointer move); the history keeps the changed tiles and undo/redo *swap* them with the layers (one copy per tile, uniform tiles stored as one value), within 256 MB / 200 entries.
- **Documents:** a scene is saved as a `.cfdscene` file (the `Scene` JSON; layers `deflate+base64`, compressed natively with `CompressionStream`) through the File System Access API where the browser has it (a download elsewhere), or on the server (`POST`/`PUT /api/scenes`); a draft is autosaved to IndexedDB while there are unsaved changes and offered back at start-up. Opening checks the file against the generated JSON Schema (ajv, loaded on demand). PNG import turns dark or opaque pixels into a mask placed as a floating selection.
- **Validation:** the server checks the scene (`POST /api/scenes/validate`) 400 ms after the last edit; a newer edit aborts the request in flight. Issues are worded by the editor from their stable code and `params` (`src/messages.ts`; the server's English text for codes it does not know), listed under the header (a click shows the cell or selects the element) and marked on the canvas; the Run tab shows the estimate (steps, memory, time, engine, Re, Ma, τ, …).
- **Runs:** **Run** (enabled when the last validation says the scene can run; warnings are confirmed first) posts the scene to `/api/simulations` and opens the job view at `#/jobs/{id}` — hash routes, so reload and back work with no router library and no server routes. The job view follows the job's WebSocket (status, progress, log, frames for the viewer, `series` rows for the plots) and pauses, resumes or cancels it; `#/jobs` lists runs. The editor's document stays in the store, so "Editor" returns to the same drawing.
- **Viewer** (Phase 5, [design](sessions/phase5-design-proposal.md)): the job view's main area. The stream asks for every frame field at most `maxSize` cells on a side (the *viewer format*: 256, 512, 1024 — the default — 2048 or every cell, a setting in a popover on the viewer's toolbar, remembered per browser in `localStorage`; another one reconnects the stream, empties the cache, rebuilds the blocks' fluid fraction and loads the playhead's frame again); frames are decoded in a **pool of Web Workers** (LFFR v2: header, a TypeScript LZ4 block decoder, byte unshuffle; f16 kept as raw bits; 1–4 workers by the machine's threads, the next frame to the least busy, streamlines in a worker of their own) that also compute each quantity's min/max and 99th percentile of |value| over fluid cells, and the buffers are transferred, not copied. A **WebGL2** fragment shader draws the f16 fields from `R16F`/`RG16F` textures (bilinear, or blocks), divided by each block's fluid fraction (samples are 0 at non-fluid cells), through a colour map generated from Rust's `cfd_io::image::Colormap` (so `cfd-cli render` and the viewer agree), with the solid cells from the run's `scene.json` at full resolution. Ranges: all frames so far (seeded from `hello.ranges`), this frame, symmetric ±p99, or manual; an HTML legend; zoom/pan from the editor's view code. WebGL2 only (no WebGPU path) in Phase 5. **Overlays** share the field renderer’s context and textures: velocity arrows are instanced `GL_LINES` (six vertices per arrow, the grid is the instance id, the vertex shader reads the velocity texture); streamlines are traced in the worker (RK4 over an f32 copy of the velocity, bilinear, jittered seeds with a minimum separation, stopped at solids, edges, stagnation or loops) and drawn as segments; **tracer particles** are an `RG32F` ping-pong texture advanced by a fragment shader (midpoint rule, simulated seconds = frames per second × the frames’ interval, split so nothing moves more than a cell per pass) and drawn as points read in the vertex shader; the **dye** is an `RGBA16F` ping-pong texture at the frames’ block resolution, advected semi-Lagrangian. Float render targets need `EXT_color_buffer_float` (reported to the UI; present in SwiftShader). **Timeline and cache** ([ADR-031](07-decisions-and-questions.md)): play/pause, step, scrub, speed and *Live* (follow the newest frame of a running job; at the end of a run the playhead goes to the last frame). Decoded frames live in a memory-only LRU (512 MiB by default, a setting from 128 MiB to 4 GiB — a smaller one evicts at once; the frames farthest from the playhead are evicted first, the next frames in the play direction are prefetched); live and stored frames are interchangeable because both are in the viewer format, and a frame not in the cache is fetched from the server with `GET …/frames/{n}?fields=&maxSize=`. There is no IndexedDB for frames: the server, which stores every frame, is the store. A lost WebGL context (a driver reset, GPU memory pressure) is taken back: the viewer calls `preventDefault()` on `webglcontextlost` and stops drawing, and on `webglcontextrestored` makes its programs, textures and buffers again, uploads the frame shown and redraws (particles and dye start afresh); meanwhile exports say the context was lost. **Sharpen:** when the timeline is paused (not playing, not Live) and one block of the viewer format covers more than one device pixel, the playhead's frame is fetched at full resolution (`GET …/frames/{n}?dtype=f16&fields=…`, no `maxSize`, in a worker, without statistics) and drawn instead, divided by a per-cell fluid fraction (1 or 0, so samples beside a wall are still means over fluid cells); playing, scrubbing to another frame, Live or zooming out go back to the cached frame. One request at a time, answers for a frame no longer shown are dropped, and a frame larger than the cache budget is not fetched (the readout says so); arrows, streamlines, particles and dye keep the viewer-format velocity. The readout shows the size of the frame drawn.
- **Export** (Phase 5): the job view’s *Export* tab. PNG: the view as drawn, or the whole domain rendered at 1/2/4 px per cell by resizing the canvas for one draw (overlays scale with it), with a 48 px footer (colour bar, ticks, unit, time, frame) painted on a 2D canvas. CSV: the run’s files as plain downloads, C_D/C_L of the chosen element, the window statistics, and the fields of the frame shown (one row per block, means over the fluid). Video: `VideoEncoder` (the first of H.264, VP9, AV1 that `isConfigSupported` accepts at the size) feeding `mp4-muxer`; the video's clock is the flow's — a speed (real time, × N) over the frames' simulated interval gives the frame rate, every n-th frame past a cap — and each frame used is loaded (the next ones prefetched through the decode workers, as in playback, within half the cache), drawn with its streamlines and overlays, and encoded with its own timestamp, so the count is exact; particles and dye advance by the simulated time between the frames used after a warm-up; *High quality* is 2 bits per pixel per frame (*Standard* 0.5). Without WebCodecs a `MediaRecorder` records the canvas in real time as WebM: frames are drawn ahead into a buffer of up to 128 MiB, the recorder is paused only when the buffer runs dry (until it is full again), the waits aim at each frame's due time, and the last frame is marked once more so the video lasts count ÷ rate (length within ≈ 2 %).
- **Plots and statistics** (Phase 5): the job view's side panel has the tabs *Plots*, *Statistics* and *Run*. The series (`forces`, `probes`, `diagnostics` and, for thermal runs, `heat`) come from the CSV files of the run, read once it has started, followed by the rows of the stream's `series` messages after the files' last step (the job flushes the files before each `progress`, so nothing falls between them); the rows are plain arrays held column by column outside React. **uPlot** draws C_D/C_L of one element (the first *body* — a wall element off the domain's edge — by default; coefficients from `meta.json`'s reference scales, or the session's override below), the probes, the mass drift and the largest Mach number, with a dashed line at the playhead; a click seeks to the closest frame, a drag sets the statistics window (shaded on every plot). Statistics in TypeScript over the window (default: the second half of the run so far): mean C_D, C_D,max, mean C_L, C_L amplitude, C_L,max and St from the upward crossings of C_L through its window mean — `cfd_verify::harness::oscillation_frequency`'s algorithm, held to it by a shared fixture. The run is compared with the benchmark of the gallery example it comes from (`src/scene/examples.ts`), using reference intervals and the validation suite's tolerance at the run's resolution generated from `cfd-verify` (`src/generated/references.json`, `cfd-cli references`). **Reference scales** are the run's (the scene's `reference` block, in `meta.json`) or overridden for the session in the Statistics tab (length, velocity and, for thermal runs, ΔT; an "overridden" marker and Reset; not saved in the scene or the browser, and reset with the job): the override is a modified copy of `meta.json`'s reference (`viewer/scales.ts`, Re follows), which every coefficient, St, Nu, plot and CSV export reads, and the tolerance uses the resolution in cells per overridden length.
- **Gallery** (Phase 5, [ADR-009](07-decisions-and-questions.md)): the *Examples* dialog lists the scenes of `scenes/` (generated by `cfd-cli example --all scenes`) with a thumbnail of their cells and, when the example reproduces a benchmark, its name. *Open* loads the scene in the editor; *Open with results* posts it with `?reuse=completed` and opens the job view of the latest completed run of the same scene and options (or of the run it starts), so the first open computes and the next ones do not. Nothing is stored in the repository or the releases. A server whose best engine is the CPU (`/api/capabilities.auto`) offers the examples' **quick variants** (`cfd_core::examples::quick`, `cfd-cli example --quick`, `scenes/<name>-quick.json`: a coarser grid and a shorter run, seconds instead of minutes); the dam break (free surface, Phase 6a) has none and cannot run yet.
- **Nusselt number** (Phase 5): for thermal runs with stored frames the Statistics tab shows `Nu = q L / (k ΔT · length)` of every thermal wall, from `heat.csv` (03 §6.2), averaged over the window (a steady flow such as the heated cavity has settled: the spread shows how much), against de Vahl Davis with the validation suite's tolerance at the run's resolution. The formula and the wall gradient are `cfd_core::heat`, shared with `cfd-verify`; a shared fixture holds the TypeScript to Rust to 1e-12.
- **Communication:** REST for CRUD, binary WebSocket for frames.
- **Optional, later:** the solver core compiled to WASM/WebGPU for low-resolution local previews.

### 2.2 API — `crates/cfd-server`
- **Stack:** Rust, `axum` 0.8 (with WebSockets), `tokio`, `serde`, `tower-http` (CORS for the Vite dev origin, gzip for JSON/CSV, tracing), `DefaultBodyLimit` (64 MB), `tracing`. Binds `127.0.0.1:7878` by default (`--listen`); no authentication (ADR-010).
- **Responsibilities:** request validation (format, consistency, stability, the server's limits, an engine that fits — stable issue codes), job management, streaming, serving results. Routes and messages in [03 §7](03-data-model.md#7-api).
- **Web app:** the same process serves `apps/web` (ADR-010): `/api/*` is the API (an unknown `/api/…` path is the JSON `404`), every other path a static file and `/` is `index.html` (hash routes need no fallback); files under `assets/` (Vite's hashed names) are `Cache-Control: immutable`, the rest `no-cache`. The files come from `--web-dir DIR` (`tower-http`'s `ServeDir`) or, with the Cargo feature `web` (default off, so the Rust CI needs no Node), from `apps/web/dist` embedded by `rust-embed` (`npm run build`, then `cargo build --release --features web`); `--open` opens the browser once the socket is bound. Source: `crates/cfd-server/src/web.rs`.
- **Errors:** one `ApiError` enum mapped to a status code and `{ "error": { code, message, issues? } }` by `IntoResponse`; an `ApiJson` extractor turns body-limit rejections into a `413` that names the `zstd+base64` layer encoding.
- **Persistence:** `sqlx` with SQLite (runtime-checked queries, migrations embedded with `sqlx::migrate!`, bundled SQLite) — tables `jobs` and `scenes`; Postgres if ever hosted. Jobs still active when the server stopped are marked `failed/interrupted` on start-up (no checkpoints yet). Results are files under `<data>/results/{job id}/` (`cfd_job::LocalStore`, the 03 §6.2 layout); an object store would turn it into a trait.

### 2.3 Queue and workers
- **Now:** jobs wait on **one `tokio::sync::Semaphore` per physical device** (FIFO): one permit per GPU — CUDA and wgpu on the same card share it, matched by PCI location — and `--cpu-jobs` (default 1) for the CPU, whose rayon pool is shared. A job's task acquires the permit, starts the job's thread and moves the permit into it, so the device is released when the thread ends, whatever the outcome. At most `maxQueuedJobs` (16) wait; more are refused with `429`.
- **Engine choice:** `cfd_engines::Inventory` opens every engine once at start-up; `auto` takes the first available hardware engine (CUDA > wgpu > CPU) whose device holds the domain (`cells × bytes per cell` against 90% of the GPU's memory and the largest buffer); a domain no engine can hold is refused at submission (`422`), never queued.
- **Scaling (later):** Redis Streams or NATS JetStream; stateless workers advertise their capabilities (GPU, memory, supported backends).

### 2.4 Simulation core — `cfd-core` and solvers
- `cfd-core`: shared types (Scene, `Domain`, units and physical ↔ lattice conversion, the `Solver` trait, sampling to physical fields: `FieldSet::from_lattice` and `sample`, the specs, samples and box filter of the output path).
- `cfd-lbm`: LBM D2Q9 (+ D2Q5 thermal), CPU backend (`CpuLbm`, the numerical reference), and the pieces every LBM backend shares (`shared`: initial state, equilibrium populations, bounce-back links and force summation; `abi`: the kernels' parameter blocks, `SampleParams`/`SampleLayout` of the sampling kernels and the generated constants). All three backends run the same fused stream-collide step, with or without the macroscopic stores (`skip_macro`, [ADR-029](07-decisions-and-questions.md)); the GPU backends also compute the output samples on the device ([ADR-028](07-decisions-and-questions.md)), against the CPU conversion as reference.
- `cfd-gpu`: wgpu backend (`WgpuLbm`): adapter selection (`GpuContext`), buffers and bind groups with the CPU layout, WGSL shaders with a prelude generated from the Rust constants ([ADR-019](07-decisions-and-questions.md)).
- `cfd-cuda`: CUDA backend (`CudaLbm`) behind the `cuda` feature: `cudarc` loaded at runtime (no toolkit needed to build), device discovery (`CudaDevice`), `.cu` kernels translated from the WGSL ones and compiled by NVRTC to an `sm_XY` cubin, the CPU layout, all `unsafe` code in one module ([ADR-021](07-decisions-and-questions.md)). Constants and the parameter block for both GPU backends are generated by `cfd_lbm::abi`.
- Common interface (`crates/cfd-core/src/solver.rs`), implemented directly by each backend type ([ADR-017](07-decisions-and-questions.md)):

```rust
pub trait Solver: Send {
    fn backend(&self) -> BackendKind;
    fn domain(&self) -> &Domain;
    fn steps_done(&self) -> u64;
    fn step(&mut self, n: u32) -> Result<(), SolverError>;                 // advance n steps; fails on NaN/Inf (GPU: may return before the work is done)
    fn finish(&mut self) -> Result<(), SolverError> { … }                  // wait for queued steps (no-op on the CPU)
    fn set_equilibrium(&mut self, f: &LatticeFields) -> Result<(), SolverError>; // custom initial fields
    fn lattice_fields(&mut self) -> LatticeFields;                         // copy ρ, u, θ to the CPU
    fn request_fields(&mut self) -> Box<dyn PendingFields> { … }          // the same copy, collected later (ADR-020)
    fn diagnostics(&mut self) -> Diagnostics;                              // mass, energy, max velocity, NaN check
    fn forces(&mut self) -> Vec<ElementForce>;                             // momentum exchange per element
    fn sample(&mut self, req: &SampleRequest) -> FieldSet { … }            // physical fields (shared code)
    fn probes(&mut self) -> Vec<ProbeSample> { … }                         // physical probe values (shared code)
}
```

Construction is backend-specific (`CpuLbm::new(domain, config)`, `WgpuLbm::new(gpu_context, domain, config)`, `CudaLbm::new(cuda_device, domain, config)`); code that picks a backend at runtime holds a `Box<dyn Solver>`. Interactive updates (`update_boundaries`, ADR-011) are added in Phase 5b.

### 2.5 CLI — `crates/cfd-cli`
Runs scenes from files without a server: essential for development, benchmarks and validation tests (`cfd-cli run scene.json --backend cuda --out out/`). `run` is a thin wrapper over `cfd-job` (a console observer, then PNG rendering); `cfd-cli submit scene.json --server URL [--cancel-after S]` is the server's test client (time to the first frame, frames/s, skipped frames, cancel latency).

Shared by the CLI and the server (ADR-024):
- `cfd-engines`: the engine kinds, opening engines (`auto` = CUDA > hardware wgpu > CPU), `SolverChoice::make` (the only code that names the concrete solver types), and the `Inventory` the server opens once (devices, memory, physical-device keys).
- `cfd-job`: `prepare` (`Scene → Domain` with the run's overrides), `run` (the command loop and output thread of §2.6), `meta.json`, the CSV series and the result directory (`ResultDir`, `LocalStore`).

### 2.6 Jobs and streaming
- **Command loop** (`cfd_job::run`, ADR-011, ADR-025): between batches the loop drains commands (`Pause`, `Resume`, `Cancel`, `Snapshot`; `PatchCells`/`SetParam` are answered "not supported" until Phase 5b); while paused it blocks on the channel. Each batch runs up to the next sample/output event and at most ≈ 50 ms of measured work (the batch size follows the measured step rate), which bounds the latency of every command; batching never changes results (tested byte for byte).
- **Output thread:** for each output event the job requests one *sample* per spec — the stored frame (full resolution, f32) and every stream format viewers watch (f16, box-downsampled) — with `Solver::request_samples`; GPU backends compute them on the device and copy only the result (ADR-028). The samples are encoded and written on a second thread fed by a bounded channel (2 frames): stored frames are never dropped — if the disk or the encoder is slower, the solver waits. A cancel drops the frames still queued. Every frame's samples also go to the job's observer (the server's stream hub, which encodes the sample of each format).
- **Streaming** (ADR-026): per job, a `tokio::sync::broadcast` channel carries events (`status`, `progress` at most 10 Hz, `log`, `done`) and, for every stream format a viewer asked for (fields, maximum size), a `tokio::sync::watch` channel holds the **latest** encoded frame (f16, box-downsampled). Each viewer's task sends the latest frame and waits for the socket: a slow viewer skips frames, the job never waits for viewers, and the final frame is always delivered before `done`.

## 3. Request flow

1. The user draws and configures → the frontend serialises the `Scene` (see [03](03-data-model.md)).
2. `POST /api/simulations` → the server validates the format, consistency and stability (`Scene → Domain` on tokio's blocking pool), its limits and an engine that can hold the domain; estimates memory, stored bytes and time (from the throughput of earlier jobs).
3. Job created (`queued`, a row in SQLite, `results/{id}/`) → `201` with the job; a task waits for the engine's device slot.
4. The client opens `WS /api/simulations/{id}/stream` → `hello` (job, grid, stream format, field ranges so far).
5. Slot acquired → the job's thread makes the solver and runs the command loop: samples (diagnostics, forces, probes → CSV, `progress` events), frames (samples on the device → output thread: stored f32 frame, f16 stream frames), commands between batches.
6. End (`completed` / `failed` with `diverged`, `error` or `interrupted` / `cancelled`) → `meta.json`, the SQLite row, a `done` event; the device slot passes to the next job.

## 4. Technology stack

| Layer | Choice | Alternatives considered |
|-------|--------|-------------------------|
| Server/solver language | **Rust** | C++ (more CFD libraries, less safety), Julia (great for prototyping, harder to deploy) |
| GPU | **wgpu + WGSL** (Vulkan/DX12/Metal) **and CUDA via `cudarc`** (NVIDIA, `cuda` feature) — both from Phase 2 | rust-gpu, HIP (AMD) |
| CPU parallelism | **rayon** + `std::simd`/`wide` | OpenMP via C++ |
| HTTP/WS | **axum** | actix-web |
| Serialisation | **serde** (JSON) + custom binary / MessagePack | Protobuf/FlatBuffers |
| Compression | **byte shuffle + lz4** (pure-Rust `lz4_flex`) for frames (Q13); zstd (`ruzstd`) for scene layers | zstd for frames (v1: 20–25× slower, 25–35% larger without a shuffle) |
| Database | SQLite → Postgres (`sqlx`) | — |
| Frontend | **TypeScript + Vite + React** | Svelte, SolidJS |
| Result rendering | WebGL2 → WebGPU | Canvas2D (slow) |
| AI (inference) | `ort` (ONNX Runtime) or `burn`/`candle` | Separate Python service |
| AI (training) | Python + PyTorch (offline) | JAX |

## 5. Repository layout (monorepo)

```
cfd-flux/
├── README.md, CLAUDE.md, docs/
├── Cargo.toml                 # Rust workspace
├── crates/
│   ├── cfd-core/              # scene format, validation, units, Scene→Domain, traits
│   ├── cfd-lbm/               # LBM solver (CPU) + shared logic
│   ├── cfd-gpu/               # wgpu infrastructure + WGSL shaders
│   ├── cfd-cuda/              # CUDA backend (cudarc + .cu kernels), `cuda` feature
│   ├── cfd-io/                # result formats, frame encoding
│   ├── cfd-engines/           # engine selection (shared by the CLI and the server)
│   ├── cfd-job/               # job runner: run loop, result layout (shared by the CLI and the server)
│   ├── cfd-server/            # axum API + jobs + streaming + SQLite
│   ├── cfd-verify/            # backend-agnostic validation cases + report (uses `Box<dyn Solver>`)
│   └── cfd-cli/               # run/render/bench/verify/compare/submit CLI
├── apps/
│   └── web/                   # TS frontend
├── schema/                    # Scene JSON Schema (generated)
├── scenes/                    # example and validation scenes (generated)
├── validation/                # reference data with sources (Ghia, Schäfer-Turek, de Vahl Davis) + generated report
├── sandbox/                   # learning prototypes, outside the Cargo workspace
└── tools/                     # scripts (benchmarks, AI data generation)
```

**Scene format:** the Rust types in `cfd-core::scene` (`serde` + `schemars`) are the source of truth. The JSON Schema in `schema/scene.schema.json` is generated from them (`cfd-cli schema`) and committed; a test fails if it is out of date. The frontend TS types are generated from the schema (`npm run gen:types`). So: Rust → JSON Schema → TypeScript, with no hand-written duplication.

## 6. Security and limits

- Hard limits: domain size, number of steps, wall-clock time, stored frames, concurrent jobs per user.
- Validation of every index/ID (no worker panics on malicious input); per-job timeouts.
- No user code is executed (expressions for inlet profiles, if ever supported, use a restricted sandboxed evaluator).
- API rate limiting; authentication (sessions or OAuth) before any public hosting.

## 7. Deployment (future)

- Local-first: a single `cfd-server` binary that also serves the frontend (see ADR-010).
- For self-hosting: container with a Vulkan runtime (NVIDIA: `nvidia-container-toolkit`), one `cfd-server` + N `cfd-worker` per GPU machine.
- Observability: `tracing` + OpenTelemetry → Prometheus/Grafana.
