//! Lightweight terminology catalog for diagnosis, labs, and units.

pub mod import;

use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};

pub const ICD11_SYSTEM: &str = "http://id.who.int/icd/release/11/mms";
pub const SNOMED_SYSTEM: &str = "http://snomed.info/sct";
pub const LOINC_SYSTEM: &str = "http://loinc.org";
pub const UCUM_SYSTEM: &str = "http://unitsofmeasure.org";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeSystem {
    Icd11,
    SnomedCt,
    Loinc,
    Ucum,
}

impl CodeSystem {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "icd11" => Some(Self::Icd11),
            "snomed" | "snomedct" | "snomed_ct" => Some(Self::SnomedCt),
            "loinc" => Some(Self::Loinc),
            "ucum" => Some(Self::Ucum),
            _ => None,
        }
    }

    pub fn canonical_url(&self) -> &'static str {
        match self {
            Self::Icd11 => ICD11_SYSTEM,
            Self::SnomedCt => SNOMED_SYSTEM,
            Self::Loinc => LOINC_SYSTEM,
            Self::Ucum => UCUM_SYSTEM,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Coding {
    pub system: String,
    pub code: String,
    pub display: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TerminologyConcept {
    pub coding: Coding,
    #[serde(default)]
    pub synonyms: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiagnosisBinding {
    pub local_value: String,
    pub clinician_label: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icd11: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snomed: Option<Coding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObservationBinding {
    pub link_id: String,
    pub label: String,
    pub loinc: Coding,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ucum: Option<Coding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LabTestBinding {
    pub local_code: String,
    pub local_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loinc: Option<Coding>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_ucum: Option<Coding>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MedicationBinding {
    pub local_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snomed: Option<Coding>,
}

pub fn search(system: CodeSystem, query: &str) -> Vec<TerminologyConcept> {
    let q = query.trim().to_ascii_lowercase();
    let concepts = match system {
        CodeSystem::Icd11 => icd11_concepts(),
        CodeSystem::SnomedCt => snomed_concepts(),
        CodeSystem::Loinc => loinc_concepts(),
        CodeSystem::Ucum => ucum_concepts(),
    };

    concepts
        .into_iter()
        .filter(|concept| {
            if q.is_empty() {
                return true;
            }
            concept.coding.code.to_ascii_lowercase().contains(&q)
                || concept.coding.display.to_ascii_lowercase().contains(&q)
                || concept
                    .synonyms
                    .iter()
                    .any(|syn| syn.to_ascii_lowercase().contains(&q))
        })
        .collect()
}

pub fn has_imported_concepts(conn: &Connection, system: CodeSystem) -> rusqlite::Result<bool> {
    if !has_sqlite_table(conn, "terminology_concepts")? {
        return Ok(false);
    }

    let concept_count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM terminology_concepts WHERE system = ?1",
        params![system.canonical_url()],
        |row| row.get(0),
    )?;
    Ok(concept_count > 0)
}

pub fn search_imported(
    conn: &Connection,
    system: CodeSystem,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<TerminologyConcept>> {
    if limit == 0 || !has_sqlite_table(conn, "terminology_concepts")? {
        return Ok(Vec::new());
    }

    let has_designations = has_sqlite_table(conn, "terminology_designations")?;
    let normalized_query = query.trim().to_ascii_lowercase();
    let like_query = format!("%{normalized_query}%");

    let mut stmt = conn.prepare(
        "SELECT c.code, c.display
         FROM terminology_concepts c
         WHERE c.system = ?1
           AND c.active = 1
           AND (
               ?2 = ''
               OR lower(c.code) LIKE ?3
               OR lower(c.display) LIKE ?3
               OR EXISTS (
                   SELECT 1
                   FROM terminology_designations d
                   WHERE d.system = c.system
                     AND d.code = c.code
                     AND lower(d.value) LIKE ?3
               )
           )
         ORDER BY c.code
         LIMIT ?4",
    )?;

    let concept_rows = stmt.query_map(
        params![
            system.canonical_url(),
            normalized_query,
            like_query,
            limit as i64
        ],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
    )?;

    let mut concepts = Vec::new();
    for row in concept_rows {
        let (code, display) = row?;
        let synonyms = if has_designations {
            imported_synonyms(conn, system.canonical_url(), &code)?
        } else {
            Vec::new()
        };
        concepts.push(TerminologyConcept {
            coding: Coding {
                system: system.canonical_url().to_string(),
                code,
                display,
            },
            synonyms,
        });
    }

    Ok(concepts)
}

pub fn search_imported_any(
    conn: &Connection,
    query: &str,
    limit: usize,
) -> rusqlite::Result<Vec<TerminologyConcept>> {
    if limit == 0 || !has_sqlite_table(conn, "terminology_concepts")? {
        return Ok(Vec::new());
    }

    let has_designations = has_sqlite_table(conn, "terminology_designations")?;
    let normalized_query = query.trim().to_ascii_lowercase();
    let like_query = format!("%{normalized_query}%");

    let mut stmt = conn.prepare(
        "SELECT c.system, c.code, c.display
         FROM terminology_concepts c
         WHERE c.active = 1
           AND (
               ?1 = ''
               OR lower(c.code) LIKE ?2
               OR lower(c.display) LIKE ?2
               OR EXISTS (
                   SELECT 1
                   FROM terminology_designations d
                   WHERE d.system = c.system
                     AND d.code = c.code
                     AND lower(d.value) LIKE ?2
               )
           )
         ORDER BY
           CASE
             WHEN lower(c.display) = ?1 THEN 0
             WHEN lower(c.display) LIKE ?1 || '%' THEN 1
             WHEN lower(c.code) = ?1 THEN 2
             ELSE 3
           END,
           c.system,
           c.display
         LIMIT ?3",
    )?;

    let concept_rows =
        stmt.query_map(params![normalized_query, like_query, limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?;

    let mut concepts = Vec::new();
    for row in concept_rows {
        let (system, code, display) = row?;
        let synonyms = if has_designations {
            imported_synonyms(conn, &system, &code)?
        } else {
            Vec::new()
        };
        concepts.push(TerminologyConcept {
            coding: Coding {
                system,
                code,
                display,
            },
            synonyms,
        });
    }

    Ok(concepts)
}

fn has_sqlite_table(conn: &Connection, table_name: &str) -> rusqlite::Result<bool> {
    let table_count: i64 = conn.query_row(
        "SELECT COUNT(*)
         FROM sqlite_master
         WHERE type = 'table' AND name = ?1",
        params![table_name],
        |row| row.get(0),
    )?;
    Ok(table_count > 0)
}

fn imported_synonyms(conn: &Connection, system: &str, code: &str) -> rusqlite::Result<Vec<String>> {
    let mut stmt = conn.prepare(
        "SELECT DISTINCT value
         FROM terminology_designations
         WHERE system = ?1 AND code = ?2
         ORDER BY value",
    )?;
    let rows = stmt.query_map(params![system, code], |row| row.get::<_, String>(0))?;

    let mut synonyms = Vec::new();
    for value in rows {
        synonyms.push(value?);
    }
    Ok(synonyms)
}

pub fn diagnosis_binding(local_value: &str, other_text: Option<&str>) -> DiagnosisBinding {
    let other_text = other_text.unwrap_or("").trim();
    match local_value {
        "malaria" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Malaria".to_string(),
            icd11: Some(coding(ICD11_SYSTEM, "1F40", "Malaria")),
            snomed: Some(coding(SNOMED_SYSTEM, "61462000", "Malaria")),
        },
        "hypertension" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Hypertension".to_string(),
            icd11: Some(coding(ICD11_SYSTEM, "BA00", "Essential hypertension")),
            snomed: Some(coding(SNOMED_SYSTEM, "59621000", "Essential hypertension")),
        },
        "diabetes" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Diabetes Mellitus".to_string(),
            icd11: Some(coding(ICD11_SYSTEM, "5A11", "Type 2 diabetes mellitus")),
            snomed: Some(coding(
                SNOMED_SYSTEM,
                "44054006",
                "Diabetes mellitus type 2",
            )),
        },
        "pneumonia" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Pneumonia".to_string(),
            icd11: None,
            snomed: Some(coding(SNOMED_SYSTEM, "233604007", "Pneumonia")),
        },
        "anemia" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Anemia".to_string(),
            icd11: None,
            snomed: Some(coding(SNOMED_SYSTEM, "271737000", "Anemia")),
        },
        "uti" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Urinary Tract Infection".to_string(),
            icd11: None,
            snomed: Some(coding(
                SNOMED_SYSTEM,
                "68566005",
                "Urinary tract infectious disease",
            )),
        },
        "diarrheal_disease" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: "Diarrheal disease".to_string(),
            icd11: None,
            snomed: Some(coding(SNOMED_SYSTEM, "62315008", "Diarrhea")),
        },
        "other" => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: if other_text.is_empty() {
                "Other diagnosis".to_string()
            } else {
                other_text.to_string()
            },
            icd11: None,
            snomed: None,
        },
        _ => DiagnosisBinding {
            local_value: local_value.to_string(),
            clinician_label: if other_text.is_empty() {
                local_value.replace('_', " ")
            } else {
                other_text.to_string()
            },
            icd11: None,
            snomed: None,
        },
    }
}

