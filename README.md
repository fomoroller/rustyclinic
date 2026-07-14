<p align="center">
  <img src="docs/brand/logo/lockup.svg" width="460" alt="rustyclinic">
</p>

# rustyclinic

**A free, offline-first EMR for every scale of care — one Rust binary from a tablet to a hospital network.**

rustyclinic is a bet that one executable can serve a solo doctor with a tablet, a health post on a Raspberry Pi, a district hospital on a commodity server, and a hospital network on a cluster. Same binary, same data model, same upgrade path: a clinic that grows never has to migrate away, and a deployment that shrinks still works.

Three commitments underpin it:

- **Offline is the default, not a failure mode.** Registration, consultations, queueing, dispensing, billing, and reporting all run locally with no internet connection; sync happens when connectivity returns. Built for places where power and connectivity are unreliable and IT staff are scarce.
- **Variation is data, not forks.** Country programs, payer rules, forms, terminology, languages, and reports are delivered as signed runtime packages (`.rcpkg`). One binary, every deployment.
- **AI agents are first-class operators.** Every workflow is exposed through governed interfaces (MCP) with the same permissions, audit trail, and co-sign requirements as human staff — no privileged back doors, no bolted-on assistant. As frontier models become cheap enough to run anywhere, rustyclinic is already built to be operated by them.

Free forever: Apache-2.0, self-hostable end to end, no cloud dependency.

> **Status: pre-release.** rustyclinic is under active development and has not been validated for production clinical use. It is not a certified medical device. Do not use it to manage real patient care yet.

## What's inside

- **Clinic core** — patient registration, encounters, observations, diagnoses, queueing, admissions, lab, pharmacy, referrals, billing, claims, payments, and waivers
- **Offline-first** — SQLite-backed local operation with an event-sourced core, backup/restore, and a sync engine with conflict workflows (PostgreSQL supported behind the `postgres` feature)
- **Form engine** — clinical forms defined as data and shipped in packages, not hardcoded screens
- **Terminology** — ICD-11, LOINC, UCUM import pipelines; SNOMED CT supported for licensed deployment territories
- **Reporting & interop** — report generation, CSV and DHIS2 export, FHIR boundary APIs; OpenHIE/HL7 v2 on the roadmap
- **Signed packages** — Ed25519-signed `.rcpkg` bundles with staged install, activate, and rollback
- **Agent-ready** — a built-in MCP server so LLM agents operate through the same audited command layer as humans, with no privileged back door
- **Web UI** — server-rendered HTML with htmx and Alpine.js; no Node toolchain, no build step

The architecture is documented in depth in [`architecture.md`](architecture.md), with per-subsystem design docs in [`docs/designs/`](docs/designs/).

## Quickstart

Requires Rust 1.85+.

```sh
# build and start everything (API, web UI, worker, scheduler) on :8080
cargo run --release -- serve all

# create the first user (only works while the facility has no users)
curl -X POST http://localhost:8080/api/auth/bootstrap \
  -H 'Content-Type: application/json' \
  -d '{"username":"admin","display_name":"Admin","password":"change-me","roles":["system_admin"]}'
```

Then open <http://localhost:8080> and log in.

Useful admin commands:

```sh
rustyclinic admin status                      # system health
rustyclinic admin migrate                     # run database migrations
rustyclinic admin backup ./backup.db          # hot backup
rustyclinic admin download-terminology icd11  # fetch and import ICD-11
rustyclinic admin install-package ./my.rcpkg  # install a signed package
```

## Project layout

A Cargo workspace of 20 crates. The important ones:

| Crate | Purpose |
|---|---|
| `rustyclinic` | The single executable: CLI, runtime roles, wiring |
| `rustyclinic-core` | Domain types, state machines, errors |
| `rustyclinic-services` | Command layer — every state change goes through here |
| `rustyclinic-db` | SQLite (and optional PostgreSQL) repositories, migrations |
| `rustyclinic-clinical` | Queueing, admissions, lab, pharmacy, referrals, programs |
| `rustyclinic-billing` | Coverage, claims, payments, waivers |
| `rustyclinic-forms` | Data-driven form engine |
| `rustyclinic-packages` | `.rcpkg` format, signing, install lifecycle |
| `rustyclinic-sync` | Offline sync protocol and conflict handling |
| `rustyclinic-terminology` | ICD-11 / LOINC / UCUM / SNOMED CT import and lookup |
| `rustyclinic-interop` | FHIR boundary APIs |
| `rustyclinic-web` | Server-rendered web UI (htmx + Alpine.js) |
| `rustyclinic-mcp` | MCP server for LLM agent access |

## Roadmap

Development follows the ten phases laid out in [`architecture.md`](architecture.md). Honest status:

| Phase | Scope | Status |
|---|---|---|
| 1 | Platform kernel — audit, outbox, idempotency, state machines, dual DB backends | ✅ Done |
| 2 | Human-operable core — web UI, registration, search, queue, encounters, printing | ✅ Done |
| 3 | Offline & sync — operation log, conflict queue, backup/restore, sync UI | ✅ Done |
| 4 | Forms & package runtime — form engine, signed packages, activation/rollback | ✅ Done |
| 5 | Clinical operations — lab, pharmacy, admissions, referrals, programs | 🔶 Service layer done; UI for lab & pharmacy only. No appointments or immunization yet |
| 6 | Billing & payers — claims, payments, waivers, eligibility | 🔶 Service layer done; no UI, no mobile money |
| 7 | Reporting & data quality — reports, CSV, DHIS2 export | 🔶 Core reporting and DHIS2 export built |
| 8 | Identity & interop — MPI, HL7 v2, terminology-backed exports | 🔶 Terminology import built; FHIR read endpoints only |
| 9 | Intelligence & MCP — CDS, co-sign flows, agent tooling | 🔶 MCP server with first tools; no CDS yet |
| 10 | Research — de-identification, cohorts, federated learning | ⬜ Not started |

## Development

```sh
cargo test --workspace                             # run all tests
cargo clippy --workspace --all-targets             # lint (unwrap/todo/dbg are denied)
cargo fmt --check                                  # formatting
```

`unsafe` code is forbidden across the workspace.

## License

Apache-2.0. See [LICENSE](LICENSE). Vendored web assets are covered in
[`crates/rustyclinic-web/static/LICENSES.md`](crates/rustyclinic-web/static/LICENSES.md).
