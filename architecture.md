# RustyClinic Architecture

## Vision

RustyClinic is a single executable, FHIR-native, package-driven EMR and health platform built for low-resource settings first and hospital networks second. It must run reliably on a tablet, laptop, Raspberry Pi, or commodity server; continue operating during connectivity and power interruptions; support shared devices and community workflows; integrate with national reporting and health information exchange; and remain operable by both humans and LLM agents without creating a privileged back door.

The core artifact is one executable named `rustyclinic`. The executable can run in different runtime roles, but product functionality is not split across separate Rust services. Country, program, insurer, terminology, form, reporting, and AI variation is delivered primarily as signed runtime packages rather than code forks.

Cloud connectivity is optional. Routine registration, consultation, queueing, dispensing, billing, and reporting must continue locally when disconnected.

## Product Scope

RustyClinic is intentionally ambitious. To keep that ambition deliverable, the product is organized into four maturity lanes that build on each other.

### Lane A — Clinic Core

- Patient registration and identification
- Encounters, observations, diagnoses, procedures, orders, appointments
- Queueing, admissions, lab, pharmacy, immunization, referrals
- Billing, coverage verification, claims, payments, waivers
- Printing, patient cards, prescriptions, receipts
- CHW tasks, home visits, screening, and defaulter tracing
- Offline-first local operation, backup, restore, update

### Lane B — Platform

- Package runtime for country, program, payer, terminology, forms, reports, and integrations
- Form engine and configurable workflows
- Projections and read models
- Jobs and scheduler
- Sync engine and conflict workflows
- Migration tooling, OpenHIE, DHIS2, HL7 v2, DICOM
- Data quality and governance

### Lane C — Intelligence

- Deterministic clinical decision support
- Local ONNX inference for offline summarization and NLP
- Cloud LLM assist when policy and connectivity allow
- MCP server and agent tooling
- Draft-first automation and human co-sign for risky actions

### Lane D — Research

- De-identification and governed cohort discovery
- Research exports and data-use controls
- Federated learning and secure aggregation

*Rationale: the architecture supports all four lanes, but Lane A must be shippable without Lane C or Lane D. Advanced intelligence and research are additive, not prerequisites for clinic operations.*

## Architecture Overview

```text
rustyclinic (single executable, multi-role runtime)
│
├── Interfaces
│   ├── REST API           — FHIR R4 endpoints and admin APIs
│   ├── GraphQL            — projection-backed frontend queries
│   ├── Web / PWA          — primary human UI
│   ├── CLI                — admin, import/export, scripted operations
│   ├── TUI                — SSH-only environments
│   ├── MCP Server         — agent tools over shared services
│   ├── Mobile Shell       — Android/iOS/desktop wrapper for PWA
│   └── SMS / USSD Adapter — reminders and limited remote workflows
│
├── Application Services
│   ├── AuthN / AuthZ / Purpose-of-Use
│   ├── Idempotency and transaction orchestration
│   ├── Package resolution and effective-date selection
│   ├── State machine enforcement
│   ├── Audit, outbox, and sync intent
│   ├── Confirmation and co-sign workflows
│   └── Documented domain commands and queries
│
├── Domain and Platform Modules
│   ├── FHIR engine and profile validation
│   ├── Identity and patient matching
│   ├── Forms and configurable workflows
│   ├── Clinical operations and vertical programs
│   ├── Billing, insurance, and payments
│   ├── Terminology and value sets
│   ├── Reporting, indicators, and data quality
│   ├── Interop and migration
│   ├── Projections and read models
│   ├── Jobs and scheduler
│   ├── Sync and conflict resolution
│   ├── AI, CDS, and MCP policy
│   └── Research and de-identification
│
└── Data Plane
    ├── SQLite or PostgreSQL
    ├── Object/file storage for documents and package artifacts
    ├── Append-only audit log and outbox
    ├── Operation log and replica cursors
    ├── Projection stores
    ├── Package registry and activation state
    ├── Search tokens and embeddings
    └── Encrypted PHI and biometric stores
```

## Architecture Invariants

These rules are normative. Crate boundaries, APIs, migrations, operational procedures, packages, and workflows must preserve them.

| Invariant | Enforcement |
|---|---|
| Tenant isolation is mandatory | Every tenant-owned row carries a mandatory scope. Facility-owned data uses `facility_id`; control-plane data uses `scope_kind` and `scope_id`. PostgreSQL tiers enforce this with row-level security, composite foreign keys, and scoped views; SQLite tiers enforce the same scope in repository and service code. |
| Every mutation is idempotent | REST, CLI, GraphQL mutations, batch imports, and MCP writes accept an idempotency key or request identifier. The original result is replayed on safe retry. |
| Domain write, audit, outbox, and sync intent commit together | A state-changing transaction must persist business rows, append-only audit, outbox event, and sync operation log together. Request handlers never publish directly to a broker and never write best-effort audit after commit. |
| Audit is append-only, hash-chained, and replayable | Corrections are compensating events, not in-place edits. Each audit row links to the previous row hash within scope so tampering is detectable. |
| Every long-running workflow is lease-based and resumable | Claims submission, exports, DHIS2 push, backups, sync replay, updates, and federated rounds persist job state, leases, heartbeats, and retries. No critical workflow relies on process memory or an in-memory cron alone. |
| Deployment-specific behavior is package-versioned and effective-dated | Country rules, payer logic, form schemas, terminology subsets, indicators, prompts, and report mappings are delivered as signed packages with compatibility, activation, and rollback metadata. |
| Real-world side effects are modeled as state machines | Claims, payments, lab processing, dispensing, admissions, queue transitions, referrals, exports, and package installs use explicit transition graphs and permission checks rather than free-form status updates. |
| Read-heavy surfaces are projection-backed | Dashboards, worklists, timelines, analytics, and GraphQL queries read from projections or materialized aggregates, not ad hoc joins over canonical transactional JSON. |
| Offline operation is a first-class mode | Registration, encounters, queueing, billing, dispensing, local reports, and document printing must work without network connectivity in nano and micro deployments. |
| Sync semantics are domain-specific | CRDTs are used only where merge rules are safe. Claims, payments, stock, bed occupancy, and queue order use deterministic operation logs, leases, or manual reconciliation. |
| Identity merges and splits are reversible and audited | Patient link, unlink, merge, split, identifier reassignment, and biometric rebind operations must preserve historical references and produce recoverable audit trails. |
| Agent actions are attributable and policy-gated | Every MCP or AI-originated action records agent identity, model/version, prompt policy, and operator confirmation or co-sign requirement where applicable. |
| PHI egress is explicitly controlled | Sending data to cloud LLMs, insurers, DHIS2, MPI, support bundles, or research exports requires an egress policy check, redaction rules, and auditable purpose-of-use. |
| Cloud services are optional, not foundational | Core clinic operation cannot depend on internet connectivity, cloud identity, or cloud AI. |
| All identifiers are UUIDs v7 | Every aggregate, entity, device, and operation-log entry uses UUIDs v7 for time-sortable, globally unique, index-friendly identifiers. No sequential integers for primary keys. |
| Audit integrity violations trigger incident response | A broken audit hash chain is a security incident. The system detects, alerts, quarantines the affected scope, and requires explicit repair. Operations continue in a degraded-trust mode with all subsequent entries flagged. |
| In-flight forms are version-pinned | When a clinician opens an encounter, the active form version is recorded. All form interactions for that encounter use the pinned version, even if a package update activates a newer version during the visit. |

## Operating Model

### Single Executable, Multi-Role Runtime

The artifact is one executable:

