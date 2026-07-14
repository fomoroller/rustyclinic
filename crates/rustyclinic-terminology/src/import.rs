use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::io::{Cursor, Read};

use anyhow::{Context, Result, anyhow};
use chrono::Utc;
use csv::ReaderBuilder;
use flate2::read::GzDecoder;
use quick_xml::Reader;
use quick_xml::events::Event;
use rusqlite::Connection;
use serde_json::Value;
use tar::Archive;
use uuid::Uuid;
use zip::ZipArchive;

use crate::{LOINC_SYSTEM, SNOMED_SYSTEM, UCUM_SYSTEM};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportSystem {
    Icd11,
    Loinc,
    Ucum,
    Fhir,
    Snomed,
}

impl ImportSystem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Icd11 => "icd11",
            Self::Loinc => "loinc",
            Self::Ucum => "ucum",
            Self::Fhir => "fhir",
            Self::Snomed => "snomed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.to_ascii_lowercase().as_str() {
            "icd11" => Some(Self::Icd11),
            "loinc" => Some(Self::Loinc),
            "ucum" => Some(Self::Ucum),
            "fhir" => Some(Self::Fhir),
            "snomed" | "snomedct" | "snomed_ct" => Some(Self::Snomed),
            _ => None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ImportSummary {
    pub concept_count: usize,
    pub designation_count: usize,
    pub artifact_count: usize,
}

/// Metadata for the most recent terminology import run per system.
/// Returned by [latest_import_runs] so the CLI can display source,
/// timestamp, and concept/designation/artifact counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LatestImportRun {
    pub system: String,
    pub source: String,
    pub imported_at: String,
    pub concept_count: i64,
    pub designation_count: i64,
    pub artifact_count: i64,
}

#[derive(Debug, Clone)]
struct ConceptRow {
    system: String,
    code: String,
    display: String,
    active: bool,
    properties: Value,
}

#[derive(Debug, Clone)]
struct DesignationRow {
    system: String,
    code: String,
    language: String,
    use_type: String,
    value: String,
}

#[derive(Debug, Clone)]
struct ArtifactRow {
    system: String,
    resource_type: String,
    canonical_url: String,
    version: Option<String>,
    payload: Value,
}

pub fn ensure_schema(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS terminology_concepts (
            system TEXT NOT NULL,
            code TEXT NOT NULL,
            display TEXT NOT NULL,
            active INTEGER NOT NULL DEFAULT 1,
            properties TEXT NOT NULL DEFAULT '{}',
            imported_at TEXT NOT NULL,
            PRIMARY KEY (system, code)
        );
        CREATE INDEX IF NOT EXISTS idx_terminology_display ON terminology_concepts(system, display);

        CREATE TABLE IF NOT EXISTS terminology_designations (
            system TEXT NOT NULL,
            code TEXT NOT NULL,
            language TEXT NOT NULL DEFAULT 'en',
            use_type TEXT NOT NULL DEFAULT 'synonym',
            value TEXT NOT NULL,
            PRIMARY KEY (system, code, language, use_type, value)
        );
        CREATE INDEX IF NOT EXISTS idx_terminology_designation_value
            ON terminology_designations(system, value);

        CREATE TABLE IF NOT EXISTS terminology_artifacts (
            system TEXT NOT NULL,
            resource_type TEXT NOT NULL,
            canonical_url TEXT NOT NULL,
            version TEXT,
            payload TEXT NOT NULL,
            imported_at TEXT NOT NULL,
            PRIMARY KEY (system, resource_type, canonical_url, version)
        );

        CREATE TABLE IF NOT EXISTS terminology_import_runs (
            id TEXT PRIMARY KEY NOT NULL,
            system TEXT NOT NULL,
            source TEXT NOT NULL,
            imported_at TEXT NOT NULL,
            concept_count INTEGER NOT NULL DEFAULT 0,
            designation_count INTEGER NOT NULL DEFAULT 0,
            artifact_count INTEGER NOT NULL DEFAULT 0,
            notes TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_terminology_import_runs_system
            ON terminology_import_runs(system, imported_at);
        ",
    )?;
    Ok(())
}

