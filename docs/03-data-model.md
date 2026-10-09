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
| `noSlip` (wall) | — | Half-way bounce-back | implemented |
| `slip` (wall) | — | Specular reflection | in format (rejected by the Phase 1 solver) |
| `moving` (wall) | tangential velocity | Bounce-back with momentum correction | implemented |
| `uniform` (inlet) | velocity vector | Velocity bounce-back (imposes `ρ₀u`) | implemented |
| `parabolic` (inlet) | peak velocity | Velocity bounce-back, one parameter slot per cell | implemented |
| `massFlow` (inlet) | kg/s | Converted into a mean velocity | planned |
| `pressure` (outlet) | gauge pressure | Anti-bounce-back (velocity from the fluid cell) | implemented |
| `zeroGradient` (outlet) | — | Copy of the interior neighbour along the outlet normal | implemented |
| periodic | pair of edges | Periodic indexing (`grid.edges`) | implemented |

All rules are link-wise and half-way, chosen so hand-drawn pixel geometry needs no normals ([ADR-014](07-decisions-and-questions.md)). A parabolic profile spans the full extent of the element's cells perpendicular to `peak`.

### 4.2 Thermal
| Type | Parameters | Status |
|------|-----------|--------|
| `adiabatic` | — (zero flux) | implemented (bounce-back of `g`; on inlets: zero gradient) |
| `fixed` | temperature (K) | implemented (anti-bounce-back of `g`) |
| `flux` | W/m² | implemented on walls (bounce-back + flux per link) |
| `convective` | h (W/m²K), ambient T | in format (rejected by the Phase 1 solver) |
| `volumetricSource` (in a solid) | W/m³ | planned |

### 4.3 Future
Roughness / wall functions (turbulence), contact angle (multiphase), porosity (porous media), time-dependent values (`value` could become `{ "type": "sine", "amp": ..., "freq": ... }` or a time-value table).

## 5. Internal domain (`Domain`, solver side)

Result of the `Scene → Domain` conversion (`crates/cfd-core/src/domain.rs`). The arrays are uploaded unchanged to every backend; cells are indexed `y * width + x` as in the layers.

- `flags: Vec<u16>` per cell:

  | Bits | Meaning |
  |------|---------|
  | 0–2 | Flow kind: 0 fluid · 1 bounce-back (walls, moving walls, velocity inlets) · 2 pressure outlet · 3 zero-gradient outlet |
  | 3–4 | Thermal kind of links into the cell: 0 adiabatic · 1 fixed temperature · 2 heat flux · 3 zero gradient |
  | 5–7 | Outlets: inward axis direction (1–4) of the interior neighbour, for zero-gradient copies |
  | 8–15 | Fluid cells: bit `7 + i` set when population `i` (1–8) is pulled through a boundary link (non-fluid neighbour or a non-periodic edge) — interior cells (mask 0) take a branch-free path |

- `bc_slot: Vec<u32>` per cell → `bc_params: Vec<BcParams>` (32-byte `#[repr(C)]`: wall/inlet velocity, outlet density, θ, heat flux, all in lattice units). Slot 0 is the implicit no-slip adiabatic wall at the domain edges; uniform elements share one slot, parabolic inlets get one per cell. `slot_element` maps slots back to element ids (forces per element).
- Lattice physics (ν, α, Boussinesq coefficient per unit θ, optional body force), initial state, run length in steps, probes.
- Conversion factors (`dx`, `dt`, `ρ0`, `T_ref`, `ΔT`), characteristic scales and dimensionless numbers (Re, Ma, Pr, Ra), the hydrostatic pressure reference ([ADR-016](07-decisions-and-questions.md)), and the warnings from validation, support and stability checks.

The direction order (rest, E, N, W, S, NE, NW, SW, SE) is `domain::LINKS`; D2Q5 uses the first five.

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
