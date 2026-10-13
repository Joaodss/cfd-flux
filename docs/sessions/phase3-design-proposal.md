# Phase 3 — server and jobs: design proposal (2026-10-13)

**Status: proposal, not approved.** Written at the end of the Phase 2c session for the author to review;
nothing here is implemented. Once approved (possibly with changes), the accepted parts move into
[02 — Architecture](../02-architecture.md), [03 §6–7](../03-data-model.md#6-results) and new ADRs, and the
plan's Phase 3 tasks are split along §11.

Goal of Phase 3 ([plan](../05-implementation-plan.md#phase-3--server-and-jobs)): simulation requests over
HTTP with results streamed in real time. **Acceptance:** a test client submits the cylinder and receives
frames in real time; cancelling works in < 1 s.

## 1. Overview

```
                         ┌──────────────── cfd-server process ─────────────────────────────────┐
 browser / test client   │ tokio runtime (async)                       dedicated threads       │
 ─────────────────────   │ ┌──────────────────────┐                   ┌──────────────────────┐ │
 REST (JSON) ───────────▶│ │ axum routes          │  Command (mpsc)   │ worker thread (job)  │ │
                         │ │  /api/simulations …  │──────────────────▶│  Box<dyn Solver>     │ │
 WS /stream ◀───────────▶│ │ JobManager (State)   │                   │  command loop        │ │
   text: progress JSON   │ │  queue + device      │  Event (broadcast)│  step / sample / …   │ │
   binary: LFFR frames   │ │  semaphores          │◀──────────────────│                      │ │
                         │ │ WS tasks (1/viewer)  │  latest frame     ├──────────────────────┤ │
                         │ │                      │◀───── (watch) ────│ output thread (job)  │ │
                         │ └──────────┬───────────┘                   │  encode, write disk  │ │
                         │            │ sqlx                          └──────────┬───────────┘ │
                         │       SQLite (jobs, scenes)            ResultStore (results/{id}/) │
                         └─────────────────────────────────────────────────────────────────────┘
```

- **One process** (ADR-006, ADR-010): axum on tokio for I/O; every running job has its own OS thread for the
  solver and one for output, so the solver never blocks the async runtime and GPU contexts stay on their
  thread. *(C#: the axum side is ASP.NET Core with async controllers; the worker is a dedicated `Thread`, not a
  `Task` on the thread pool, because it runs for minutes and owns a GPU context.)*
- **Local-first:** binds `127.0.0.1:7878` by default; no authentication (ADR-010); limits still enforced
  (§9) because the browser sends whatever the user draws.

## 2. Crates: share the run loop and backend selection with the CLI

Today the CLI owns two things the server needs: backend selection (`cfd-cli/src/backend.rs`) and the run loop
that writes frames, CSV series and `meta.json` (`cfd-cli/src/run.rs`). Copying them would give two result
layouts that drift. Proposal:

| Crate | Content | Used by |
|-------|---------|---------|
| **`cfd-engines`** (new, small) | The engine inventory (`Engines::probe()` opens CUDA, wgpu adapters, CPU once per process and keeps the `Arc` handles), `EngineKind`, `SolverChoice::make`, the `auto` order CUDA > wgpu > CPU, device descriptions and memory for `/api/capabilities`. No clap. | cli, server, verify tests |
| **`cfd-job`** (new) | The **job runner**: `Scene → Domain`, the command loop (§4), sampling and frame encoding, the result layout of [03 §6.2](../03-data-model.md#62-storage) behind `ResultStore` (§7), `meta.json`. Events go to a `JobObserver` trait. | cli `run` (no commands, a console observer), server (channels) |
| `cfd-server` | axum routes, `JobManager`, WebSocket streaming, SQLite. | binary `cfd-server` |
| `cfd-cli` | Argument parsing; `run` becomes a thin wrapper over `cfd-job`. | — |

So `cfd-cli run` and a server job produce **byte-identical result directories** (a test runs both on the same
scene), and `cfd-cli render` works on server results. *(C#: two class libraries referenced by the console app
and the web app.)*

## 3. HTTP API (axum)

Routes refine [03 §7](../03-data-model.md#7-api-draft); all JSON is camelCase like the scene format.

| Method | Route | Body / query | Response |
|--------|-------|--------------|----------|
| `GET` | `/api/capabilities` | — | engines, devices, limits, defaults (§8) |
| `POST` | `/api/scenes/validate` | `Scene` | `{ ok, issues: [{ code, severity, message, params }], estimate }` |
| `POST` | `/api/simulations` | `{ scene, options? }` | `201 { id, status, backend, device, estimate, warnings }`; `422` with issues |
| `GET` | `/api/simulations` | `?status=` | list (from SQLite) |
| `GET` | `/api/simulations/{id}` | — | status, progress, last diagnostics, timings |
| `POST` | `/api/simulations/{id}/pause` · `/resume` | — | `202` (applied between batches) |
| `DELETE` | `/api/simulations/{id}` | `?purge=true` | cancel; with `purge`, also delete the results |
| `WS` | `/api/simulations/{id}/stream` | `?fields=velocity,vorticity&maxSize=1024` | §5 |
| `GET` | `/api/simulations/{id}/frames/{n}` | `?dtype=f32` | one stored frame (LFFR) |
| `GET` | `/api/simulations/{id}/files/{name}` | `scene.json`, `meta.json`, `probes.csv`, `forces.csv`, `diagnostics.csv` | the file |
| `GET/POST/DELETE` | `/api/scenes`, `/api/scenes/{id}` | `Scene` | saved scenes (SQLite) |
| `GET` | `/*` | — | the built frontend (`apps/web/dist`, embedded with `rust-embed`; ADR-010, can wait) |

- `options`: `backend` (`auto|cpu|wgpu|cuda`, default: the scene's `run.backend`), `latticeVelocity`,
  `endTime`, `outputInterval`, `sampleInterval` — the same overrides as `cfd-cli run`.
- **Validation** is `cfd_core::validate` plus `Domain::from_scene`'s stability checks; issues keep their stable
  codes (`stability.tau`, `domain.noPressureReference`, …) and gain `params` (numbers as data) so the browser
  can localise messages. The estimate: cells, fluid cells, steps, memory per engine (B/cell × cells), and a time
  estimate from the engine's benchmarked MLUPS.
- **Layers:** `tower-http` — `TraceLayer` (`tracing` spans per request), `CorsLayer` (the Vite dev origin only),
  `RequestBodyLimitLayer` (64 MB: a 4096² scene with `raw+base64` layers is up to ≈ 90 MB, but the scene
  format already offers `zstd+base64` layers, typically > 50× smaller for drawn layers — open question Q6),
  `CompressionLayer` for JSON responses.
  *(C#: middleware.)*
- **Errors:** one `ApiError` enum (`thiserror`) mapped to status codes with `IntoResponse`; `anyhow` stays inside
  handlers' internals. *(C#: exception filters → ProblemDetails.)*
- axum *extractors* (`Path<JobId>`, `Json<Submit>`, `State<AppState>`) are the model binding; `AppState` is an
  `Arc` cloned into every handler. *(C#: a DI singleton.)*

## 4. Jobs: manager, devices, worker command loop

### 4.1 States

```
queued ──▶ running ⇄ paused
   │          │  ╲
   ▼          ▼   ╲──▶ failed (diverged | error | interrupted)
cancelled ◀───┘        completed
```

`diverged` keeps `cfd-cli run`'s semantics (last valid frame kept, message with hints). `interrupted`: the server
stopped while the job ran (on start-up, SQLite rows still `running` become `failed/interrupted`; no automatic
restart in Phase 3).

### 4.2 JobManager and per-device semaphores

- `JobManager` (in `AppState`) holds `HashMap<JobId, JobHandle>` behind a `std::sync::Mutex` (short critical
  sections, never held across `.await`). A `JobHandle` has the command sender, the event channels (§5), the
  status and the progress snapshot.
- **One semaphore per physical device**, not per engine: CUDA device 0 and the wgpu adapter for the same RTX 4060
  share one permit (matched by PCI bus id when both report it, else by name). The CPU gets its own semaphore
  with 1 permit by default (rayon's global pool is shared: two CPU jobs would only halve each other).
  *(C#: `SemaphoreSlim`.)*
- Submission: validate → choose engine (§8; `auto` takes the best engine whose device has **memory** for the
  domain, falling back like the CLI) → insert the SQLite row → spawn a tokio task that awaits
  `semaphore.acquire_owned()` (FIFO), then starts the worker thread and moves the permit into it — the permit is
  released when the thread ends, whatever the outcome (RAII; *C#: `using`*).
- A job that cannot fit any device is rejected at submission (`422`), never queued forever.

### 4.3 The worker thread: a command loop (ADR-011)

```rust
enum Command { Pause, Resume, Cancel, Snapshot /* frame now, for a new viewer */,
               PatchCells(Patch) /* 5b */, SetParam(Param) /* 5b */ }

loop {
    // 1. Drain commands without blocking (`try_recv`); while paused, block on `recv` instead.
    // 2. Due work: diagnostics/probes/forces every sample interval (→ CSV + progress event),
    //    a frame every output interval (request_fields / device sampling → output thread).
    // 3. Step a batch: up to the next due event, and at most ~50 ms of work
    //    (batch = measured steps/s × 0.05) — this bounds the latency of Pause/Cancel.
    // 4. On divergence: write the last frame and meta.json, report `failed/diverged`.
}
```

- It is `cfd-cli run`'s loop with step 1 added and step 3 capped in time, which is why it moves into
  `cfd-job` (§2). **Cancel latency** = one batch (≤ 50 ms) + `finish()` (≤ 2 GPU batches in flight), far below
  the 1 s acceptance.
- Commands use `std::sync::mpsc` (the worker is not async; sending from a handler never blocks).
  *(C#: `BlockingCollection<Command>`.)*
- The solver is created **inside** the worker thread (CUDA contexts are bound to a thread; `Solver` is `Send`
  but not `Sync`, so it is never shared — only messages cross threads).
- `PatchCells`/`SetParam` are in the enum from day one but answered with "not supported" until Phase 5b.

### 4.4 The output thread

Frame conversion, encoding (zstd) and disk writes run on a second thread fed by a **bounded** channel
(`sync_channel(2)`): the solver keeps stepping while a frame is encoded, and if the disk cannot keep up the
solver waits — **stored frames are never dropped**. Only streaming drops frames (§5).

## 5. WebSocket streaming and backpressure

- **Messages** (server → client): text JSON `{"type":"hello"|"status"|"progress"|"log"|"done", …}` and binary
  LFFR v1 frames (f16, optionally downsampled; `jobId`/`frameIndex`/`step` in the frame header).
  `progress` at most 10 Hz: step, simulated time, %, MLUPS, mass drift, max Mach.
- **Two channels per job, with different loss rules** *(C#: Rx subjects)*:
  - frames → `tokio::sync::watch` (holds only the **latest** encoded stream frame; *≈ `BehaviorSubject`*).
    Each viewer's task sends the latest frame, awaits the socket, then waits for the next change: a slow
    client simply skips intermediate frames, a fast one gets all of them. The worker never waits for viewers.
  - events → `tokio::sync::broadcast` (capacity 64; a lagging viewer gets `Lagged` and resynchronises from
    the job's status snapshot). Terminal events (`done`) are also in the snapshot, so they are never missed.
- **Late joiners** get `hello` (scene summary, units, field ranges so far) and the latest frame at once.
- **Per-viewer options** (`fields`, `maxSize`) are applied by the encoder: the job encodes one stream frame per
  distinct option set actually requested (usually one).
- Client → server text messages are reserved for 5b (`patch`, `setParam`); REST covers pause/resume/cancel.

## 6. On-device sampling and f16 (moved from Phase 2a)

Today a frame copies ρ, ux, uy, θ (16 B/cell) and converts them on the CPU (`FieldSet::from_lattice`:
pressure with the hydrostatic part, vorticity, temperature), then to f16. At 2048² that is 64 MB over PCIe and
tens of milliseconds of single-threaded CPU work per frame, more than a frame interval at interactive rates.

- New kernels (WGSL + CUDA, generated constants from `cfd_lbm::abi`): compute the **physical output fields** on
  the device — velocity (2 components), pressure incl. hydrostatic, vorticity (the same one-sided differences at
  boundaries), temperature — optionally **box-downsampled** by an integer factor, written as **f16** (stream) or
  f32 (stored frames), packed per field. Copied with the existing asynchronous readback (ADR-020/021).
- `Solver` gains `request_sample(&SampleSpec) -> Box<dyn PendingSample>` with a default implementation (CPU
  path: lattice fields → `FieldSet::from_lattice` → convert), so the CPU backend and the trait's other users are
  unchanged.
- **Parity:** f32 output fields from CUDA must equal the CPU's `FieldSet::from_lattice` bit for bit
  (`--fmad=false`, same operation order); f16 conversion is round-to-nearest-even on both sides (`half` on the
  CPU, `__float2half_rn` in CUDA) → bit-identical. For WGSL's `pack2x16float` the rounding mode still has to
  be checked against the spec and the drivers; until then wgpu is held to ≤ 1 f16 ulp. New cases in
  `cfd_verify::parity`.

## 7. Macroscopic fields only when needed

The fused kernel stores ρ, ux, uy (θ) every step; nothing reads them except at output, diagnostics and probes —
and, inside the kernel, **only the pressure outlet**, which reads its own cell's velocity from the previous
step (checked in `lbm.wgsl`: no other boundary rule or thermal path reads the planes; θ comes from the
populations).

- Rule: `step(n)` runs steps `1…n−1` with the `skip_macro` variant and step `n` with `write_macro`. Every read
  of fields (`diagnostics`, `request_fields`, `lattice_fields`, probes, sampling) happens between `step` calls, so
  it always sees fresh values — the trait does not change.
- `skip_macro` still stores u for cells that have an anti-bounce-back (pressure) link (flag already in the
  link mask: a few columns), and still writes the divergence status word.
- CUDA keeps one captured graph per variant (50 skipping steps; the last step of a call launched with the
  writing variant). wgpu: a pipeline-overridable constant, as for `THERMAL`. The CPU kernel takes the same flag
  (no gain there, identical results: all three backends stay the same code path).
- **Parity:** results are bit-identical to always writing (only stores are skipped): a new parity case runs
  both and compares with `==`; then ≈ 72 instead of 84 B per update, ≈ +15% MLUPS at ≥ 1024²
  (`bench --backends cuda,wgpu` before/after, quiet desktop).

## 8. Capabilities and device inventory

`Engines::probe()` at start-up (≈ 0.5 s: NVRTC compile, adapter enumeration) and kept for the process:

```json
{ "version": "0.2.0", "methods": ["lbm"],
  "engines": [
    { "kind": "cuda", "available": true, "device": "NVIDIA GeForce RTX 4060", "memoryBytes": 8589934592,
      "computeCapability": "8.9", "driver": "13.4", "physicalDevice": "gpu0" },
    { "kind": "wgpu", "available": true, "device": "NVIDIA GeForce RTX 4060 (Vulkan, DiscreteGpu)",
      "maxBufferBytes": 2147483648, "physicalDevice": "gpu0" },
    { "kind": "cpu", "available": true, "device": "AMD Ryzen 7 3700X", "threads": 16 } ],
  "auto": "cuda",
  "limits": { "maxCells": 16777216, "maxSteps": 100000000, "maxStoredBytes": 10737418240,
              "maxQueuedJobs": 16, "maxBodyBytes": 67108864 },
  "defaults": { "streamDtype": "f16", "streamMaxSize": 1024 } }
```

Unavailable engines carry `reason` (the same messages as `--backend auto`'s notes). Phase 8a will add the
bandwidth figure used to suggest a storage mode.

## 9. ResultStore and SQLite

- `trait ResultStore` (in `cfd-job`): create a job directory, write/read frames and named files, list, delete.
  `LocalStore` writes `results/{job_id}/` in exactly the 03 §6.2 layout. S3/MinIO later (ADR-010 cross-cutting).
- **Stored frames:** f32 at full resolution every `outputInterval` (ADR-007), zstd-compressed; a per-job
  `maxStoredBytes` limit stops storing (not the job) with a warning in `meta.json`. 1024² × 4 values × 4 B ≈
  16 MB/frame before compression, so long runs need either a coarser `outputInterval` or f16 storage —
  open question Q4.
- **SQLite with `sqlx`** (runtime-checked queries, so no database is needed at build time; embedded migrations
  with `sqlx::migrate!`; the bundled SQLite builds on Windows/MSVC and in CI): tables `scenes(id, name, json,
  created_at, updated_at)` and `jobs(id, scene_id?, scene_sha256, status, reason, backend, device, options_json,
  steps_done, steps_total, created_at, started_at, finished_at, result_dir)`. Frames never go into SQLite.
  `rusqlite` (synchronous, simpler) is the alternative — open question Q3.

## 10. Testing

- **Unit:** state machine transitions; `auto` engine choice with memory limits (fake inventory, as
  `pick_auto` is tested now); batch sizing; semaphore keying by physical device.
- **`cfd-job`:** `cfd-cli run` output unchanged (the existing end-to-end tests), and server-job vs CLI result
  directories byte-identical on the small channel.
- **Integration** (`cfd-server/tests`, tokio + `reqwest` + `tokio-tungstenite`, server on an ephemeral port,
  temporary results directory and in-memory SQLite): submit → `hello` → ≥ N frames → `done/completed`;
  **cancel in < 1 s** (measured); pause → no progress for 0.5 s → resume; a deliberately slow viewer skips
  frames while the disk has all of them; two viewers; a diverging scene ends `failed/diverged` with its last
  frame; `422` with issue codes for an invalid scene; capabilities without a GPU. GPU variants skip like the
  other tests (`CFD_REQUIRE_GPU`/`CFD_REQUIRE_CUDA`).
- **Parity:** on-device sampling (§6) and `skip_macro` (§7) as new `cfd_verify::parity` cases on both GPU
  backends.
- **Acceptance client:** `cfd-cli submit scene.json --server http://127.0.0.1:7878 [--cancel-after 2s]` prints
  frames/s and latency — a "test client (script)" that needs no extra language and doubles as a debugging tool.

## 11. Proposed order (each step tested before the next)

1. `cfd-engines` and `cfd-job` extracted from the CLI, with no behaviour change (all current tests pass;
   result directories identical).
2. Command loop with time-capped batches, `JobObserver`, output thread; `cfd-cli run` on top of it.
3. `cfd-server` skeleton: config, tracing, `/api/capabilities`, `/api/scenes/validate`, limits, CORS.
4. `JobManager`, per-device semaphores, submit/status/pause/resume/cancel, SQLite, `LocalStore`.
5. WebSocket streaming with `watch`/`broadcast`; `cfd-cli submit`; integration tests; **acceptance**.
6. On-device sampling + f16 kernels (CUDA, WGSL), parity cases, wired into the stream.
7. `skip_macro` variant on all three backends, parity case, benchmark before/after.
8. Docs (02, 03 §6–7, ADRs for the crate split, job model and streaming), plan checkboxes.

Steps 6–7 are independent of 3–5 and could also come first if the author prefers kernel work.

## 12. Open questions for the author

| # | Question | Proposal |
|---|----------|----------|
| Q1 | New crates `cfd-engines` and `cfd-job` (§2), or keep everything in `cfd-server` and leave the CLI as it is? | New crates: one result layout, one loop, tested once |
| Q2 | CUDA and wgpu on the same GPU share one job slot? CPU jobs: 1 at a time? | Yes / yes (configurable `--cpu-jobs`) |
| Q3 | `sqlx` (async, Postgres later) or `rusqlite` (sync, simpler)? | `sqlx` with runtime queries |
| Q4 | Stored frames: f32 at every output interval (ADR-007), or f16 by default with f32 on request? | f32, with a per-job size limit; revisit after measuring |
| Q5 | Restart behaviour: mark running jobs `interrupted`, or resume them (needs checkpoints of the populations)? | `interrupted` now; checkpoints later (they are also useful for 5b) |
| Q6 | Large scenes: the browser sends `zstd+base64` layers (already in the scene format) and the server keeps a 64 MB body limit, or the limit is raised for `raw+base64`? | `zstd+base64` from the editor; 64 MB limit; a clear `413` message naming the encoding |
| Q7 | Default port and bind address | `127.0.0.1:7878`; `--listen` to change it |
| Q8 | Order: server first (§11 steps 1–5) or the two kernel items first (6–7)? | Server first: the acceptance criterion is about streaming |
