# Session checklist — Phase 4, web editor (2026-10-18)

Request: see [prompt-phase4.md](prompt-phase4.md). Branch `feat/phase4-editor`, draft PR
[#13](https://github.com/Joaodss/cfd-flux/pull/13). Design: [phase4-design-proposal.md](phase4-design-proposal.md).
Stop after the first two implementation steps if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| H1 | Rename `live-fluids` → `cfd-flux` (Q12) | ✅ |
| H2 | Commit the regenerated `apps/web/package-lock.json` | ✅ |
| H3 | CI: `ubuntu-26.04`, `upload-artifact@v7` | ✅ Linux jobs pass on 26.04 without warnings |
| D | Phase 4 design proposal, open questions answered | ✅ every proposal of §14 accepted |
| 1 | Shell and canvas: layers, WebGL2 renderer, zoom/pan, status bar | ✅ |
| 2 | Drawing tools and history | ✅ |
| 3 | Palette and elements: active elements, automatic creation, eyedropper, element list, probes | ✅ |
| 4 | Properties panels (Element, Scene, Run) and the fluid/material library (`cfd_core::library`) | ✅ |
| 5 | Save/load: `deflate+base64`, `.cfdscene` files, server save/open with `PUT`, draft autosave, PNG import, example gallery | ✅ |
| 6 | Validation: issue `params`/`at` and a fuller estimate in Rust, debounced validation, issue list and canvas markers, estimate in the Run tab | ✅ |
| 7 | Run: submit, job view at `#/jobs/{id}` (WebSocket progress, pause/resume/cancel), back to the editor, runs list | ✅ |
| 8 | Acceptance: Playwright scenario (+ `web-e2e` CI job), ADR-030, docs, final check | ✅ scripted run passes in ≈ 3–4 s; hand timing is the author's |

## Found along the way

- The server's default database file is now `<data>/cfd-flux.db`: an existing `data/live-fluids.db` is not
  opened any more (rename it to keep the job and scene history).
- **Renderer (step 1):** a 4096² document uploads (`R8UI` + `R16UI` textures) and draws in 83 ms; changing a
  64×64 tile and redrawing takes 3 ms; read-back pixels match single changed cells exactly. The browser pane
  reports `MAX_TEXTURE_SIZE` 16384.
- `fzstd` decodes the layers `ruzstd` writes (every example scene round-trips through `fromScene`).
- **Tools (step 2):** shapes are drawn live into the layers through a revertible `Stroke` (redrawn on every
  move), so the preview is exactly the result and no preview renderer is needed; the Canvas2D overlay only draws
  the selection outline and the brush cursor. Tools report operations through `ctx.commit` (the select tool
  commits a floating selection on pointer-down when the click lands outside it). A 20 × 20 ellipse drag gives
  exactly the cells of the example cylinder (the disk of `examples.rs`). At 4096²: a half-domain bucket fill
  173 ms, its undo 68 ms, and the entry costs 128 kB (uniform tiles stored as one value).
- **CI (`cfd-job`):** `cancel_stops_a_long_run_promptly` compared the frame files with `run.frames` (output
  events) instead of `run.framesStored`; a cancel drops frames still queued for the output thread, so it failed
  when one was queued (first seen on ubuntu-26.04). Fixed in the test; the runner behaves as documented.
- **Palette (step 3):** the paint is derived from the palette's kind and that kind's active element; a painting
  stroke with no (existing) active element creates one first, and the creation rides on the stroke's history entry
  (table before/after + tiles), so one undo removes both; a stroke that changes nothing removes the element again.
  Undo can remove an active element: the next stroke then creates a new one (ids are max + 1, so it gets the same
  id back). Probes are table-only history entries. Cell counts per element are recomputed per history change (one
  pass over the id layer), not per pointer move.
- **CI:** one run on ubuntu-26.04 failed the wgpu parity case `taylor_green_vortex` with `status readback failed`
  (no Rust change on the branch; an earlier run of the same code passed). Probable cause: `Mapping::wait` ignores
  the result of `device.poll(Wait)`, so a poll that returns before the map callback leaves the staging buffer
  unmapped. Job re-run; a separate task was proposed to make the readback wait robust.

- **Properties (step 4):** fields commit on Enter/blur (one history entry per edit, so no merging of
  keystrokes was needed); while unfocused they show the store's value, so undo needs no synchronisation code.
  A grid resize or refinement replaces the layers object; its history entry keeps both grids and undo swaps
  them, while tile entries recorded before it still apply to the old grid (undo replays in order).
- **Bug found by the new tests:** `commitTable` read `s.table` inside an immer recipe, so the history kept a
  draft proxy (revoked when the recipe ends) and every edit looked like a change; it now reads the table with
  `get()` outside the recipe. Earlier probe undo tests passed only because they never redid a probe edit.
- Component tests (Testing Library on happy-dom) cover the number field, the inlet and thermal forms and the
  Run tab; the tests' tsconfig gained the DOM and Vite types, JSX and JSON modules.

- **Documents (step 5):** `deflate+base64` in `cfd-core` (miniz_oxide, already in the tree; inflation bounded by
  the layer size) and in the editor (native `CompressionStream`/`DecompressionStream`); a sample made by Node's
  `CompressionStream` is a Rust unit test, and an editor-written cylinder file is committed and checked in Rust
  (same layers and table as the example) — semantic, not byte, equality, so zlib versions do not matter. The
  editor's cylinder file is 2.9 kB (the zstd example: 3.1 kB). Checked against a running server: gallery, save to
  server, save again (PUT: one record, renamed), reopen, autosave/restore after a reload, PNG import.
- An imported image is centred in the view but kept inside the grid when it fits (with the view off the domain,
  the block was clipped at the edge). ajv and the schema load on demand: the main chunk went from 604 to 467 kB.
- `toScene` writes the keys in the Rust serializer's order, so editor files read like the CLI's.

- **Validation (step 6):** `Report::error`/`warning` return the new issue, so call sites chain `.param(..)`
  and `.at(x, y)`; `cell.element` keeps its code and gains a `problem` param (five kinds) instead of new codes.
  `/validate` returns the estimate whenever the scene could be prepared (also on `limits.*` and `engine.*`).
  A vitest test reads `cfd-core`'s sources and fails if an issue code has no wording in `src/messages.ts`.
- **Not obvious:** the Reynolds number shown is the solver's own (`Dimensionless`: the largest boundary or
  initial velocity and the shorter side of the fluid's bounding box), so the "Cylinder Re=100" example shows
  Re ≈ 615 (1.5 m/s peak × 0.41 m / 1e-3 m²/s), not the benchmark's 100 (mean velocity × diameter).
- Checked against a running server: the cylinder validates on CUDA ("Ready to run", Re and Ma in the status bar);
  an inlet cell inside the cylinder gives one error at that cell with `problem: noFluidNeighbour`.

- **Runs (step 7):** checked end to end against a release `cfd-server`: the cylinder submitted from the editor
  runs on CUDA, the job view follows it through Vite's WebSocket proxy (≈ 1,060–1,250 MLUPS on the 442 × 84
  grid), pause/resume/cancel take effect, "Editor" returns to the same drawing, `#/jobs` lists the run and
  reopens it. Progress messages carry no frame count, so the view shows frames once a run is over.