```bash
rustyclinic serve api
rustyclinic serve worker
rustyclinic serve sync
rustyclinic serve scheduler
rustyclinic serve mcp
rustyclinic serve all
rustyclinic admin ...
```

The same codebase and configuration model support all roles. Small sites usually run `serve all`. Larger deployments split roles across processes or nodes for throughput and fault isolation.

*Rationale: one executable keeps installation and upgrades simple, while explicit roles avoid the fragility of a single giant process doing everything.*

### Facility Plane and Control Plane

RustyClinic distinguishes between two architectural scopes:

- **Facility plane**: local clinical operations, billing, local packages, queueing, dispensing, reporting, and site autonomy.
- **Control plane**: optional district or network services for package distribution, facility registry cache, MPI coordination, aggregate reporting, fleet updates, and federated orchestration.

A facility must continue to function when the control plane is unreachable.

*Rationale: this preserves local autonomy in low-resource settings while allowing network-level coordination where available.*

## Deployment Tiers

| Tier | Typical setting | Default topology | Database | Optional infrastructure | Notes |
|---|---|---|---|---|---|
| Nano | CHW device, outreach tablet, mobile registration | Embedded `rustyclinic` in mobile shell | SQLite | None | Optimized for intermittent connectivity, local caching, and later sync. |
| Micro | Rural clinic, standalone health center, private practice | One executable on laptop or Raspberry Pi | SQLite | None | Local queueing, billing, printing, USB backup, and offline insurer cache. |
| Standard | District hospital or multi-department site | One or more runtime roles plus PostgreSQL | PostgreSQL | Redis and NATS if justified | PostgreSQL is the default backbone. Redis and NATS are optional, not assumed. |
| Enterprise | Hospital network or national rollout | Multiple runtime roles across nodes or Kubernetes | PostgreSQL cluster | NATS, Redis, object storage, KMS | Supports larger event fan-out, fleet management, and cross-site control plane. |

Compilation still uses tiered feature flags, but deployment variation inside a tier is primarily package-driven rather than compile-time specialization.

### Per-Tier Resource Budgets

These are design targets that inform feature-flag decisions, package loading behavior, and background job scheduling. They are not hard limits.

| Tier | Target device | RAM budget (rustyclinic process) | Disk budget (data) | CPU assumption | Typical concurrent users |
|---|---|---|---|---|---|
| Nano | Budget Android tablet, 2-3 GB total RAM | ≤150 MB | ≤2 GB | 4 ARM cores | 1-2 |
| Micro | Laptop or Raspberry Pi 4/5, 4-8 GB RAM | ≤512 MB | ≤10 GB | 4 cores | 5-15 |
| Standard | Commodity server or VM, 8-16 GB RAM | ≤2 GB (excluding PostgreSQL) | ≤100 GB | 4-8 cores | 50-200 |
| Enterprise | Server cluster | Scaled horizontally per node | Scaled by storage tier | Per-node sizing | 200+ |

**Nano constraints.** The nano binary is compiled with feature flags that exclude ONNX inference, research modules, DICOM, and HL7 v2. ONNX models are loaded only on micro or higher tiers where RAM allows. Projection rebuilds and non-essential background jobs are deferred to charging or idle periods to conserve battery. The target executable size for a nano build is under 30 MB.

**Micro constraints.** Full offline functionality including local reports, billing, and pharmacy. Background jobs are throttled during configured clinic hours to avoid saturating the device. ONNX models for local NLP are optional and loaded on demand, adding 50-200 MB when active.

**Standard and enterprise constraints.** PostgreSQL is the primary memory consumer beyond the `rustyclinic` process itself. Redis and NATS, when enabled, add their own memory footprint. Resource budgets for these tiers are deployment-configured rather than compile-time constrained.

**Why not 2 GB on a tablet.** A 2 GB process on a device with 2-3 GB total RAM is a non-starter. The nano tier is explicitly designed to stay under 150 MB by excluding optional subsystems at compile time and loading data lazily. A Rust executable with SQLite, an embedded HTTP server, and core clinical logic fits comfortably within this budget.

## Workspace Layout

The workspace is organized by responsibility, not by transport.

### Core and Platform Crates

| Crate | Responsibility |
|---|---|
| `rustyclinic-core` | Core types, errors, configuration, FHIR primitives, crypto traits, shared invariants |
| `rustyclinic-db` | SQLite/PostgreSQL abstraction, migrations, repositories, encryption helpers |
| `rustyclinic-events` | Outbox relay, consumer idempotency, optional broker integration, audit replay helpers |
| `rustyclinic-auth` | Password/PIN/biometric/smart-card auth, sessions, shared-device mode, RBAC, purpose-of-use checks, OAuth2/OIDC, SMART on FHIR |
| `rustyclinic-identity` | Patient identity, enterprise person, identifier history, biometric links, MPI adapters |
| `rustyclinic-forms` | Questionnaire runtime, UI schema, rule DSL, renderer contracts, print layouts, migration of form versions |
| `rustyclinic-packages` | Package registry, signature verification, compatibility checks, activation, rollback, package APIs |
| `rustyclinic-jobs` | Scheduler, leases, retries, heartbeats, maintenance windows, job definitions |
| `rustyclinic-projections` | Read-model builders, projection stores, lag monitoring, rebuild tooling |
| `rustyclinic-sync` | Operation log, snapshots, replica cursors, transport adapters, conflict queue |
| `rustyclinic-terminology` | Terminology services, local subsets, value sets, mappings, translation |
| `rustyclinic-services` | Shared application-service layer for commands, queries, transactions, state machines, audit, and policy |

### Domain Crates

| Crate | Responsibility |
|---|---|
| `rustyclinic-programs` | Vertical programs and community workflows such as HIV, TB, ANC, IMCI, malaria, family planning, NCDs, CHW visits, and defaulter tracing |
| `rustyclinic-billing` | Coverage, eligibility, tariffs, claims, payments, waivers, mobile money |
| `rustyclinic-reporting` | Indicators, HMIS, DHIS2, registers, printable reports, data quality rules |
| `rustyclinic-interop` | OpenHIE, MPI, facility registry, HL7 v2, DICOM, document exchange, migration adapters |
| `rustyclinic-ai` | Deterministic CDS, ONNX inference, embeddings, LLM gateways, prompt policy, MCP tool metadata |
| `rustyclinic-research` | De-identification, cohort discovery, data-use agreements, governed exports, federated coordination |

### Interface Crates

| Crate | Responsibility |
|---|---|
| `rustyclinic-api` | REST API, GraphQL, web asset serving, HTTP middleware |
| `rustyclinic-cli` | Administrative CLI, import/export, scripts, TUI shell |
| `rustyclinic-mcp` | MCP transport built over shared services |
| `rustyclinic-web` | Web/PWA frontend using projection-backed APIs and shared form renderers |

### Dependency Rules

```text
interfaces ──▶ services ──▶ platform/domain crates ──▶ db/events/core
                                       │
                                       └──────────────▶ packages/forms/identity/sync/projections/jobs
```

Rules:

1. Interfaces may validate transport-level input and shape transport responses, but they do not implement business rules.
2. All writes flow through `rustyclinic-services`.
3. `rustyclinic-core` is a leaf crate with no internal dependencies.
4. Projection builders consume committed outbox events or canonical rows, never uncommitted request state.
5. MCP never wraps HTTP handlers directly; it invokes the same service commands as other interfaces.
6. Optional WASM plugins, if enabled, run only through a host API with explicit capability manifests and no direct database or unrestricted network access.
7. Domain crates may depend on platform crates but inter-domain dependencies must be explicit and acyclic. Specifically: `rustyclinic-programs` may depend on `rustyclinic-billing` (to check coverage); `rustyclinic-billing` may depend on `rustyclinic-forms` (to validate claim forms). Circular dependencies between domain crates are forbidden. All cross-domain coordination that does not fit a direct dependency goes through `rustyclinic-services`.
8. Each domain command in `rustyclinic-services` is a separate module (one command per file). The services crate is organized by domain, then by command — not as methods on a monolithic service struct. This prevents merge conflicts as the command set grows.

