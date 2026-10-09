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
│  Output: field sampling → encoding (f16 + zstd) → stream/disk             │
└───────────────────────────────────────────────────────────────────────────┘
```

Initially **server and worker run in the same process** (tokio tasks + a dedicated solver thread). Splitting into separate processes/machines only happens when needed (several GPUs, several users).

## 2. Components

### 2.1 Frontend — `apps/web`
- **Stack:** TypeScript + Vite + React, Zustand, Radix/shadcn + Tailwind, uPlot; Vitest + Playwright; Three.js/react-three-fiber for the 3D stage (see [ADR-005](07-decisions-and-questions.md)).
- **Editor:** Canvas2D with layers stored as `Uint8Array`/`Uint16Array` (one per logical layer). Zoom/pan, grid lines above a certain zoom level, drawing with Bresenham / scanline fill.
- **State:** a simple store (Zustand) with history for undo/redo (diffs per affected rectangle).
- **Viewer:** WebGL2 (universal fallback) with colour-map shaders; WebGPU when available for particles and streamlines.
- **Communication:** REST for CRUD, binary WebSocket for frames.
- **Optional, later:** the solver core compiled to WASM/WebGPU for low-resolution local previews.

### 2.2 API — `crates/cfd-server`
- **Stack:** Rust, `axum`, `tokio`, `serde`, `tower-http` (CORS, compression, limits), `tracing`.
- **Responsibilities:** authentication (later), request validation, job management, streaming, serving results.
- **Persistence:** `sqlx` with SQLite at first, Postgres if ever hosted. Results stored as files (`ResultStore` abstraction → local disk / S3).

### 2.3 Queue and workers
- **MVP:** in-memory queue (`tokio::sync::mpsc`) + one semaphore per device (1 job per GPU at a time, N CPU jobs).
- **Scaling:** Redis Streams or NATS JetStream; stateless workers advertise their capabilities (GPU, memory, supported backends).
- **Scheduling:** backend chosen from the estimated memory requirement (`cells × bytes per cell`) and availability.

### 2.4 Simulation core — `cfd-core` and solvers
- `cfd-core`: shared types (Scene, Domain, Field, units, physical ↔ lattice conversion, materials, `Solver` and `Backend` traits).
- `cfd-lbm`: LBM D2Q9 (+ D2Q5 thermal), CPU kernels.
- `cfd-gpu`: `wgpu` setup, buffer management, compute pipelines, WGSL shaders.
- `cfd-cuda`: CUDA backend (`cudarc` + NVRTC kernels) behind the `cuda` feature.
- Common interface:

```rust
pub trait Solver {
    fn init(domain: &Domain, cfg: &SolverConfig, backend: BackendKind) -> Result<Self> where Self: Sized;
    fn step(&mut self, n: u32) -> Result<()>;                              // advance n steps
    fn sample(&mut self, req: &SampleRequest) -> Result<FieldSet>;         // copy requested fields to the CPU
    fn update_boundaries(&mut self, patch: &BoundaryPatch) -> Result<()>;  // interactive mode
    fn diagnostics(&self) -> Diagnostics;                                  // total mass, energy, max velocity, NaN check
}
```

### 2.5 CLI — `crates/cfd-cli`
Runs scenes from files without a server: essential for development, benchmarks and validation tests (`cfd-cli run scene.json --backend cuda --out out/`).

## 3. Request flow

1. The user draws and configures → the frontend serialises the `Scene` (see [03](03-data-model.md)).
2. `POST /api/simulations` → the server validates schema, limits and physical consistency; estimates memory and cost.
3. Job created (`queued`) → returns `{ id }`.
4. The client opens `WS /api/simulations/{id}/stream`.
5. Worker: `Scene → Domain` (rasterisation, lattice units, stability check) → `Solver::init` → `step` / `sample` / `encode` / `publish` loop.
6. Every frame is sent over the WebSocket and written to the `ResultStore`.
7. End (`completed` / `failed` / `cancelled`) → summary with diagnostics.

## 4. Technology stack

| Layer | Choice | Alternatives considered |
|-------|--------|-------------------------|
| Server/solver language | **Rust** | C++ (more CFD libraries, less safety), Julia (great for prototyping, harder to deploy) |
| GPU | **wgpu + WGSL** (Vulkan/DX12/Metal) **and CUDA via `cudarc`** (NVIDIA, `cuda` feature) — both from Phase 2 | rust-gpu, HIP (AMD) |
| CPU parallelism | **rayon** + `std::simd`/`wide` | OpenMP via C++ |
| HTTP/WS | **axum** | actix-web |
| Serialisation | **serde** (JSON) + custom binary / MessagePack | Protobuf/FlatBuffers |
| Compression | **zstd** (pure-Rust `ruzstd`) | lz4 |
| Database | SQLite → Postgres (`sqlx`) | — |
| Frontend | **TypeScript + Vite + React** | Svelte, SolidJS |
| Result rendering | WebGL2 → WebGPU | Canvas2D (slow) |
| AI (inference) | `ort` (ONNX Runtime) or `burn`/`candle` | Separate Python service |
| AI (training) | Python + PyTorch (offline) | JAX |

## 5. Repository layout (monorepo)

```
live-fluids/
├── README.md, CLAUDE.md, docs/
├── Cargo.toml                 # Rust workspace
├── crates/
│   ├── cfd-core/              # scene format, validation, units, Scene→Domain, traits
│   ├── cfd-lbm/               # LBM solver (CPU) + shared logic
│   ├── cfd-gpu/               # wgpu infrastructure + WGSL shaders
│   ├── cfd-cuda/              # CUDA backend (cudarc + .cu kernels), `cuda` feature
│   ├── cfd-io/                # result formats, frame encoding
│   ├── cfd-server/            # axum API + jobs + streaming
│   └── cfd-cli/               # run/benchmark/validation CLI
├── apps/
│   └── web/                   # TS frontend
├── schema/                    # Scene JSON Schema (generated)
├── scenes/                    # example and validation scenes (generated)
├── validation/                # reference data (Ghia, Schäfer-Turek, ...) and scripts
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
