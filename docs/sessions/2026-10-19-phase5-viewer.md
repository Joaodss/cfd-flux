# Session checklist — Phase 5, viewer and playback (2026-10-19/20, continued 2026-10-21)

Request: see [prompt-phase5.md](prompt-phase5.md). Branch `feat/phase5-viewer`. Design:
[phase5-design-proposal.md](phase5-design-proposal.md). Stop after the first implementation steps if the session
gets long, with everything tested and committed. Second session (2026-10-21): housekeeping, then steps 4–5 (stop
there if long; step 6 only with room left).

| # | Task | Status |
|---|------|--------|
| H1 | Record the author's hand-timed Phase 4 acceptance run | ✅ 9 min 30 s, misses < 2 min (recorded 2026-10-21; Q14) |
| H2 | Local folder moved: absolute paths, build/tests/e2e from the new place | ✅ `D:\GithubProjects\live-fluids` (2026-10-21, below) |
| H3 | Reference scales (proposal §7.4, option A) | ✅ |
| D | Phase 5 design proposal, open questions answered | ✅ every proposal of §16 accepted |
| 1 | Frame decoder (LFFR v2 in TypeScript, worker) and its contract test | ✅ |
| 2 | Viewer in the job view: WebGL2 field renderer, colour maps, legend, ranges, solid mask | ✅ |
| 3 | Timeline: frame cache, stored frames in the viewer format, play/pause/scrub/speed/Live | ✅ (end-of-run fix 2026-10-21) |
| 4 | Series: `JobObserver::sample`, `series` messages, flushed CSVs; Plots tab with uPlot | ✅ 2026-10-21 |
| 5 | Statistics: window, St/C_D/C_L, contract test, generated references, example benchmarks, cylinder `endTime` | ✅ 2026-10-21 (`endTime` 20 s, not 12 — below) |
| 6–8 | Vectors/streamlines, particles and dye, export | ✅ 2026-10-22 |
| 9 | `cfd-server` serves the app (`web` feature, `--web-dir`, `--open`); gallery: Open with results, quick variants | ✅ 2026-10-22 (below) |
| 10 | Nusselt number: `heat.csv`, Statistics tab, de Vahl Davis | ✅ 2026-10-22 (below) |
| 11 | Acceptance (`@full` Playwright test), ADR-031, docs | ✅ 2026-10-22 (below) |

## Found along the way

- **H1:** the author's hand-timed Phase 4 acceptance took **9 min 30 s** (target < 2 min): the walls, inlet and
  outlet were drawn by hand before the author found out that the domain edges are walls already and that an edge
  can be painted as an inlet/outlet in one action. Recorded in the plan and the Phase 4 log; the proposed fix
  (visible edge presets, or a channel template) is open question Q14 in `07`, not implemented.
- **H2:** the session ran in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (no `cfd-flux` folder on C:, D: or
  E:). No tracked file names an absolute path except the old session prompts (kept as written, Q12), and there is no
  launch configuration in the repository. Build, tests and e2e pass from this checkout (final check below).
  **2026-10-21:** the folder is now `D:\GithubProjects\live-fluids` (the session ran in a worktree under it). No
  tracked file or launch configuration names the old path (only the old session prompts and the line above, kept as
  written; the WSL example in `development.md` now uses the new place). From there, after `npm ci`: `cargo build
  --workspace` and `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace` (`CARGO_BUILD_JOBS=4`, now in
  `development.md`): **212 tests pass**; `npm test`: **143 pass**; `npm run typecheck` clean; `npm run test:e2e`:
  **2 pass** (the release server built from scratch in ≈ 3 min).
