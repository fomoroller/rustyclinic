# TODOS

## P1 — Blocks Near-Term Implementation

### ~~Per-Subsystem Design Docs (Sync Protocol + Auth/Session)~~
**DONE** — 2026-03-20. Written to `docs/designs/sync-protocol.md` and `docs/designs/auth-session.md`.

### ~~ID Generation Strategy~~
**DONE** — 2026-03-20. Decision: UUIDs v7 for all aggregates. Documented in architecture.md invariants table.

### ~~Form Engine Design Doc~~
**DONE** — 2026-03-20. Written to `docs/designs/form-engine.md`.

### ~~Apply Architecture Amendments from Reviews~~
**DONE** — 2026-03-20. All 13 decisions from CEO + Eng reviews applied to architecture.md, plus 6 accepted expansions (observability, fleet ops, power resilience, training, compliance, accessibility).

### Dual-Backend Integration Test Strategy
Define and implement a test harness that runs every repository test against both SQLite and PostgreSQL, ensuring identical behavior. The dual-backend is the highest-risk architectural decision — without systematic testing, bugs surface as production data inconsistencies between nano/micro (SQLite) and standard (PostgreSQL) deployments. Strategy is now documented in architecture.md CI/CD section; implementation needed during Phase 1 repository layer buildout.
- **Effort:** M (human) → S (CC)
- **Depends on:** Phase 1 repository layer
- **Source:** CEO Review 2026-03-19, Section 6

## P2 — Blocks Platform Phase

### Package Format Specification
Define the concrete package file format, signing infrastructure, and development workflow. The architecture specifies what packages contain and their activation rules, but not their physical format. Unblocks the package SDK and Phase 4.
- **Effort:** M (human) → S (CC)
- **Depends on:** Architecture review complete
- **Blocks:** Phase 4, developer experience expansion
- **Source:** CEO Review 2026-03-19, architecture-qa.md Q7
