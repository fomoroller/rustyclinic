# Form Engine Design

## Context

The architecture document states: "configurable forms and program workflows are half the product." The form engine is a core platform service that powers patient registration, encounter capture, lab ordering, pharmacy dispense, billing forms, community health worker visit forms, and program-specific workflows (ANC, HIV, TB, immunization, etc.).

This design doc specifies the technical approach for the form engine defined in `rustyclinic-forms`.

## Requirements from Architecture

Each form definition includes:
- a canonical FHIR `Questionnaire`
- renderer metadata for web, mobile shell, and TUI
- skip logic and conditional visibility
- computed fields
- validation rules
- mapping rules to FHIR resources and domain commands
- print layout definitions
- data-quality checks
- effective dates and protocol version metadata
- draft migration rules when the form schema changes

Additional requirements from reviews:
- In-flight forms are version-pinned (Eng Review decision #6)
- Form draft auto-save (CEO Review: power resilience expansion)
- Must work on nano tier (150MB RAM budget, 4 ARM cores)
- Forms are delivered as packages

## Design Decisions

### 1. Expression Language: JSON Rule Trees

Skip logic, computed fields, and validation rules use a **JSON-based expression tree**, not an embedded scripting language.

**Decision rationale:**
- Lua/WASM scripting adds runtime complexity, binary size, and security surface
- JSON rule trees are serializable, versionable, diffable, and package-friendly
- FHIR `Questionnaire` already uses `enableWhen` expressions — we extend that model
- Rule trees can be statically analyzed for correctness before activation
- Rule trees are fast to evaluate (no interpreter overhead)

**Expression node types:**

```text
Expression = Literal | FieldRef | BinaryOp | UnaryOp | FunctionCall | Conditional

Literal     = { type: "literal", value: any, dataType: "string"|"integer"|"decimal"|"boolean"|"date"|"coding" }
FieldRef    = { type: "field", linkId: string, property?: "value"|"exists"|"count"|"length" }
BinaryOp    = { type: "op", op: "eq"|"ne"|"gt"|"lt"|"ge"|"le"|"and"|"or"|"add"|"sub"|"mul"|"div", left: Expression, right: Expression }
UnaryOp     = { type: "not"|"exists"|"empty", operand: Expression }
FunctionCall= { type: "fn", name: string, args: Expression[] }
Conditional = { type: "if", condition: Expression, then: Expression, else: Expression }
```

**Built-in functions:**

| Function | Description | Example |
|----------|-------------|---------|
| `age(date)` | Years between date and now | `age(field("dob"))` |
| `bmi(weight_kg, height_cm)` | BMI calculation | `bmi(field("weight"), field("height"))` |
| `gestational_age(lmp_date)` | Weeks since LMP | `gestational_age(field("lmp"))` |
| `days_between(date1, date2)` | Day count | `days_between(field("visit_date"), today())` |
| `today()` | Current date | — |
| `now()` | Current datetime | — |
| `sum(field_linkId)` | Sum of repeating group values | `sum("medication_qty")` |
| `count(field_linkId)` | Count of repeating group entries | `count("diagnoses")` |
| `contains(coding_field, system, code)` | Coding membership test | `contains(field("dx"), "icd10", "B20")` |
| `lookup(table, key)` | Package-provided lookup table | `lookup("tariffs", field("procedure_code"))` |

Functions are a closed set defined in `rustyclinic-forms`. Packages cannot add arbitrary functions — they can only compose expressions from built-in nodes. This prevents packages from injecting executable code.

### 2. Skip Logic and Conditional Visibility

Each form item can declare an `enableWhen` expression:

```json
{
  "linkId": "hiv_arv_regimen",
  "type": "choice",
  "text": "Current ARV regimen",
  "enableWhen": {
    "type": "op",
    "op": "eq",
    "left": { "type": "field", "linkId": "hiv_status" },
    "right": { "type": "literal", "value": "positive", "dataType": "string" }
  }
}
```

**Evaluation semantics:**
- `enableWhen` is re-evaluated on every field change that could affect it
- Dependency tracking: the engine builds a directed acyclic graph (DAG) of field dependencies at form load time
- Only affected expressions are re-evaluated on each change (not the entire form)
- Hidden fields retain their values but are excluded from submission and validation

```text
DEPENDENCY DAG EXAMPLE (ANC Visit Form):

  hiv_status ──▶ hiv_arv_regimen
              ──▶ hiv_viral_load
              ──▶ pmtct_section

  gestational_age(lmp) ──▶ trimester_display
                       ──▶ anemia_screening_due
                       ──▶ ultrasound_recommended

  weight, height ──▶ bmi_display
                 ──▶ nutritional_risk_flag
```

### 3. Computed Fields

Computed fields use the same expression tree as skip logic. They are read-only fields whose value is derived from other fields.

```json
{
  "linkId": "bmi_display",
  "type": "decimal",
  "text": "BMI",
  "readOnly": true,
  "computedValue": {
    "type": "fn",
    "name": "bmi",
    "args": [
      { "type": "field", "linkId": "weight_kg" },
      { "type": "field", "linkId": "height_cm" }
    ]
  }
}
```

**Evaluation order:** Computed fields are evaluated in topological order of the dependency DAG. Circular dependencies are detected at form load time and rejected with a diagnostic.

### 4. Validation Rules

Validation rules are expressions that must evaluate to `true` for the form to be submittable. Each rule has a severity and a human-readable message (localized via the terminology/translation system).

```json
{
  "linkId": "weight_kg",
  "validation": [
    {
      "expression": { "type": "op", "op": "gt", "left": { "type": "field", "linkId": "weight_kg" }, "right": { "type": "literal", "value": 0, "dataType": "decimal" } },
      "severity": "error",
      "message": { "key": "validation.weight.positive" }
    },
    {
      "expression": { "type": "op", "op": "lt", "left": { "type": "field", "linkId": "weight_kg" }, "right": { "type": "literal", "value": 300, "dataType": "decimal" } },
      "severity": "warning",
      "message": { "key": "validation.weight.plausible" }
    }
  ]
}
```

**Severity levels:**
- `error` — blocks submission
- `warning` — shows warning, allows submission with acknowledgment
- `info` — advisory only

### 5. FHIR Mapping Rules

Each form defines mapping rules that transform a `QuestionnaireResponse` into domain commands and FHIR resources.

```json
{
  "mappings": [
    {
      "type": "create_resource",
      "resourceType": "Observation",
      "fields": {
        "code": { "system": "http://loinc.org", "code": "29463-7" },
        "valueQuantity.value": { "type": "field", "linkId": "weight_kg" },
        "valueQuantity.unit": "kg"
      },
      "condition": { "type": "field", "linkId": "weight_kg", "property": "exists" }
    },
    {
      "type": "service_command",
      "command": "CreateEncounterObservation",
      "args": {
        "encounter_id": { "type": "context", "key": "encounter_id" },
        "observation_code": "weight",
        "value": { "type": "field", "linkId": "weight_kg" }
      }
    }
  ]
}
```

Mappings are evaluated after successful validation. They produce a list of domain commands that are submitted through `rustyclinic-services` in a single transaction.

### 6. Form Versioning and In-Flight Pinning

**Version pinning rule:** When a clinician opens an encounter or visit, the form engine resolves the active form version for that form family at that moment. The resolved version is recorded on the encounter/visit record. All subsequent form interactions for that encounter use the pinned version, even if a new package activates a newer form version during the visit.

```text
FORM VERSION LIFECYCLE:

  [Package v1 active]
       │
  Nurse opens encounter ──▶ encounter.form_version = "anc-visit:1.2.0"
       │
  [Package v2 activates]    (no effect on this encounter)
       │
  Nurse continues form ──▶ still uses anc-visit:1.2.0
       │
  Nurse submits form ──▶ QuestionnaireResponse references anc-visit:1.2.0
       │
  Next encounter ──▶ encounter.form_version = "anc-visit:2.0.0" (new version)
```

**Historical rendering:** Old form versions remain available in the package registry (expired packages are retained for historical interpretation, per the architecture). When viewing a past encounter, the form engine renders the response using the form version recorded on that encounter.

### 7. Draft Auto-Save and Recovery

**Auto-save interval:** Every 30 seconds while a form has unsaved changes, the current field state is persisted to a local draft store.

**Draft storage:**
- Key: `(user_id, encounter_id, form_family, form_version)`
- Value: JSON snapshot of all field values + cursor position + scroll offset
- Storage: SQLite table `form_draft` in the local database (not synced)

**Recovery flow:**
1. On encounter open, check for an existing draft matching the key.
2. If found, prompt: "You have unsaved changes from [timestamp]. Resume or discard?"
3. If resumed, restore field values and re-evaluate all expressions.
4. Drafts are deleted on successful form submission.
5. Drafts older than 7 days are pruned by the maintenance job.

**Power loss scenario:**
- SQLite WAL mode ensures the draft write is durable (survives unclean shutdown).
- On restart after crash, the encounter list shows a "draft" indicator for encounters with saved drafts.
- Worst-case data loss: up to 30 seconds of form entry (the auto-save interval).

### 8. Draft Migration

When a form version changes and a user has a saved draft under the old version, draft migration rules are applied:

```json
{
  "draftMigration": {
    "from": "anc-visit:1.2.0",
    "to": "anc-visit:2.0.0",
    "fieldMappings": [
      { "from": "weight", "to": "weight_kg", "transform": null },
      { "from": "bp_systolic", "to": "bp.systolic", "transform": null },
      { "from": "hiv_test_result", "to": "hiv_rapid_test.result", "transform": null }
    ],
    "droppedFields": ["old_field_name"],
    "newRequiredFields": ["new_field_that_needs_input"]
  }
}
```

**Migration semantics:**
- Field values are mapped from old linkIds to new linkIds.
- Dropped fields are discarded (with a user notification listing what was lost).
- New required fields are highlighted as needing input.
- If no migration rules exist for a version pair, the draft is preserved as-is under the old version (version pinning applies).

### 9. Print Layout

Print layouts are defined per form as a separate artifact within the package:

```text
PRINT LAYOUT STRUCTURE:

  Form Definition
    ├── questionnaire.json     (FHIR Questionnaire + extensions)
    ├── ui-schema.json         (renderer metadata per platform)
    ├── print-layout.json      (print template definition)
    └── migrations/            (draft migration rules)
```

Print layouts support:
- A4 and thermal receipt paper formats
- Header/footer with facility branding (from deployment package)
- Field placement by grid coordinates
- Conditional sections (same expression tree as skip logic)
- Barcode/QR code generation for patient identifiers
- Localized labels from the terminology system

Print rendering happens server-side (the `rustyclinic` process generates PDF or direct ESC/POS commands) to avoid browser print inconsistencies.

### 10. Nano-Tier Performance Targets

| Metric | Target | Rationale |
|--------|--------|-----------|
| Form load time | < 200ms | Includes expression DAG construction, draft check, and initial evaluation |
| Field change response | < 50ms | Re-evaluate affected expressions, update computed fields, re-render affected items |
| Memory per loaded form | < 2MB | A complex ANC form with 100+ fields, 50+ rules, 20+ computed fields |
| Form definition size | < 100KB | Compressed JSON in package. Largest forms (ANC, HIV) stay under this |
| Submission mapping | < 500ms | Transform QuestionnaireResponse → domain commands |
| Print generation | < 2s | PDF generation for A4 print layout |

**Performance strategy:**
- Expression trees are compiled to a flat instruction array at form load time (not interpreted as JSON trees at evaluation time)
- The instruction array uses stack-based evaluation (similar to a bytecode VM)
- No dynamic memory allocation during expression evaluation — all intermediate values use a pre-allocated stack
- Field dependency DAG enables minimal re-evaluation on each change

```text
EXPRESSION COMPILATION:

  JSON tree:                          Flat instructions:
  { op: "and",                        PUSH field("hiv_status")
    left: { op: "eq",                 PUSH literal("positive")
      left: field("hiv_status"),      EQ
      right: literal("positive")      PUSH field("on_art")
    },                                PUSH literal(true)
    right: { op: "eq",               EQ
      left: field("on_art"),          AND
      right: literal(true)
    }
  }
```

### 11. Renderer Contracts

The form engine produces a **renderer-neutral evaluation state** that each renderer consumes:

```text
FORM ENGINE (rustyclinic-forms)          RENDERERS

  ┌──────────────────────┐              ┌──────────────┐
  │ Form Definition      │              │ Web/PWA      │
  │ + Field Values       │──evaluate──▶ │ Renderer     │
  │ + Expression Engine  │              └──────────────┘
  │                      │              ┌──────────────┐
  │ Produces:            │──evaluate──▶ │ Mobile Shell │
  │ - visibility[]       │              │ Renderer     │
  │ - computed_values[]  │              └──────────────┘
  │ - validation_errors[]│              ┌──────────────┐
  │ - field_states[]     │──evaluate──▶ │ TUI Renderer │
  │                      │              └──────────────┘
  └──────────────────────┘              ┌──────────────┐
                                   ──▶  │ Print        │
                                        │ Renderer     │
                                        └──────────────┘
```

The evaluation state is a flat struct:

```rust
pub struct FormEvaluationState {
    pub field_values: HashMap<LinkId, FieldValue>,
    pub visibility: HashMap<LinkId, bool>,
    pub computed_values: HashMap<LinkId, FieldValue>,
    pub validation_results: Vec<ValidationResult>,
    pub is_submittable: bool,
    pub dirty_since_last_save: bool,
    pub pinned_version: FormVersion,
}
```

Each renderer (web, mobile, TUI, print) consumes this state and renders appropriately for its platform. The form engine does NOT produce HTML or UI trees — that's the renderer's job.

### 12. Data Quality Checks

Data quality checks are post-submission rules that flag records for review without blocking the clinical workflow:

- **Completeness**: required fields that were left empty (severity: warning, not error — sometimes data genuinely isn't available)
- **Consistency**: cross-field plausibility (e.g., gestational age > 42 weeks, weight loss > 20% between visits)
- **Timeliness**: form submitted more than 48 hours after the visit date
- **Duplicate detection**: same patient, same form family, same visit date

Data quality issues are recorded as `DataQualityIssue` aggregates (per the architecture) and surfaced in the data quality projection.

## Crate Boundary

`rustyclinic-forms` contains:
- Form definition types and parser
- Expression tree types and compiler (JSON → flat instructions)
- Stack-based expression evaluator
- Dependency DAG builder
- Validation engine
- Draft storage interface (trait, not impl — storage is in `rustyclinic-db`)
- FHIR mapping engine (produces domain commands for `rustyclinic-services`)
- Print layout types (rendering is in `rustyclinic-web` for PDF, `rustyclinic-api` for ESC/POS)
- Form version resolution (delegates to `rustyclinic-packages` for active version lookup)

`rustyclinic-forms` does NOT contain:
- UI rendering code (that's `rustyclinic-web`)
- Database storage (that's `rustyclinic-db`)
- Package loading (that's `rustyclinic-packages`)
- Terminology resolution (that's `rustyclinic-terminology`)

## Open Questions

1. **Should the expression language support FHIRPath?** FHIRPath is the standard expression language for FHIR `Questionnaire`. Supporting a subset would improve interoperability with existing form definitions from WHO and other standard bodies. However, full FHIRPath is complex and may be overkill. Decision: support a FHIRPath-compatible subset that maps to our expression tree nodes. Document which FHIRPath functions are supported and which are not.

2. **Should forms support repeating groups with dynamic add/remove?** Yes — immunization schedules, medication lists, and diagnosis lists all require repeating groups. The expression tree supports `count()` and `sum()` over repeating groups. The DAG tracks dependencies per group instance.

3. **Should the print renderer support custom fonts?** For localization to Amharic and other non-Latin scripts, yes. Font assets are included in deployment packages. The print renderer embeds fonts in generated PDFs.
