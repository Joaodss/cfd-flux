# Phase 4 — web editor (drawing board): design proposal (2026-10-18)

**Status: approved by the author on 2026-10-18, as proposed** (every proposal of §14 accepted). Implemented on `feat/phase4-editor`; the accepted
parts move into [02 §2.1](../02-architecture.md#21-frontend--appsweb), [03](../03-data-model.md) and new ADRs as
they are implemented, and the plan's Phase 4 tasks follow §13.

Goal of Phase 4 ([plan](../05-implementation-plan.md#phase-4--web-editor-drawing-board)): draw and configure
scenes in the browser (FR-01–FR-08). **Acceptance:** draw the cylinder-in-a-channel case from scratch in < 2 min
and submit it without errors.

## 1. Overview

```
 apps/web (browser)
 ┌──────────────────────────────────────────────────────────────────────────────────────────┐
 │ React components (panels, toolbar, dialogs)          imperative code (no React)            │
 │  ┌───────────┐ ┌──────────┐ ┌──────────────┐        ┌───────────────────────────────────┐ │
 │  │ Toolbar   │ │ Palette  │ │ Properties   │        │ Document: pixel layers            │ │
 │  │ (tools)   │ │ elements │ │ element/scene│        │  cellType Uint8Array              │ │
 │  └─────┬─────┘ └────┬─────┘ └──────┬───────┘        │  elementId Uint16Array            │ │
 │        │ select     │ read/update  │                │  (fluidId Uint8Array, later)      │ │
 │        ▼            ▼              ▼                └──────▲──────────────┬─────────────┘ │
 │  ┌──────────────────────────────────────────┐   tools write│ tiles        │ dirty tiles   │
 │  │ Zustand store: scene table (elements,    │◀── history ──┤              ▼               │
 │  │ fluids, run, …), UI state, issues        │   (§5)       │   ┌───────────────────────┐ │
 │  └──────────────┬───────────────────────────┘              │   │ WebGL2 renderer (§3)   │ │
 │                 │ debounced                         pointer│   │ integer textures +     │ │
 │                 ▼                                   events │   │ palette, grid lines    │ │
 │  ┌──────────────────────────┐                              │   └───────────────────────┘ │
 │  │ API client (fetch, WS)   │                       ┌──────┴──────────┐                  │
 │  └────────────┬─────────────┘                       │ Tools (§4)      │ ← canvas events  │
 └───────────────┼─────────────────────────────────────┴─────────────────┴──────────────────┘
                 │ /api/scenes/validate, /api/simulations, /api/scenes  (Vite proxy in dev)
                 ▼
            cfd-server (127.0.0.1:7878)
```

- **Two worlds, as ADR-005 requires:** pixels live in typed arrays owned by plain TypeScript objects; React only
  renders panels. A stroke never re-renders a component. *(C#/WPF: think of the canvas as a `WriteableBitmap`
  you write into directly, while the panels are XAML bound to a view model.)*
- **The scene table** (everything in `Scene` except `layers`) is small and lives in a Zustand store as immutable
  plain objects. The `Scene` JSON is assembled only to save, validate or submit (§2.2).
- **The server does the physics checks** (§8): it is always running next to the editor (ADR-010), and
  `cfd-core` stays the only implementation of validation and unit conversion.

### React and TypeScript in two paragraphs (for this design)

- A **component** is a function that returns UI for the current state; React calls it again (re-renders) when
  its state or props change, and patches the DOM with the difference. There are no long-lived control objects
  as in WPF: what persists between calls is kept by **hooks** (`useState`, `useRef`, `useEffect`), which React
  stores per component instance. `useRef` is a mutable box React does not watch — that is where the canvas
  element and the renderer live.
- A **Zustand store** is a singleton object with state and actions; components subscribe to a *slice*
  (`useEditor(s => s.elements)`) and re-render only when that slice changes (compared by reference, which is why
  the state is updated immutably — a new object instead of a mutated one). Non-React code reads and writes it
  with `useEditor.getState()` / `setState()`. *(C#: a view model implementing `INotifyPropertyChanged`, with
  each binding listening to one property; immutable updates are like C# `record` + `with`.)*

## 2. Editor state and its mapping to `Scene`

### 2.1 The document

```ts
// Pixel data: plain objects outside React. index = y * width + x, y = 0 at the bottom (03 §3).
class Layers {
  readonly width: number
  readonly height: number
  readonly cellType: Uint8Array     // CellType: 0 fluid, 1 solid, 2 inlet, 3 outlet, 4 heat source, 5 empty
  readonly elementId: Uint16Array   // 0 = none
  // fluidId: Uint8Array | null     — later (multi-fluid is rejected by the solver today, §14 Q9)
  version: number                   // bumped on every change (cheap "has it changed" for validation)
  readonly dirty: TileSet           // 64×64 tiles changed since the renderer last uploaded
}

// Everything else: the Zustand store, immutable values typed by the generated `Scene` types.
type SceneTable = Omit<Scene, 'layers'>      // name, grid, fluids, solidMaterials, elements, probes,
                                             // initial, physics, run, schemaVersion
interface EditorState {
  table: SceneTable
  ui: { tool: ToolId; brush: number; activeKind: PaletteKind; activeElement: Record<Kind, number | null>;
        selectedElement: number | null; view: { zoom: number; panX: number; panY: number } }
  validation: { status: 'idle' | 'pending' | 'ok' | 'invalid' | 'offline'; issues: Issue[]; estimate?: Estimate }
  // actions: setTool, updateElement(id, patch), addElement(kind), … each one goes through the history (§5)
}
```

- **Typed arrays** are fixed-size arrays of one numeric type over a raw `ArrayBuffer` — the closest thing to C#'s
  `byte[]`/`ushort[]` (and `subarray` is a view without a copy, like `Span<T>.Slice`). Ordinary JS arrays of
  numbers would cost 8+ bytes per cell and lose the type.
- **Element ↔ cell type** coupling, enforced by the tools so a drawn scene is valid by construction (03 §3):
  painting an element writes both its cell type and its id (wall → solid, inlet, outlet, heat source); painting
  fluid or empty writes the cell type and id 0. The palette (§6) never lets the two disagree.
- **Probes** are a list in the table (`probes`), placed by the probe tool, drawn as markers.

### 2.2 To and from `Scene`

- `toScene(table, layers): Scene` encodes the layers (§7.1) and adds them to the table; elements without cells
  are kept (an element whose cells were all erased can be drawn again; §14 Q4).
- `fromScene(scene)` decodes the layers (`raw+base64`, `zstd+base64` and the new encoding of §7.1), checks their
  length and splits the table off. Loading a file also runs it through the JSON Schema with `ajv` (already a dev
  dependency; it becomes a runtime one) so a broken file is rejected with a readable message.
- The generated types stay the contract: the editor compiles against `src/generated/scene.ts`, so a change in
  `cfd-core::scene` that the editor does not follow fails `npm run typecheck`.
- **Coordinates:** tools work in scene cells (y up); only the renderer and the pointer-to-cell conversion know
  that the screen's y points down.

## 3. Rendering: WebGL2

| | Canvas2D | WebGL2 (proposed) |
|---|---|---|
| Data path | layers → an RGBA `ImageData` mirror (4 B/cell: 64 MB at 4096²) → `putImageData` → `drawImage` scaled with smoothing off | layers uploaded as they are into integer textures (`R8UI` cell type, `R16UI` element id) with `texSubImage2D` per dirty tile |
| Recolouring (element colour, highlight the selected element, show only one kind) | repaint the RGBA mirror: every cell (≈ 50–80 ms at 4096² in JS) | change a small palette texture or a uniform: free |
| Grid lines, zoom 1–32 px/cell | a second pass of lines per frame (thousands of `lineTo` at high zoom) | computed in the fragment shader from the cell coordinate (`fract`) |
| Code shared with the viewer (Phase 5) | none (the viewer needs WebGL2 for float16 textures and colour maps) | camera (zoom/pan), texture upload, the overlay of solids |
| Complexity | lower | a ≈ 150-line renderer (two shaders, one quad) |

**Proposal: WebGL2** for the cells and grid, with a transparent **Canvas2D overlay** on top for tool previews
(the rectangle being dragged, selection outline, probe markers, issue markers). Details:

- One quad covering the domain; the vertex shader applies the view transform (zoom, pan, y flip); the fragment
  shader fetches `cellType` and `elementId` with `texelFetch` (nearest, no filtering of integer textures) and
  looks the colour up in a palette texture indexed by element id (element colours: a hue per element, shaded by
  kind), or by cell type for fluid/empty. Grid lines when the zoom is ≥ 8 px/cell; domain border always.
- Zoom: 1–32 px/cell in steps of √2 around the pointer (wheel), pan with middle button / space+drag, "fit" on
  load. Zoom < 1 (a 4096² domain on a 1080p screen) is allowed for an overview: the shader then picks one cell
  per pixel (no averaging; a 1-cell wall may flicker at zoom < 1 — acceptable for an overview).
- Texture size: WebGL2 guarantees only `MAX_TEXTURE_SIZE ≥ 2048`; desktop GPUs report 16384. If a device reports
  less than the grid, the renderer splits the layers into texture tiles (implemented only if needed; a clear
  message otherwise).
- Rendering is on demand (`requestAnimationFrame` when something changed), not a loop.
- *WebGL in one line:* you upload data into GPU textures/buffers once, and every frame you run two small programs
  (vertex and fragment shaders, GLSL ES 3.0) over a rectangle; the same model as the WGSL kernels, but for pixels.

## 4. Tools

All tools implement one interface and work in cell coordinates; they write through a `Stroke` that records the
tiles it touches for the history and the renderer (§5). Painting writes the **active paint**: (cell type,
element id) from the palette.

```ts
interface Tool {
  down(p: Cell, e: Modifiers): void
  move(p: Cell, e: Modifiers): void
  up(p: Cell, e: Modifiers): void
  preview?(overlay: CanvasRenderingContext2D, view: View): void   // shapes while dragging
}
```

| Tool | Behaviour | Algorithm |
|------|-----------|-----------|
| Pencil `B` | square brush of size 1–64 (round option), continuous strokes | stamp along a Bresenham line between pointer events (fast mouse moves leave no gaps) |
| Eraser `E` | pencil that paints the background (fluid, or empty with a free surface) | same |
| Line `L` | drag; Shift snaps to 0/45/90° | Bresenham, stamped with the brush |
| Rectangle `R` | drag; filled or outline (brush width); Shift = square | direct fill |
| Ellipse `O` | drag a bounding box, Alt = from the centre; filled or outline; Shift = circle | midpoint ellipse; filled by spans, exact for odd and even sizes |
| Bucket `G` | fills the 4-connected region of equal (cell type, element id) | scanline flood fill (no recursion; ≈ 100 ms at 16 M cells) |
| Select `M` | rectangle selection; drag to move, Alt+drag or Ctrl+C/V to copy, Delete to clear, arrows nudge | the selection is a floating patch on the overlay until dropped (Enter/click outside) |
| Probe `P` | click to place, drag to move, Delete to remove | adds to `probes` |
| Eyedropper `I` (Alt+click in any tool) | picks the clicked cell's element as the active paint | — |

- **Pixel-exact shapes:** a rectangle from (x0, y0) to (x1, y1) covers exactly those cells; the cylinder of the
  acceptance case (D = 20 cells) is an ellipse tool drag with Shift, and the size readout in the status bar shows
  cells *and* metres (`20 × 20 cells = 0.100 m`).
- **Status bar:** cursor cell `(x, y)`, physical position in metres, cell type/element under the cursor, zoom,
  drawing size while dragging.
- Keyboard shortcuts as above, plus `Ctrl+Z`/`Ctrl+Shift+Z` (`Ctrl+Y`), `F` fit, `+`/`−` zoom, `[`/`]` brush size.

### Automatic element creation

Each palette kind (wall, inlet, outlet, heat source) has an **active element**. Drawing with a kind that has none
creates one with defaults (`Wall 1`: no-slip, adiabatic; `Inlet 1`: uniform 0.1 m/s along +x, the first fluid;
`Outlet 1`: pressure 0 Pa; `Heat source 1`: fixed 350 K) and selects it in the properties panel. **New element**
(`N`, or the `+` next to the kind) makes the next stroke start another element of that kind — e.g. a second
wall that is heated. Clicking an element in the palette list (or the eyedropper) makes it active.

*Alternative considered:* one element per connected region, detected automatically. It surprises when two regions
touch (they merge and one loses its properties) or a region is split by the eraser; explicit elements are
predictable and match what the solver needs (properties per element id). §14 Q3.

## 5. Undo/redo

**Patch based, by tiles, with swap.** The domain is split into 64×64-cell tiles. An operation (a stroke, a fill,
a move, a property edit, a resize) is one history entry:

```ts
interface Entry {
  label: string                              // "Rectangle", "Inlet 1: velocity" — for the Edit menu
  tiles?: Map<number, TileData>              // pixel tiles touched: their content *before* the operation
  table?: { before: SceneTable; after: SceneTable }  // immutable, so just two references
}
```

- A `Stroke` copies a tile the first time the operation writes into it (copy-on-write); untouched tiles cost
  nothing. **Undo swaps** each stored tile with the layers' current content, so the entry then holds the "after"
  data and redo swaps back — one copy per tile, not two.
- A tile is 4 KiB of cell types + 8 KiB of element ids; a tile that is one value (common: fills, fresh domains)
  is stored as that value only.
- **Memory at 4096²:** layers 48 MB (+16 MB with `fluidId`), GPU textures the same. A bucket fill of the whole
  domain stores 4,096 tiles ≤ 48 MB (≈ nothing if the area was uniform); a typical stroke a few tiles. The
  history is capped by a **byte budget (256 MB)** and by 200 entries; the oldest entries go first.
- Table edits are entries too, so *creating an element and painting its first cells* is one undo step, and
  typing in a property field merges consecutive edits of the same field into one entry (no undo per keystroke).
- *Alternative:* command based (store "rectangle (x0,y0,x1,y1) with element 3" and re-run) — small, but a flood
  fill's result depends on everything before it, so undo would need replay from a snapshot; patches are simpler
  and exact.

## 6. Panels, palette and the fluid/material library

Layout (desktop first; the editor is not designed for phones):

```
┌──────────────────────────────────────────────────────────────────────────────────────────┐
│ cfd-flux  [New] [Open] [Save ▾] [Examples ▾] [Import PNG]      issues ●2 ▲1   [▶ Run]   │
├────┬───────────────────────────────────────────────────────────────┬─────────────────────┤
│tool│                                                               │ Palette             │
│ B  │                                                               │  ■ Fluid            │
│ E  │                    canvas (WebGL2 + overlay)                  │  ■ Wall ▸ Walls  +  │
│ L  │                                                               │  ■ Inlet ▸ Inlet 1 +│
│ R  │                                                               │  ■ Outlet           │
│ O  │                                                               │  ■ Heat source      │
│ G  │                                                               │  ■ Empty (gas)*     │
│ M  │                                                               │  ● Probe            │
│ P  │                                                               ├─────────────────────┤
│    │                                                               │ Properties          │
│    │                                                               │ [Element][Scene][Run]│
├────┴───────────────────────────────────────────────────────────────┴─────────────────────┤
│ (212, 41) = 1.060 m, 0.205 m · Wall "Cylinder" · 8 px/cell · 442 × 84 cells · Re 100 · Ma 0.09 │
└──────────────────────────────────────────────────────────────────────────────────────────┘
                                                     * only when physics.freeSurface is on
```

- **Element tab** (selected element): name, colour, cell count, and the fields of its kind, driven by the
  generated discriminated unions — `velocity.type` picks the form (`noSlip`/`slip`/`moving` for walls,
  `uniform`/`parabolic` for inlets), `thermal.type` likewise, `material`/`fluid` from the scene's lists. Options
  the solver rejects today (`slip`, `convective`, …, the `support.*` issues) stay selectable and show the issue.
  *(TypeScript narrows a union by its tag: inside `if (v.type === 'moving')` the compiler knows `v.velocity`
  exists — like a C# `switch` on a type pattern, but checked on a string field.)*
- **Scene tab:** name, description; grid (width × height, cell size, the physical size it gives, edges
  wall/periodic); fluids and solid materials (list + library picker + custom values); initial conditions;
  physics (gravity, thermal, buoyancy, free surface).
- **Run tab:** end time, output interval, output fields, method, precision, backend preference — and the
  estimate (§8): steps, frames, memory, stored bytes, time, engine, Re, Ma, τ.
- **Number fields** show the SI unit, accept `1e-3` and `0.005`, and commit on Enter/blur; invalid text keeps the
  last value and shows why. Units stay physical: the editor never converts to lattice units (CLAUDE.md).
- **Resizing the grid** keeps the drawing anchored at the bottom-left cell (cropping or extending with
  background) and is one undo entry; "scale drawing" (resample by an integer factor, to refine a mesh) is a
  separate action (§14 Q8).

### Fluid and material library

Preset values at 20 °C / 1 atm, with their sources in a comment: fluids **air, water, engine oil (SAE 30),
glycerol, ethanol**; solids **aluminium, copper, steel, glass, wood (pine), concrete**. Choosing a preset fills
the fields and sets `preset`; editing a value afterwards keeps the name but clears `preset` ("custom").

**Proposal: the library lives in Rust** (`cfd_core::library`, used by the examples — `examples.rs` already has
its own `air()`), exported as `apps/web/src/generated/library.json` by `cfd-cli library` with an up-to-date test
like the schema's. One source of truth for the values the CLI, the examples and the editor use (§14 Q6).

## 7. Save and load

### 7.1 Layer encoding: add `deflate+base64`

The editor must send compressed layers (Phase 3 Q6: a 4096² scene is 48 MB raw, 64 MB in base64, over the
64 MB body limit), but browsers have no built-in zstd *encoder*; they do have **`CompressionStream('deflate-raw')`**
(native, asynchronous, in every current browser). Options:

| Option | Browser side | Rust side | Cost |
|--------|--------------|-----------|------|
| **A. New encoding `deflate+base64` (proposed)** | native `CompressionStream`; decoding `zstd+base64` scenes (examples, CLI output) with `fzstd` (pure TS, ≈ 8 kB gz) | `miniz_oxide` (pure Rust) to decode/encode | one enum variant, schema and types regenerated |
| B. zstd in WASM (`zstd-wasm` ≈ 300 kB) | WASM module loaded on save | none | a WASM dependency for one function |
| C. A minimal zstd encoder in TS (RLE + raw blocks) | ≈ 200 lines, poor ratio on non-uniform rows | none | own codec to maintain |

With A, `.cfdscene` files and submissions from the editor use `deflate+base64`; `cfd-cli` keeps writing
`zstd+base64`; both are read everywhere. Deflate on drawn layers: the cylinder's two layers (37 k cells) shrink
from 111 kB to ≈ 1 kB; the ratio limit of deflate (≈ 1000:1) only matters for empty 4096² domains (48 kB).

### 7.2 Files, server, browser storage

- **`.cfdscene`** = the `Scene` JSON, exactly as `cfd-cli validate` reads it (a different extension so the OS
  can associate it; the content is plain JSON). Save: the File System Access API (`showSaveFilePicker`) where
  available (Chromium: "Save" overwrites the same file), a download elsewhere. Open: file picker or drag and
  drop onto the canvas; `.json` accepted too.
- **Server:** "Save to server" → `POST /api/scenes` (`201 { id }`); "Open from server" lists `GET /api/scenes`.
  The API has no update, so saving twice makes two records; proposal: add `PUT /api/scenes/{id}` (§14 Q5).
- **Draft autosave** to IndexedDB every few seconds after a change (the scene encoded as for a file), offered
  back on the next start — a browser crash or refresh never loses a drawing.
- **Examples** (FR-08): the four scenes of `scenes/` bundled by Vite (already imported by the Phase 0 page),
  opened from the Examples menu as a new document.

### 7.3 PNG import as a mask

Import → the image is decoded by the browser (`createImageBitmap` → `getImageData`), converted to a mask
(pixels darker than a threshold, or alpha > 50%; a slider with a live preview), optionally scaled to a target
width in cells (nearest), and dropped as a **floating selection** painted with the active paint — positioned
with the select tool like a pasted block, then committed (one undo entry). No server involvement.

## 8. Validation and estimates

**Proposal: the server's `POST /api/scenes/validate`, debounced, as the single implementation.**

- After an edit, when the editor has been idle for 400 ms, it encodes the scene and posts it (a newer edit
  aborts the request in flight with an `AbortController`); the status dot shows pending/ok/issues/offline.
  For the cylinder this takes milliseconds; at 4096² encoding and validation should take a fraction of a second (to be measured) — still
  off the drawing path (`CompressionStream` is asynchronous and `fetch` never blocks the UI thread).
- **Why not WASM now:** compiling `cfd-core` (validation, units, `Scene → Domain`) to WASM would give instant
  checks without a server, but the editor cannot submit without the server anyway (ADR-010: it is always local),
  and WASM adds a toolchain (`wasm-bindgen`, `wasm-pack`) to the web build and CI. It becomes worth it for the
  GitHub Pages demo (ADR-010's suggestion) or interactive previews — then the same `validate` compiles to WASM
  unchanged. A few **instant client checks** that need no physics stay in TS because they are about the drawing
  itself: no inlet/outlet drawn yet, a probe outside the domain, the server unreachable (§14 Q7).
- **Issues on the canvas:** issues get an optional cell location, so the editor can zoom to "inlet 2 touches no
  fluid" and mark the cells.
- **Changes in Rust** (03 §7, ADR-027 follow-up):
  - `Issue` gains `params` (a JSON object: `{"element": 2, "mach": 0.31, "limit": 0.3}`) and optional `at`
    (`[x, y]`); the English `message` stays as the fallback. The editor has a message table keyed by `code` that
    formats `params` (English only for now; translations are just more tables).
  - `Estimate` gains the dimensionless numbers and lattice parameters already computed in `Domain`
    (`re`, `ma`, `pr`, `ra`, `tau`, `dt`, `latticeVelocity`), and `validate` returns the estimate whenever the
    scene could be prepared — also when the only failure is a server limit or a missing engine, so the Run tab
    can show why ("17,000 steps over the limit").

## 9. Submitting a job and the hand-over to the viewer (Phase 5)

- **Run** is enabled when the last validation is `ok` (warnings allowed, listed in the confirmation). It posts
  `POST /api/simulations { scene, options }` (options from the Run tab: backend override, store frames) and
  gets the job (`201`, `Location`). `422`/`429` show their `issues`/message in the issues panel.
- The app then switches to the **job view** at `#/jobs/{id}` — a hash route, so reload and back work without a
  router library and without server-side routes. In Phase 4 this view is a placeholder that opens the WebSocket
  and shows status, progress (step, %, MLUPS, Mach) and pause/resume/cancel; Phase 5 puts the WebGL2 viewer in
  the same place. *(Async in the browser is single-threaded like a UI thread with `async/await` everywhere:
  `fetch` and WebSocket events are callbacks on the event loop, so no locks — but long synchronous loops, like a
  16 M-cell flood fill, block painting until they return.)*
- The editor document stays in memory (and in the draft autosave): **Back to editor** returns to the same
  drawing to change it and run again. A "Jobs" list (`GET /api/simulations`) lets the user reopen earlier runs.
- **Dev setup:** Vite proxies `/api` (HTTP and WebSocket) to `127.0.0.1:7878`, so the app always uses relative
  URLs; in a release the same relative URLs work once `cfd-server` serves the built app (ADR-010 — not done yet;
  §14 Q10).

## 10. Tests

| Level | Tool | What |
|-------|------|------|
| Logic | Vitest (node) | raster algorithms (Bresenham, ellipse spans incl. even sizes, scanline fill, brush stamps) against small hand-checked grids; tiles and history (undo/redo/swap, byte budget, merging); `toScene`/`fromScene` round trip of every example scene (zstd in, deflate out, identical layers); PNG mask thresholds; view transform (screen ↔ cell, zoom about the pointer); message formatting of issue `params` |
| Contract | Vitest + Rust test | the editor writes a scene fixture (`apps/web/tests/fixtures/drawn-cylinder.cfdscene`, committed); a `cfd-core` test reads and validates it, and the library/schema up-to-date tests cover the generated files — both sides of the `deflate+base64` encoding tested against each other |
| Components | Vitest + Testing Library + happy-dom | property forms switch with the union tag, number fields parse and commit, the palette creates elements |
| Browser | Playwright (ADR-005) | the **acceptance scenario**: start `cfd-server` (CPU engine, temporary data dir), open the editor, new 442 × 84 grid at 5 mm, draw walls, inlet (parabolic), outlet, the cylinder with the ellipse tool, set the fluid, check validation is clean, Run, see the job reach `running` and stream progress, cancel. Plus a pixel check of the renderer (read back a few canvas pixels) and undo/redo through the UI |

- WebGL does not exist in Node, so the renderer is only exercised by Playwright (Chromium, software GL in CI).
- The scripted acceptance test proves the flow works; the "< 2 min by hand" criterion is a manual check by the
  author, timed, recorded in the session log.
- **CI:** a new `web-e2e` job builds `cfd-server` (cached) and runs Playwright with Chromium (§14 Q11).

## 11. Libraries (against ADR-005)

| Library | Why | ADR-005 |
|---------|-----|---------|
| `zustand` | the store of §2: selectors limit re-renders; usable from tools and the renderer outside React | listed |
| `immer` (with Zustand's middleware) | write `draft.elements[i].velocity.value[0] = 0.2` and get an immutable update — nested discriminated unions are tedious to copy by hand | new, small (≈ 5 kB); optional (§14 Q12) |
| `tailwindcss` 4 (`@tailwindcss/vite`) | styling with utility classes, no CSS files per component | listed |
| shadcn/ui on `radix-ui` | accessible primitives (dialogs, menus, popovers, tabs, select, tooltip, slider); shadcn is not a dependency but component source copied into `src/components/ui/`, so it can be edited | listed |
| `lucide-react` | the icon set shadcn uses (tool icons) | comes with shadcn |
| `fzstd` | decode `zstd+base64` layers of existing scenes (§7.1) | new, tiny, pure TS |
| `ajv` | schema check when opening a file (already used by the tests) | moves to `dependencies` |
| `@playwright/test`, `@testing-library/react`, `happy-dom` | §10 | Playwright listed; the others are its usual companions for component tests |

Not proposed: a router (hash routes are two views), a form library (a dozen fields with one `NumberField`
component), a data-fetching library (a 50-line typed `fetch` wrapper; reconsider for the Phase 5 jobs list),
a canvas framework (Konva/Pixi — the renderer is one quad).

## 12. Source layout

```
apps/web/src/
  app/          App.tsx, layout, routes (#/ editor, #/jobs/:id), keyboard shortcuts
  editor/
    layers.ts   Layers, tiles, dirty tracking
    history.ts  entries, swap undo, budget
    tools/      pencil.ts, line.ts, shapes.ts (rect, ellipse), bucket.ts, select.ts, probe.ts, raster.ts
    render/     renderer.ts (WebGL2), shaders.ts, overlay.ts, view.ts (zoom/pan math)
    store.ts    Zustand store: table, ui, validation; actions
    elements.ts defaults, colours, kind ↔ cell type
  scene/        codec.ts (layers ↔ base64/deflate/zstd), io.ts (toScene/fromScene, files, IndexedDB), png.ts
  api/          client.ts (fetch, errors), validate.ts (debounce), jobs.ts, stream.ts (WS messages)
  panels/       Palette.tsx, ElementPanel.tsx, ScenePanel.tsx, RunPanel.tsx, Issues.tsx, StatusBar.tsx
  components/ui shadcn components
  generated/    scene.ts, library.json (generated, do not edit)
  messages.ts   issue code → message
```

## 13. Proposed order (each step tested and committed before the next)

1. **Shell and canvas:** Tailwind + shadcn setup, app layout, `Layers` + tiles, WebGL2 renderer with zoom/pan,
   grid lines and status bar; opens the example scenes (`fzstd` decoding) to have something to look at.
2. **Drawing tools + history:** raster algorithms, pencil/eraser/line/rectangle/ellipse/bucket/select-move-copy,
   overlay previews, tile history with undo/redo.
3. **Palette and elements:** kinds, active elements, automatic creation, eyedropper, element list with colours
   and cell counts, probes.
4. **Properties panels and library:** element forms, Scene and Run tabs, `cfd_core::library` + generated JSON.
5. **Save/load:** `deflate+base64` in `cfd-core` (schema, types), `.cfdscene` files, server save/open (+ `PUT`
   if accepted), draft autosave, PNG import, example gallery.
6. **Validation and estimates:** issue `params`/`at` and the extended estimate in Rust (03 §7), debounced
   validation, issues panel and canvas markers, Run tab estimate.
7. **Submit and job view:** Run, job placeholder view with progress over WebSocket, back to editor, jobs list.
8. **Acceptance:** Playwright scenario (+ CI job), timed manual run by the author; docs (02 §2.1, 03, ADRs),
   plan checkboxes.

Steps 1–2 are the "stop here if the session gets long" point.

## 14. Open questions — answered (2026-10-18): every proposal accepted

| # | Question | Proposal |
|---|----------|----------|
| Q1 | Renderer: WebGL2 (§3) or Canvas2D (what 02 §2.1 says today)? | WebGL2 + a Canvas2D overlay; 02 §2.1 updated |
| Q2 | Layer encoding from the browser: new `deflate+base64` (A), zstd in WASM (B) or own TS zstd encoder (C)? | A (`miniz_oxide`, native `CompressionStream`; `fzstd` to read zstd) |
| Q3 | Elements: explicit active element per kind with automatic creation of the first (§4), or one element per connected region? | Explicit, auto-created on first use, `N` for a new one |
| Q4 | Elements whose cells were all erased: keep them (listed as "0 cells"), or drop them on save? | Keep; show "0 cells" and a delete button (the validator could warn: new code `element.unused`) |
| Q5 | Add `PUT /api/scenes/{id}` so "Save to server" updates a scene instead of adding a copy? | Yes |
| Q6 | Fluid/material library in Rust (`cfd_core::library` → generated JSON) or only in TypeScript? | Rust, one source for the CLI, the examples and the editor |
| Q7 | Validation: the server's endpoint now (debounced), WASM later (Pages demo)? | Yes; plus issue `params`/`at` and the extended estimate in Rust (§8) |
| Q8 | Grid resize: anchor at the bottom-left and crop/extend; plus a separate "scale drawing ×k"? | Yes to both |
| Q9 | `fluidId` (initial fluid per cell): skip in Phase 4 (the solver rejects multi-fluid: `support.multiFluid`), keeping the palette's "Empty (gas)" for free-surface scenes? | Skip; FR-03's "initial fluid region" is the fluid/empty split for now, `fluidId` painting comes with multiphase (Phase 6b) |
| Q10 | `cfd-server` serving the built web app (ADR-010: one binary, `--open`) — in Phase 4 or Phase 5? | Phase 5, when the viewer makes it worth using alone; Phase 4 uses the Vite dev server |
| Q11 | Playwright acceptance test in CI (a new job that builds `cfd-server`, ≈ 3–4 min cached), or locally only? | In CI, CPU engine |
| Q12 | `immer` for store updates: yes, or plain immutable spreads? | Yes |
| Q13 | Default new scene: size and contents (e.g. 400 × 200 at 5 mm, air, all fluid, edges walls)? | 400 × 200, 5 mm, air, edges walls, `endTime` 10 s, output every 0.05 s |