- **Reference scales (H3):** `Scene.reference` (optional length, velocity, ΔT) feeds `Dimensionless::reference` (Re,
  Ra with those scales), the estimate's `numbers.reference`, `meta.json` and the defaults of `cfd-cli compare
  --statistics`; the time step is unchanged (a test compares the unit systems). The cylinder example now reads
  Re = 100 in the editor (the solver's own scales, 615, are shown next to it). New issue code `reference.scale`.
- **Decoder (step 1):** Node's `Buffer.slice` shares memory (unlike `Uint8Array.slice`), so the uncompressed-codec
  path copied nothing and read the header bytes; the contract test caught it, the decoder copies explicitly now.
  Node 24 has `DataView.getFloat16`, so the f16 table is checked against the platform on all 65,536 values.
- **Viewer (step 2):** the first symmetric vorticity range was ±337 1/s and washed the wake out: `hello.ranges`
  (min/max, start-up transient included) had been taken as a percentile. Ranges seeded from `hello` now carry no
  percentile; the cylinder's ±p99 is ≈ ±50 1/s and the street is clear. Checked against a release server on CUDA
  and wgpu (1,450 and 816 MLUPS with the viewer attached).
- **Timeline (step 3):** a stored frame fetched with the stream's `maxSize` is byte-identical to the streamed frame
  on the CPU engine (Rust test), so live and stored frames share one cache. The browser pane stops
  `requestAnimationFrame` while the app window is hidden, so playback was verified by the Playwright test (headless
  Chromium, software WebGL), not by hand. **2026-10-21:** a fast GPU run ended with the playhead on frame 728 of
  1000: at the end of the run the timeline left Live at once, and the frames still being decoded in the worker were
  then not shown. The end of a live run now asks for the last frame (cache, stream or server); three full cylinder
  runs in a row end on 1000/1000.
- **Series (step 4):** `cfd-job` hands every sample to `JobObserver::sample` as the numbers of its CSV line (one
  formatting path: `f64`'s `Display`, so the step column stays an integer and the files are byte-identical to
  before) and flushes the files before each `progress`; the server batches the rows per job and sends them as a
  `series` message right before a (throttled) `progress` and before `done`, so every row a viewer was sent is on
  disk. A viewer that joins late reads the CSVs and appends the stream's rows past their last step (tests in Rust:
  rows equal the CSV lines, every row on disk at each progress, the stream's rows are the end of the files).
  `forces.csv` has a force for every element — the channel walls, inlet and outlet too — so the Plots tab shows one
  element, the first *body* (a wall element off the domain's edge) by default, with a selector.
- **Statistics (step 5):** the approved `endTime` of 12 s does not work: on CUDA the shedding's overshoot decays until
  ≈ 10 s, so the default window (the second half, 6–12 s) reads C_D,max = 3.419 (**+5.8 %**, outside 5 %). Over
  2 s windows C_D,max is 3.584 (4–6 s), 3.419, 3.383, 3.375, 3.373, …, 3.371 (22–24 s); St is 0.2973 throughout. The
  author chose 20 s with a frame every 0.02 s (below); `cfd-cli compare` already took its statistics from 10 s of
  a 20 s run. The St contract: a trimmed real `forces.csv` (8.5–10 s, 1,501 rows) and the frequency
  `oscillation_frequency` finds in it are committed; TypeScript finds the same St to 1e-12. References and the
  suite's tolerance by resolution (5 % at D = 20, 2 % at the finest size) come from `cfd-verify` through
  `cfd-cli references`.
- **Acceptance numbers (so far):** the cylinder example as shipped (20 s, CUDA, 120,000 steps in 1.9 s), default
  window 10–20 s: **St 0.2973 (−0.9 %) ✓, C_D,max 3.375 (+4.5 %) ✓**, C_L,max 1.040 (+4.0 %, information), 28
  periods, read in the Statistics tab after a run through the server.
- uPlot is not a React component: each chart is created in an effect and fed with `setData`; it lands in the job
  view's chunk (99 kB), the main chunk stays at 494 kB. happy-dom has no 2D canvas, so the component test replaces
  uPlot with a stand-in that records its data; the Playwright test checks the real canvases and the click-to-seek.
- A cleanup `git checkout` of a test file once threw away an uncommitted edit of it (re-applied at once); the
  e2e data directory held 1.3 GB of frames after the full-length runs and was deleted.

## Decisions taken with the author

- Design proposal approved as written (2026-10-19): every proposal of §16 accepted (reference scales in the scene,
  own LZ4 decoder in a worker, one viewer format for stream and stored frames, memory-only cache, `series` messages,
  statistics in TypeScript, `heat.csv` for Nu, colour maps from Rust, WebGL2 only, MP4 export, gallery results on
  first open, embedded web app, 5 % acceptance at D = 20 with `endTime` 12 s, the new dependencies).
- **Cylinder example: `endTime` 20 s and a frame every 0.02 s** (2026-10-21, instead of 12 s at 0.01 s): the second
  half is periodic and passes (St −0.9 %, C_D,max +4.5 %), 1,000 frames (434 MB) instead of 1,200. Recorded in the
  design proposal §14.

## Final check (2026-10-20)

- Web (`apps/web`): `npm run typecheck`, `lint`, `format:check`, `build` clean (the job view is its own chunk, so
  the main chunk stays at 492 kB); `npm test`: **143 tests pass** (was 113); `npm run test:e2e`: **2 pass** (Phase 4
  acceptance; the new viewer test), software WebGL.
- Rust: `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` and the `--no-default-features` run
  clean; `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **212 tests pass** (was 207): reference
  scales, the frame and colour-map fixtures, stored frames in a stream format.