pub fn observation_binding(link_id: &str) -> Option<ObservationBinding> {
    match link_id {
        "weight_kg" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Body weight".to_string(),
            loinc: coding(LOINC_SYSTEM, "29463-7", "Body weight"),
            ucum: Some(coding(UCUM_SYSTEM, "kg", "kg")),
        }),
        "height_cm" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Body height".to_string(),
            loinc: coding(LOINC_SYSTEM, "8302-2", "Body height"),
            ucum: Some(coding(UCUM_SYSTEM, "cm", "cm")),
        }),
        "temperature_c" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Body temperature".to_string(),
            loinc: coding(LOINC_SYSTEM, "8310-5", "Body temperature"),
            ucum: Some(coding(UCUM_SYSTEM, "Cel", "degC")),
        }),
        "pulse_rate" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Heart rate".to_string(),
            loinc: coding(LOINC_SYSTEM, "8867-4", "Heart rate"),
            ucum: Some(coding(UCUM_SYSTEM, "/min", "beats/minute")),
        }),
        "blood_pressure_systolic" | "bp_systolic" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Systolic blood pressure".to_string(),
            loinc: coding(LOINC_SYSTEM, "8480-6", "Systolic blood pressure"),
            ucum: Some(coding(UCUM_SYSTEM, "mm[Hg]", "mmHg")),
        }),
        "blood_pressure_diastolic" | "bp_diastolic" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Diastolic blood pressure".to_string(),
            loinc: coding(LOINC_SYSTEM, "8462-4", "Diastolic blood pressure"),
            ucum: Some(coding(UCUM_SYSTEM, "mm[Hg]", "mmHg")),
        }),
        "bmi" => Some(ObservationBinding {
            link_id: link_id.to_string(),
            label: "Body mass index".to_string(),
            loinc: coding(LOINC_SYSTEM, "39156-5", "Body mass index (BMI)"),
            ucum: Some(coding(UCUM_SYSTEM, "kg/m2", "kg/m2")),
        }),
        _ => None,
    }
}