### Database Abstraction Pattern

Every data access module uses **repository traits per aggregate** with separate implementations for SQLite and PostgreSQL:

```rust
// In rustyclinic-db
pub trait PatientRepo {
    fn create(&self, patient: &Patient) -> Result<()>;
    fn find_by_id(&self, id: Uuid) -> Result<Option<Patient>>;
    fn search(&self, query: &PatientSearch) -> Result<Vec<Patient>>;
}

// Separate implementations
pub struct SqlitePatientRepo { /* ... */ }
pub struct PgPatientRepo { /* ... */ }
```

Both implementations must produce identical results for identical inputs. The dual-backend integration test harness (see CI/CD section) verifies this property for every repository method.

### State Machine Framework

All state-machine-driven workflows share a common framework defined in `rustyclinic-core`:

```rust
pub trait StateMachine: Sized {
    type State: Clone + PartialEq;
    type Transition;
    type Context;

    fn current_state(&self) -> &Self::State;
    fn allowed_transitions(&self, ctx: &Self::Context) -> Vec<Self::Transition>;
    fn apply(&mut self, transition: Self::Transition, ctx: &Self::Context) -> Result<()>;
}
```

Each of the 10 state-machine workflows (queue, claims, lab, pharmacy, admission, referral, program enrollment, export, package install, federated round) implements this trait. Transition validation, permission checks, and audit logging are handled by the framework, not reimplemented per workflow.

## Configuration and Package System

### Static Configuration vs Runtime Packages

RustyClinic uses two layers of configuration:

- **Static configuration** for infrastructure and runtime concerns such as database settings, ports, auth providers, key stores, backup locations, role selection, and transport endpoints.
- **Runtime packages** for deployment-specific clinical and administrative content.

Runtime packages are first-class architecture, not an afterthought.

### Package Types

| Package type | Typical contents |
|---|---|
| Deployment pack | Country, region, or network defaults; enabled languages; legal notices; facility profile; default workflows |
| Program pack | HIV, TB, malaria, ANC, child health, family planning, NCD forms and rules |
| Payer pack | Scheme verification rules, tariffs, claim mappings, adjudication codes, exemption policies |
| Form pack | `Questionnaire`, UI schema, skip logic, computed fields, validation rules, printable layouts, migration rules |
| Terminology pack | Local value sets, code maps, translations, subsets of SNOMED, ICD-10, LOINC, RxNorm, WHO EML |
| Report pack | Indicator definitions, register layouts, DHIS2 metadata mappings, export templates |
| Integration pack | MPI endpoint config, DHIS2 channels, SMS/USSD flows, mobile money adapters, printer/scanner profiles |
| Model pack | ONNX models, prompt templates, safety policies, model metadata, evaluation thresholds |

### Package Manifest

Every package includes a signed manifest with at least:

- package identifier and type
- semantic version
- compatible `rustyclinic` versions
- dependency list
- effective start and end dates
- scope of activation: facility, district, network, or global reference
- checksum and signature metadata
- localization coverage
- migration hooks for superseded versions
- rollback instructions where applicable

### Package Activation Rules

1. Package installation is validated and staged before activation.
2. Activation is transactional and auditable.
3. At most one active version of a package family can apply to a given scope and effective date unless the package type explicitly supports layered override.
4. Package changes may require projection rebuilds, form draft migration, or indicator recomputation; these are handled as jobs.
5. Expired or revoked packages remain available for historical interpretation of old records.
6. Package artifacts are syncable and can be delivered through network or signed USB media.

### Package Dependency Resolution

At install time, the package resolver validates three conditions before staging a package:

1. **Executable compatibility**: the package manifest declares a semver range of compatible `rustyclinic` versions. If the running executable version is outside that range, installation is blocked with a diagnostic recommending an executable update or a compatible package version.
2. **Dependency satisfaction**: all packages declared in the dependency list must be present locally and their version constraints must be satisfiable. If a dependency is missing, the resolver reports which package and version range is required and where to obtain it (network sync or USB).
3. **Scope conflict check**: no two packages in the same family may be active for the same scope and overlapping effective-date range, unless the package type explicitly supports layered override (e.g., a facility-level form pack overriding a district-level default).

If two packages declare incompatible requirements for a shared dependency (e.g., Package A requires Terminology Pack ≥3.0 while Package B requires <3.0), installation of the later package fails. The operator must either upgrade the shared dependency to a version satisfying both constraints or choose compatible package versions.

Resolution is deterministic and reproducible — given the same set of installed packages and the same candidate, the resolver always produces the same accept/reject decision.

### Offline Package Behavior

Packages are pre-installed on the device, not fetched on demand. A nano or micro device must have all required packages locally before it can operate offline. Package installation happens during provisioning, during a connectivity window, or via signed USB media.

**During offline operation**, the installed packages remain fully functional. There is no runtime package fetch that could fail due to missing connectivity.

**After extended offline periods**, when a device reconnects:

1. The sync engine delivers a package update manifest listing new versions, expirations, and revocations that occurred while the device was offline.
2. Packages whose effective-end-date has passed are marked expired. They remain available for historical interpretation — a record created under expired rules can still be viewed and understood in context — but they no longer apply to new clinical activity.
3. New package versions are staged, validated, and offered for activation. Activation follows the standard transactional process and may trigger projection rebuilds.
4. Records created under now-superseded package rules (e.g., claims using a stale tariff schedule, encounters using an old form version) are flagged with the package version that was active at creation time. The upstream system can accept, re-validate, or queue for review these records. Whether to accept or reject a stale-rule claim is a domain decision configured in the payer package, not a platform-level automatic rejection.

Package artifacts are included in backup and sync manifests so that a restored or bootstrapped device always has a complete, operational package set.

*Rationale: country and program variation belongs in data and signed content packages, not in long-lived code forks.*


## Extension Model

Third-party extensions are optional and available in standard and enterprise deployments through WASM modules delivered as signed packages.

Rules:

- the host exposes a versioned SDK and capability manifest
- plugins have no direct database access
- plugins have no unrestricted network access
- all writes still go through service commands and policy checks
- plugin activation, upgrade, and rollback are auditable
- plugin failure must not crash the clinical core

*Rationale: extensibility is useful, but it must not become a second uncontrolled application platform.*

## Data Model and FHIR Strategy

### FHIR at the Boundary, Profile-First Inside

FHIR R4 is the canonical interoperability model and external API surface. RustyClinic does not attempt to be an unbounded general-purpose FHIR server on day one. It supports a profile-constrained subset that maps cleanly to operational workflows.

Supported first-class resource families include:

- **Identity and administration**: `Patient`, `Practitioner`, `PractitionerRole`, `Organization`, `Location`, `Coverage`, `Consent`
- **Clinical**: `Encounter`, `Observation`, `Condition`, `AllergyIntolerance`, `Procedure`, `CarePlan`, `CareTeam`
- **Orders and results**: `ServiceRequest`, `Specimen`, `DiagnosticReport`, `Task`
- **Medication**: `Medication`, `MedicationRequest`, `MedicationDispense`
- **Scheduling and communication**: `Appointment`, `Communication`
- **Immunization and forms**: `Immunization`, `Questionnaire`, `QuestionnaireResponse`
- **Financial**: `Claim`, `ClaimResponse`, `ExplanationOfBenefit`
- **Documents and provenance**: `DocumentReference`, `Binary`, `Media`, `Provenance`

