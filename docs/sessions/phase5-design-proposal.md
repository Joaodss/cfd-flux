# Phase 5 — visualisation and playback (the MVP viewer): design proposal (2026-10-19)

**Status: approved by the author on 2026-10-19, as proposed** (every proposal of §16 accepted). Implemented on
`feat/phase5-viewer`. The accepted parts move
into [02 §2.1](../02-architecture.md#21-frontend--appsweb), [03](../03-data-model.md) and a new ADR as they are
implemented, and the plan's Phase 5 tasks follow §15.

Goal of Phase 5 ([plan](../05-implementation-plan.md#phase-5--visualisation-and-playback--mvp)): view and analyse
results (FR-20–FR-24) in the job view, live while a run goes on and afterwards. **Proposed acceptance (§14):** run the
"Cylinder Re=100" example from the editor, watch the vortex street form live, scrub back through it, and read St and
C_D,max within the Schäfer–Turek tolerances of [04 §7](../04-numerical-methods.md#7-validation) for that resolution.

## 1. Overview

```
 apps/web — the job view (#/jobs/{id})
 ┌──────────────────────────────────────────────────────────────────────────────────────────────────┐
 │ React: header (status, pause/resume/cancel) · field/colour-map/range controls · timeline ·       │
 │        plots (uPlot) · statistics · export                                                       │
 │                                                                                                  │
 │   WebSocket ──text──▶ job reducer (status, progress, series rows)          GET files/scene.json  │
 │      │                                                                       │ (layers → mask)   │
 │      └─binary (LFFR f16)──┐                     GET frames/{n}?dtype=f16&…   ▼                   │
 │                           ▼                          │               ┌──────────────────────┐    │
 │                 ┌──────────────────────┐ transfer    │               │ Viewer (no React)     │    │
 │                 │ decode worker        │◀────────────┘               │  FrameCache (LRU)     │    │
 │                 │  LFFR header, LZ4,   │──── decoded fields ───────▶ │  Timeline (playhead)  │    │
 │                 │  unshuffle, ranges   │     (Uint16Array, f16)      │  FieldRenderer WebGL2 │    │
 │                 └──────────────────────┘                             │  overlays, particles  │    │
 │                                                                      └──────────────────────┘    │
 └──────────────────────────────────────────────────────────────────────────────────────────────────┘
```

The same rule as the editor (ADR-005, ADR-030): **pixels never go through React state.** Frames are typed arrays owned
by a plain TypeScript `Viewer` object and uploaded to WebGL textures; React renders the controls and subscribes to a
small Zustand store (`viewerStore`: current field, colour map, range, playhead, play state, cache status).

### TypeScript, workers and WebGL idioms used below (with .NET parallels)

- **Typed arrays** (`Uint8Array`, `Uint16Array`, `Float32Array`) are *views* over an `ArrayBuffer` (raw bytes), like
  `Span<T>`/`Memory<T>` over a `byte[]`: `new Uint16Array(buf, offset, n)` reinterprets bytes without copying (the
  offset must be a multiple of the element size). An f16 value has no JS number type of its own in older browsers,
  so f16 data stays as `Uint16Array` (the raw bits, like `ushort` holding a `System.Half`) and is converted only where
  numbers are needed (ranges, the value under the cursor), through a 65,536-entry lookup table.
- **Web Workers** are background threads with *no shared memory*: they talk by `postMessage`, like a dedicated thread
  fed by a `Channel<T>`. Normally a message is copied (structured clone); an `ArrayBuffer` passed in the *transfer
  list* is **moved** instead — the sender's buffer becomes empty (detached), the receiver owns the bytes, nothing is
  copied. That is Rust's move semantics, or handing a buffer to another thread and promising never to touch it again.
- **`requestAnimationFrame(cb)`** calls `cb` once before the browser's next repaint (≈ 60 Hz, paused in hidden tabs),
  like WPF's `CompositionTarget.Rendering`. Playback and particles run in such a loop; everything else draws on
  demand (as the editor does).
- **Textures and shaders:** a texture is a 2D array on the GPU that a shader reads with optional bilinear filtering.
  A *fragment shader* is a function run for every screen pixel (an HLSL pixel shader); it gets *uniforms* (constants
  for one draw call, like shader constant buffers) and samples textures. `R16F`/`RG16F` are one- and two-channel
  half-float textures: WebGL2 uploads them straight from a `Uint16Array` of f16 bits (`type = HALF_FLOAT`) and —
  unlike 32-bit float textures — **filters them linearly without any extension**. That is the main reason the stream
  is f16 (ADR-007).

## 2. Frame decoding in the browser

### 2.1 Decoder

`src/frames/lffr.ts`, pure functions with no DOM dependency (so Vitest runs them in Node):

- `parseHeader(bytes)`: magic `LFFR`, version (2; 1 is rejected with a clear message — v1 frames are zstd and only
  exist in old result directories, 03 §6.1), header length, the JSON `FrameHeader`.
- `decodeLz4Block(src, dstLength)`: the raw LZ4 *block* format (token, literal run, 16-bit offset, match length with
  255-continuation bytes, overlapping copies). ≈ 60 lines; every access is bounds-checked so a corrupt frame throws
  instead of reading garbage, and the output must be exactly the expected length.
- `unshuffle(src, valueSize)`: `out[i·s + b] = in[b·m + i]`, the inverse of `cfd-io`'s shuffle.
- `decodeFrame(buffer) → DecodedFrame { header, fields: [{ name, components, dtype, data: Uint16Array | Float32Array }] }`
  for the codecs `shuffle+lz4` and `none` (the two that are streamed or served).

**Own decoder, not `lz4js`.** The block format is tiny and fully specified; `lz4js` has been unmaintained since 2017,
ships no types and its block API allocates. Writing it ourselves also makes the contract test meaningful.

**In a Web Worker.** Decoding a 1024² frame with four f16 values per cell (8 MiB) takes an estimated 10–20 ms in
JavaScript; on the page's thread that is a dropped animation frame every time one arrives. The worker
(`src/frames/decoder.worker.ts`, bundled by Vite with `new Worker(new URL(…), { type: 'module' })`) receives the
compressed `ArrayBuffer` *transferred*, decodes it, computes each field's statistics (min, max, and a 256-bin histogram
of |value| for percentile ranges, §4.3), and transfers the decoded buffers back. A small promise wrapper
(`FrameDecoder.decode(buf): Promise<DecodedFrame>`, request ids, no library) hides the messages. One worker is enough:
frames arrive in order and decoding is the only work; it also fetches stored frames itself (`fetch` works in
workers), so the bytes never touch the main thread.

### 2.2 Contract test against `cfd-io`

As with the Phase 4 scene fixture: a Rust test in `cfd-io` writes `apps/web/tests/fixtures/frames/` when
`UPDATE_FIXTURES=1` and otherwise **fails if the files differ** from what it would write:

- `stream-f16.lffr` — `shuffle+lz4`, f16, a scalar and a 2-component field of 97 × 53 values (odd sizes: no
  accidental alignment), values from a closed formula with runs of repeats (long LZ4 matches, overlapping copies);
- `stored-f32.lffr` — the same in f32; `none-f16.lffr` — the uncompressed codec;
- `frames.json` — the headers and the expected values (f32 as numbers, f16 as their bits).

Vitest decodes each file and compares every value bit for bit. The other direction is not needed: the browser never
encodes frames.

## 3. Frame buffer: live stream and stored frames

### 3.1 What the viewer asks for: one *viewer format* for live and stored frames

The viewer works at one resolution per session, the **viewer format**: all the run's frame fields (the scene's
`outputFields`), box-downsampled to at most `maxSize` cells on a side, f16. Default `maxSize = 1024` (the server's
default; a setting offers 256/512/1024/2048/"full"); changing it reconnects and empties the cache.

- **Live:** `WS …/stream?maxSize=1024` (no `fields` = every frame field). Each binary message is a frame of the
  viewer format; its `frameIndex` is the output index. The server writes the stored frame *before* it publishes the
  stream frame (`FrameEvent::stored`), so frames `0..=frameIndex` exist on disk when one arrives.
- **Stored:** `GET …/frames/{n}?dtype=f16` returns the frame at **full resolution** today. **Proposed server change:**
  the same `fields` and `maxSize` query as the stream, applied on the server (the CPU box filter of
  `cfd_core::sample`, the reference the GPU samples are tested against), so a fetched frame has exactly the viewer
  format and live and stored frames are interchangeable in the cache. Test: on the CPU engine a fetched frame equals
  the streamed frame of the same index byte for byte.

Asking for every field instead of only the displayed one costs the server one encoding per frame either way (one
format), keeps field switches instant and lets overlays (vectors need velocity while pressure is shown) work without a
second stream.

### 3.2 Memory budget

f16 bytes per frame = cells × components × 2; the cylinder's fields (velocity, pressure, vorticity) are 4 components:

| Grid | Cells | Viewer frame (f16, ≤ 1024) | Stored frame (f32, full) | 800 frames in the viewer format |
|------|-------|-----------------------------|--------------------------|---------------------------------|
| 442 × 84 (cylinder) | 37 k | 0.30 MB | 0.59 MB | 238 MB |
| 1024² | 1.05 M | 8.4 MB | 16.8 MB | 6.7 GB |
| 4096² (factor 4 → 1024²) | 16.8 M | 8.4 MB | 268 MB | 6.7 GB |

So the cache cannot hold every frame of a large run, and does not need to: the **server is the store** (frames are
already on disk, served over loopback). The `FrameCache` keeps decoded frames in an LRU map up to a budget
(**512 MiB** by default, a setting), evicts the frames farthest from the playhead first, and prefetches the next few
frames in the play direction (two requests in flight). The cylinder fits entirely; a 1024² run keeps ≈ 60 frames
and plays from the server at an estimated 20–30 frames/s (the f32 → f16 conversion and downsampling of a 16.8 MB
stored frame on the server dominates). A 4096² run plays at a few frames/s from stored frames — the server decodes
268 MB per frame; a downsampled preview stored next to each frame would fix it and is left for later.

**No IndexedDB.** It would duplicate what the server keeps, needs quota management and gains nothing locally. (A
future static demo, ADR-010, would ship precomputed frames as files, still not IndexedDB.)

**Full resolution on demand:** when zoomed in beyond the viewer format (more than one screen pixel per downsampled
block), the viewer can fetch the current frame at full resolution (`maxSize` omitted) for the paused playhead only —
a "sharpen" step, not a second cache.

## 4. WebGL2 rendering

### 4.1 Field renderer

`src/viewer/render/fieldRenderer.ts`, the same structure as the editor's `Renderer`: one full-screen triangle, a
fragment shader that maps each screen pixel to a cell through the shared `View` (zoom/pan), on-demand drawing.

- **Field textures:** one per field of the current frame: `R16F` (scalars) or `RG16F` (vectors), uploaded with
  `texImage2D(…, HALF_FLOAT, Uint16Array)` when the playhead changes frame. `LINEAR` filtering (optional `NEAREST`,
  "show cells"). Derived scalars are chosen in the shader: |u|, u_x, u_y from the velocity texture; pressure,
  vorticity, temperature, density as stored.
- **Solid mask:** the scene comes from `GET files/scene.json` (the exact scene of the run) and its `cellType` layer is
  decoded with the editor's `fromScene` and uploaded as the editor does (`R8UI`, full resolution). Non-fluid cells are
  drawn with a solid colour (or their element colours, "show elements"), so boundaries stay crisp even when the field
  is downsampled. **Zero dilution:** samples are 0 at non-fluid cells and the box filter averages them in, so blocks
  next to walls are biased towards 0 (pressure near the cylinder would look wrong). The viewer computes once, from the
  mask, the **fluid fraction** of every block (the same box filter) as an `R16F` texture, and the shader divides by it
  — exactly the mean over the block's fluid cells.
- **Colour maps:** 256 × 1 RGBA8 lookup textures. The CLI's `cfd_io::image::Colormap` (viridis, inferno, cool-warm;
  nine samples each) becomes the source of truth for both: `cfd-cli colormaps apps/web/src/generated/colormaps.json`
  (checked as up to date like `library.json`), adding greys, magma and a diverging blue-red — so a PNG from
  `cfd-cli render` and the viewer look the same. Defaults per field: |u| viridis, components and vorticity cool-warm
  (symmetric), pressure cool-warm (symmetric around 0 gauge), temperature inferno, density viridis.
- **Reuse from the editor:** `view.ts` (zoom ladder, `zoomAt`, `fitView`, `screenToCell`), the canvas sizing and
  pointer/wheel handling of `EditorCanvas`, the WebGL helpers (`link`, `bind`, `integerTexture`, moved to
  `src/gl/`) and the cell colours. The editor's tools, history and overlay are not used.

### 4.2 Legend and readout

An HTML/SVG legend next to the canvas (the colour bar from the same LUT, five ticks with the field's unit: m/s, Pa,
1/s, K, kg/m³) — React, not WebGL, so it is crisp and exportable. The status bar shows the cell and value under the
pointer (f16: ≈ 3 significant digits; "exact value" fetches the f32 stored frame for that cell's frame once, paused).

### 4.3 Ranges

Per field, one of:
- **Automatic, all frames so far** (default): the union of the `ranges` in `hello` (frames before the viewer joined)
  and every decoded frame's min/max. Stable while scrubbing; it only grows.
- **Automatic, this frame:** the frame's own min/max (or percentiles).
- **Symmetric** (diverging fields): ±the 99th percentile of |value| over fluid cells by default — the vorticity peak at
  the cylinder surface is 10× the wake's, and a min/max range hides the street (`cfd-verify`'s snapshots use the
  same 0.99 quantile). Percentiles come from the worker's histogram.
- **Manual:** min/max typed in, or dragged on the legend.

f16 limits: values beyond ±65,504 become ∞ (a hydrostatic pressure of ≈ 6.7 m of water); the viewer reports such
cells instead of drawing them silently. Fine detail on top of a large offset (hydrostatic pressure) loses digits in
f16; a "dynamic pressure" display would need the hydrostatic part subtracted on the server — noted, not proposed now.

## 5. Field selector, overlays and derived quantities (FR-20, FR-21)

- **Field selector:** the frame's fields and their derived scalars (|u|, u_x, u_y, p, ω, T, ρ), each with its
  colour-map default and remembered range.
- **Vectors:** arrows on a regular screen-space grid (every 24 px, rescaled with the zoom), drawn as instanced line
  segments whose vertex shader reads the velocity texture (texture reads in vertex shaders are core WebGL2); length ∝
  |u| with a reference arrow in the legend.
- **Streamlines:** RK4 on the client, in the worker, over the current frame's velocity (bilinear interpolation, f32
  copy of the f16 data), seeded on a jittered grid with a minimum separation (a simplified Jobard–Lefer), stopped at
  non-fluid cells, domain edges, stagnation or a maximum length; the polylines are drawn as line strips. Recomputed
  when the frame changes while paused (≈ 10–50 ms); during playback every few frames.
