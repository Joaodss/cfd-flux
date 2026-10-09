# 01 — Vision and requirements

## 1. Vision

A web application where anyone (student, engineer, curious mind) can **sketch a flow problem in minutes** and get a physically credible simulation computed on a GPU server. The pixel grid is both the drawing interface and the computational grid, which removes the mesh-generation step from the first version.

In the long run the project should become a **testbed for numerical methods**: several solvers behind a common interface, comparable on the same scenario, with modern optimizations (adaptive meshes, multi-GPU, AI).

**Context (2026-10-09):** a portfolio and personal exploration project, **open source and public**, focused on education and quick simulation examples. The MVP covers **gases and liquids** in 2D; then come interactive mode, granular media, plasma and all phase changes (including granular → liquid); **3D is Stage 2**, after 2D is complete and validated. Development GPU: NVIDIA GeForce RTX 4060, 8 GB. License: MIT OR Apache-2.0.

## 2. Goals

1. **Low barrier to entry:** draw → configure → simulate, with no meshing knowledge required.
2. **Performance:** 2D simulations at 512×512 in near real time on a consumer GPU; 2048×2048 in batch mode.
3. **Verifiable accuracy:** every solver validated against published reference cases (see [04](04-numerical-methods.md#7-validation)).
4. **Extensibility:** new methods, boundary types and fluids without changing the frontend or the protocol.
5. **Portability:** GPUs from any vendor (Vulkan/Metal/DX12) and an automatic CPU fallback.

## 3. Non-goals (for now)

- Imported CAD geometry (STEP/IGES). Possible later by rasterising onto the grid.
- 3D in the first stage — the data format is designed to allow it (voxels), but the MVP is 2D.
- Certification for industrial use.
- Multi-tenant hosting at scale with billing.

## 4. Users and use cases

| Persona | Typical use case |
|---------|------------------|
| Student | See how the Reynolds number changes the flow around a cylinder (von Kármán vortex street) |
| Engineer / maker | Assess the cooling of an enclosure with a fan and hot components |
| Researcher / developer | Compare LBM vs. projection on the same scenario; test an AI surrogate model |
| Curious user | Play with smoke, falling water, natural convection |

## 5. Functional requirements

### 5.1 Editor (frontend)
- FR-01 Create a domain of W×H pixels/cells with a physical scale (metres per pixel).
- FR-02 Drawing tools: pencil, eraser, line, rectangle, ellipse, bucket (flood fill), select/move, image import (PNG → mask).
- FR-03 Palette of **cell types**: fluid, solid/wall, inlet, outlet, heat source, initial fluid region (e.g. water), probe (measurement point).
- FR-04 Properties panel per **boundary element** (each region drawn with the same ID): velocity condition (no-slip, slip, imposed velocity), thermal condition (fixed temperature, heat flux, adiabatic), solid material (conductivity, if conduction in the solid is modelled).
- FR-05 Define fluids: choose from a library (air, water, oil…) or set custom properties (density, viscosity, conductivity, heat capacity, expansion coefficient).
- FR-06 Global parameters: gravity, total physical time, output interval, numerical method, precision (f32/f16/f64), backend preference (auto/GPU/CPU).
- FR-07 Client-side validation before submitting (e.g. a domain with an inlet but no outlet → warning; estimated Mach/stability).
- FR-08 Save/load scenes (locally and on the server), undo/redo, built-in examples.

### 5.2 Simulation (server)
- FR-10 Accept simulation requests, validate them, queue them and return a job ID.
- FR-11 Pick the backend automatically (GPU if available, otherwise CPU).
- FR-12 Stream progress and result frames (WebSocket).
- FR-13 Cancel, pause and resume jobs.
- FR-14 Store results (fields and probe time series) for playback and download.
- FR-15 (Future) Interactive mode: change parameters/geometry while the simulation is running.

### 5.3 Visualisation
- FR-20 Colour maps of fields: velocity (magnitude and components), pressure, temperature, density, phase fraction, vorticity.
- FR-21 Streamlines, vector glyphs, tracer particles, smoke/dye.
- FR-22 Time playback (play/pause/scrub), adjustable colour scale, legends.
- FR-23 Probe plots over time; statistics (drag/lift forces, Strouhal number, Nusselt number).
- FR-24 Export: PNG/MP4/GIF, probe CSV, fields in binary/VTK format.

## 6. Non-functional requirements

| ID | Requirement | Initial target |
|----|-------------|----------------|
| NFR-01 | GPU performance (LBM D2Q9, f32) | ≥ 2,000 MLUPS on the RTX 4060 (wgpu and CUDA). Bandwidth 272 GB/s ÷ 72 B/cell/step (9 reads + 9 writes in f32) ⇒ theoretical ceiling ~3,700 MLUPS |
| NFR-02 | CPU performance | ≥ 50 MLUPS on 8 cores |
| NFR-03 | Latency to first frame | < 2 s for 512×512 |
| NFR-04 | Maximum domain size (MVP) | 4096×4096 (~1.4 GB in f32; fits in 8 GB of VRAM with room for thermal and free-surface data) |
| NFR-05 | Reproducibility | Same request + same solver version ⇒ deterministic results on the CPU backend; GPU vs CPU difference below a defined tolerance |
| NFR-06 | Security | Strict input validation, per-job size/time limits, no execution of user code |
| NFR-07 | Observability | Structured logs, metrics (jobs/s, MLUPS, GPU usage), tracing |
| NFR-08 | Portability | Windows, Linux; NVIDIA/AMD/Intel/Apple GPUs via wgpu |

*MLUPS = million lattice-cell updates per second (the standard LBM performance metric).*

## 7. Glossary

- **Cell / pixel:** unit of the computational grid; in the MVP 1 pixel = 1 cell.
- **Boundary element:** set of cells with the same type and element ID, sharing properties.
- **Backend:** concrete implementation of a solver for one kind of hardware (GPU/CPU).
- **Job:** a simulation request that is running or queued.
- **Frame:** snapshot of the fields at one output instant.
