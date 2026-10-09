# Session checklist — English translation (2026-10-09)

Request: translate the whole project (code, docs, UI) to English for the author's CV / portfolio.

| # | Task | Status |
|---|------|--------|
| 1 | Switch to up-to-date `master`, create branch `docs/english-translation` | ✅ |
| 2 | Rename doc files and folders to English; fix references in code | ✅ |
| 3 | Translate UI strings (`App.tsx`, `index.html` lang) | ✅ |
| 4 | Translate `README.md`, `CLAUDE.md`, `sandbox/README.md` | ✅ |
| 5 | Translate docs 01–04 | ✅ |
| 6 | Translate docs 05–08 and `development.md` | ✅ |
| 7 | Translate session logs and the Phase 1 prompt | ✅ |
| 8 | Update rules: English as the project language (CLAUDE.md, ADR) | ✅ |
| 9 | Verify: no Portuguese left, all internal links valid, Rust + web checks green | ✅ |

**Final check:**
- No Portuguese text left in tracked files (search for common Portuguese words/old file names: 0 hits).
- 41 internal links and anchors checked, 0 broken.
- Rust: fmt, clippy `-D warnings`, 16 tests pass (native Windows/MSVC). Web: generated types unchanged, typecheck, lint, format, 6 tests, build pass.
- Also fixed while translating: the data-model doc now matches the implemented format; the planned physics validation suite is `cfd-cli verify` (the existing `cfd-cli validate` checks scene files); ADR-013 records English as the project language; Q12 (project name `cfd-flux` vs `live-fluids`) added.
- Not committed: waiting for the author.