pub fn import_from_source(
    conn: &Connection,
    system: ImportSystem,
    source: &str,
    replace_existing: bool,
) -> Result<ImportSummary> {
    ensure_schema(conn)?;

    let bytes = if source.starts_with("http://") || source.starts_with("https://") {
        tracing::info!(
            system = system.as_str(),
            source,
            "downloading terminology source"
        );
        let source = source.to_string();
        std::thread::spawn(move || -> Result<Vec<u8>> {
            reqwest::blocking::get(&source)
                .with_context(|| format!("failed to fetch source from {source}"))?
                .error_for_status()
                .with_context(|| format!("source returned non-success status: {source}"))?
                .bytes()
                .context("failed to read response body")
                .map(|b| b.to_vec())
        })
        .join()
        .map_err(|_| anyhow!("download worker thread panicked"))??
    } else {
        fs::read(source).with_context(|| format!("failed to read source file '{source}'"))?
    };

    let imported_at = Utc::now().to_rfc3339();
    let mut concepts = Vec::new();
    let mut designations = Vec::new();
    let mut artifacts = Vec::new();

    match system {
        ImportSystem::Loinc => parse_loinc(&bytes, &mut concepts, &mut designations)?,
        ImportSystem::Ucum => parse_ucum(&bytes, &mut concepts, &mut designations)?,
        ImportSystem::Fhir => parse_fhir(&bytes, &mut concepts, &mut artifacts)?,
        ImportSystem::Snomed => parse_snomed(&bytes, &mut concepts, &mut designations)?,
        ImportSystem::Icd11 => parse_icd11(&bytes, &mut concepts, &mut designations)?,
    }

    let tx = conn.unchecked_transaction()?;
    if replace_existing {
        tx.execute(
            "DELETE FROM terminology_designations WHERE system = ?1",
            rusqlite::params![system.as_str()],
        )?;
        tx.execute(
            "DELETE FROM terminology_concepts WHERE system = ?1",
            rusqlite::params![system.as_str()],
        )?;
        tx.execute(
            "DELETE FROM terminology_artifacts WHERE system = ?1",
            rusqlite::params![system.as_str()],
        )?;
    }

    for concept in &concepts {
        tx.execute(
            "INSERT OR REPLACE INTO terminology_concepts
             (system, code, display, active, properties, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                concept.system,
                concept.code,
                concept.display,
                if concept.active { 1 } else { 0 },
                concept.properties.to_string(),
                imported_at,
            ],
        )?;
    }

    for designation in &designations {
        tx.execute(
            "INSERT OR REPLACE INTO terminology_designations
             (system, code, language, use_type, value)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![
                designation.system,
                designation.code,
                designation.language,
                designation.use_type,
                designation.value,
            ],
        )?;
    }

    for artifact in &artifacts {
        tx.execute(
            "INSERT OR REPLACE INTO terminology_artifacts
             (system, resource_type, canonical_url, version, payload, imported_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                artifact.system,
                artifact.resource_type,
                artifact.canonical_url,
                artifact.version,
                artifact.payload.to_string(),
                imported_at,
            ],
        )?;
    }

    let run_id = Uuid::now_v7().to_string();
    tx.execute(
        "INSERT INTO terminology_import_runs
         (id, system, source, imported_at, concept_count, designation_count, artifact_count, notes)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        rusqlite::params![
            run_id,
            system.as_str(),
            source,
            imported_at,
            concepts.len() as i64,
            designations.len() as i64,
            artifacts.len() as i64,
            Value::Null.to_string(),
        ],
    )?;

    tx.commit()?;

    Ok(ImportSummary {
        concept_count: concepts.len(),
        designation_count: designations.len(),
        artifact_count: artifacts.len(),
    })
}

