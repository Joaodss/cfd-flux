# live-fluids

A computational fluid dynamics (CFD) simulator in the browser. You draw the domain **pixel by pixel**, place walls, fluid inlets and outlets, temperatures and boundary properties, and the simulation runs on a local Rust server on the GPU (CUDA or wgpu), with the CPU as a fallback. Results are streamed back and visualised in the browser.

> Status: **Phase 1 complete** — a validated CPU LBM solver (D2Q9 + D2Q5 thermal, TRT, link-wise boundaries, Boussinesq, forces) with a headless CLI (`run`, `render`, `bench`, `verify`). It reproduces Ghia (1982), Schäfer & Turek (1996) and de Vahl Davis (1983) within 2% — see the [validation report](validation/report/README.md) — at 65–180 MLUPS on a desktop CPU ([benchmarks](docs/benchmarks.md)). Next: GPU backends (wgpu and CUDA).

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
cargo run --release -p cfd-cli -- bench            # MLUPS
cd apps/web && npm install && npm run dev       # frontend
```

Requirements and known issues (Windows, CUDA): [docs/development.md](docs/development.md).

## Layout

```
crates/
  cfd-core/    Scene format, layer encoding, validation, examples
  cfd-io/      binary format of result frames, colour maps and PNG
  cfd-cli/     CLI: schema, example, validate, run, render, bench, verify
  cfd-lbm/     LBM solver on the CPU          (Phase 1)
  cfd-gpu/     wgpu/WGSL backend              (Phase 2a)
  cfd-cuda/    CUDA backend, `cuda` feature   (Phase 2b)
  cfd-verify/  backend-agnostic validation suite and report
  cfd-server/  HTTP/WebSocket API and jobs    (Phase 3)
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