Each deployment chooses concrete profiles through packages. Profiles constrain search parameters, required fields, terminology bindings, validation, and workflow semantics.

### Operational Aggregates

FHIR resources alone are not enough for hot-path operational workflows. RustyClinic therefore models a set of explicit aggregates and state machines, including:

- `QueueEntry`
- `ClaimCase`
- `InventoryLedger`
- `BedOccupancy`
- `ProgramEnrollment`
- `EligibilityCheck`
- `SyncReplica`
- `PackageInstall`
- `DataQualityIssue`

These aggregates are persisted in canonical tables and projected into FHIR resources where appropriate, rather than forcing every operational step through generic resource CRUD.

### Storage Pattern

Canonical clinical resources are stored as full JSON plus derived columns or companion tables for:

- scope and ownership
- optimistic concurrency metadata
- derived search tokens
- operational state
- effective-date linkage to packages and protocols
- projection and sync metadata

PostgreSQL uses JSONB and indexed derived columns. SQLite uses JSON1 plus explicit columns and side tables. The storage pattern is intentionally similar across both so that sync, tests, and migrations behave consistently.

### Search and Concurrency

- Plaintext PHI is not indexed directly in PostgreSQL.
- Search uses normalized tokens, phonetic hashes, identifier hashes, and coarse non-PHI filters.
- Resources support versioning and ETag / `If-Match` semantics for mutable APIs.
- High-volume read surfaces use projections instead of repeated FHIR search over transactional tables.

#### Optimistic Concurrency and Conflict Surfacing

Every mutable resource carries a version number incremented on each write. API write requests must include an `If-Match` header with the current ETag. A version mismatch returns HTTP 409 Conflict with the current server version attached.

**Two-user scenario.** If two clinicians load the same encounter and both submit edits:

1. The first submission succeeds and increments the version.
2. The second submission fails with a 409 because its ETag is stale.
3. The UI fetches the current version and presents a conflict view showing what changed since the second clinician's load.
4. The second clinician can merge their changes into the updated version, override with reason (audited), or discard their edits.

**Automatic merge for non-overlapping changes.** When structured fields do not overlap — for example, one clinician added an observation while the other added an allergy — the UI can auto-merge the additions without user intervention. Overlapping edits to the same field (e.g., both changed the primary diagnosis) always require human resolution.

**Distinction from sync conflicts.** ETag-based optimistic concurrency handles real-time contention between users on the same server or device. Sync conflicts (described in the Sync Model section) handle eventual-consistency contention between devices that were offline. The resolution mechanisms are different: ETag conflicts are immediate and interactive; sync conflicts may be queued for later review.

*Rationale: this preserves interoperability fidelity while keeping operational workflows fast, safe, and offline-friendly.*

## Forms and Configurable Workflows

The form engine is a core platform service. It is not just FHIR `Questionnaire` storage.

Every form definition includes:

- a canonical `Questionnaire`
- renderer metadata for web, mobile shell, and TUI
- skip logic and conditional visibility
- computed fields
- validation rules
- mapping rules to FHIR resources and domain commands
- print layout definitions
- data-quality checks
- effective dates and protocol version metadata
- draft migration rules when the form schema changes

Forms are delivered primarily through packages. A maternal health deployment can install an ANC package with visit schedules, high-risk flags, register layouts, and indicator mappings without changing core code.

The web/PWA renderer is the primary human interface. The same form definitions must render consistently in the mobile shell and, where practical, in TUI mode.

*Rationale: in real deployments, configurable forms and program workflows are half the product.*


## Localization and Documents

Localization is package-driven and applies to both interface and workflow content.

Required capabilities:

- Fluent-based interface translation
- deployment-level language bundles, typically including English, Kinyarwanda, Swahili, French, Portuguese, and Amharic where needed
- locale-specific date, number, currency, and naming conventions
- translated terminology display names and patient-facing instructions
- localized print templates for prescriptions, receipts, patient cards, labels, and reports
- right content for the right scope, so a district can standardize templates while a facility overrides branding

Historical documents must remain renderable according to the package versions active when they were generated.

*Rationale: localization in healthcare is not cosmetic; it changes comprehension, workflow adherence, and legal validity.*

## State Machines for Critical Workflows

The following workflows are modeled as explicit state machines with legal transitions, actor permissions, compensating actions, and audit requirements.

| Workflow | Typical states |
|---|---|
| Queue entry | `created → waiting → called → in_service → transferred → completed` plus `no_show` and `cancelled`. The `called` transition uses an `assigned_to` field with optimistic locking to prevent two nurses from calling the same patient simultaneously. |
| Claim case | `draft → validated → batched → submitted → acknowledged → adjudicated → paid` plus `rejected`, `voided`, `reopened` |
| Lab workflow | `ordered → sample_pending → collected → received → in_process → resulted → verified` plus `amended` and `cancelled` |
| Medication dispense | `draft → prepared → dispensed` plus `partial`, `returned`, `voided` |
| Admission / transfer / discharge | `planned → admitted → transferred → discharged` plus audited reversal paths |
| Referral | `drafted → sent → received → accepted → completed` plus `declined` and `cancelled` |
| Program enrollment | `eligible → enrolled → active` plus `paused`, `completed`, `withdrawn` |
| Export run | `queued → building → ready → transmitted → acknowledged` plus `failed`, `expired`, `revoked` |
| Package install | `uploaded → verified → staged → activated` plus `rolled_back` and `revoked` |
| Federated round | `planned → open → training → aggregating → published` plus `aborted` |

Transitions happen through documented service commands, never by arbitrary status patching.

## Canonical Write, Read, and Long-Running Flows

### Write Flow

Every state-changing operation follows this sequence:

1. Resolve authenticated actor, device, purpose-of-use, and tenant scope.
2. Resolve active package set and effective protocol version. The active package set for a facility is cached in memory and invalidated by package activation events to avoid per-write database queries.
3. Validate permissions and any required human confirmation or co-sign.
4. Validate or create the idempotency record.
5. Open a database transaction.
6. Persist canonical domain rows or FHIR resources.
7. Persist append-only audit records and hash-chain metadata.
8. Persist outbox events.
9. Persist sync operation-log entries for sync-eligible aggregates.
10. Commit.
11. Background workers publish events, update projections, and schedule follow-up jobs.

### Read Flow

1. Resolve scope and authorization filters.
2. Query a projection or optimized read model by default.
3. Fetch canonical resources only when projection detail is insufficient.
4. Attach provenance, version, and package/protocol interpretation metadata where relevant.

### Long-Running Flow

1. Persist a job or workflow instance.
2. Acquire a lease.
3. Execute one idempotent step.
4. Heartbeat progress and checkpoint state.
5. Complete, retry with backoff, or release for recovery.
6. Record outcome in audit and projection surfaces.

*Rationale: explicit write, read, and job flows keep side effects durable and recoverable under power loss, retries, and partial failure.*


## Eventing and Integration Backbone

The transactional outbox is the authoritative event boundary.

- Small deployments use in-process delivery after commit.
- Standard deployments can operate with PostgreSQL alone.
- NATS or another broker is introduced only when fan-out, throughput, or cross-node decoupling justify it.
- Consumers are idempotent and rebuildable.
- Projection builders, notifications, sync relays, and interop connectors all consume committed events rather than request-time callbacks.