pub fn lab_test_binding(local_code: &str) -> LabTestBinding {
    match local_code {
        "malaria_rdt" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Malaria RDT".to_string(),
            loinc: None,
            default_ucum: None,
        },
        "cbc" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Complete Blood Count".to_string(),
            loinc: Some(coding(
                LOINC_SYSTEM,
                "57021-8",
                "CBC W Auto Differential panel",
            )),
            default_ucum: None,
        },
        "blood_glucose" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Blood Glucose".to_string(),
            loinc: Some(coding(
                LOINC_SYSTEM,
                "2339-0",
                "Glucose [Mass/volume] in Blood",
            )),
            default_ucum: Some(coding(UCUM_SYSTEM, "mg/dL", "mg/dL")),
        },
        "hemoglobin" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Hemoglobin".to_string(),
            loinc: Some(coding(
                LOINC_SYSTEM,
                "718-7",
                "Hemoglobin [Mass/volume] in Blood",
            )),
            default_ucum: Some(coding(UCUM_SYSTEM, "g/dL", "g/dL")),
        },
        "urinalysis" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Urinalysis".to_string(),
            loinc: Some(coding(LOINC_SYSTEM, "24356-8", "Urinalysis complete panel")),
            default_ucum: None,
        },
        "hiv_rapid" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "HIV Rapid Test".to_string(),
            loinc: None,
            default_ucum: None,
        },
        "urine_pregnancy" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Urine Pregnancy Test".to_string(),
            loinc: Some(coding(
                LOINC_SYSTEM,
                "2106-3",
                "Choriogonadotropin (pregnancy test) [Presence] in Urine",
            )),
            default_ucum: None,
        },
        "stool_exam" => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: "Stool Examination".to_string(),
            loinc: None,
            default_ucum: None,
        },
        _ if is_loinc_shaped_code(local_code) => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: local_code.to_string(),
            loinc: Some(coding(
                LOINC_SYSTEM,
                local_code,
                loinc_display_for_code(local_code).unwrap_or(local_code),
            )),
            default_ucum: None,
        },
        _ => LabTestBinding {
            local_code: local_code.to_string(),
            local_name: local_code.replace('_', " "),
            loinc: None,
            default_ucum: None,
        },
    }
}

