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
  "reference": { "length": 0.1, "velocity": 1.0 },   // optional: scales of reported Re, Ra, C_D, C_L, St, Nu
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

**Reference scales** (`reference`, optional; `length` m, `velocity` m/s, `temperatureDifference` K, each optional): the scales of the dimensionless numbers *reported* for the scene — Re and Ra in the estimate, the force coefficients, St and Nu in the viewer — e.g. a cylinder's diameter and the mean inlet velocity. They never change the simulation: the time step still follows the fastest prescribed velocity. A missing value is the solver's characteristic scale (the shorter side of the fluid's bounding box, the largest prescribed or buoyancy velocity, the spread of the prescribed temperatures).

## 3. Pixel layers

Each layer is a W×H array (row by row, origin at the bottom-left corner to match the physical convention; the frontend flips the Y axis when rendering): `index = y * width + x`.

| Layer | Type | Meaning |
|-------|------|---------|
| `cellType` | `u8` | 0 = fluid, 1 = solid, 2 = inlet, 3 = outlet, 4 = heat source, 5 = empty (gas/void above a free surface) |
| `elementId` | `u16` | Element ID (`elements` table); 0 = none |
| `fluidId` | `u8`, optional | Initial fluid per cell (multiphase); 0 = `initial.fluid` |
| `probes` | sparse list (outside `layers`) | Probe coordinates (no array needed) |

**Transport encoding:** each layer is sent as base64 of raw little-endian bytes (`raw+base64`), of `zstd(raw bytes)` (`zstd+base64`, what `cfd-cli` writes) or of raw DEFLATE (RFC 1951, no zlib header: `deflate+base64`, what the web editor writes — browsers compress it natively with `CompressionStream("deflate-raw")` and have no zstd encoder). Drawings have large uniform areas; typical compression > 50×. Every reader accepts all three; inflating is bounded by the layer's expected size.

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

Result of the `Scene → Domain` conversion (`crates/cfd-core/src/domain.rs`). The arrays are uploaded unchanged to every backend; cells are indexed `y * width + x` as in the layers. On the GPU the `u16` flags travel as the same bytes, two cells per 32-bit word (WGSL has no 16-bit storage type; [ADR-019](07-decisions-and-questions.md)).

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
Implemented in `crates/cfd-io/src/frame.rs` (LFFR v2; v1 frames are still read):
```
magic "LFFR" | version u16 = 2 | header length u32 | JSON header | payload
FrameHeader { jobId, frameIndex, step, simTime, width, height, codec, fields: [FieldDesc] }
FieldDesc   { name, components (1|2), dtype (f16|f32) }
codec       "shuffle+lz4" (written) | "zstd" (every v1 frame; no `codec` field there) | "none"
payload     shuffle+lz4: per field, in header order: u32 length L | L bytes of lz4 block(shuffle(field bytes))
            zstd:        zstd(field 0 bytes ‖ field 1 bytes ‖ …)
            none:        field 0 bytes ‖ field 1 bytes ‖ …
```
- **Field bytes:** `width · height · components` little-endian values (f16 or f32), row-major from the bottom-left
  corner, components interleaved.
- **Byte shuffle** (as in HDF5/Blosc): the `m` values of `s` bytes of a field are stored as every value's byte 0,
  then every byte 1, … (`out[b·m + i] = in[i·s + b]`). The sign/exponent bytes of neighbouring values are alike and
  compress; the low mantissa bytes are noise. Each field is one lz4 *block* (the raw LZ4 block format, no frame
  header or checksum; its decompressed size is known from the header), compressed in parallel on the server.
- **Decoding in the browser** (`apps/web/src/frames/`, in a Web Worker; contract fixtures written by `cfd-io`'s
  test `browser_fixtures`): read the prefix and the JSON header, then per field an LZ4 block
  decoder into a buffer of the known size (a ≈ 100-line TypeScript function, or `lz4js`'s block decoder), the
  unshuffle loop, and a `Float32Array`/`Uint16Array` view of the result (f16 is uploaded as-is to a `R16F`/
  `RG16F` texture). No WASM is needed; v1 (zstd) frames only exist in result directories written before v2 and
  are not streamed.
- **Stored frames:** f32 at full resolution every output interval (`frames/NNNNNN.bin`); `GET …/frames/{n}?dtype=f16` converts on the fly, and `&fields=…&maxSize=…` returns it in a stream format (the viewer's timeline). Stored and stream frames are *samples* computed on the GPU when the job runs there ([ADR-028](07-decisions-and-questions.md)): bit-identical to the CPU conversion on CUDA, within ~1e-7 on wgpu.
- **Stream frames** (WebSocket): **f16**, only the fields the viewer asked for, box-downsampled by the integer factor `k = ceil(max(width, height) / maxSize)` to `ceil(width/k) × ceil(height/k)` (partial blocks at the edges average what they have). `frameIndex` is the output index; a snapshot (a frame taken for a viewer that joined a paused job, or a new stream format) repeats the last output index and carries its own `step`.
- **Cost** (1024² cavity, 3 fields, f32): 13.6 ms to 63.7% of the raw size, decoded in 17 ms; `ruzstd` (v1) took
  328 ms for 91% ([benchmarks](benchmarks.md#frame-codec-lffr-v2--amd-ryzen-7-3700x-2026-10-16)).

### 6.2 Storage
```
results/{job_id}/
  scene.json          # exact scene used (reproducibility)
  meta.json           # solver version, backend, GPU, timings, diagnostics, Re/Ma/...
  frames/000123.bin   # encoded frames
  png/<field>/000123.png, png/ranges.json   # colour-mapped images (cfd-cli run / render only)
  probes.csv          # probe time series (u, v, p, T per probe)
  forces.csv          # force per element (N/m, gauge)
  diagnostics.csv     # mass, kinetic energy, max velocity/Mach, heat
  heat.csv            # thermal runs with stored frames: heat through each thermal wall, per frame
```
`heat.csv` has a row per *stored frame* (`step, time_s, {element}.q_W_m, {element}.length_m` for each element that is a `wall` or `heatSource` with a fixed-temperature or flux condition and touches the fluid; the element is named like its columns in `forces.csv`). `q_W_m` is the heat per metre of span in W/m, positive into the fluid: for a fixed temperature, -k times the wall-normal temperature gradient summed over the element's fluid-facing cell faces (`cfd_core::heat`: a second-order one-sided difference at the half-way wall, from the stored full-resolution temperature, so it does not depend on the engine); for a flux, the prescribed value times the length. `length_m` is the number of faces times dx. Adiabatic, convective (unsupported) and inlet/outlet elements are not listed. The output thread writes and flushes each line before it tells observers, so a `heat` row in a `series` message is on disk. The Nusselt number is `Nu = (q / length) L / (k ΔT)` with `L` and `ΔT` from `meta.json`'s `dimensionless.reference` and `k` from its `conductivity`.

The CSV series get a row every sample interval; the files are flushed before each progress report (so a viewer that reads them while the run goes on gets whole lines, up to some step) and the same rows reach viewers as `series` messages (§7). Written by `cfd-job` for both `cfd-cli run --out DIR` and server jobs (`<data>/results/{job id}/`), so `cfd-cli render` works on server results. `meta.json` records:
- `run.status`: `running` while the job runs, then `completed`, `diverged` (with a message and hints) or `cancelled`;
- `run.heat`: `true` when the run has a `heat.csv`; `dimensionless.reference.conductivity` (W/(m·K), thermal runs): the initial fluid's, for the Nusselt number;
- `run.frames` (output events) and `run.framesStored` (frame files; fewer with `--no-frames` / `storeFrames: false`, or past the server's per-job `maxStoredBytes`, which adds a `storage.limit` warning — the run goes on without storing frames);
- `generator`: `cfd-cli x.y.z` or `cfd-server x.y.z`.

Later: Zarr/HDF5 for efficient partial reads; VTK (`.vti`) export for ParaView.

## 7. API

Implemented by `cfd-server` (Phase 3). All JSON is camelCase like the scene format; times are Unix milliseconds.

| Method | Route | Body / query | Response |
|--------|-------|--------------|----------|
| `GET` | `/api/capabilities` | — | engines (kind, availability or `reason`, device, memory, `physicalDevice`), `auto`, limits, stream defaults |
| `POST` | `/api/scenes/validate` | `Scene` | `{ ok, code?, message?, issues: [{ code, severity, message, params?, at? }], estimate }` — a scene that cannot run is `ok: false`, not an error status; the estimate is `null` only when the scene could not be prepared (format or consistency errors) |
| `POST` | `/api/simulations` | `{ scene \| sceneId, options? }`; `?reuse=completed` | `201` + `Location` with the job (status, backend, device, estimate, warnings, notes); with `?reuse=completed`, `200` + `Location` with the latest completed job of the same scene and options instead of starting one (the SHA-256 of the validated scene's canonical JSON plus the options that change the result, not the engine; jobs whose results were purged are skipped); `422` with `issues`; `429` when the queue is full |
| `GET` | `/api/simulations` | `?status=&limit=` | jobs, newest first |
| `GET` | `/api/simulations/{id}` | — | the job: status, reason, message, progress, frames, timings |
| `POST` | `/api/simulations/{id}/pause` · `/resume` | — | `202` (applied between batches); `409` unless running/paused |
| `DELETE` | `/api/simulations/{id}` | `?purge=true` | cancel: `202`; with `purge`, also delete results and record (`204`, or `202` while an active job stops) |
| `WS` | `/api/simulations/{id}/stream` | `?fields=velocity,vorticity&maxSize=1024` | see below; `404` before the upgrade for an unknown job |
| `GET` | `/api/simulations/{id}/frames/{n}` | `?dtype=f32\|f16&fields=…&maxSize=…` | one stored frame (LFFR); with `fields`/`maxSize`, in that stream format (the named fields in frame order, box-downsampled like the stream): byte-identical to the stream frame of the same index on the CPU engine, so the viewer mixes both |
| `GET` | `/api/simulations/{id}/files/{name}` | `scene.json`, `meta.json`, `probes.csv`, `forces.csv`, `diagnostics.csv`, `heat.csv` (thermal runs with stored frames; `404` otherwise) | the file |
| `GET` · `POST` | `/api/scenes` | `Scene` | list (id, name, times) · save (`201 { id, name }`) |
| `GET` · `PUT` · `DELETE` | `/api/scenes/{id}` | — · `Scene` · — | the scene JSON · `{ id, name }` (replaced; `404` if unknown) · `204` |

- **Issues:** `code` is stable; `message` is English; `params` holds the values in the message (element, fluid
  and probe ids, Mach number, τ, limits, and for `cell.element` a `problem`: `noElement`, `missingElement`,
  `kindMismatch`, `noFluidNeighbour`, `unexpectedElement`) so a UI can word it itself; `at` is the cell
  `[x, y]` an issue is about (per-cell issues, at most 5 per kind).
- **Estimate:** `cells`, `fluidCells`, `steps`, `frames`, `thermal`, `memoryBytes`, `storedBytes`, `engine`
  (`null` when none can run it), `seconds` (once a job has measured that engine's throughput) and `numbers`:
  the characteristic `velocity` and `length`, `reynolds`, `mach`, `prandtl?`, `rayleigh?`, `tau`, `tauThermal?`,
  `dx`, `dt`, `latticeVelocity` and `reference` (`length`, `velocity`, `temperatureDifference` with the defaults filled
  in, and the `reynolds` and `rayleigh?` they give). A scene over a server limit or with no engine still gets its estimate.
- **Options** (`options` of a submission; the same overrides as `cfd-cli run`): `backend` (`auto|cpu|wgpu|cuda`; default: the scene's `run.backend`), `latticeVelocity`, `endTime`, `outputInterval`, `sampleInterval` (s), `storeFrames` (default `true`; streaming works either way). Unknown options are a `400`.
- **Job status:** `queued → running ⇄ paused → completed | failed | cancelled`; `failed` carries `reason`: `diverged` (the frames before stay valid), `error` (backend or I/O) or `interrupted` (the server stopped; jobs are not resumed).
- **Errors:** `{ "error": { "code", "message", "issues"? } }` with stable codes: `request.json` (400, not JSON or not the expected shape), `scene.format` (400, not a scene), `scene.invalid` (422, with the validation `issues`), `limits.cells` / `limits.steps` (422), `engine.unavailable` / `engine.memory` (422), `request.tooLarge` (413: send layers as `zstd+base64`), `queue.full` (429), `notFound`, `conflict`, `internal`.
- **Web app:** every path outside `/api` is a static file of the web app (`--web-dir` or the embedded build; 02 §2.2).
- **Limits** (`/api/capabilities`): 4096² cells, 10⁸ steps, 10 GiB of stored frames per job, 16 queued jobs, 64 MB request bodies.

### WebSocket messages

Server → client; text messages are JSON with a `type`, binary messages are stream frames (§6.1).

| `type` | When | Content |
|--------|------|---------|
| `hello` | first | `job` (as `GET /api/simulations/{id}`), `grid` (`width`, `height`, `dx`, `dt`, `thermal`; `null` until the job starts), `format` (`fields`, `maxSize`, `factor`, `width`, `height`, `dtype`), `ranges` (min/max of each field so far; velocity: of its magnitude), `series` (`forces`, `probes`, `diagnostics` and, for thermal runs with stored frames, `heat`: the column names, i.e. the CSV headers; `null` until the job starts) |
| `status` | on a change | `status`, `reason?`, `message?` |
| `series` | before each `progress` and before `done` | `rows`: `{ forces?, probes?, diagnostics?, heat? }`, each a list of rows `[step, time, …]` in the columns of `hello.series` (the values of the CSV lines; `NaN` is `null`), sampled since the last `series` message and already on disk; `resync: true` (with no rows) after the viewer fell behind: read the files again |
| `progress` | at most 10 Hz | `step`, `total`, `simTime`, `percent`, `mlups`, `massDrift`, `maxMach` |
| `log` | occasionally | `message` (e.g. the stored-frame limit was reached) |
| `done` | last | `status`, `reason?`, `message?`, `stepsDone`, `frames`; the server then closes the socket |

- A viewer gets the latest frame of its format; one that reads slowly skips frames (never the last one, sent before `done`), and the job never waits for viewers. Events are buffered per viewer (64); one that falls further behind gets the current `status`/`progress` again.
- A viewer of a finished job gets `hello` and `done` at once (the series are in the files then).
- A viewer that joins a running job reads `files/{forces,probes,diagnostics}.csv` (complete lines up to some step) and appends the `series` rows whose step is past the files' last one: every row sent before it joined is in the files, every later one comes in a message.
- Client → server messages are reserved for interactive mode (Phase 5b): `{ "type": "patch", "rect": [...], "layer": "cellType", "data": ... }`, `{ "type": "setParam", ... }`; until then they are ignored. Pause, resume and cancel go through REST.