fn parse_loinc(
    bytes: &[u8],
    concepts: &mut Vec<ConceptRow>,
    designations: &mut Vec<DesignationRow>,
) -> Result<()> {
    let mut content = bytes.to_vec();
    if is_zip(bytes) {
        content = read_zip_member(bytes, |name| {
            name.ends_with("Loinc.csv") || name.ends_with("loinc.csv")
        })?;
    }

    let mut reader = csv::Reader::from_reader(content.as_slice());
    let headers = reader.headers()?.clone();
    let idx_code = header_index(&headers, &["LOINC_NUM", "LoincNumber"])?;
    let idx_display = header_index(
        &headers,
        &["LONG_COMMON_NAME", "LongCommonName", "COMPONENT"],
    )?;
    let idx_short = header_index_optional(&headers, &["SHORTNAME", "ShortName"]);
    let idx_status = header_index_optional(&headers, &["STATUS", "Status"]);

    for row in reader.records() {
        let row = row?;
        let code = row.get(idx_code).unwrap_or("").trim();
        let display = row.get(idx_display).unwrap_or("").trim();
        if code.is_empty() || display.is_empty() {
            continue;
        }

        let active = idx_status
            .and_then(|i| row.get(i))
            .map(|status| !status.eq_ignore_ascii_case("deprecated"))
            .unwrap_or(true);

        concepts.push(ConceptRow {
            system: LOINC_SYSTEM.to_string(),
            code: code.to_string(),
            display: display.to_string(),
            active,
            properties: serde_json::json!({
                "short_name": idx_short.and_then(|i| row.get(i)).unwrap_or("").trim(),
            }),
        });

        if let Some(idx_short) = idx_short {
            let short = row.get(idx_short).unwrap_or("").trim();
            if !short.is_empty() && short != display {
                designations.push(DesignationRow {
                    system: LOINC_SYSTEM.to_string(),
                    code: code.to_string(),
                    language: "en".to_string(),
                    use_type: "short".to_string(),
                    value: short.to_string(),
                });
            }
        }
    }

    Ok(())
}

