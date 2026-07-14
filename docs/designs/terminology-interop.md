# Terminology and Interop Design

## Context

RustyClinic needs structured clinical data that is usable by humans first, and then by reporting, billing, interoperability, and AI layers without rework. That requires a terminology strategy that distinguishes between:

- exchange format
- classification systems
- rich clinical vocabularies
- observation/lab coding
- units of measure

This design doc captures the current direction for `rustyclinic-terminology` and `rustyclinic-interop`.

## Goals

- Make diagnosis, observation, lab, and medication data machine-usable without making frontline workflows terminology-hostile.
- Keep the internal operational model independent from raw FHIR CRUD.
- Support package-driven local subsets, translations, and mappings.
- Allow gradual adoption: useful with curated starter subsets first, then stronger with imported official releases.

## Standards Roles

### FHIR

FHIR R4 is the interoperability boundary. It defines how RustyClinic exchanges data with external systems and how standards-shaped APIs are exposed.

FHIR is not the primary internal workflow model. Operational workflows remain explicit aggregates and state machines, with FHIR resources projected from those canonical records.

### ICD-11

ICD-11 is the primary diagnosis classification layer for:

- morbidity and mortality reporting
- claims and reimbursement workflows where applicable
- health information exchange and exports that expect diagnosis classification

Clinicians should not be forced to browse the full ICD hierarchy. The system should provide search, suggestion, and confirmation around clinician-friendly labels.

### SNOMED CT

SNOMED CT is the richer semantics layer for:

- diagnoses
- symptoms
- findings
- procedures
- medication concepts

SNOMED CT is optional at first-touch UX in many deployments, but the data model should leave room for it now so we do not paint ourselves into a corner later.

### LOINC

LOINC is the coding layer for:

- vital signs
- observations
- lab orders
- lab results

LOINC should back the platform’s canonical bindings for measurements and tests even when the user sees deployment-local labels.

### UCUM

UCUM is the canonical units layer for:

- weight
- height
- temperature
- blood pressure
- pulse
- glucose
- hemoglobin
- other measured values

Using UCUM consistently is essential for safe export, analytics, and AI consumption.

## Architectural Decisions

### 1. Layered Model

RustyClinic uses:

- FHIR for exchange
- ICD-11 for classification
- SNOMED CT for richer clinical semantics
- LOINC for observations and lab coding
- UCUM for units

No single one of these standards replaces the others.

### 2. Structured-First Clinical Data

Clinical capture should be hybrid:

- structured fields for high-value canonical facts
- short narrative fields for nuance and context

This supports:

- better validation
- better summaries
- better claims and reporting
- cleaner AI context windows

### 3. Human-Friendly UX, Terminology-Backed Data

The UI should prefer:

- search and typeahead
- common-condition shortcuts
- local language display strings
- deployment-specific subsets

The system should persist the selected coding metadata underneath that UX.

### 4. Import Pipelines Are Part of the Platform

Terminology import is not a one-time setup script. The platform needs:

- repeatable imports
- source tracking
- import run metadata
- release/version visibility
- replace/update behavior

This is why the terminology schema includes concepts, designations, artifacts, and import runs.

## Current Implementation Direction

### `rustyclinic-terminology`

Owns:

- code-system constants and starter mappings
- terminology search helpers
- binding helpers for diagnoses, observations, lab tests, medications, and UCUM units
- import pipelines for:
  - ICD-11
  - LOINC
  - UCUM
  - FHIR artifacts
  - SNOMED CT

Persisted terminology tables:

- `terminology_concepts`
- `terminology_designations`
- `terminology_artifacts`
- `terminology_import_runs`

### `rustyclinic-interop`

Owns:

- terminology search APIs
- FHIR-facing export surfaces
- transformation from canonical RustyClinic data into FHIR resources

Current direction is export-first rather than fully generic FHIR server behavior.

## Rollout Plan

### Phase A — Foundation

- Add terminology crate and interop crate
- Add starter bindings for diagnoses, vitals, and lab tests
- Add terminology import schema
- Add admin import/download commands

### Phase B — Structured Clinical Capture

- Bind diagnoses to ICD-11
- Bind observations and labs to LOINC plus UCUM
- Preserve clinician-facing label alongside code metadata
- Keep room for SNOMED CT enrichment

### Phase C — UX Integration

- terminology-backed diagnosis selection
- terminology-backed lab ordering
- terminology-backed result entry
- package-specific subsets and translations

### Phase D — Interop Hardening

- profile-aware FHIR validation
- stronger terminology bindings in exports
- deployment-specific mappings and packaging

## Source Strategy

The platform should prefer official or officially structured sources:

- ICD-11: WHO API or WHO-compatible export workflow
- LOINC: official release archive
- UCUM: official artifact source
- FHIR: official HL7 core packages and terminology artifacts
- SNOMED CT: official RF2 releases for licensed territory

Open source distribution of RustyClinic does not remove the need to respect upstream licensing and distribution terms for terminology content.

## Documentation Follow-Up

This design doc should stay aligned with:

- `architecture.md`
- package design
- interop design
- future clinical UX work for diagnosis, observations, orders, and labs