- Not done: steps 4–11 of the proposal (§15); ADR-031 comes with the acceptance step.

## Final check (2026-10-21)

- Web (`apps/web`): `npm run typecheck`, `lint`, `format:check`, `build` clean (main chunk 494 kB, the job view
  99 kB with uPlot); `npm test`: **160 tests pass** (was 143): series buffer and feed, plots, elements, statistics,
  the St contract, references, the Plots and Statistics tabs; `npm run test:e2e`: **2 pass** (the viewer test now
  also checks the force plot, click to seek and the Statistics tab).
- Rust: `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` and `-p cfd-cli -p cfd-cuda
  --no-default-features` clean; `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace` (`CARGO_BUILD_JOBS=4`):
  **216 tests pass** (was 212): series rows and flushed files, `series` messages, references up to date, the St
  fixture.
- Not done: steps 6–11 of the proposal (§15: vectors and streamlines next); ADR-031 comes with the acceptance step.

## Session of 2026-10-22: steps 6–7 (vectors, streamlines, particles, dye)

- PR #16's CI was green (the Ubuntu fixture fix of 2026-10-21). The branch was already checked out in another
  worktree, so this session worked in a second one on the same commits and pushes to `feat/phase5-viewer`.
- **Vectors (step 6):** instanced `GL_LINES` — six vertices per arrow, the instance id is the position on a 24 px
  screen-space grid anchored to the domain, the vertex shader reads `RG16F` velocity and the fluid fraction with
  `textureLod`; arrows on non-fluid cells collapse outside the clip volume. The fastest speed seen so far maps to
  90 % of the spacing; the legend shows a reference arrow (1, 2 or 5 × 10ⁿ m/s). The arrows need the velocity
  even when another field is shown, so the renderer keeps a texture of its own for it.
- **Streamlines (step 6):** `viewer/streamlines.ts` (pure, runs in the decode worker): RK4 on the unit direction
  field (arc-length steps, so slow and fast flow use the same step), bilinear between block centres, seeds on a
  jittered grid in a seeded random order, a bucket grid of drawn points for the minimum separation, and stops at
  solids, edges, stagnation (1 % of the largest speed), the maximum length, other lines and loops that meet
  themselves. Tests: uniform flow (straight, collinear to 1e-4, spaced), solid-body rotation (radius error < 1e-4
  cells with 0.5-cell steps, both ends back at the seed after one turn), solid blocks, a saddle, determinism. The
  worker keeps the run's cells; requests carry a copy of the velocity. Playing: a new set every 4 frames; paused:
  at once; one request at a time.
- **Particles and dye (step 7):** `viewer/render/simulation.ts` (`SimLayer`): `EXT_color_buffer_float` is requested
  once; without it the checkboxes are disabled. Particles: 256 × 128 `RG32F` states (x < 0 = not released),
  midpoint rule, killed at non-fluid cells; released with probability 0.03 per pass at the fluid cells beside
  inlets (a texture of spawn points, subsampled to 16,384), along a drawn line, or anywhere (the default without
  inlets). Dye: `RGBA16F` at the frames\’ block resolution, three dyes; inlet stripes of each dye in turn,
  a brush, decay 0.4 /s. Time: simulated seconds per animation frame = playback frames per second × the frames\’
  interval, so a paused frame keeps carrying particles at that rate; passes are split to keep a step under 0.75
  cell. The Playwright test reads the released particle count and the dye total back from the float textures in
  SwiftShader.