- Component tests drive the job view with a stand-in `WebSocket` class (`vi.stubGlobal`) and `fetch` stubs.

- **Acceptance (step 8):** the Playwright scenario draws the cylinder in a channel through the UI and the result
  equals `cfd-core`'s example cell for cell (cell types and element ids 1–4); validation reports no issues and
  Re = 615; the run completes. Chromium uses SwiftShader (software WebGL): headless Chromium no longer falls back
  to it alone, and without WebGL2 the canvas cannot start (CI runners have no GPU).

## Decisions taken with the author

- Design proposal approved as written (2026-10-18): every proposal of §14 accepted (WebGL2 renderer,
  `deflate+base64` layers, explicit elements, Rust library, server validation with issue `params`, `PUT
  /api/scenes/{id}`, Playwright in CI, `immer`, the default new scene).

## Final check (2026-10-19)

- Web (`apps/web`): `npm run typecheck`, `lint`, `format:check`, `build` clean; generated types up to date;
  `npm test`: **113 tests pass** (logic, components on happy-dom, contracts with Rust); `npm run test:e2e`: the
  acceptance scenario passes (≈ 3–4 s, software WebGL), also as the new `web-e2e` CI job.
- Rust: `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` and the `--no-default-features`
  run (cli, cuda, engines, job, server): clean. `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`:
  **207 tests pass** (was 199): the library, `deflate+base64`, the editor-written fixture, issue params and cells,
  `PUT /api/scenes/{id}`, the extended estimate.
- **Hand-timed acceptance run (recorded 2026-10-21): 9 min 30 s, missing the < 2 min target.** The author drew the
  walls, inlet and outlet by hand (pencil, then rectangles) before discovering that the domain edges are walls
  already and that an edge can be painted as an inlet or outlet in one action. The scripted run shows the tools can
  draw the scene; the miss is discoverability. Proposed fix (not implemented): open question Q14 in
  [07](../07-decisions-and-questions.md#open-questions).
- **Edge presets (Q14 option B, 2026-10-27, branch `feat/editor-edge-presets`, issue #23):** the canvas overlay
  draws a band outside each domain edge, coloured and named by stretch (wall, periodic dashed, or the inlet/outlet
  element drawn on the edge cells), so a new scene shows walls all around; the Scene tab's "Domain edges" group
  lists each edge's contents and has "Make inlet" / "Make outlet": a new element (an inlet points into the domain)
  painted on the edge's cells except drawn walls and heat sources, one undo entry, the Scene tab staying open for
  the next edge. With the channel walls drawn first, the buttons give the same cells as the scripted drags (a
  Vitest test, and a second Playwright variant of the acceptance test, both equal to `cfd-core`'s example). Without
  walls drawn, an edge inlet also takes the two corner cells, which then show in the neighbouring edges' summaries.
  Checks: `npm run typecheck`, `lint`, `format:check`, `build`; `npm test` 215 tests; `npm run test:e2e` 5 tests
  (both acceptance variants). **Re-timed hand run (2026-10-27): 2 min 03 s** against < 2 min (was 9 min 30 s), on
  a second attempt: the first one went into finding which properties needed changing, in no
  particular order. The author suggested an automatic side wall too (the two wall rectangles were what was left to
  draw), unsure whether it helps beyond this scene.
- **Make wall (2026-10-27, same branch):** a third button per edge (the buttons now read Wall / Inlet / Outlet;
  three "Make …" labels did not fit the panel) paints the edge, corners included, with the active wall element, so
  bottom and top become one "Wall 1" and `N`/+ starts another — the same mechanism serves a cavity's moving lid or
  heated side walls, not only this channel. It keeps inlets, outlets and heat sources; drawn walls on the canvas
  are now labelled by element name. `npm test` 219 tests; `npm run test:e2e` 6 tests (the acceptance scenario in
  three variants: all dragged, walls dragged with the inlet/outlet buttons, all buttons). Not re-timed by hand.
- Not done / follow-ups: the wgpu readback fix
  (`fix/wgpu-readback-wait`, its own session) is not on this branch; `cfd-server` does not serve the built app yet
  (Phase 5, Q10); the Reynolds number shown uses the solver's characteristic scales (see above).