fn parse_ucum(
    bytes: &[u8],
    concepts: &mut Vec<ConceptRow>,
    designations: &mut Vec<DesignationRow>,
) -> Result<()> {
    let mut xml = bytes.to_vec();
    if is_zip(bytes) {
        xml = read_zip_member(bytes, |name| name.ends_with(".xml"))?;
    }

    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();

    loop {
        match reader.read_event_into(&mut buf) {
            Ok(Event::Start(event)) | Ok(Event::Empty(event)) => {
                let tag = String::from_utf8_lossy(event.name().as_ref()).to_string();
                if tag.ends_with("unit") || tag.ends_with("base-unit") || tag.ends_with("prefix") {
                    let mut code = None;
                    let mut display = None;
                    let mut print_symbol = None;

                    for attr in event.attributes().flatten() {
                        let key = String::from_utf8_lossy(attr.key.as_ref()).to_string();
                        let value = attr
                            .decode_and_unescape_value(reader.decoder())
                            .unwrap_or_default()
                            .to_string();
                        match key.as_str() {
                            "Code" | "CODE" | "code" => code = Some(value),
                            "name" | "Name" => display = Some(value),
                            "printSymbol" | "printsymbol" => print_symbol = Some(value),
                            _ => {}
                        }
                    }

                    if let Some(code) = code.filter(|v| !v.is_empty()) {
                        let label = display
                            .clone()
                            .or(print_symbol.clone())
                            .unwrap_or_else(|| code.clone());

                        concepts.push(ConceptRow {
                            system: UCUM_SYSTEM.to_string(),
                            code: code.clone(),
                            display: label,
                            active: true,
                            properties: serde_json::json!({
                                "print_symbol": print_symbol,
                                "tag": tag,
                            }),
                        });

                        if let Some(display) = display
                            && display != code
                        {
                            designations.push(DesignationRow {
                                system: UCUM_SYSTEM.to_string(),
                                code,
                                language: "en".to_string(),
                                use_type: "name".to_string(),
                                value: display,
                            });
                        }
                    }
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(anyhow!("failed to parse UCUM XML: {e}")),
            _ => {}
        }
        buf.clear();
    }

    dedupe_concepts(concepts);
    Ok(())
}

fn parse_fhir(
    bytes: &[u8],
    concepts: &mut Vec<ConceptRow>,
    artifacts: &mut Vec<ArtifactRow>,
) -> Result<()> {
    if is_gzip(bytes) {
        let decoder = GzDecoder::new(Cursor::new(bytes));
        let mut archive = Archive::new(decoder);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let path = entry.path()?.to_string_lossy().to_string();
            if !path.ends_with(".json") || path.ends_with("package.json") {
                continue;
            }
            let mut entry_bytes = Vec::new();
            entry.read_to_end(&mut entry_bytes)?;
            if let Ok(resource) = serde_json::from_slice::<Value>(&entry_bytes)
                && resource.get("resourceType").is_some()
            {
                parse_fhir_resource(&resource, concepts, artifacts)?;
            }
        }
        return Ok(());
    }

    let value: Value = if is_zip(bytes) {
        let json_bytes = read_zip_member(bytes, |name| name.ends_with(".json"))?;
        serde_json::from_slice(&json_bytes)?
    } else {
        serde_json::from_slice(bytes)?
    };

    match value.get("resourceType").and_then(Value::as_str) {
        Some("Bundle") => {
            if let Some(entries) = value.get("entry").and_then(Value::as_array) {
                for entry in entries {
                    if let Some(resource) = entry.get("resource") {
                        parse_fhir_resource(resource, concepts, artifacts)?;
                    }
                }
            }
        }
        Some(_) => parse_fhir_resource(&value, concepts, artifacts)?,
        None => return Err(anyhow!("FHIR import expects a resource or bundle JSON")),
    }

    Ok(())
}

fn parse_fhir_resource(
    resource: &Value,
    concepts: &mut Vec<ConceptRow>,
    artifacts: &mut Vec<ArtifactRow>,
) -> Result<()> {
    let resource_type = resource
        .get("resourceType")
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("FHIR resource missing resourceType"))?;
    let canonical = resource
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or(resource_type);
    let version = resource
        .get("version")
        .and_then(Value::as_str)
        .map(str::to_string);

    artifacts.push(ArtifactRow {
        system: "fhir".to_string(),
        resource_type: resource_type.to_string(),
        canonical_url: canonical.to_string(),
        version,
        payload: resource.clone(),
    });

    if resource_type == "CodeSystem" {
        let system_url = resource
            .get("url")
            .and_then(Value::as_str)
            .unwrap_or("urn:rustyclinic:fhir:codesystem");
        if let Some(concept_array) = resource.get("concept").and_then(Value::as_array) {
            extract_codesystem_concepts(system_url, concept_array, concepts)?;
        }
    }

    Ok(())
}

fn extract_codesystem_concepts(
    system_url: &str,
    concepts_value: &[Value],
    concepts: &mut Vec<ConceptRow>,
) -> Result<()> {
    for concept in concepts_value {
        let code = concept
            .get("code")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim();
        if code.is_empty() {
            continue;
        }
        let display = concept
            .get("display")
            .and_then(Value::as_str)
            .or_else(|| concept.get("definition").and_then(Value::as_str))
            .unwrap_or(code);
        concepts.push(ConceptRow {
            system: system_url.to_string(),
            code: code.to_string(),
            display: display.to_string(),
            active: true,
            properties: concept.clone(),
        });

        if let Some(children) = concept.get("concept").and_then(Value::as_array) {
            extract_codesystem_concepts(system_url, children, concepts)?;
        }
    }
    Ok(())
}

