# TODOS

Working plan for the remaining build. The architecture phase table in
`architecture.md` is the map; this file is the route. Phases 1–4 are done
(kernel, web core, offline/sync, forms/packages) plus the service layers of
5–7. What remains is organized as four milestones, each independently
shippable and each ending in something demoable.

Sizes: S = a session, M = a few sessions, L = a week-plus of sessions.

## M1 — Clinic completeness (finish Lane A)

Goal: a nurse can run an entire clinic day without leaving the app.
This closes the Phase 5/6 gaps between service layer and UI.

### 1. Appointments & scheduling — L
The one Phase 5 aggregate that doesn't exist at any layer.
- `Appointment` aggregate + state machine (`booked → confirmed → arrived → fulfilled` plus `no_show`, `cancelled`), repo (SQLite + PG), service commands
- Agenda projection; day/week views; book/reschedule/cancel from patient detail
- Arrival flow feeds the existing queue (`arrived` → enqueue)
- Acceptance: book → arrive → queue → encounter round-trip in the web UI; no-show visible on the agenda

### 2. Immunization — M
- Immunization records on encounters (vaccine, dose, lot, route) with terminology bindings
- Vaccine schedules delivered as a program package (EPI schedule as the reference pack)
- Due/overdue projection surfaced on patient detail
- Acceptance: record a dose from an encounter; patient card shows history + next due

### 3. Admissions, referrals, programs UI — M
Service layer and state machines exist; build the screens.
- Bed board (admit/transfer/discharge), referral create/track, program enrollment list + status transitions
- Acceptance: every transition offered by the service layer is reachable from a screen

### 4. Billing UI — L
Service layer (claims, payments, waivers, eligibility) exists; no screens.
- Invoice creation from encounter charges; record payment (cash first); grant waiver with reason
- Claims worklist (draft → validated → batched → submitted) driven by the existing state machine
- Day-end cash summary (printable)
- Mobile money: stub behind a trait; real adapters (MTN MoMo) deferred to M3
- Acceptance: encounter → invoice → payment → receipt print, plus a claim reaching `submitted`

## M2 — Deployment hardening (what a real pilot needs before real patients)

Goal: the gap between "works on my laptop" and "safe at a clinic in Rwanda".

### 5. Encryption at rest — M
Architecture mandates SQLCipher; currently plaintext SQLite.
- Switch rusqlite to `bundled-sqlcipher-vendored-openssl`; key from facility key + device salt (envelope model per architecture.md)
- Migration path for existing unencrypted DBs (`admin migrate --encrypt`)
- Acceptance: DB file unreadable without key; all tests pass on encrypted DB

### 6. Backup verification & scheduled jobs — M
Jobs crate is a generic lease queue with no registered jobs yet.
- Register real jobs: nightly backup, restore-verification drill, op-log prune, projection rebuild
- `admin diagnose`: DB integrity, audit hash-chain verify, sync state, credential freshness (the command architecture.md promises)
- Acceptance: kill the process mid-write, restart, diagnose reports clean; backup restores on a second machine

### 7. Auth hardening — M
- Login rate limiting / lockout with audit
- Offline credential expiry enforcement (14-day default per spec) + revocation delta on reconnect
- Break-glass access: explicit, time-bounded, reason-coded, audited
- Acceptance: brute-force test locks out; expired offline credential forces re-auth; break-glass leaves an audit trail

### 8. Localization — L
Deployment target is Kinyarwanda/French-speaking; currently English-only.
- Fluent bundles wired through templates; `en` + `rw` + `fr`
- Locale-aware dates/numbers; language toggle per session (shared devices)
- Translated strings shipped as part of deployment packages
- Acceptance: full registration → encounter flow usable in all three languages

### 9. Observability — S
- Request/trace IDs propagated through logs and audit
- `/metrics` endpoint (queue depth, sync lag, projection lag, job failures)
- Acceptance: one curl shows the metrics the architecture's "Key Metrics" table lists