- Not done: step 8 (export: PNG, CSV, video) and steps 9–11. `npm test`: **176 pass** (was 160); `npm run test:e2e`
  viewer test passes with the new overlays.

### Step 8: export (2026-10-22)

- `viewer/ExportPanel.tsx` (the *Export* tab) over `viewer/export/`: `csv.ts` (coefficients, statistics, the shown
  frame), `annotate.ts` (footer), `video.ts` (codec choice, `mp4-muxer`, `MediaRecorder` fallback), `download.ts`.
  New dependency: `mp4-muxer`.
- The domain image is drawn by resizing the WebGL canvas to `cells × scale`, drawing with view zoom 1 at a
  device-pixel ratio of `scale` (so arrows and particles scale too) and restoring the size in the same task.
- Video: `Viewer.exportVideo` pauses the timeline, loads every frame of the range (cache or server), waits for its
  streamlines, steps particles and dye by the frames\’ simulated interval and hands the composed canvas to the
  encoder; the timeline returns to its frame. Sizes are cropped to even numbers.
- Tests: CSV and footer text, codec choice with stand-in support, bit rates (Vitest); the Playwright test saves a
  PNG (IHDR size = domain × 2 + footer), the three CSVs and a five-frame video (MP4 `ftyp` when WebCodecs offers a
  codec, else WebM). `npm test`: **183 pass**; `npm run test:e2e`: **2 pass**.
- Not done: steps 9–11 (the server serving the app, Nu, acceptance and ADR-031). The MediaRecorder fallback and
  the video\’s image content were not looked at by eye.

## Session of 2026-10-22 (second): steps 9–10 (serving the app, gallery, Nusselt)

Branch `feat/phase5-viewer` was checked out in another worktree again; this worktree was reset to `origin/feat/phase5-viewer` and pushes `HEAD:feat/phase5-viewer`.

### Step 9: the server serves the app

- `crates/cfd-server/src/web.rs`: `Web::{None, Dir, Embedded}` in `Config`. The API routes are nested under `/api` with their own fallback, so an unknown `/api/…` path stays the JSON 404 and every other path goes to the static files; a small middleware sets `Cache-Control` (`immutable` under `assets/`, `no-cache` elsewhere, none on `/api`). `--web-dir` is `tower-http`'s `ServeDir` (no hand-rolled reads); the embedded files are `rust-embed` behind the Cargo feature `web` (`debug-embed`, so debug and release serve the same bytes), with an ETag and `304`. `--open` uses `webbrowser` once the socket is bound; `0.0.0.0`/`[::]` open `127.0.0.1`; a failure is a warning.
- Tests (`tests/web.rs`): a temporary `--web-dir` with `/`, an asset (content type and cache headers), unknown `/api…` paths (also `/api`), `..` traversal sent percent-encoded, POST refused; with `--features web` the embedded `/`, its ETag/`304` and its first asset. Unit test for the browser URL. CI: the web-e2e job now also runs `npm run build` and `cargo test --release -p cfd-server --features web --test web` (the release server it builds anyway, so the extra cost is one crate); documented in `development.md`.
- Rust idioms met: a Cargo feature is a compile-time `#[cfg(feature = …)]` switch (like `#if` with a define); `#[derive(RustEmbed)]` is a source generator that reads the folder when compiling; `Router::nest` is `MapGroup`; `middleware::from_fn` is `app.Use`.

### Step 9: gallery