fn is_loinc_shaped_code(code: &str) -> bool {
    let mut parts = code.split('-');
    let Some(prefix) = parts.next() else {
        return false;
    };
    let Some(check_digit) = parts.next() else {
        return false;
    };
    if parts.next().is_some() {
        return false;
    }

    let prefix_len = prefix.len();
    if !(1..=7).contains(&prefix_len) {
        return false;
    }

    prefix.chars().all(|c| c.is_ascii_digit())
        && check_digit.len() == 1
        && check_digit.chars().all(|c| c.is_ascii_digit())
}

fn loinc_display_for_code(code: &str) -> Option<&'static str> {
    match code {
        "29463-7" => Some("Body weight"),
        "8302-2" => Some("Body height"),
        "8310-5" => Some("Body temperature"),
        "8867-4" => Some("Heart rate"),
        "8480-6" => Some("Systolic blood pressure"),
        "8462-4" => Some("Diastolic blood pressure"),
        "39156-5" => Some("Body mass index (BMI)"),
        "57021-8" => Some("CBC W Auto Differential panel"),
        "718-7" => Some("Hemoglobin [Mass/volume] in Blood"),
        "2339-0" => Some("Glucose [Mass/volume] in Blood"),
        "24356-8" => Some("Urinalysis complete panel"),
        "2106-3" => Some("Choriogonadotropin (pregnancy test) [Presence] in Urine"),
        _ => None,
    }
}

pub fn medication_binding(name: &str) -> MedicationBinding {
    let lower = name.trim().to_ascii_lowercase();
    let snomed = match lower.as_str() {
        "amoxicillin" => Some(coding(SNOMED_SYSTEM, "27658006", "Amoxicillin")),
        "paracetamol" => Some(coding(SNOMED_SYSTEM, "387517004", "Paracetamol")),
        _ => None,
    };
    MedicationBinding {
        local_name: name.to_string(),
        snomed,
    }
}

pub fn ucum_for_display(unit: &str) -> Option<Coding> {
    match unit.trim() {
        "kg" => Some(coding(UCUM_SYSTEM, "kg", "kg")),
        "cm" => Some(coding(UCUM_SYSTEM, "cm", "cm")),
        "mmHg" => Some(coding(UCUM_SYSTEM, "mm[Hg]", "mmHg")),
        "degC" | "C" | "Cel" => Some(coding(UCUM_SYSTEM, "Cel", "degC")),
        "beats/minute" | "bpm" | "/min" => Some(coding(UCUM_SYSTEM, "/min", "beats/minute")),
        "kg/m2" => Some(coding(UCUM_SYSTEM, "kg/m2", "kg/m2")),
        "g/dL" => Some(coding(UCUM_SYSTEM, "g/dL", "g/dL")),
        "mg/dL" => Some(coding(UCUM_SYSTEM, "mg/dL", "mg/dL")),
        _ => None,
    }
}

fn coding(system: &str, code: &str, display: &str) -> Coding {
    Coding {
        system: system.to_string(),
        code: code.to_string(),
        display: display.to_string(),
    }
}

