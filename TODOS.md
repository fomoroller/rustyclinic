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

### ~~Dual-Backend Integration Test Strategy~~
**DONE** — 2026-03-21. TestBackend trait + backend_test! macro. PostgreSQL repo implementations behind `postgres` feature flag. 16 backend-agnostic tests run against SQLite always, PG when `RUSTYCLINIC_PG_URL` is set. Full PG migration file (migration_pg.rs).

## P2 — Blocks Platform Phase

### ~~Package Format Specification~~
**DONE** — 2026-03-21. `.rcpkg` binary format with magic bytes, JSON header, Ed25519 signature, zstd-compressed tar payload, SHA-256 checksums. PackageBuilder + PackageReader + signing infra. 8 new tests including full round-trip.

### Terminology + Interop Design Doc
**DONE** — 2026-03-24. Written to `docs/designs/terminology-interop.md`. Captures the current goals and rollout for ICD-11, SNOMED CT, LOINC, UCUM, FHIR boundary APIs, and terminology import pipelines.

## P3 — Next Terminology / Interop Steps

### Full Official Terminology Ingestion
- Import official ICD-11 exports through WHO-compatible workflows.
- Import official LOINC release archive after authenticated download.
- Import official SNOMED CT RF2 release for licensed deployment territory.
- Add artifact checksums, release metadata, and version visibility in admin status UI.

### Terminology-Aware Clinical UX
- Replace local diagnosis shortcuts with searchable terminology-backed selection.
- Use imported LOINC/UCUM bindings in vitals, lab order, and result-entry screens.
- Persist clinician label plus code-system metadata consistently across diagnoses, observations, medications, and orders.

### Interop Hardening
- Expand FHIR export coverage beyond the current minimal surfaces.
- Add profile-aware validation and deployment-specific terminology bindings.
- Document package-driven overrides for terminology subsets and national/local mappings.
