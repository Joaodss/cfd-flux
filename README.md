# cfd-flux

A computational fluid dynamics (CFD) simulator in the browser. You draw the domain **pixel by pixel**, place walls, fluid inlets and outlets, temperatures and boundary properties, and the simulation runs on a local Rust server on the GPU (CUDA or wgpu), with the CPU as a fallback. Results are streamed back and visualised in the browser.

> Status: **Phase 2 complete** — a validated LBM solver (D2Q9 + D2Q5 thermal, TRT, link-wise boundaries, Boussinesq, forces) on three engines: the CPU, wgpu/WGSL and CUDA, with a headless CLI (`run`, `render`, `bench`, `verify`, `compare`). It reproduces Ghia (1982), Schäfer & Turek (1996), de Vahl Davis (1983) and the Rayleigh-Bénard onset within 2% on every backend — see the validation reports ([CPU](validation/report/README.md), [wgpu](validation/report-wgpu/README.md), [CUDA](validation/report-cuda/README.md)) — and the [engine comparison](validation/compare/README.md) shows CUDA bit-identical to the CPU on every validation geometry and wgpu within 1e-5. ~2,870 MLUPS on an RTX 4060 for both GPU engines and 65–180 MLUPS on a desktop CPU ([benchmarks](docs/benchmarks.md)). **Phase 3 (server and jobs): acceptance met** — `cfd-server` (axum + SQLite) queues jobs on one slot per GPU, streams frames over WebSocket with backpressure, and pauses or cancels between batches of at most 50 ms (a cancel takes effect in 30–46 ms); `cfd-cli submit` is its test client. **Phase 3 complete (2026-10-16):** frames are sampled on the GPU (physical fields, box filter, f16; CUDA bit-identical to the CPU conversion), stored and streamed as LFFR v2 (byte shuffle + lz4, 24× faster than zstd and 30% smaller), and the kernels skip the macroscopic stores on steps nobody reads (+16% MLUPS at ≥ 1024²: ~3,340 on the RTX 4060). **Phase 4 complete (2026-10-19):** the web editor — draw on a WebGL2 canvas (pencil, line, rectangle, ellipse, fill, select/move, PNG import), give walls, inlets, outlets and heat sources their conditions, pick fluids and materials from a library, save `.cfdscene` files or on the server, see the server's validation (issues marked on the cells, Re/Ma/τ estimates) and run the scene, following its progress live. **Phase 5 complete (2026-10-22):** the viewer — the job view plays a run live and from the stored frames (timeline, scrub, speed), colours any field (velocity, pressure, vorticity, temperature), draws arrows, streamlines, tracer particles and dye, plots the force coefficients and probes, reads St, C_D and Nu over a window against the benchmarks, and exports a PNG, CSV and video. The Cylinder Re=100 example on the GPU reads St 0.2973 (−0.9 %) and C_D,max 3.375 (+4.5 %) in the Statistics tab ([ADR-031](docs/07-decisions-and-questions.md)); the server serves the app itself.

## The idea in 30 seconds

```
 Browser (pixel editor)  ──HTTP/JSON+binary──▶  cfd-server (Rust/axum)
        ▲                                            │
        │ WebSocket (result frames)                  ▼
        └───────────────────────────────────  Jobs ──▶ Solver
                                                       ├─ CUDA (NVIDIA)
                                                       ├─ wgpu / WGSL (any GPU)
                                                       └─ CPU (rayon) — numerical reference
```

- **Gases and liquids** in the MVP (2D); later granular media, plasma and phase changes; **3D** as a second stage.
- **First method:** Lattice Boltzmann (D2Q9 + D2Q5 thermal, free surface) — maps naturally onto a pixel grid and runs very efficiently on GPUs.
- **Quantitative validation** against published benchmarks (Ghia, Schäfer-Turek, de Vahl Davis, Martin-Moyce).

## Getting started

```bash
cargo test --workspace                          # Rust tests
cargo run -p cfd-cli -- example --list          # example scenes
cargo run -p cfd-cli -- validate scenes/*.json  # validate scenes
cargo run --release -p cfd-cli -- run scenes/cylinder-re100.json --out out/cyl --end-time 12   # frames + PNGs
cargo run --release -p cfd-cli -- verify --quick   # validation suite (CI subset)
cargo run --release -p cfd-cli -- bench --backends cuda,wgpu   # MLUPS, one column per engine
cargo run --release -p cfd-cli -- compare scenes/channel.json --out out/cmp   # same scene on every engine
cd apps/web && npm install && npm run dev       # frontend (development server)
(cd apps/web && npm ci && npm run build) && cargo run --release -p cfd-server -- --web-dir apps/web/dist --open   # the app from the server
```

Requirements and known issues (Windows, CUDA): [docs/development.md](docs/development.md).

## Layout

```
crates/
  cfd-core/    Scene format, layer encoding, validation, examples
  cfd-io/      binary format of result frames, colour maps and PNG
  cfd-cli/     CLI: schema, example, validate, run, render, bench, verify, compare, submit
  cfd-lbm/     LBM solver on the CPU          (Phase 1)
  cfd-gpu/     wgpu/WGSL backend              (Phase 2a, parity-tested against cfd-lbm)
  cfd-cuda/    CUDA backend, `cuda` feature   (Phase 2b, bit-identical to cfd-lbm)
  cfd-verify/  backend-agnostic validation suite, parity cases and engine comparison
  cfd-engines/ engine selection: auto (CUDA > wgpu > CPU), device inventory, making solvers
  cfd-job/     job runner: command loop, output thread, result layout (shared by CLI and server)
  cfd-server/  HTTP/WebSocket API, jobs, streaming, SQLite (Phase 3)
apps/web/      React + TypeScript + Vite frontend
schema/        JSON Schema of the scene (generated from Rust)
scenes/        example scenes (generated)
validation/    reference data (with sources) and the validation report
sandbox/       learning prototypes (outside the workspace)
docs/          design and planning documentation
```

## Documentation

| Document | Contents |
|----------|----------|
| [01 — Vision and requirements](docs/01-vision-and-requirements.md) | Goals, scope, users, functional and non-functional requirements |
| [02 — Architecture](docs/02-architecture.md) | Components, request flow, technology stack, repository layout |
| [03 — Data model](docs/03-data-model.md) | Scene format, pixel layers, boundary types, materials, result format, API |
| [04 — Numerical methods](docs/04-numerical-methods.md) | LBM, thermal, projection, liquids, compressible, stability, validation cases |
| [05 — Implementation plan](docs/05-implementation-plan.md) | Phases, tasks, acceptance criteria, MVP |
| [06 — Optimizations and AI](docs/06-optimizations-and-ai.md) | AMR, multigrid, multi-GPU, modern meshes, AI surrogate models |
| [07 — Decisions and questions](docs/07-decisions-and-questions.md) | ADRs and open questions |
| [08 — Validation guide](docs/08-validation-guide.md) | How accuracy is measured: references, error norms, convergence order |
| [Development](docs/development.md) | Environment, commands, conventions |
| [Sessions](docs/sessions/) | Work-session checklists |

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
