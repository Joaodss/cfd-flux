# Session checklist — Phase 4, web editor (2026-10-18)

Request: see [prompt-phase4.md](prompt-phase4.md). Branch `feat/phase4-editor`, draft PR
[#13](https://github.com/Joaodss/cfd-flux/pull/13). Design: [phase4-design-proposal.md](phase4-design-proposal.md).
Stop after the first two implementation steps if the session gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| H1 | Rename `live-fluids` → `cfd-flux` (Q12) | ✅ |
| H2 | Commit the regenerated `apps/web/package-lock.json` | ✅ |
| H3 | CI: `ubuntu-26.04`, `upload-artifact@v7` | ✅ committed; CI pending |
| D | Phase 4 design proposal, open questions answered | ⏳ waiting for the author |
| 1 | Shell and canvas: layers, WebGL2 renderer, zoom/pan, status bar | — |
| 2 | Drawing tools and history | — |

## Found along the way

- The server's default database file is now `<data>/cfd-flux.db`: an existing `data/live-fluids.db` is not
  opened any more (rename it to keep the job and scene history).