*Rationale: the outbox preserves correctness under failure; the broker is an optimization, not the source of truth.*

## Projections and Read Models

Read models are first-class architecture, not convenience tables. They are rebuildable from canonical data and outbox history.

### Required Projections

- `PatientSummaryProjection`
- `LongitudinalTimelineProjection`
- `QueueBoardProjection`
- `AppointmentAgendaProjection`
- `InventoryStatusProjection`
- `ClaimWorklistProjection`
- `ProgramRegisterProjection`
- `HMISAggregateProjection`
- `SyncHealthProjection`
- `DataQualityProjection`

### Projection Rules

- Projections are versioned.
- Projection builders are idempotent.
- Lag and rebuild status are observable.
- Projection schemas may depend on active report or form packages.
- Projection rebuilds run as jobs and are safe to resume.

GraphQL and dashboard APIs should read projections by default. FHIR REST search remains available for interoperability, but it is not the main engine for every operational screen.

### Projection Staleness Strategy

Projection rebuilds triggered by package activation are scheduled outside configured clinic hours on micro and nano tiers. Critical projections (`QueueBoardProjection`, `PatientSummaryProjection`) are prioritized and rebuilt first. The UI displays a staleness indicator on any projection-backed view where the projection lag exceeds a configurable threshold (default: 5 minutes). This prevents users from acting on stale data without realizing it.

*Rationale: normalized writes and fast reads are both required; projections are how the system gets both.*

## Jobs and Scheduler

The scheduler is a core subsystem with persisted definitions, leases, retries, maintenance windows, and facility-aware execution policy.

### Job Classes

- sync replay and bootstrap
- DHIS2 export and retries
- claim batch submission and reconciliation
- backups, restore verification, and retention cleanup
- projection rebuilds
- terminology and package activation follow-up
- reminders and notifications
- update checks and staged rollouts
- data-quality scans
- federated training windows
- key rotation and re-encryption tasks

### Scheduler Rules

- No critical cron exists only in memory.
- Jobs are scoped by facility or control-plane domain.
- Jobs support exponential backoff and dead-letter or manual review where needed.
- Micro and nano tiers may delay non-essential jobs based on battery state, connectivity, or configured clinic hours.
- Jobs must be observable from the UI and CLI.

*Rationale: many of the platform’s ambitions are time-driven, not just request-driven.*

## Identity and Patient Matching

Identity is modeled in layers.

### Identity Layers

1. **Facility patient record**: the local clinical identity used for direct care at a site.
2. **Enterprise person**: a cross-facility identity used for network matching and MPI integration.
3. **Identifier registry**: MRNs, national identifiers, insurer identifiers, program identifiers, and historical aliases.
4. **Biometric links**: encrypted fingerprint template references and enrollment metadata.
5. **External links**: MPI references and other external registry connections.

### Matching Strategy

Matching uses a staged pipeline:

1. deterministic identifiers
2. demographic rules
3. probabilistic matching
4. biometric lookup where consented and available
5. manual review

### Identity Operations

The architecture explicitly supports:

- link and unlink
- merge and split
- identifier reassignment
- duplicate worklists
- wrong-patient correction
- biometric re-enrollment
- external MPI synchronization

All such operations are reversible, audited, and projection-aware so that references remain traceable.

*Rationale: cross-facility care, insurer verification, and biometrics all fail if identity is treated as a single flat patient table.*

## Sync Model

Sync is an early platform requirement, not a late add-on.

### Protocol Components

- append-only operation log
- replica state and cursors
- bootstrap snapshots
- conflict queue
- package artifact transfer
- projection catch-up markers
- transport adapters for HTTPS, LAN peer sync, and signed USB media

### Domain Replication Rules

| Domain | Replication model | Conflict rule |
|---|---|---|
| Patient demographics | Operation log plus field-aware merge | Structured merges for non-critical fields; conflicting DOB, sex, or identifier edits become manual review. |
| Encounters, observations, conditions, procedures | Append-only clinical events | Concurrent appends merge; conflicting status transitions require review. |
| Claims and adjudication | Single-writer state machine per owning facility | Conflicting remote transitions are rejected into the conflict queue. |
| Payments and inventory ledger | Ordered immutable transactions | Duplicate events ignored by event id; counters are never merged directly. |
| Queue operations | Facility-local operational state | Queue order does not replicate cross-facility except as aggregate analytics. |
| Bed occupancy and ADT | Lease plus transition log | First committed lease wins; competing transitions become conflicts. |
| Forms and program registers | Canonical responses plus derived projections | Canonical responses sync; register projections rebuild locally. |
| Reporting indicators | Derived aggregates | Recomputed from canonical events and package definitions, never hand-edited replicas. |
| Packages and terminology | Versioned artifact sync | Replace by version and checksum, with staged activation and rollback. |
| Audit, outbox, and op-log | Append-only replication | Consumers must be idempotent and preserve origin metadata. |
| Embeddings and caches | Disposable local state | Rebuild locally and never block care. |

### Sync Transport and Operation-Log Mechanics

**Serialization and wire format.** Each operation-log entry is a self-describing record containing the aggregate type, aggregate ID, facility scope, sequence number, timestamp, operation payload, and origin metadata. Entries are serialized as CBOR (not raw JSON) for compactness on constrained devices. Batches of entries are compressed with zstd before transmission.

**Transport flow.** Nano and micro devices act as sync clients. They push local operations and pull remote operations over HTTPS to a sync endpoint hosted by a standard or enterprise tier node. The protocol is pull-based with client-initiated push — the server never pushes unsolicited data to a device.

```text
Nano/Micro (SQLite)                    Standard/Enterprise (PostgreSQL)
     │                                          │
     ├── push: local ops since last ack ──────▶ │ ── validate, deduplicate, apply
     │                                          │
     │ ◀──── pull: remote ops since cursor ──── ├── serve ops from op-log
     │                                          │
     ├── ack: update local cursor ────────────▶ │
     │                                          │
     └── pull: package update manifest ───────▶ │
```

Alternative transports for connectivity-constrained sites:

- **LAN peer sync**: two devices on the same local network can sync directly using mDNS discovery and mutual TLS, without an upstream server. This enables clinic-to-clinic transfer when both are offline from the district.
- **Signed USB media**: operations are exported to an encrypted, signed file on USB. The receiving device validates the signature, applies the operations, and writes a return file with its own operations. This supports fully disconnected sites.

**Operation-log growth and pruning.** On nano and micro devices with limited storage, the op-log is pruned after upstream acknowledgement. The retention policy is:

1. Acknowledged operations are retained for a configurable window (default: 30 days) to support local audit and troubleshooting.
2. Beyond the retention window, acknowledged operations are pruned. The pruning job runs during idle or charging periods on nano devices to avoid impacting clinic hours.
3. Unacknowledged operations are never pruned — they remain until successfully synced.
4. A facility-scoped op-log on a nano device with typical CHW workloads (20-50 patient interactions per day) grows at roughly 5-15 MB per month before pruning.

**Bootstrap for new devices.** When a new device joins a facility, it does not replay the entire op-log history. Instead:

1. The upstream node generates a point-in-time snapshot of the facility's canonical data.
2. The snapshot is transferred to the device (over HTTPS or USB).
3. The device applies the snapshot and sets its cursor to the snapshot's op-log position.
4. Incremental sync proceeds from that cursor.

Bootstrap for a typical micro-tier clinic (5,000-10,000 patients, 2 years of history) targets under 500 MB of snapshot data and under 30 minutes of transfer time on a 1 Mbps link.

