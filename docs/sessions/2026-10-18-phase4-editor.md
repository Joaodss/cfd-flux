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
- **Until step 3** the tools paint solid cells without an element (id 0); the palette replaces this default.

## Decisions taken with the author

- Design proposal approved as written (2026-10-18): every proposal of §14 accepted (WebGL2 renderer,
  `deflate+base64` layers, explicit elements, Rust library, server validation with issue `params`, `PUT
  /api/scenes/{id}`, Playwright in CI, `immer`, the default new scene).