- **Tracer particles (GPU):** positions in an `RG32F` texture, advanced by a fragment shader into a second texture
  (ping-pong; rendering to float textures needs `EXT_color_buffer_float`, available everywhere WebGL2 runs, also in
  SwiftShader), drawn as points read in the vertex shader. Particles move with the displayed frame's velocity at the
  playback's physical rate (simulated seconds per wall second), so their speed is physical; they are released at the
  inlets (from the scene's layers) or along a line the user draws, and recycled when they leave the fluid.
- **Smoke / dye (GPU):** a concentration texture (`RGBA16F`, three dyes) advected semi-Lagrangian by the same velocity,
  injected at inlets or by painting, slowly decaying; drawn over or instead of the colour map.
- **WebGPU:** not in Phase 5. One rendering path (WebGL2) that CI can test on software WebGL; WebGPU (02 §2.1) can
  come with 3D or when particle counts need it.

## 6. The timeline (FR-22)

`src/viewer/timeline.ts`, a small state machine outside React (tested on its own):

- **Frames:** indices `0 … last`; `last` grows with every stream frame while the job runs (and is `job.frames − 1`
  afterwards; with `storeFrames: false` or past the storage limit only the live frame exists, and the timeline says
  so). Each frame's simulated time comes from its header.
- **Modes:** *Live* (the playhead follows the newest frame; the default while running), *Paused*, *Playing* (from
  the cache/server at a speed in frames per second, 1–60, or in simulated seconds per wall second, with loop).
  Dragging the slider or pressing ←/→ leaves Live; "Live" jumps back.
- **Playing:** a `requestAnimationFrame` loop advances the playhead by `speed × elapsed`; it shows the newest frame
  that is ready and never waits on the server inside a frame (the cache prefetches; a missing frame shows a
  "buffering" state and the loop holds until it arrives).
- **Scrubbing while the run continues:** stream frames keep arriving and go into the cache (they are of the viewer
  format), so going back to Live is instant. Pausing the *job* (REST) and pausing *playback* are separate buttons.

## 7. Plots and statistics (FR-23)

### 7.1 Series, live

The force, probe and diagnostic series are written every sample interval (by default a tenth of the output interval:
1,000 rows per simulated second for the cylinder). Today they reach the browser only as files, and the files are
behind a `BufWriter` flushed at the end of the run. **Proposed:**
- a new WebSocket text message **`series`**, sent with each `progress` (≤ 10 Hz), carrying the rows sampled since the
  last one: `{ type: "series", rows: { forces: [[step, time, fx₁, fy₁, …]], probes: [[…]], diagnostics: [[…]] } }`;
  `hello` gains `series: { forces: [column names], probes: [...], diagnostics: [...] }` (the CSV headers);
- the series files are flushed at each progress tick, so a viewer that joins late reads `files/forces.csv` (complete
  lines up to some step) and appends the stream's rows after that step;
- after the run, the files are the source (a finished job's stream sends only `hello` and `done`).

The rows go through a `JobObserver::sample` hook in `cfd-job` (one call per sample, the values `Series` already
writes), batched by the server's stream hub like progress. Plots: **uPlot** (ADR-005; ≈ 50 kB, made for 10⁵-point
series): C_D/C_L (or F_x/F_y) per element, probe velocity/pressure/temperature, mass drift and max Mach; a vertical
cursor at the playhead's time; click on a plot moves the playhead there; drag selects the statistics window.

### 7.2 Statistics

Over a window (default: the second half of the run; adjustable by dragging on a plot):
- force coefficients `C = 2F / (ρ U² L)` (forces are per unit span, N/m), mean C_D, C_D,max, mean C_L, C_L
  amplitude (half the peak-to-peak) and C_L,max;
- **St = f L / U**, f from the upward crossings of C_L through its window mean (whole periods between the first and the
  last crossing) — the algorithm of `cfd_verify::harness::oscillation_frequency`;
- **Nu** for thermal elements (§7.3).

These are post-processing of time series, not physics, so they are computed in TypeScript (interactive windows);
a contract test runs the same committed series (a real cylinder `forces.csv`, trimmed) through
`oscillation_frequency` and the TS code and requires the same St to 1e-12.

**References:** `cfd_verify::references` (Schäfer–Turek intervals, de Vahl Davis values) are generated into
`apps/web/src/generated/references.json`; a gallery example names its benchmark (e.g. `schaferTurek2D2`) in
`src/scene/examples.ts`, and the statistics panel shows the value, the reference interval and a ✓ within the
tolerance the validation suite uses at that resolution.

### 7.3 Nusselt number

`forces.csv` has no heat flux, and nothing computes one during a run. **Proposed:** a per-frame series `heat.csv`
(`step, time_s, {element}.q_W_m, {element}.length_m` for every element with a fixed-temperature or flux condition),
computed on the job's output thread from the stored frame's temperature sample with the wall-gradient code that
`cfd-verify`'s heated cavity uses today (moved to `cfd-core` so both share it). Then Nu = q L / (k ΔT · length) on
that element. Only thermal runs with stored frames get it; Nu therefore lags the force statistics in the order (§15)
and is not part of the first acceptance.

### 7.4 Reference scales (housekeeping item, decided before any code)

The editor shows the solver's own Re (largest prescribed velocity × shorter side of the fluid's bounding box), so the
"Cylinder Re=100" reads Re ≈ 615; C_D, C_L, St and Nu need the benchmark's scales (Ū = 1 m/s, D = 0.1 m). Two options:

- **A — in the scene (proposed):** an optional top-level block, physical units, used only for reported numbers:
  ```jsonc
  "reference": { "length": 0.1, "velocity": 1.0, "temperatureDifference": 10.0 }   // each optional
  ```
  `cfd_core::scene::ReferenceScales` (`serde` + `schemars`, so schema and TS types follow); each missing value falls
  back to today's characteristic scale. **It does not change the simulation** — the time step still comes from the
  fastest prescribed velocity (Mach safety). `Domain::numbers` keeps the solver's numbers (Ma, τ) and gains the
  reference Re/Ra; the estimate's `numbers` gains `reference: { length, velocity, temperatureDifference, reynolds,
  rayleigh? }`; the editor's Scene tab gets three optional fields (placeholders show the defaults) and the status bar
  shows the reference Re ("Re 100"), the Run tab both. Examples: cylinder `{ length: 0.1, velocity: 1.0 }` → Re 100;
  the heated cavity's defaults are already its scales. Validation: positive and finite. Additive and optional, so
  `schemaVersion` stays 1.
- **B — asked by the viewer:** fields in the statistics panel, kept per job in local storage. Nothing to change in
  Rust, but every user must know and retype the benchmark's scales, the editor keeps showing Re 615, and a scene file
  does not carry its own definition of C_D.

A records the definition with the problem (where an engineer expects it), works for the CLI too (`compare
--statistics` could default `--ref-length/--ref-velocity` to it) and makes the gallery examples self-describing. The
viewer still lets you override the scales for a session (not saved) and shows which ones it used.

## 8. Export (FR-24)

- **PNG:** the current view (what is on screen, at device resolution) or the whole domain at 1 px per cell (or ×2, ×4),
  with the legend, time and field drawn into the image (rendered into an offscreen framebuffer, read back,
  composed on a 2D canvas, `canvas.toBlob`).
- **CSV:** the run's `forces.csv`/`probes.csv`/`diagnostics.csv` as they are; the force coefficients and the
  statistics of the window; the current frame's field(s) at the viewer format (x, y, values).
- **Video:** frame-exact, not real-time: each frame of a range is rendered offscreen and encoded with **WebCodecs**
  (`VideoEncoder`, H.264 or VP9) into MP4 with the small `mp4-muxer` library; where WebCodecs is missing,
  `MediaRecorder` on `canvas.captureStream()` records WebM in real time. Overlays and particles included.
- **Not in Phase 5:** GIF (FR-24 lists it; MP4/WebM covers sharing), VTK `.vti` (better as `cfd-cli export` on the
  server, from the f32 frames), raw frames (already downloadable: `frames/{n}`).

## 9. Quick-examples gallery (ADR-009)

Every example except the dam break (free surface, Phase 6a) runs in seconds on the RTX 4060 (the cylinder: 48,000
steps × 37 k cells ≈ 2 s of solver time on CUDA, ≈ 20 s on the CPU). So:

- **Results are generated on first open** (proposed): "Open with results" in the gallery submits the example (or
  reopens the latest completed run of the *same scene*, matched by a hash of its JSON) and opens the job view. Nothing
  is stored in the repository or the releases: the results depend on the engine and the version, and 800 cylinder
  frames are ≈ 470 MB in f32.
- **Low resolution for the CPU:** each example has its benchmark resolution; examples too slow on the CPU get a
  `quick` variant (half resolution, shorter run) offered when the engine is the CPU, so the < 10 s target holds there.
- **Precomputed bundles** (frames in f16, downsampled, plus the CSVs) only for the future static demo (ADR-010),
  built by a script from local runs — not in Phase 5.

## 10. `cfd-server` serves the web app (ADR-010)

- The built app (`apps/web/dist`) is **embedded** in the binary with `rust-embed` behind a Cargo feature `web`
  (default off, so `cargo build` and the Rust CI need no Node); release builds run `npm run build` first and enable it.
  `--web-dir DIR` serves a directory instead (development, or a build without the feature).
- Routes: `/api/*` as today; everything else is a static file, `index.html` for `/` (hash routes need no fallback);
  hashed assets get `Cache-Control: immutable`, `index.html` `no-cache`; gzip already applies.
- `--open` opens the default browser at the listen address (the `webbrowser` crate) once the server is listening.
- Same origin, so CORS stays only for the Vite dev server. A test starts the server with a temporary `--web-dir` and
  fetches `/` and an asset.

## 11. Job view layout

The viewer takes the main area of `#/jobs/{id}`: header (as now) → a toolbar (field, colour map, range, overlays,
export) → canvas with legend → timeline; a right-hand panel with tabs *Plots*, *Statistics*, *Run* (today's status
cards and log). The editor's "Run" still opens it, and `#/jobs` still lists runs.

## 12. Source layout (new)

```
apps/web/src/
  frames/      lffr.ts (decoder), half.ts (f16 table), decoder.worker.ts, decoder.ts (promise wrapper), cache.ts
  viewer/      Viewer.ts (owns cache, timeline, renderer), timeline.ts, store.ts (viewerStore),
               render/ fieldRenderer.ts, shaders.ts, colormaps.ts, vectors.ts, particles.ts, dye.ts
               streamlines.ts (RK4, worker side), statistics.ts, series.ts
  gl/          shared WebGL helpers (from editor/render)
  app/         JobView.tsx (layout), ViewerCanvas.tsx, Timeline.tsx, Legend.tsx, PlotsPanel.tsx, StatsPanel.tsx
  generated/   colormaps.json, references.json (new, from Rust)
```

## 13. Tests

- **Vitest (Node):** LZ4 blocks (literal-only, long matches, overlapping copies, 255-continuations, truncated and
  corrupt input), unshuffle, the f16 table (all 65,536 values against `Math.f16round` where available), the frame
  contract fixtures (§2.2), ranges and percentiles, the timeline state machine, the cache's eviction, statistics (a
  synthetic sine: frequency, amplitude; the Rust contract series), streamlines on analytic fields (uniform flow:
  straight lines; solid-body rotation: closed circles, RK4 error), the colour maps against Rust's samples, the
  fluid-fraction texture.
- **Components (happy-dom):** the job view with a stand-in `WebSocket` (as in Phase 4) sending `hello`, binary frames
  and `series`; the timeline controls; the legend.
- **Rust:** frames with `fields`/`maxSize` (equal to stream frames on the CPU), `series` messages and flushed CSVs,
  `heat.csv` (later), serving the app, the generated files up to date, reference scales (scene, schema, numbers).
- **Playwright (software WebGL, CI):** open the cylinder example, run a short version on the CI's CPU engine
  (`endTime` ≈ 1 s through the run options), check that frames arrive (frame counter), that a solid cell of the canvas
  has the solid colour and the wake does not, that scrubbing to frame 0 changes the image, that the force plot has
  points; pixels read with `readPixels` through a test hook (`window.__viewer`, development builds only).
- **Acceptance (§14)** as a Playwright test tagged `@full`, run locally (GPU) and not in CI: the CPU runner would need
  ≈ 1 min of simulation.

## 14. MVP acceptance criterion (proposed)

From the editor, open **Cylinder Re=100**, Run on the GPU; in the job view the vortex street forms **live**; after
the run, scrub back to the start and play it again; the statistics over the periodic window read
**St within 5 % of 0.300 and C_D,max within 5 % of 3.23** (C_L,max shown for information). 5 % is the validation
suite's tolerance at this resolution (D = 20 cells, `--quick`); the report has St −1.2 % and C_D,max +4.9 % there,
and 2 % is only required at D = 80. The example runs 8 s; the shedding becomes periodic at ≈ 7 s, which leaves about
three periods — **proposed: `endTime` 12 s for the example** (≈ 15 periods in the window 6–12 s, ≈ 3 s of GPU time).
**Changed with the author (2026-10-21): `endTime` 20 s, a frame every 0.02 s.** Measured on CUDA, the shedding's
overshoot decays until ≈ 10 s, so the second half of a 12 s run reads C_D,max = 3.419 (+5.8 %, a fail); over 10–20 s
the run is periodic (as `cfd-cli compare`'s statistics already assume): St 0.2973 (−0.9 %), C_D,max 3.375 (+4.5 %).
1,000 frames (434 MB stored) instead of 1,200 at 0.01 s.
Plus the plan's MVP line: export a PNG, the force CSV and a video of the street.

## 15. Proposed order (each step tested and committed before the next)

0. Housekeeping (one commit each): the hand-timed Phase 4 acceptance run; folder paths; **reference scales** (§7.4,
   once decided): scene, schema, TS types, examples, estimate, editor field.
1. **Frame decoder:** `lffr.ts`, f16 table, the worker and its wrapper; Rust-written fixtures and the contract test.
2. **Viewer in the job view:** field renderer (`R16F`/`RG16F`), colour maps from Rust, legend, automatic/manual
   ranges, solid mask and fluid fraction from `scene.json`, zoom/pan, field selector; live stream at the viewer
   format.
3. **Timeline:** frame cache, `GET frames/{n}?fields=&maxSize=` on the server, play/pause/scrub/speed/Live.
4. Series: `series` messages, flushed CSVs, uPlot plots with the playhead cursor.
5. Statistics and references (St, C_D, C_L; generated references); `endTime` 12 s for the cylinder example.
6. Vectors and streamlines.
7. Tracer particles and dye.
8. Export: PNG, CSV, video.
9. `cfd-server` serves the app (`web` feature, `--web-dir`, `--open`); gallery "Open with results" and quick variants.
10. Nu (`heat.csv`) for the heated cavity.
11. Acceptance: Playwright (CI part and `@full`), ADR-031, docs (02 §2.1, 03 §6–7), plan, final check.

Steps 1–3 are the "first implementation steps" of this session; the session stops after them if it gets long.

## 16. Open questions — answered (2026-10-19): every proposal accepted

| # | Question | Proposal |
|---|----------|----------|
| Q1 | Reference scales: optional `reference` block in the scene (A) or values typed in the viewer (B)? (§7.4) | A, with a per-session override in the viewer |
| Q2 | LZ4 decoding: own TypeScript block decoder or `lz4js`; in a Web Worker or on the page's thread? (§2.1) | Own decoder, in one worker that also fetches stored frames |
| Q3 | Viewer format: every frame field at `maxSize` 1024 (setting) for both the stream and stored frames, with `fields`/`maxSize` added to `GET frames/{n}`? (§3.1) | Yes |
| Q4 | Frame buffer: in memory only (LRU, 512 MiB default), server as the store, no IndexedDB? (§3.2) | Yes |
| Q5 | Live series: new `series` WebSocket message + CSV flushed at each progress tick (a `JobObserver::sample` hook in Rust), or poll the CSV files? (§7.1) | `series` messages |
| Q6 | Statistics in TypeScript with a contract test against `cfd-verify`'s St, references generated from `cfd_verify::references`? (§7.2) | Yes |
| Q7 | Nu: per-frame `heat.csv` computed in Rust from the stored temperature (late in the order), or leave Nu for after the MVP? (§7.3) | `heat.csv`, step 10 |
| Q8 | Colour maps generated from Rust's `Colormap` (CLI PNGs and viewer identical), adding greys, magma and a blue-red? (§4.1) | Yes |
| Q9 | WebGL2 only in Phase 5 (no WebGPU path); streamlines on the CPU in the worker; particles and dye on the GPU? (§5) | Yes |
| Q10 | Export: PNG, CSV and MP4 (WebCodecs + `mp4-muxer`, MediaRecorder fallback); GIF and VTK left out? (§8) | Yes |
| Q11 | Gallery results generated on first open (reusing the latest run of the same scene), CPU quick variants, no precomputed results in the repo/releases? (§9) | Yes |
| Q12 | Serving the app: embedded with `rust-embed` behind a `web` feature, `--web-dir`, `--open`? (§10) | Yes |
| Q13 | Acceptance: St and C_D,max within 5 % at D = 20 cells (the suite's tolerance at that resolution), the cylinder example's `endTime` 8 → 12 s, the full check as a local `@full` Playwright test? (§14) | Yes (`endTime` changed to 20 s on 2026-10-21, §14) |
| Q14 | New dependencies: `uPlot` (ADR-005), `mp4-muxer` (export step); Rust: `rust-embed`, `webbrowser`. | Yes |
