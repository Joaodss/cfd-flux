# 03 — Data model

## 1. Principles

- **Separate geometry from properties:** pixels only store small *IDs*; properties live in an element table. Changing a wall's temperature does not touch any pixel.
- **Physical units in the scene, lattice units in the solver.** Conversion happens in the solver (`cfd-core` units module), never in the frontend.
- **Versioned:** every scene has a `schemaVersion`; migrations are explicit.
- **Method-independent:** the scene describes the physical problem, not the algorithm. The method is just a parameter.

> The reference implementation of the format is `crates/cfd-core/src/scene.rs`; the generated schema is `schema/scene.schema.json`. Where this document and the code disagree, the code wins.

## 2. Scene structure (`Scene`)

```jsonc
{
  "schemaVersion": 1,
  "name": "Cylinder Re=100",
  "description": "optional",
  "grid": { "width": 800, "height": 300, "cellSize": 0.001,         // metres per pixel
            "edges": { "left": "wall", "right": "wall", "bottom": "wall", "top": "wall" } },  // or "periodic"
  "fluids": [
    { "id": 1, "name": "Air", "preset": "air",
      "density": 1.204, "kinematicViscosity": 1.5e-5,
      "thermalConductivity": 0.0257, "specificHeat": 1005, "thermalExpansion": 3.4e-3 }
  ],
  "solidMaterials": [
    { "id": 1, "name": "Aluminium", "thermalConductivity": 205, "density": 2700, "specificHeat": 900 }
  ],
  "elements": [
    { "id": 1, "kind": "wall",   "velocity": { "type": "noSlip" },
      "thermal": { "type": "adiabatic" }, "material": 1 },
    { "id": 2, "kind": "inlet",  "velocity": { "type": "uniform", "value": [0.15, 0.0] },
      "thermal": { "type": "fixed", "value": 293.15 }, "fluid": 1 },
    { "id": 3, "kind": "outlet", "pressure": { "type": "pressure", "value": 0.0 } },
    { "id": 4, "kind": "heatSource", "thermal": { "type": "fixed", "value": 350.0 }, "material": 1 }
  ],
  "probes": [ { "name": "Probe A", "position": [600, 150] } ],   // cell coordinates
  "initial": { "fluid": 1, "velocity": [0, 0], "temperature": 293.15, "pressure": 0 },
  "physics": { "gravity": [0, -9.81], "thermal": true, "buoyancy": "boussinesq", "freeSurface": false },
  "run": {
    "method": "lbm",                 // more methods added in later phases
    "backend": "auto",               // auto | cpu | wgpu | cuda
    "precision": "f32",              // f32 | f64
    "endTime": 2.0,                  // physical seconds
    "outputInterval": 0.01,
    "outputFields": ["velocity", "pressure", "temperature", "vorticity"]
  },
  "layers": { "...": "see section 3" }
}
```

## 3. Pixel layers

Each layer is a W×H array (row by row, origin at the bottom-left corner to match the physical convention; the frontend flips the Y axis when rendering): `index = y * width + x`.

| Layer | Type | Meaning |
|-------|------|---------|
| `cellType` | `u8` | 0 = fluid, 1 = solid, 2 = inlet, 3 = outlet, 4 = heat source, 5 = empty (gas/void above a free surface) |
| `elementId` | `u16` | Element ID (`elements` table); 0 = none |
| `fluidId` | `u8`, optional | Initial fluid per cell (multiphase); 0 = `initial.fluid` |
| `probes` | sparse list (outside `layers`) | Probe coordinates (no array needed) |

**Transport encoding:** each layer is sent as base64 of either raw little-endian bytes or `zstd(raw bytes)` (drawings have large uniform areas; typical compression > 50×). An indexed PNG is a possible alternative (also handy for import/export in the editor).

```jsonc
"layers": {
  "cellType":  { "encoding": "zstd+base64", "dtype": "u8",  "data": "KLUv/..." },
  "elementId": { "encoding": "zstd+base64", "dtype": "u16", "data": "KLUv/..." }
}
```

Consistency rules (validated on the client **and** the server; implemented in `cfd-core::validate`):
- Solid/inlet/outlet/heat-source cells require a valid `elementId` of the matching `kind`; fluid and empty cells must have none.
- Inlets and outlets must touch at least one fluid cell.
- `empty` cells are only allowed when `physics.freeSurface` is on.
- Domain edges where nothing is drawn behave as no-slip adiabatic walls (or periodic, in opposite pairs).