## M3 — Network & fleet (many devices, one district)

Goal: more than one device, imperfect connectivity, someone responsible for a fleet.

### 10. Sync transports beyond HTTPS — L
- Signed USB export/import (encrypted bundle, signature check, return file)
- LAN peer sync (mDNS discovery + mTLS) for clinic-to-clinic while both offline
- Acceptance: two laptops sync patient data via USB stick with no network

### 11. Device lifecycle & fleet basics — L
- Device registration (CSR → admin approval → cert), suspension, revocation
- Fleet health view on the control plane: last sync, pending conflicts, package drift
- Acceptance: revoked device is refused sync and receives wipe command on reconnect

### 12. PostgreSQL productionization — M
- PG feature in CI (service container) so parity is enforced per-commit, not just locally
- Row-level security policies for facility scoping
- Acceptance: full test suite green against PG in CI; cross-tenant read provably blocked

### 13. DHIS2 push pipeline — M
Export exists; make it a durable workflow.
- Job-based push with retries/backoff, acknowledgement tracking, failure review UI
- Acceptance: simulated DHIS2 endpoint outage recovers without data loss or duplicates

### Decision to make in M3: GraphQL
The architecture names a GraphQL layer for UI queries. The server-rendered
htmx UI hasn't needed it. Recommendation: **cut it** and amend
architecture.md — projections already serve reads; a second query surface is
maintenance without a consumer. Revisit only if a native mobile shell lands.

## M4 — Interop & intelligence (Phases 8–9 proper)

### 14. FHIR surface expansion — L
Read-only Patient + Encounter bundle today.
- Write endpoints with If-Match/ETag optimistic concurrency
- Add Observation, Condition, MedicationRequest/Dispense, Immunization, Coverage
- Capability statement; profile validation hooks (package-driven)

### 15. Official terminology ingestion (carried from old P3) — M
- ICD-11 via WHO workflows, LOINC release archive, SNOMED RF2 for licensed territories
- Artifact checksums + release metadata in admin status

### 16. Terminology-aware clinical UX (carried from old P3) — M
- Searchable terminology-backed diagnosis selection; LOINC/UCUM bindings in vitals/lab screens
- Persist clinician label + code metadata across diagnoses, observations, orders

### 17. Migration tooling — M
- OpenMRS/CSV importer as a documented, resumable job (biggest real-world adoption lever)
- Acceptance: import a sample OpenMRS export; registers reconcile

### 18. MCP expansion + deterministic CDS — L
- MCP tools beyond the current 4: search patients, queue board, order lab, encounter summary (reads first, writes gated)
- Confirmation/co-sign flow for agent-initiated writes per the agent-policy invariants
- Deterministic CDS as package-delivered rules (drug interaction, allergy, protocol reminders) with override reason capture
- Acceptance: an LLM agent can safely run a triage-summary workflow end-to-end against a demo clinic

## Explicitly parked (not scheduled)

- **Phase 10 research** (de-identification, cohorts, federated learning) — after real deployments generate real data
- **Mobile shell / TUI / SMS-USSD** — PWA covers tablets today (`sw.js` shell caching exists); revisit on pilot feedback
- **ONNX/local inference** — after deterministic CDS proves the delivery path
- **HL7 v2 / DICOM** — when a partner site actually speaks them
- **Package SDK & playground mode** — valuable for community, not for the first pilot

## Suggested order

M1.4 (billing UI) and M1.1 (appointments) are the two biggest holes in daily
clinic operation — start there. M2 must complete before any real-patient
pilot. M3/M4 items can interleave on demand; each is independent.

## Done (Phases 1–4 + service layers, details in git history)

Design docs, UUIDs v7, form engine, dual-backend test harness, `.rcpkg`
package format, terminology/interop design — all landed 2026-03. Open-source
prep (README, LICENSE, CI, brand) landed 2026-07-14.
