# Session checklist — Phase 5, viewer and playback (2026-10-19/20)

Request: see [prompt-phase5.md](prompt-phase5.md). Branch `feat/phase5-viewer`. Design:
[phase5-design-proposal.md](phase5-design-proposal.md). Stop after the first implementation steps if the session
gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| H1 | Record the author's hand-timed Phase 4 acceptance run | ✅ 9 min 30 s, misses < 2 min (recorded 2026-10-21; Q14) |
| H2 | Local folder moved: absolute paths, build/tests/e2e from the new place | ✅ `D:GithubProjectslive-fluids` (2026-10-21, below) |
| H3 | Reference scales (proposal §7.4, option A) | ✅ |
| D | Phase 5 design proposal, open questions answered | ✅ every proposal of §16 accepted |
| 1 | Frame decoder (LFFR v2 in TypeScript, worker) and its contract test | ✅ |
| 2 | Viewer in the job view: WebGL2 field renderer, colour maps, legend, ranges, solid mask | ✅ |
| 3 | Timeline: frame cache, stored frames in the viewer format, play/pause/scrub/speed/Live | ✅ |
| 4–11 | Series and plots, statistics, overlays, particles, export, serving the app, gallery, Nu, acceptance | next session |

## Found along the way

- **H2:** the session ran in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (no `cfd-flux` folder on C:, D: or
  E:). No tracked file names an absolute path except the old session prompts (kept as written, Q12), and there is no
  launch configuration in the repository. Build, tests and e2e pass from this checkout (final check below).
  **2026-10-21:** the folder is now `D:GithubProjectslive-fluids` (the session ran in a worktree under it). No tracked
  file or launch configuration names the old path (only the old session prompts and the line above, kept as
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
  Chromium, software WebGL), not by hand.
- `cargo test --workspace` with the default 16 parallel jobs ran out of paging file on this machine (rustc
  "paging file is too small"); `CARGO_BUILD_JOBS=4` works.

## Decisions taken with the author

- Design proposal approved as written (2026-10-19): every proposal of §16 accepted (reference scales in the scene,
  own LZ4 decoder in a worker, one viewer format for stream and stored frames, memory-only cache, `series` messages,
  statistics in TypeScript, `heat.csv` for Nu, colour maps from Rust, WebGL2 only, MP4 export, gallery results on
  first open, embedded web app, 5 % acceptance at D = 20 with `endTime` 12 s, the new dependencies).

## Final check (2026-10-20)

- Web (`apps/web`): `npm run typecheck`, `lint`, `format:check`, `build` clean (the job view is its own chunk, so
  the main chunk stays at 492 kB); `npm test`: **143 tests pass** (was 113); `npm run test:e2e`: **2 pass** (Phase 4
  acceptance; the new viewer test), software WebGL.
- Rust: `cargo fmt --check`; `cargo clippy --workspace --all-targets -D warnings` and the `--no-default-features` run
  clean; `CFD_REQUIRE_GPU=1 CFD_REQUIRE_CUDA=1 cargo test --workspace`: **212 tests pass** (was 207): reference
  scales, the frame and colour-map fixtures, stored frames in a stream format.
- Not done: steps 4–11 of the proposal (§15); ADR-031 comes with the acceptance step.