**Extended offline recovery.** When a device has been offline for an extended period (months), incremental sync may involve a large backlog. The system handles this as follows:

- If the backlog is within a configurable threshold (default: 100,000 operations or 500 MB), normal incremental sync proceeds with progress reporting in the UI.
- If the backlog exceeds the threshold, the system offers a re-bootstrap option: a fresh snapshot transfer followed by incremental sync from the snapshot point.
- In either case, the device's locally created operations are pushed first and conflicts are queued for review before remote operations are applied.
- Package version drift is reconciled before clinical data sync: the device must stage and activate any required package updates so that incoming data is interpreted under the correct rules.

### Sync UX

Users need visibility into sync, not just background magic. The UI must expose:

- connection state
- pending operations
- last successful sync
- package update state
- conflicts requiring attention
- backup health

*Rationale: offline-first systems fail when conflict handling is invisible or treated as purely technical.*

## Security, Trust, and Compliance

### Key Management and Encryption

RustyClinic uses envelope encryption:

- deployment key or KMS-managed key encrypts facility keys
- facility keys encrypt PHI-bearing values
- rotated keys trigger resumable re-encryption jobs

Encrypted stores include not only canonical patient data but also biometric templates, audit payload fragments, outbox payloads when they carry PHI, sync conflict payloads, and research work products.

### Authentication and Access

Supported auth modes include:

- password
- PIN
- biometric
- smart card / NFC
- optional MFA
- offline cached credentials with bounded lifetime and revocation rules

Authorization combines role-based permissions with contextual policy such as facility scope, purpose-of-use, and workflow state.

#### Session Lifecycle and Offline Credential Management

**Token format.** Online sessions use short-lived signed JWTs issued by the `rustyclinic` auth service, with refresh tokens for session extension. On nano and micro tiers running SQLite, sessions are tracked as local database rows with the same expiry and scope semantics — no external identity provider is required.

**Offline cached credentials.** When a user authenticates while online, the system stores a salted Argon2id hash of their credential on the device. This cached credential has a deployment-configurable maximum lifetime (default: 14 days, adjustable from 1 to 90 days). The raw password or PIN is never stored. During offline operation, users authenticate against the cached hash. When the cached credential expires, the user must reconnect to re-authenticate — the system will not allow indefinite offline access with stale credentials.

**Offline-to-online transition.** When a device regains connectivity, it performs a credential and policy refresh before resuming sync:

1. Present device certificate and session token to the upstream auth endpoint.
2. Pull the revocation delta — a compact list of user disablements, role changes, credential resets, and permission updates since the device's last sync cursor.
3. Terminate any locally active sessions for revoked users immediately.
4. Update the local role and permission cache.
5. If the revocation delta fetch fails, the device continues operating with its last-known policy state but displays a staleness warning in the UI and audit log.

**Shared-device session isolation.** On shared devices (common in clinics with one workstation), each user switch creates an isolated session. Form drafts, clipboard state, and in-progress workflows are scoped to the authenticated user. Idle timeout triggers a lock screen (not a logout), so the user's context is recoverable with re-authentication. A different user can authenticate to a new session without destroying the locked session's state, up to a configurable maximum of concurrent locked sessions (default: 3).

### Break-Glass and Shared Devices

- Break-glass access is explicit, time-bounded, and reason-coded.
- Shared-device mode supports fast user switching with session isolation.
- Device identity and last-sync posture are available to policy checks.

### PHI Egress Control

Before data leaves the identified clinical boundary, the platform applies:

- destination policy
- minimum-necessary redaction
- purpose-of-use validation
- consent and legal-basis checks where required
- audit and operator acknowledgement where required

This applies to cloud LLM prompts, insurer APIs, DHIS2 exports, MPI requests, support bundles, and research exports.

### Governance

The platform includes governance workflows for:

- retention vs erasure conflicts
- legal hold
- research approvals and DUA expiry
- package approval and revocation
- prompt and model approval
- CDS rule approval and rollback
- break-glass review
- cross-border export decisions

*Rationale: encryption and RBAC are necessary but not sufficient; the trust model must cover keys, egress, approvals, and recoverability.*


## Clinical Safety Governance

Clinical safety is governed explicitly rather than implied.

Required controls:

- approval workflow for protocol and rule changes
- versioned alert logic with rollback
- reason capture for overrides of drug interaction, allergy, or protocol alerts
- incident review workflow for unsafe recommendations or incorrect automation
- evaluation gates before model or prompt promotion
- forced fallback to deterministic or manual workflow when AI confidence, package validity, or sync health is below threshold

These controls apply to both human-authored configuration and AI-assisted behavior.

*Rationale: clinical correctness is a safety property, not just a feature quality metric.*

## Interfaces and User Experience

### Human Interfaces

- **Web / PWA** is the primary human interface and ships early.
- **Mobile shell** wraps the PWA for Android, iOS, and kiosk-style desktop deployments where app packaging is helpful.
- **TUI** is for SSH-only or very constrained environments, focused on admin and fallback operations.
- **CLI** is for admin, import/export, support, automation, and disaster recovery.

#### Mobile Shell Architecture

The mobile shell is a thin native wrapper around the PWA, not a native UI rewrite. The architecture is:

1. The `rustyclinic` executable runs as a local background service on the device.
2. A native Android (Kotlin) or iOS (Swift) shell hosts a WebView that connects to `http://localhost:<port>`.
3. The same web/PWA frontend renders inside the WebView — there is one UI codebase, not separate native UIs.

There is no JNI or Swift FFI bridge for UI rendering. Platform-specific native bridges exist only for hardware access that the WebView cannot reach:

- biometric and fingerprint scanner integration
- NFC and smart-card reader access
- camera for document and barcode capture
- background service lifecycle (keeping `rustyclinic` alive on Android, responding to low-memory pressure)
- local notification scheduling

On desktop kiosk deployments, the same pattern applies: a Tauri-style shell wraps the web frontend against a local `rustyclinic` process. This keeps the Rust investment in the server and domain logic while avoiding the cost and fragility of maintaining separate native UIs per platform.

### Machine Interfaces

- **FHIR REST** is the external interoperability surface — used by other health information systems (MPI, DHIS2, insurers, lab systems) to exchange clinical data in standard FHIR R4 format. It is not the primary data source for the built-in UI.
- **GraphQL** serves the built-in frontend UI and reporting dashboards. It reads from projections and materialized read models optimized for screen rendering. GraphQL is a UI query layer, not an AI or agent interface.
- **MCP** (Model Context Protocol) is the agent and AI interface. It exposes tool contracts for LLM-driven automation by invoking the same service commands as other interfaces, gated by the same authorization, audit, and policy checks. Agents interact through MCP; humans interact through GraphQL-backed screens.

The separation is intentional: GraphQL is optimized for fast, projection-backed reads that drive UI components. MCP is optimized for structured tool invocation with confirmation and co-sign semantics. FHIR REST is optimized for standards-based data exchange with external systems. All three call the same service layer underneath.

### Peripheral and Messaging Interfaces

- printer and scanner integration
- fingerprint scanner support
- patient cards and barcode/QR workflows
- SMS and USSD reminders, status checks, and limited remote interactions
- SSE or websocket queue displays for waiting rooms

The same domain rules, package resolution, and authorization policies apply across all interfaces.

*Rationale: the architecture serves humans first, agents second, and must degrade gracefully to the interfaces sites can actually support.*

## Reporting, Data Quality, and Interoperability

### Reporting Pipeline

Transactional facts feed projections, which feed indicators, registers, exports, and dashboards. Indicator definitions and DHIS2 mappings are package-driven and effective-dated.