## 4. Boundary condition types

### 4.1 Velocity / pressure
| Type | Parameters | LBM implementation (MVP) | Status |
|------|------------|--------------------------|--------|
| `noSlip` (wall) | — | Half-way bounce-back | in format |
| `slip` (wall) | — | Specular reflection | in format |
| `moving` (wall) | tangential velocity | Bounce-back with momentum correction | in format |
| `uniform` (inlet) | velocity vector | Zou-He / velocity bounce-back | in format |
| `parabolic` (inlet) | peak velocity | Zou-He per cell | in format |
| `massFlow` (inlet) | kg/s | Converted into a mean velocity | planned |
| `pressure` (outlet) | gauge pressure | Pressure Zou-He / anti-bounce-back | in format |
| `zeroGradient` (outlet) | — | Extrapolation / convective outflow | in format |
| periodic | pair of edges | Periodic indexing (`grid.edges`) | in format |

### 4.2 Thermal
| Type | Parameters | Status |
|------|-----------|--------|
| `adiabatic` | — (zero flux) | in format |
| `fixed` | temperature (K) | in format |
| `flux` | W/m² | in format |
| `convective` | h (W/m²K), ambient T | in format |
| `volumetricSource` (in a solid) | W/m³ | planned |

### 4.3 Future
Roughness / wall functions (turbulence), contact angle (multiphase), porosity (porous media), time-dependent values (`value` could become `{ "type": "sine", "amp": ..., "freq": ... }` or a time-value table).

## 5. Internal domain (`Domain`, solver side)

Result of the `Scene → Domain` conversion in `cfd-core` (Phase 1):
- Per-cell flag mask (`u16` bits: fluid, solid, boundary, BC type, solid neighbour in each direction — precomputed for branch-free kernels).
- Per-element parameter tables already in **lattice units** (velocity, density, τ, dimensionless temperature).
- Conversion factors (`dx`, `dt`, `ρ0`, `T_ref`, `ΔT`) and dimensionless numbers (Re, Ma, Pr, Ra) for reporting and stability checks.

## 6. Results

### 6.1 Frame
Implemented in `crates/cfd-io/src/frame.rs`:
```
magic "LFFR" | version u16 | header length u32 | JSON header | zstd(field data)
FrameHeader { jobId, frameIndex, step, simTime, width, height, fields: [FieldDesc] }
FieldDesc   { name, components (1|2), dtype (f16|f32) }
```
- For streaming visualisation: **f16** and optionally downsampling (e.g. 2048² → 1024²).
- For download/analysis: f32 at full resolution.

### 6.2 Storage
```
results/{job_id}/
  scene.json          # exact scene used (reproducibility)
  meta.json           # solver version, backend, GPU, timings, diagnostics, Re/Ma/...
  frames/000123.bin   # encoded frames
  probes.csv          # probe time series
  forces.csv          # drag/lift per element (if requested)
```
Later: Zarr/HDF5 for efficient partial reads; VTK (`.vti`) export for ParaView.

## 7. API (draft)

| Method | Route | Description |
|--------|-------|-------------|
| `POST` | `/api/simulations` | Creates a job from a `Scene` → `{ id, estimatedMemory, warnings[] }` |
| `GET` | `/api/simulations/{id}` | Status, progress, diagnostics |
| `DELETE` | `/api/simulations/{id}` | Cancel |
| `POST` | `/api/simulations/{id}/pause` · `/resume` | Control |
| `WS` | `/api/simulations/{id}/stream` | Messages: `progress` (JSON), `frame` (binary), `log`, `done` |
| `GET` | `/api/simulations/{id}/frames/{n}` | Single frame |
| `GET` | `/api/simulations/{id}/probes.csv` | Probe data |
| `POST` | `/api/scenes/validate` | Validate without running (stability warnings) |
| `GET/POST` | `/api/scenes` | Save/list scenes |
| `GET` | `/api/capabilities` | Available methods, backends and limits |

WebSocket control messages (future interactive mode): `{ "type": "patch", "rect": [...], "layer": "cellType", "data": ... }`, `{ "type": "setParam", ... }`.