fn parse_snomed(
    bytes: &[u8],
    concepts: &mut Vec<ConceptRow>,
    designations: &mut Vec<DesignationRow>,
) -> Result<()> {
    if !is_zip(bytes) {
        return Err(anyhow!("SNOMED CT import expects an RF2 zip file"));
    }

    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let concept_name = find_zip_name(&mut archive, |name| {
        name.contains("sct2_Concept_") && name.ends_with(".txt")
    })?
    .ok_or_else(|| anyhow!("could not find SNOMED concept snapshot file"))?;

    let desc_name = find_zip_name(&mut archive, |name| {
        name.contains("sct2_Description_") && name.ends_with(".txt")
    })?
    .ok_or_else(|| anyhow!("could not find SNOMED description snapshot file"))?;

    let concept_bytes = read_named_zip_member(&mut archive, &concept_name)?;
    let desc_bytes = read_named_zip_member(&mut archive, &desc_name)?;

    let mut active_concepts = BTreeSet::new();
    let mut concept_reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .from_reader(concept_bytes.as_slice());
    let headers = concept_reader.headers()?.clone();
    let id_idx = header_index(&headers, &["id"])?;
    let active_idx = header_index(&headers, &["active"])?;

    for row in concept_reader.records() {
        let row = row?;
        if row.get(active_idx).unwrap_or("") == "1" {
            active_concepts.insert(row.get(id_idx).unwrap_or("").to_string());
        }
    }

    let mut best_display: BTreeMap<String, String> = BTreeMap::new();
    let mut desc_reader = ReaderBuilder::new()
        .delimiter(b'\t')
        .from_reader(desc_bytes.as_slice());
    let headers = desc_reader.headers()?.clone();
    let concept_idx = header_index(&headers, &["conceptId"])?;
    let term_idx = header_index(&headers, &["term"])?;
    let active_idx = header_index(&headers, &["active"])?;
    let type_idx = header_index_optional(&headers, &["typeId"]);

    for row in desc_reader.records() {
        let row = row?;
        if row.get(active_idx).unwrap_or("") != "1" {
            continue;
        }
        let concept_id = row.get(concept_idx).unwrap_or("");
        if !active_concepts.contains(concept_id) {
            continue;
        }
        let term = row.get(term_idx).unwrap_or("").trim();
        if term.is_empty() {
            continue;
        }

        let is_fsn = type_idx
            .and_then(|idx| row.get(idx))
            .map(|v| v == "900000000000003001")
            .unwrap_or(false);

        if is_fsn || !best_display.contains_key(concept_id) {
            best_display.insert(concept_id.to_string(), term.to_string());
        }

        designations.push(DesignationRow {
            system: SNOMED_SYSTEM.to_string(),
            code: concept_id.to_string(),
            language: "en".to_string(),
            use_type: if is_fsn { "fsn" } else { "synonym" }.to_string(),
            value: term.to_string(),
        });
    }

    for concept_id in active_concepts {
        let display = best_display
            .get(&concept_id)
            .cloned()
            .unwrap_or_else(|| concept_id.clone());
        concepts.push(ConceptRow {
            system: SNOMED_SYSTEM.to_string(),
            code: concept_id,
            display,
            active: true,
            properties: Value::Object(Default::default()),
        });
    }

    Ok(())
}

fn parse_icd11(
    bytes: &[u8],
    concepts: &mut Vec<ConceptRow>,
    designations: &mut Vec<DesignationRow>,
) -> Result<()> {
    if is_zip(bytes) {
        let json_bytes = read_zip_member(bytes, |name| {
            name.ends_with(".json") || name.ends_with(".csv")
        })?;
        return parse_icd11(&json_bytes, concepts, designations);
    }

    if bytes.first() == Some(&b'[') || bytes.first() == Some(&b'{') {
        let value: Value = serde_json::from_slice(bytes)?;
        parse_icd11_json_value(&value, concepts, designations)?;
        return Ok(());
    }

    let mut reader = csv::Reader::from_reader(bytes);
    let headers = reader.headers()?.clone();
    let code_idx = header_index(&headers, &["code", "Code"])?;
    let title_idx = header_index(&headers, &["title", "Title", "name"])?;
    for row in reader.records() {
        let row = row?;
        let code = row.get(code_idx).unwrap_or("").trim();
        let title = row.get(title_idx).unwrap_or("").trim();
        if code.is_empty() || title.is_empty() {
            continue;
        }
        concepts.push(ConceptRow {
            system: crate::ICD11_SYSTEM.to_string(),
            code: code.to_string(),
            display: title.to_string(),
            active: true,
            properties: Value::Object(Default::default()),
        });
    }
    Ok(())
}

