# Session checklist — Phase 5, viewer and playback (2026-10-19)

Request: see [prompt-phase5.md](prompt-phase5.md). Branch `feat/phase5-viewer`. Design:
[phase5-design-proposal.md](phase5-design-proposal.md). Stop after the first two implementation steps if the session
gets long, with everything tested and committed.

| # | Task | Status |
|---|------|--------|
| H1 | Record the author's hand-timed Phase 4 acceptance run | ⏳ waiting for the time |
| H2 | Local folder `cfd-flux`: absolute paths, build/tests/e2e from the new place | ⏳ see below |
| H3 | Reference scales (proposal §7.4, option A) | |
| D | Phase 5 design proposal, open questions answered | ✅ every proposal of §16 accepted |
| 1 | Frame decoder (LFFR v2 in TypeScript, worker) and its contract test | |
| 2 | Viewer in the job view: WebGL2 field renderer, colour maps, legend, ranges, solid mask | |
| 3 | Timeline: frame cache, stored frames in the viewer format, play/pause/scrub/speed/Live | |

## Found along the way

- **H2:** the session ran in `C:\Users\joaoa\Desktop\GithubProjects\live-fluids` (no `cfd-flux` folder on C:, D: or
  E:). No tracked file names an absolute path except the old session prompts (kept as written, Q12), and there is no
  launch configuration in the repository.

## Decisions taken with the author

- Design proposal approved as written (2026-10-19): every proposal of §16 accepted (reference scales in the scene,
  own LZ4 decoder in a worker, one viewer format for stream and stored frames, memory-only cache, `series` messages,
  statistics in TypeScript, `heat.csv` for Nu, colour maps from Rust, WebGL2 only, MP4 export, gallery results on
  first open, embedded web app, 5 % acceptance at D = 20 with `endTime` 12 s, the new dependencies).