### Data Quality Subsystem

Data quality is a first-class module with:

- completeness checks
- consistency rules
- duplicate detection worklists
- late-entry detection
- unresolved conflict tracking
- missing-result tracking
- register-to-source reconciliation
- indicator discrepancy reports

### Interoperability

Core interop targets are:

- DHIS2 via ADX and API push/pull workflows
- OpenHIE client registry and facility registry patterns
- MPI integration
- HL7 v2 for legacy systems
- DICOM and imaging references
- document exchange via `DocumentReference` and `Binary`
- migration from OpenMRS, OpenEMR, CSV, and other legacy datasets

Interop operations run through jobs and state machines where appropriate so that retries, acknowledgements, and failure review are visible and auditable.

*Rationale: reporting and interop are operational obligations, not side projects. They need durable workflows and data-quality guardrails.*

## AI, MCP, and Research

### Capability Ladder

RustyClinic introduces intelligence in a controlled order:

1. deterministic CDS and rule-based checks
2. local ONNX summarization, coding assist, and NLP
3. cloud LLM summarization and drafting with egress policy
4. draft operational commands for human review
5. co-signed operational execution for narrow, high-confidence workflows

### MCP and Agent Policy

Every MCP tool declares:

- required permissions
- scope rules
- idempotency behavior
- whether human confirmation is required
- whether a co-sign is required
- whether PHI can leave the local boundary
- expected result schema and provenance fields

Agents never receive a privileged bypass around service-layer policies.

### Research and Federated Learning

Research capabilities are isolated from core care workflows:

- de-identification profiles
- governed cohort discovery
- dataset exports with approval workflows
- federated rounds, secure aggregation, and differential privacy

Federated learning is optional and must not sit on the critical path for core EMR delivery.

*Rationale: AI should make the system more useful, not more fragile; research should be enabled without slowing clinical operations.*

## Storage and Logical Schemas

The physical schema differs slightly by backend, but the logical domains are consistent.

| Schema / domain | Representative tables or stores |
|---|---|
| `auth` | `user`, `role`, `session`, `offline_token`, `device`, `shared_device_session` |
| `identity` | `patient`, `enterprise_person`, `identifier`, `identity_link`, `biometric_template`, `mpi_link` |
| `clinical` | `resource_store`, `encounter_index`, `queue_entry`, `lab_order`, `specimen`, `medication_dispense`, `admission`, `referral` |
| `billing` | `coverage`, `eligibility_check`, `tariff`, `claim_case`, `payment`, `waiver`, `mobile_money_txn` |
| `ops` | `audit_log`, `outbox_event`, `idempotency_record`, `scheduled_job`, `job_run`, `projection_checkpoint` |
| `sync` | `operation_log`, `replica_state`, `snapshot`, `conflict_queue`, `transfer_batch` |
| `packages` | `package_artifact`, `installed_package`, `package_activation`, `package_dependency`, `package_revocation` |
| `reporting` | `indicator_definition`, `aggregate_fact`, `report_run`, `export_run`, `data_quality_issue` |
| `ai` | `resource_embedding`, `model_registry`, `prompt_policy`, `agent_action_log` |
| `research` | `cohort_definition`, `dataset_export`, `data_use_agreement`, `federated_round` |
| `files` | documents, scans, reports, images, and package blobs in local or object storage |

Facility-scoped data is the default. Control-plane tables are limited and explicit.

## Backup, Restore, and Update

### Backup and Restore

- encrypted local backups for SQLite and PostgreSQL
- verified restore drills as scheduled jobs
- USB export and import for offline movement
- optional remote or object-store backup for online deployments
- package artifacts and activation state included in backup consistency rules

### Updates

The executable and packages are updated separately.

- Executable updates are signed, staged, health-checked, and rollback-capable.
- Package updates are staged, verified, and activated transactionally.
- Offline sites can update by signed USB media.
- Failed package or executable activation reverts automatically to the last known good state.

*Rationale: operational recovery matters as much as feature depth in low-resource settings.*

## CI/CD and Verification Strategy

The test strategy must verify architecture, not just code style.

### Build Matrix

- compile every supported tier
- compile every runtime role
- verify package compatibility metadata
- verify optional infrastructure combinations, including PostgreSQL-only standard deployments

### Automated Test Categories

- dual-backend parity tests: every repository method is tested against both SQLite and PostgreSQL using a shared test harness that runs each test case on both backends and asserts identical results
- property-based tests for idempotency, hash-chain integrity, sync commutativity, and package dependency resolution determinism
- state machine transition exhaustive tests: every declared state machine is tested for all valid transitions, all invalid transitions (must be rejected), and permission requirements per transition
- migration tests
- row-level security and scope-isolation tests
- idempotency property tests
- outbox and projection replay tests
- sync and conflict tests
- package install and rollback tests
- power-loss and crash-recovery tests
- backup and restore verification tests
- clinical safety regression suites for CDS and workflow transitions
- de-identification and export policy tests
- UI form-render parity tests across web and mobile shell

### Release Gates

A release is not eligible unless it passes:

- security scanning and dependency review
- migration forward and backward verification
- signed artifact generation
- smoke deployment in SQLite and PostgreSQL modes
- representative package activation tests
- sync replay from an old replica state
- rollback verification for executable and package updates

## Observability and Operational Logging

### Structured Logging

All crates use the `tracing` crate with structured JSON output. Every log line includes:

- `timestamp` (ISO 8601)
- `level` (trace, debug, info, warn, error)
- `span` (request ID, command name, job ID)
- `facility_id` (tenant context)
- `user_id` (actor, if authenticated)
- `device_id` (originating device)

Correlation: every HTTP request, service command, and background job generates a trace ID that propagates through all log lines and audit entries for that operation.

### Key Metrics

- Sync latency (push/pull p50, p95, p99)
- Projection rebuild time and lag
- Active sessions per device
- Queue wait times (mean, p95)
- Form submission success/failure rate
- Package activation success/failure rate
- Op-log pending count per device

### Per-Tier Alerting

- **Nano/micro (offline):** Alerts are displayed as UI banners and queued for next sync push to the control plane.
- **Standard/enterprise (online):** Metrics exported to Prometheus-compatible endpoint. Alerts via configured channels (SMS, email, control plane dashboard).
- **Fleet-level:** Each device periodically reports health metrics (disk usage, RAM, sync status, battery level, credential freshness) to the control plane. The control plane aggregates into a fleet dashboard.

## Fleet Operations

The control plane provides fleet-level visibility for district IT officers managing multiple clinic installations:

- **Fleet health dashboard:** Which devices synced, which are stale (>48h), which have unresolved conflicts, which have pending package updates.
- **Remote diagnostics:** Devices self-report health metrics to the control plane on each sync. No SSH required.
- **Automated alert escalation:** Device hasn't synced in 48 hours → SMS alert to district IT officer. Configurable per facility.
- **Capacity planning:** Disk usage trending, patient volume growth, op-log growth rate per device.
- **CLI diagnostics:** `rustyclinic diagnose` runs local health checks (database integrity, op-log consistency, credential freshness, sync state, package version drift) and outputs a structured report. `rustyclinic repair` runs safe automatic fixes (projection rebuilds, op-log re-chain, stale draft cleanup).

## Power and Connectivity Resilience

### Crash Recovery

- SQLite runs in WAL mode with `PRAGMA synchronous = NORMAL` for a balance of durability and performance. On nano devices with flash storage, `synchronous = FULL` is available as a configuration option for maximum durability at the cost of write latency.
- On startup after unclean shutdown, the application runs a recovery sequence: WAL checkpoint, op-log hash-chain verification, draft recovery check, and projection staleness assessment.
- The write flow (11 steps) is atomic via a single database transaction. Power loss mid-commit results in a rollback on restart — no partial state.