- **Open with results:** migration `0002_scene_hash.sql` adds `jobs.scene_hash`: the SHA-256 of the validated scene's canonical JSON (the serialisation of the parsed `Scene`, so formatting does not matter) plus the options that change the result (everything but the engine). `POST /api/simulations?reuse=completed` answers `200` with the latest completed job of that hash whose results are still on disk, else starts one (`201`); other values of `reuse` are `request.reuse` (400). Test: new job, a second one while the first is running, reuse after completion, another engine/format still hits, other options/scene/no `reuse` do not, a purged job is skipped.
- **Quick variants** (`cfd_core::examples::quick`, `cfd-cli example NAME --quick`, `--all` writes them too, `scenes/*-quick.json`, tests keep them valid, round-tripping and current). CPU timings (16 threads, release):
  - channel 200 × 50 cells, 10 s: 5,000 steps, 0.7 s;
  - heated cavity 64² (the suite's N), 80 s: 58,104 steps, 5.9 s (kinetic energy within 0.1 % after ≈ 60 s);
  - **cylinder: half resolution cannot work.** D = 10 cells (τ = 0.5100) diverges at step 1,290 of 30,000 at the default lattice velocity (Ma 0.087; a higher lattice velocity breaks the Mach limit). 58 cells across the channel (D ≈ 14.1, dx = 7.07 mm, τ = 0.5141) is stable: 42,439 steps for 10 s in 6.1 s (123 MLUPS), C_L amplitude ≈ 1.1. It is a 0.7× coarsening, and the description says "a coarser grid".
  The gallery fetches `/api/capabilities` and, when `auto` is the CPU, offers the variants (a checkbox, on by default); the dam break has none and its button is disabled. Quick runs use the same benchmark as their example, with `toleranceAt` at their own resolution; the cylinder's St/C_D acceptance applies to the full example only.

### Step 10: Nusselt number

- `cfd_core::heat`: `wall_gradient` (the second-order one-sided difference at the half-way wall, moved from `cfd-verify`'s heated cavity, which calls it now; same expression, same results), `HeatWalls` (the cell faces of each counted element, found once from the scene) and `nusselt`. **Counted elements:** `Wall` and `HeatSource` with a `Fixed` (heat from the gradient, `-k Σ gradient`) or `Flux` (the prescribed flux × length) condition; adiabatic, convective (unsupported) and inlets/outlets are not (inlets carry heat by advection). Faces across periodic edges are not followed.
- `heat.csv` (`step, time_s, {element}.q_W_m, {element}.length_m`): written and flushed by the **output thread** from the stored frame's temperature before it tells the observer (`FrameEvent::heat`), so it is the same for every engine and every row a viewer is sent is on disk. The server adds it to the `series` messages (`rows.heat`) and `hello.series.heat`; `meta.json` says `run.heat` and gives `dimensionless.reference.conductivity`. Only thermal runs with stored frames (and a `temperature` frame field) have it. Tests: `cfd-job` recomputes every row from the frame files alone (bit-identical), `meta.json`, absent without frames/thermal walls; the server test streams a cavity run and checks, on each `series` message, that the file already holds the streamed rows.
- Statistics tab: Nu per wall = `(q / length) L / (k ΔT)` averaged over the window, with the spread ((max − min)/mean: "steady" is a number), compared with de Vahl Davis for the heated wall; the force table is hidden for a run without bodies. `references.json` gained `deVahlDavis1e5` (`nuHot` checked, `nuMean` for information; 2 % from `cfd-verify`'s own constant); the heated-cavity example names it, so does its quick variant.
- **Rayleigh number: Ra = 10⁵** — the suite's case `heated-cavity-ra1e5` (it also has 10³, 10⁴, 10⁶; the example's name was already Ra = 1e5, and the run's Ra from `meta.json` is 99,994.9 with ΔT = 0.9572 K, air, L = 0.1 m). The suite checks both Nusselt numbers within 2 % at N = 128 (full level); at N = 64 its table reads +0.84 %. The viewer applies 2 % at N ≥ 64.
- Numbers: the quick cavity (N = 64, CPU), window 40–80 s: **Nu_hot = 4.5416 (+0.72 % vs 4.509 ✓)**, Nu_cold = −4.5416, spread 0.83 % (still settling from above); the last frame 4.548. The full example on the CUDA engine passes the same check in the Playwright test.
- **Contract:** `apps/web/tests/fixtures/series/cavity-heat.csv` (a real run, 40–80 s, 40 rows) and `cavity-nusselt.json` (written by `UPDATE_FIXTURES=1 cargo test -p cfd-verify --test web_contract`); TypeScript's mean/min/max/spread agree to 1e-12. Only arithmetic is involved, no libm functions, so Linux CI computes the same numbers.
- Note: frames store temperature as f32 kelvin (≈ 3·10⁻⁵ K at 293 K); with ΔT ≈ 1 K the wall gradient of a single face has a rounding error of a few 0.1 %, which averages out over the faces (a synthetic linear profile shows 0.5 %, a real run is within the suite's 0.84 %).

### Final check (2026-10-22)

- Web: `typecheck`, `lint`, `format:check`, `build` clean; `npm test`: **201 pass** (was 183); `test:e2e`: the new gallery test passes (full cavity on CUDA; CI runs the quick variant on the CPU).
- Rust: `fmt --check`; `clippy --workspace --all-targets -D warnings`, `--no-default-features` and `-p cfd-server --features web` clean; `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace` all pass; `cargo test -p cfd-server --features web --test web` passes with a built `dist`.
- Not done: step 11 (acceptance, ADR-031); a Nu plot in the Plots tab; the CPU quick path of the gallery e2e was not run locally (no CPU-only server).

## Session of 2026-10-22 (third): step 11 (acceptance, ADR-031)

Branch `feat/phase5-viewer` was checked out in another worktree; this worktree was reset to `origin/feat/phase5-viewer` (`e8c1a73`) and pushes `HEAD:feat/phase5-viewer`.

- **Test:** `apps/web/e2e/acceptance-full.e2e.ts`, tagged `@full`. It follows the §14 criterion: the gallery's *Cylinder Re=100* (it asserts that the server's `auto` engine is a GPU, so the quick variant cannot be the one opened), Run, the job view, a `MutationObserver` installed before the page loads that records the job status and the timeline position at every change (the street forms live if the playhead moves while the status is not yet `completed`), the vorticity sign changes along the wake's centre line at the end (the street) and at frame 0 (none), scrub to 0 and play at 60 fps to the end, the Statistics tab (St and C_D,max against 0.300 and 3.23, ±5 %, and the tab's own ✓), then PNG (signature, IHDR size, IEND, size), the force CSV (header, rows, last time, C_D,max over 10–20 s against the tab), the run's `forces.csv` and a 51-frame video (MP4 `ftyp` or WebM EBML header, size).
- **How it is kept out of CI:** `playwright.config.ts` has `grepInvert: /@full/`, so `npm run test:e2e` lists 3 tests; `playwright.full.config.ts` spreads the same config, selects `@full` and allows 10 minutes; `npm run test:e2e:full`. Shared helpers (`pixel`, `SOLID`, `saved`) moved to `e2e/helpers.ts` (not matched by `testMatch`) and `viewer.e2e.ts` uses them.
- **Measured (RTX 4060, CUDA, twice):** **St 0.2973 (−0.9 %) ✓, C_D,max 3.375 (+4.5 %) ✓**, C_L,max 1.040 (information); 860 playhead positions seen before the job completed; 11 sign changes in the wake at the end; PNG 119,861 B (884 × 216); 10,001 coefficient rows and 10,001 `forces.csv` rows; MP4 154,358 B; the test takes ≈ 45 s. The thresholds are not tight to these values (≥ 3 live positions, ≥ 8 sign changes, ≥ 4 half way through, < 2 at frame 0).
- **Docs:** ADR-031; the plan's Phase 5 acceptance line; `CLAUDE.md` and `README.md` status; 02 §2.1 (the Runs bullet still said frames were ignored; the timeline and the cache were missing; the stack line), 03 §6.1 (it still offered `lz4js` for the decoder) and the `heat.csv` read of a late viewer in §7; `development.md` has `test:e2e:full`.
- **Not done / left open:** a Nusselt plot in the Plots tab; `heat.csv` has no download link in the Export tab (the run's files list is fixed); the MediaRecorder fallback and the video's picture were not looked at by eye; open question Q14 (editor discoverability, the hand-timed Phase 4 run) is unchanged. The main JavaScript chunk has grown to 500,590 bytes (494 kB at step 5), just past Vite's 500 kB warning; the job view is its own chunk (174 kB).

### Final check (2026-10-22, third session)

- Web (`apps/web`): `typecheck`, `lint`, `format:check`, `build` clean; `npm test`: **201 pass**; `npm run test:e2e`: **3 pass** (Phase 4 acceptance, the gallery's Nusselt number on CUDA, the viewer); `npm run test:e2e:full`: **1 pass**.
- Rust (`CARGO_BUILD_JOBS=4`): `cargo fmt --check`; `clippy --workspace --all-targets -D warnings`, with `--no-default-features` and `-p cfd-server --features web` clean; `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **230 tests pass**; `cargo test --release -p cfd-server --features web --test web` passes.
