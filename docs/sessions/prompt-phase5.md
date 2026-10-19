Let's start **Phase 5 — visualisation and playback (the MVP)** of the project in `<project folder>` (repo github.com/Joaodss/cfd-flux, branch `master`).

Context: first read `CLAUDE.md`, the session log `docs/sessions/2026-10-18-phase4-editor.md`, `docs/01-vision-and-requirements.md` §5.3 (FR-20–FR-24), `docs/02-architecture.md` §2.1 (frontend: editor, documents, validation, runs, viewer) and §2.6, `docs/03-data-model.md` §6 (LFFR v2 frames, stream frames, stored frames, the result layout) and §7 (API, WebSocket messages and stream formats), ADR-005, ADR-007, ADR-010, ADR-026, ADR-028 and ADR-030 in `docs/07-decisions-and-questions.md`, the "Phase 5" section of `docs/05-implementation-plan.md` and `docs/development.md`. Phase 4 is complete (PR #13):
- the editor (`apps/web`) draws scenes on a WebGL2 canvas, saves `.cfdscene` files (`deflate+base64` layers) and server scenes, validates through `/api/scenes/validate` (issues with `params`/`at`, an estimate with Re, Ma, τ), and runs them;
- the job view (`#/jobs/{id}`) follows the job's WebSocket (status, progress, pause/resume/cancel) but ignores the frames: it asks for the smallest stream (`fields=velocity&maxSize=64`);
- a Playwright acceptance test runs in CI (`web-e2e`, software WebGL), and the wgpu readback fix (PR #14) is merged.

Session goals:
1. **Housekeeping first (small, one commit each):**
   - Record my hand-timed Phase 4 acceptance run (I will give you the time) in the plan and the session log.
   - The local folder is now `cfd-flux` (moved to another disk): fix any absolute path that still names the old location (docs, launch configs), and check that the build, the tests and the e2e test run from the new place.
   - Reference scales: the editor shows the solver's Reynolds number (largest boundary velocity × shorter side of the fluid's bounding box), so the "Cylinder Re=100" example reads Re ≈ 615. Phase 5 needs the same choice for C_D, C_L, St and Nu. Propose (in the design below, before any code) whether the scene gets an optional reference length/velocity/temperature difference (`cfd-core::scene`, schema, editor field, examples) or the viewer asks for them.
2. **Design proposal for Phase 5** (`docs/sessions/phase5-design-proposal.md`), with open questions for me, before any code. It should cover:
   - frame decoding in the browser: LFFR v2 (header, byte unshuffle, LZ4 block) in TypeScript or `lz4js`, in a Web Worker or not; a contract test against frames written by `cfd-io` (as the Phase 4 scene fixture);
   - the frame buffer: live stream vs stored frames (`GET …/frames/{n}?dtype=f16`), memory budget at 1024²/4096², IndexedDB or not, which stream format (`fields`, `maxSize`) the viewer asks for;
   - WebGL2 rendering: f16 textures (`R16F`/`RG16F`), colour maps (viridis, coolwarm, …), automatic/manual ranges (the stream's `ranges`), legend, the solid mask from the scene's layers, and what it reuses from the editor's renderer and view (zoom/pan);
   - derived quantities and overlays: field selector, vectors, streamlines (RK4 on the client), tracer particles/smoke on the GPU;
   - the timeline: play/pause/scrub/speed over stored frames while the run continues;
   - probe and force plots (uPlot) from `probes.csv`/`forces.csv` (or new endpoints/stream messages), statistics (St, mean C_D and C_L amplitude, Nu) and how they are checked against the validation references (Schäfer & Turek, de Vahl Davis);
   - export (PNG, CSV, video with WebCodecs/MediaRecorder);
   - the quick-examples gallery with precomputed or low-resolution results (< 10 s): where the results live (repository, release assets, generated on first run);
   - `cfd-server` serving the built web app (`--open`, ADR-010), so a release is one binary;
   - the test strategy (Vitest for decoding and maths, Playwright for the viewer on software WebGL in CI) and the acceptance criterion for the MVP (e.g. run the cylinder Re 100 and watch the vortex street live, scrub it, and read St and C_D within the 04 §7 tolerances).
   Go through the open questions with me and wait for my OK.
3. **Then implement in the proposal's order,** each step tested and committed before the next, up to the acceptance test. Start with the frame decoder (with its contract test), the viewer's WebGL2 colour map with the solid mask in the job view, and the timeline.
4. Docs (02 §2.1, 03 if the API or formats change, ADRs), plan checkboxes, session checklist and final check (`npm run typecheck`, `lint`, `format:check`, `test`, `build`, `test:e2e`, plus the Rust checks if anything in Rust changes).

How to work with me: as before. I have a strong CFD background, little Rust/GPU experience and little TypeScript/React experience (I come from C#):
- explain TypeScript, React and WebGL idioms as they come up (workers and transferable buffers, typed arrays and half floats, shaders and textures, `requestAnimationFrame`), with C#/.NET parallels;
- propose designs before coding;
- the Rust types stay the source of truth (generated schema and TS types; tests fail if they are out of date);
- physical units in the scene, lattice units only in the solver;
- work on a branch (`feat/phase5-viewer`), small commits, everything in the repository in English;
- stop after the first two implementation steps of goal 3 if the session gets long, with everything tested and committed.