### Battery-Aware Scheduling

On nano devices, background jobs (projection rebuilds, op-log pruning, data quality scans) are deferred to charging or idle periods. The scheduler queries battery state via the platform API (Android) or a configurable threshold (default: defer non-essential jobs below 20% battery).

### Form Auto-Save

Form drafts are auto-saved every 30 seconds to a local SQLite table keyed by `(user_id, encounter_id, form_family, form_version)`. On unclean shutdown, drafts are recoverable on next login. Maximum data loss: 30 seconds of form entry. See the form engine design doc for details.

### Network Flap Handling

Sync connections that drop mid-transfer resume from the last acknowledged batch, not from the beginning. Push and pull operations are chunked and idempotent. Exponential backoff (1s, 2s, 4s, 8s, max 60s) prevents thundering herd on connectivity restoration.

## Device Security Lifecycle

Every device is registered with the control plane and has a certificate-based identity. Device status transitions are audited.

- **Registration:** Device presents a certificate signing request. Admin approves. Device receives a signed certificate.
- **Suspension:** Temporarily blocks sync and auth. Reversible by admin.
- **Revocation:** Permanently blocks the device. Sync requests return 403. On next connection, the device receives a remote wipe command.
- **Lost/stolen response:** Admin marks device as lost. All sync is blocked. On reconnection, device is wiped (patient data, credentials, drafts, sync state). The executable and wipe confirmation log are retained for audit.
- **Encryption at rest:** SQLite databases are encrypted with SQLCipher (AES-256). Key is derived from the facility key and a device-specific salt.

## Training and Guided Workflows

- **Training facility sandbox:** A special "training" facility scope with synthetic patients. Data created in training mode never syncs to production. Toggled via a visible banner: "TRAINING MODE — no real patient data."
- **Guided walkthrough:** First-time users are offered a step-by-step walkthrough of core workflows (register patient, add to queue, open encounter, submit form) using the training sandbox.
- **Context-sensitive help:** Form fields and workflow steps can include help text delivered via packages. Help content is localized and versioned alongside forms.
- **Competency checkpoints:** Facility admins can require completion of training workflows before granting access to advanced features (pharmacy dispense, billing, lab results). Checkpoints are tracked per user.
- **Offline training materials:** Training content is bundled as a package type and available offline.

## Data Sovereignty and Regulatory Compliance

- **Data residency:** Health data remains within the deployment boundary (facility, district, or country) unless an explicit egress policy permits transfer. Cross-border data transfer requires policy configuration and audit logging.
- **Compliance as packages:** Country-specific regulatory requirements (consent forms, audit report formats, data retention rules, government access provisions) are delivered as deployment packages. This allows multi-country deployments with per-country compliance without code forks.
- **Consent management:** Beyond FHIR Consent resources, the platform supports informed consent workflows for data sharing, research participation, and biometric enrollment. Consent status is checked as part of the PHI egress policy.
- **Regulatory audit support:** Audit logs can be exported in formats required by national health authorities. Government audit access is scoped and logged.

## Accessibility and Low-Literacy UX

- **Large touch targets:** Minimum 48px for all interactive elements (buttons, inputs, nav items, table rows).
- **Icon-heavy navigation:** Sidebar nav uses icons alongside labels. On mobile, bottom navigation uses icon-only with labels below.
- **High contrast:** The color palette (warm slate + burnt orange) is designed for outdoor sunlight readability. WCAG AA contrast ratios are the minimum.
- **Color-blind safe:** No information is encoded by color alone. Status badges use color + icon + label.
- **Voice-guided workflows (future):** The architecture supports audio prompts for form fields, delivered as package content. This is a Phase 5+ feature.
- **Low-bandwidth optimization:** Single font family (Source Sans 3), minimal CSS, no decorative assets. The PWA shell caches aggressively. Images are compressed and lazy-loaded.

## Developer Experience

- **`rustyclinic dev`:** A CLI command that starts a local development environment with a SQLite database, seed data (synthetic patients, sample forms, test packages), and the web UI. One command from clone to running app.
- **Package SDK:** A development toolkit for authoring form packs, terminology packs, report packs, and deployment packs without touching core Rust code. Includes a validator, a local test harness, and documentation.
- **Playground mode:** A demo mode with pre-loaded packages and synthetic data for conferences, training, and evaluation. Accessible via `rustyclinic serve all --playground`.
- **Architecture Decision Records:** Major design decisions are recorded in `docs/designs/` with context, alternatives considered, and rationale. This is the canonical location for design docs produced by reviews.

## Implementation Phases

| Phase | Deliverable | What must work |
|---|---|---|
| 1 | Platform kernel | Core types, services, auth skeleton, DB abstraction, audit, outbox, idempotency, basic package registry |
| 2 | Human-operable core | Web/PWA shell, patient registration, search, queue, encounter capture, printing, shared-device mode |
| 3 | Offline and sync foundation | SQLite/PostgreSQL parity, operation log, snapshots, backup/restore, sync status UI, conflict queue baseline |
| 4 | Forms and package runtime | Form engine, deployment packs, terminology packs, report packs, package activation and rollback |
| 5 | Clinical operations | Appointments, lab and specimen flow, pharmacy dispense, immunization, referrals, admissions and bed state |
| 6 | Billing and payer workflows | Coverage verification, tariffs, claims, waivers, payments, mobile money, claims worklists |
| 7 | Reporting and data quality | Registers, HMIS indicators, DHIS2 export, discrepancy dashboards, monthly reporting workflows |
| 8 | Identity and interop | Enterprise person, MPI integration, facility registry, HL7 v2, DICOM references, migration tooling |
| 9 | Deterministic intelligence and MCP | CDS rules, read-only summarization, note drafting, MCP tool contracts, confirmation and co-sign flows |
| 10 | Research and federated features | De-identification, governed cohort discovery, research export, federated rounds |

The architecture is complete at the end of Phase 10, but usable clinic deployments should already exist by the end of Phase 4 and expand materially by the end of Phase 6.

## Normative Design Decisions

1. **Single executable, multi-role runtime**  
   Keeps distribution and updates simple while preserving process-level separation where scale demands it.

2. **Packages over forks**  
   Country, insurer, and program variation changes faster than platform code. Signed packages make that variability governable and deployable.

3. **FHIR at the edge, state machines in the core**  
   FHIR is excellent for interoperability; explicit aggregates are better for operational correctness.

4. **Projection-backed reads**  
   Clinical systems need both normalized writes and fast dashboards. Projections are the contract between those requirements.

5. **Scheduler as a core subsystem**  
   Claims, exports, backups, retries, sync catch-up, and updates are time-based operations that must survive restarts.

6. **Identity as a layered model**  
   Local care, cross-facility matching, MPI linkage, and biometrics are separate concerns that must cooperate without collapsing into one brittle identifier.

7. **Security as a trust model, not a checkbox**  
   RBAC and encryption are table stakes; egress control, key management, break-glass review, and agent provenance are equally necessary.

8. **Offline-first as an architecture decision**  
   It affects storage, sync, UI, jobs, packages, and recovery. It cannot be deferred to the end.

9. **AI is helpful only when policy-aware**  
   Deterministic rules come first, draft-first automation comes before autonomous execution, and all agent actions remain attributable.

10. **Research is optional but governed**  
    The platform enables research and federated learning without making them a dependency for care delivery.
