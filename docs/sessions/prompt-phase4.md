Let's start **Phase 4 — the web editor (drawing board)** of the project in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the session log `docs/sessions/2026-10-16-phase3-kernels.md`, `docs/01-vision-and-requirements.md` §5.1 (FR-01–FR-08), `docs/02-architecture.md` §2.1 (frontend) and the scene-format paragraph, `docs/03-data-model.md` (the scene format, layer encoding `zstd+base64`, §7 API), ADR-005, ADR-009, ADR-010, ADR-011, ADR-024, ADR-027 and open question Q12 in `docs/07-decisions-and-questions.md`, the "Phase 4" and "Phase 5" sections of `docs/05-implementation-plan.md` and `docs/development.md`. Phase 3 is complete:
- `cfd-server` (axum + SQLite) validates scenes, queues jobs on one slot per device and streams frames over WebSocket.
- Frames are sampled on the GPU and encoded as LFFR v2 (byte shuffle + lz4).
- The stream-collide kernels skip the macroscopic stores on steps whose fields are not read.

The web app (`apps/web`) is still the Phase 0 skeleton: Vite + React 19 + TypeScript, oxlint, prettier, vitest, and scene types generated from `cfd-core::scene` (`npm run gen:types`). The scene format's source of truth stays the Rust types.

Session goals:
1. **Housekeeping first (small, one commit each):**
   - Q12 (answered: the project is `cfd-flux`): rename `live-fluids` in the docs, package descriptions, the web app and generated files. Leave the local folder name to me.
   - Commit the regenerated `apps/web/package-lock.json`: the committed one still says `web` 0.0.0. My main checkout has the updated file uncommitted.
   - Clear the two CI warnings: Node 20 deprecation for `actions/upload-artifact@v4`, and the `ubuntu-latest` → Ubuntu 26 migration on 2026-10-19 (check that the jobs still pass).
2. **Design proposal for Phase 4** (`docs/sessions/phase4-design-proposal.md`), with open questions for me, before any code. It should cover:
   - the editor's state model (layers as `Uint8Array`/`Uint16Array`, elements, global properties) and how it maps to `Scene`;
   - rendering of the grid at zoom levels from 1 px/cell to 32 px/cell, up to 4096² cells (Canvas2D vs WebGL2);
   - the tools (pencil/eraser/line/rectangle/ellipse/bucket/select-move-copy) and automatic element creation;
   - undo/redo (command or patch based; memory at 4096²);
   - the properties panels and the fluid/material library;
   - save/load (`.cfdscene` file, server `scenes` table, PNG import as a mask);
   - validation and estimates: reuse the server's `/api/scenes/validate` and estimates, or compile `cfd-core::validate` to WASM; issue `params` for localised messages (ADR-027);
   - how the editor submits a job and hands over to the viewer (Phase 5);
   - the test strategy (vitest for logic, a browser test for the acceptance scenario);
   - the libraries proposed (state, UI components), each justified against ADR-005.
   Go through the open questions with me and wait for my OK.
3. **Then implement in the proposal's order,** each step tested and committed before the next, up to the acceptance test: draw the cylinder-in-a-channel case from scratch in < 2 min and submit it without errors. Start with canvas + layers + zoom/pan and the drawing tools, then the palette/elements and the properties panels.
4. Docs (02 §2.1, 03 if the scene format or API changes, ADRs), plan checkboxes, session checklist and final check (`npm run typecheck`, `lint`, `format:check`, `test`, `build`, plus the Rust checks if anything in Rust changes).

How to work with me: as before. I have a strong CFD background, little Rust/GPU experience and little TypeScript/React experience (I come from C#):
- explain TypeScript and React idioms as they come up (components and hooks vs classes, state and re-rendering, typed arrays, the canvas/WebGL APIs, async in the browser), with C#/.NET parallels (WPF/XAML, `INotifyPropertyChanged`, `Span<T>`);
- propose designs before coding;
- the Rust scene types stay the source of truth (generated schema and TS types; tests fail if they are out of date);
- physical units in the scene, lattice units only in the solver;
- work on a branch (`feat/phase4-editor`), small commits, everything in the repository in English;
- stop after the first two implementation steps of goal 3 if the session gets long, with everything tested and committed.