fn parse_icd11_json_value(
    value: &Value,
    concepts: &mut Vec<ConceptRow>,
    designations: &mut Vec<DesignationRow>,
) -> Result<()> {
    match value {
        Value::Array(values) => {
            for entry in values {
                parse_icd11_json_value(entry, concepts, designations)?;
            }
        }
        Value::Object(map) => {
            let code = map
                .get("code")
                .or_else(|| map.get("theCode"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim();
            let title = map
                .get("title")
                .and_then(Value::as_object)
                .and_then(|t| t.get("@value"))
                .and_then(Value::as_str)
                .or_else(|| map.get("title").and_then(Value::as_str))
                .or_else(|| map.get("label").and_then(Value::as_str))
                .unwrap_or("")
                .trim();

            if !code.is_empty() && !title.is_empty() {
                concepts.push(ConceptRow {
                    system: crate::ICD11_SYSTEM.to_string(),
                    code: code.to_string(),
                    display: title.to_string(),
                    active: true,
                    properties: Value::Object(Default::default()),
                });
            }

            if let Some(include) = map.get("inclusion").and_then(Value::as_array) {
                for alias in include {
                    if let Some(text) = alias.as_str() {
                        designations.push(DesignationRow {
                            system: crate::ICD11_SYSTEM.to_string(),
                            code: code.to_string(),
                            language: "en".to_string(),
                            use_type: "inclusion".to_string(),
                            value: text.to_string(),
                        });
                    }
                }
            }

            if let Some(children) = map.get("children").and_then(Value::as_array) {
                for child in children {
                    parse_icd11_json_value(child, concepts, designations)?;
                }
            }
            if let Some(children) = map.get("child").and_then(Value::as_array) {
                for child in children {
                    parse_icd11_json_value(child, concepts, designations)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn is_zip(bytes: &[u8]) -> bool {
    bytes.len() >= 4 && bytes[0..4] == [0x50, 0x4b, 0x03, 0x04]
}

fn is_gzip(bytes: &[u8]) -> bool {
    bytes.len() >= 2 && bytes[0] == 0x1f && bytes[1] == 0x8b
}

fn read_zip_member(bytes: &[u8], predicate: impl Fn(&str) -> bool) -> Result<Vec<u8>> {
    let mut archive = ZipArchive::new(Cursor::new(bytes))?;
    let name = find_zip_name(&mut archive, predicate)?
        .ok_or_else(|| anyhow!("could not find matching file in zip archive"))?;
    read_named_zip_member(&mut archive, &name)
}

fn find_zip_name(
    archive: &mut ZipArchive<Cursor<&[u8]>>,
    predicate: impl Fn(&str) -> bool,
) -> Result<Option<String>> {
    for i in 0..archive.len() {
        let file = archive.by_index(i)?;
        let name = file.name().to_string();
        if predicate(&name) {
            return Ok(Some(name));
        }
    }
    Ok(None)
}

fn read_named_zip_member(archive: &mut ZipArchive<Cursor<&[u8]>>, name: &str) -> Result<Vec<u8>> {
    let mut file = archive.by_name(name)?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

fn header_index(headers: &csv::StringRecord, candidates: &[&str]) -> Result<usize> {
    header_index_optional(headers, candidates)
        .ok_or_else(|| anyhow!("required column missing: one of {:?}", candidates))
}

fn header_index_optional(headers: &csv::StringRecord, candidates: &[&str]) -> Option<usize> {
    headers
        .iter()
        .position(|header| candidates.contains(&header))
}

fn dedupe_concepts(concepts: &mut Vec<ConceptRow>) {
    let mut seen = BTreeSet::new();
    concepts.retain(|concept| seen.insert((concept.system.clone(), concept.code.clone())));
}

pub fn concept_counts(conn: &Connection) -> Result<HashMap<String, i64>> {
    ensure_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT system, COUNT(*) FROM terminology_concepts GROUP BY system ORDER BY system",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
    })?;

    let mut counts = HashMap::new();
    for row in rows {
        let (system, count) = row?;
        counts.insert(system, count);
    }
    Ok(counts)
}

/// Returns the most recent `terminology_import_runs` row for each system,
/// ordered by system name for stable iteration.  Returns an empty vector if
/// no import runs have been recorded.
pub fn latest_import_runs(conn: &Connection) -> Result<Vec<LatestImportRun>> {
    ensure_schema(conn)?;
    let mut stmt = conn.prepare(
        "SELECT system, source, imported_at, concept_count, designation_count, artifact_count
         FROM (
             SELECT *, ROW_NUMBER() OVER (
                 PARTITION BY system ORDER BY imported_at DESC
             ) AS rn
             FROM terminology_import_runs
         )
         WHERE rn = 1
         ORDER BY system",
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(LatestImportRun {
            system: row.get(0)?,
            source: row.get(1)?,
            imported_at: row.get(2)?,
            concept_count: row.get(3)?,
            designation_count: row.get(4)?,
            artifact_count: row.get(5)?,
        })
    })?;

    let mut runs = Vec::new();
    for row in rows {
        runs.push(row?);
    }
    Ok(runs)
}

pub fn preset_source(system: ImportSystem) -> Result<&'static str> {
    match system {
        ImportSystem::Ucum => {
            Ok("https://raw.githubusercontent.com/ucum-org/ucum/main/ucum-essence.xml")
        }
        ImportSystem::Fhir => Ok("https://packages2.fhir.org/packages/hl7.fhir.r4.core/4.0.1"),
        ImportSystem::Icd11 => Err(anyhow!(
            "ICD-11 does not have a simple anonymous flat-file download in this workflow. Use the WHO ICD API/local deployment export and then run `admin import-terminology icd11 <file-or-url>`."
        )),
        ImportSystem::Loinc => Err(anyhow!(
            "LOINC download requires a free LOINC account. Download the official archive from https://loinc.org/downloads/ and then run `admin import-terminology loinc <zip-file>`."
        )),
        ImportSystem::Snomed => Err(anyhow!(
            "SNOMED CT requires licensed access to an official RF2 distribution. Download the release for your licensed territory and then run `admin import-terminology snomed <zip-file>`."
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memdb() -> Connection {
        Connection::open_in_memory().expect("db")
    }

    #[test]
    fn import_loinc_csv_works() {
        let conn = memdb();
        let csv =
            b"LOINC_NUM,LONG_COMMON_NAME,SHORTNAME,STATUS\n29463-7,Body weight,WEIGHT,ACTIVE\n";
        let path = std::env::temp_dir().join(format!("loinc-{}.csv", Uuid::now_v7()));
        fs::write(&path, csv).expect("write");
        let summary = import_from_source(
            &conn,
            ImportSystem::Loinc,
            path.to_str().expect("path"),
            true,
        )
        .expect("import");
        assert_eq!(summary.concept_count, 1);
        let counts = concept_counts(&conn).expect("counts");
        assert_eq!(counts.get(LOINC_SYSTEM), Some(&1));
        let _ = fs::remove_file(path);
    }

    #[test]
    fn import_fhir_codesystem_extracts_concepts() {
        let conn = memdb();
        let json = serde_json::json!({
            "resourceType": "CodeSystem",
            "url": "http://example.org/codes",
            "version": "1",
            "concept": [
                { "code": "A", "display": "Alpha" }
            ]
        });
        let path = std::env::temp_dir().join(format!("fhir-{}.json", Uuid::now_v7()));
        fs::write(&path, json.to_string()).expect("write");
        let summary = import_from_source(
            &conn,
            ImportSystem::Fhir,
            path.to_str().expect("path"),
            true,
        )
        .expect("import");
        assert_eq!(summary.artifact_count, 1);
        assert_eq!(summary.concept_count, 1);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn import_icd11_json_tree_works() {
        let conn = memdb();
        let json = serde_json::json!({
            "code": "BA00",
            "title": "Essential hypertension",
            "children": [
                { "code": "BA00.0", "title": "Child node" }
            ]
        });
        let path = std::env::temp_dir().join(format!("icd11-{}.json", Uuid::now_v7()));
        fs::write(&path, json.to_string()).expect("write");
        let summary = import_from_source(
            &conn,
            ImportSystem::Icd11,
            path.to_str().expect("path"),
            true,
        )
        .expect("import");
        assert_eq!(summary.concept_count, 2);
        let _ = fs::remove_file(path);
    }

    #[test]
    fn latest_import_runs_returns_one_per_system() {
        let conn = memdb();
        let csv1 =
            b"LOINC_NUM,LONG_COMMON_NAME,SHORTNAME,STATUS\n29463-7,Body weight,WEIGHT,ACTIVE\n";
        let path1 = std::env::temp_dir().join(format!("loinc1-{}.csv", Uuid::now_v7()));
        fs::write(&path1, csv1).expect("write");
        import_from_source(
            &conn,
            ImportSystem::Loinc,
            path1.to_str().expect("temporary path must be valid UTF-8"),
            true,
        )
        .expect("import1");

        let csv2 =
            b"LOINC_NUM,LONG_COMMON_NAME,SHORTNAME,STATUS\n8302-2,Body height,HEIGHT,ACTIVE\n";
        let path2 = std::env::temp_dir().join(format!("loinc2-{}.csv", Uuid::now_v7()));
        fs::write(&path2, csv2).expect("write");
        import_from_source(
            &conn,
            ImportSystem::Loinc,
            path2.to_str().expect("temporary path must be valid UTF-8"),
            true,
        )
        .expect("import2");

        let xml = br#"<?xml version="1.0"?>
            <root xmlns:ucum="urn:oid:2.16.840.1.113883.4.642.3.1">
                <unit code="kg" name="kilogram" printSymbol="kg"/>
            </root>"#;
        let path3 = std::env::temp_dir().join(format!("ucum-{}.xml", Uuid::now_v7()));
        fs::write(&path3, xml).expect("write");
        import_from_source(
            &conn,
            ImportSystem::Ucum,
            path3.to_str().expect("temporary path must be valid UTF-8"),
            true,
        )
        .expect("import3");

        let runs = latest_import_runs(&conn).expect("latest");

        let loinc_runs: Vec<_> = runs.iter().filter(|r| r.system == "loinc").collect();
        assert_eq!(loinc_runs.len(), 1, "exactly one loinc run");
        assert_eq!(loinc_runs[0].concept_count, 1);

        let ucum_runs: Vec<_> = runs.iter().filter(|r| r.system == "ucum").collect();
        assert_eq!(ucum_runs.len(), 1, "exactly one ucum run");

        let systems: Vec<_> = runs.iter().map(|r| r.system.clone()).collect();
        assert_eq!(systems, vec!["loinc", "ucum"], "ordered by system");

        let _ = fs::remove_file(path1);
        let _ = fs::remove_file(path2);
        let _ = fs::remove_file(path3);
    }

    #[test]
    fn latest_import_runs_returns_empty_when_no_runs() {
        let conn = memdb();
        let runs = latest_import_runs(&conn).expect("latest");
        assert!(runs.is_empty());
    }

    #[test]
    fn latest_import_runs_preserves_source_and_counts() {
        let conn = memdb();
        let csv =
            b"LOINC_NUM,LONG_COMMON_NAME,SHORTNAME,STATUS\n29463-7,Body weight,WEIGHT,ACTIVE\n";
        let path = std::env::temp_dir().join(format!("source-test-{}.csv", Uuid::now_v7()));
        let source_path = path.to_str().expect("temporary path must be valid UTF-8");
        fs::write(&path, csv).expect("write");
        import_from_source(&conn, ImportSystem::Loinc, source_path, true).expect("import");

        let runs = latest_import_runs(&conn).expect("latest");
        assert_eq!(runs.len(), 1);
        let run = &runs[0];
        assert_eq!(run.system, "loinc");
        assert!(run.source.ends_with(".csv"), "source path recorded");
        assert!(!run.imported_at.is_empty(), "imported_at populated");
        assert_eq!(run.concept_count, 1);
        assert_eq!(run.designation_count, 1);
        assert_eq!(run.artifact_count, 0);

        let _ = fs::remove_file(path);
    }
}