fn concept(system: &str, code: &str, display: &str, synonyms: &[&str]) -> TerminologyConcept {
    TerminologyConcept {
        coding: coding(system, code, display),
        synonyms: synonyms.iter().map(|s| (*s).to_string()).collect(),
    }
}

fn icd11_concepts() -> Vec<TerminologyConcept> {
    vec![
        concept(ICD11_SYSTEM, "1F40", "Malaria", &["severe malaria"]),
        concept(
            ICD11_SYSTEM,
            "BA00",
            "Essential hypertension",
            &["hypertension", "high blood pressure"],
        ),
        concept(
            ICD11_SYSTEM,
            "5A11",
            "Type 2 diabetes mellitus",
            &["diabetes"],
        ),
    ]
}

fn snomed_concepts() -> Vec<TerminologyConcept> {
    vec![
        concept(SNOMED_SYSTEM, "61462000", "Malaria", &[]),
        concept(
            SNOMED_SYSTEM,
            "59621000",
            "Essential hypertension",
            &["hypertension"],
        ),
        concept(
            SNOMED_SYSTEM,
            "44054006",
            "Diabetes mellitus type 2",
            &["type 2 diabetes"],
        ),
        concept(SNOMED_SYSTEM, "233604007", "Pneumonia", &[]),
        concept(SNOMED_SYSTEM, "271737000", "Anemia", &[]),
        concept(
            SNOMED_SYSTEM,
            "68566005",
            "Urinary tract infectious disease",
            &["uti"],
        ),
        concept(
            SNOMED_SYSTEM,
            "62315008",
            "Diarrhea",
            &["diarrheal disease"],
        ),
    ]
}

fn loinc_concepts() -> Vec<TerminologyConcept> {
    vec![
        concept(LOINC_SYSTEM, "29463-7", "Body weight", &["weight"]),
        concept(LOINC_SYSTEM, "8302-2", "Body height", &["height"]),
        concept(LOINC_SYSTEM, "8310-5", "Body temperature", &["temperature"]),
        concept(LOINC_SYSTEM, "8867-4", "Heart rate", &["pulse"]),
        concept(
            LOINC_SYSTEM,
            "8480-6",
            "Systolic blood pressure",
            &["bp systolic"],
        ),
        concept(
            LOINC_SYSTEM,
            "8462-4",
            "Diastolic blood pressure",
            &["bp diastolic"],
        ),
        concept(LOINC_SYSTEM, "39156-5", "Body mass index (BMI)", &["bmi"]),
        concept(
            LOINC_SYSTEM,
            "57021-8",
            "CBC W Auto Differential panel",
            &["cbc", "complete blood count"],
        ),
        concept(
            LOINC_SYSTEM,
            "718-7",
            "Hemoglobin [Mass/volume] in Blood",
            &["hemoglobin", "hb"],
        ),
        concept(
            LOINC_SYSTEM,
            "2339-0",
            "Glucose [Mass/volume] in Blood",
            &["blood glucose"],
        ),
        concept(
            LOINC_SYSTEM,
            "24356-8",
            "Urinalysis complete panel",
            &["urinalysis"],
        ),
        concept(
            LOINC_SYSTEM,
            "2106-3",
            "Choriogonadotropin (pregnancy test) [Presence] in Urine",
            &["urine pregnancy"],
        ),
    ]
}

fn ucum_concepts() -> Vec<TerminologyConcept> {
    vec![
        concept(UCUM_SYSTEM, "kg", "kg", &["kilogram"]),
        concept(UCUM_SYSTEM, "cm", "cm", &["centimeter"]),
        concept(UCUM_SYSTEM, "Cel", "degC", &["celsius"]),
        concept(UCUM_SYSTEM, "mm[Hg]", "mmHg", &["millimeter of mercury"]),
        concept(UCUM_SYSTEM, "/min", "beats/minute", &["bpm"]),
        concept(UCUM_SYSTEM, "kg/m2", "kg/m2", &["bmi"]),
        concept(UCUM_SYSTEM, "g/dL", "g/dL", &[]),
        concept(UCUM_SYSTEM, "mg/dL", "mg/dL", &[]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::{Connection, params};

    fn memdb() -> Connection {
        Connection::open_in_memory().expect("db")
    }

    #[test]
    fn search_finds_weight_in_loinc() {
        let results = search(CodeSystem::Loinc, "weight");
        assert!(results.iter().any(|r| r.coding.code == "29463-7"));
    }

    #[test]
    fn diagnosis_binding_maps_hypertension() {
        let binding = diagnosis_binding("hypertension", None);
        assert_eq!(
            binding.icd11.as_ref().map(|c| c.code.as_str()),
            Some("BA00")
        );
        assert_eq!(
            binding.snomed.as_ref().map(|c| c.code.as_str()),
            Some("59621000")
        );
    }

    #[test]
    fn observation_binding_maps_bp_unit() {
        let binding = observation_binding("blood_pressure_systolic").expect("binding");
        assert_eq!(binding.loinc.code, "8480-6");
        assert_eq!(
            binding.ucum.as_ref().map(|c| c.code.as_str()),
            Some("mm[Hg]")
        );
    }

    #[test]
    fn lab_test_binding_accepts_raw_loinc_code() {
        let binding = lab_test_binding("57021-8");
        let loinc = binding.loinc.expect("loinc");
        assert_eq!(loinc.system, LOINC_SYSTEM);
        assert_eq!(loinc.code, "57021-8");
        assert_eq!(loinc.display, "CBC W Auto Differential panel");
    }

    #[test]
    fn lab_test_binding_raw_loinc_falls_back_to_code_display_when_unknown() {
        let binding = lab_test_binding("9999999-9");
        let loinc = binding.loinc.expect("loinc");
        assert_eq!(loinc.system, LOINC_SYSTEM);
        assert_eq!(loinc.code, "9999999-9");
        assert_eq!(loinc.display, "9999999-9");
    }

    #[test]
    fn has_imported_concepts_detects_existing_system_data() {
        let conn = memdb();
        import::ensure_schema(&conn).expect("schema");

        assert!(!has_imported_concepts(&conn, CodeSystem::Loinc).expect("has concepts"));

        conn.execute(
            "INSERT INTO terminology_concepts (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, 1, '{}', '2026-01-01T00:00:00Z')",
            params![LOINC_SYSTEM, "29463-7", "Body weight"],
        )
        .expect("insert concept");

        assert!(has_imported_concepts(&conn, CodeSystem::Loinc).expect("has concepts"));
        assert!(!has_imported_concepts(&conn, CodeSystem::SnomedCt).expect("has concepts"));
    }

    #[test]
    fn search_imported_matches_code_display_and_synonym_with_limit() {
        let conn = memdb();
        import::ensure_schema(&conn).expect("schema");

        conn.execute(
            "INSERT INTO terminology_concepts (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, 1, '{}', '2026-01-01T00:00:00Z')",
            params![LOINC_SYSTEM, "29463-7", "Body weight"],
        )
        .expect("insert concept");
        conn.execute(
            "INSERT INTO terminology_concepts (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, 1, '{}', '2026-01-01T00:00:00Z')",
            params![LOINC_SYSTEM, "8302-2", "Body height"],
        )
        .expect("insert concept");
        conn.execute(
            "INSERT INTO terminology_designations (system, code, language, use_type, value)
             VALUES (?1, ?2, 'en', 'synonym', ?3)",
            params![LOINC_SYSTEM, "29463-7", "patient mass"],
        )
        .expect("insert synonym");

        let by_code = search_imported(&conn, CodeSystem::Loinc, "29463", 10).expect("search code");
        assert_eq!(by_code.len(), 1);
        assert_eq!(by_code[0].coding.code, "29463-7");

        let by_display =
            search_imported(&conn, CodeSystem::Loinc, "body height", 10).expect("search display");
        assert_eq!(by_display.len(), 1);
        assert_eq!(by_display[0].coding.code, "8302-2");

        let by_synonym =
            search_imported(&conn, CodeSystem::Loinc, "mass", 10).expect("search synonym");
        assert_eq!(by_synonym.len(), 1);
        assert_eq!(by_synonym[0].coding.code, "29463-7");
        assert!(by_synonym[0].synonyms.iter().any(|s| s == "patient mass"));

        let limited = search_imported(&conn, CodeSystem::Loinc, "body", 1).expect("search limit");
        assert_eq!(limited.len(), 1);
    }
}
