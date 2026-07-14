//! Conflict detection and domain-specific auto-merge rules.
//!
//! Detection happens when incoming remote operations overlap with local
//! unacknowledged operations on the same aggregate. Domain-specific rules
//! determine whether the overlap can be automatically resolved.

use chrono::Utc;
use rustyclinic_core::types::new_id;
use rustyclinic_events::OpLogEntry;
use serde_json::Value as JsonValue;

use crate::types::{ConflictStatus, ConflictType, SyncConflict};

/// Critical patient demographic fields that require manual resolution when
/// both sides have changed them concurrently.
const CRITICAL_PATIENT_FIELDS: &[&str] = &["date_of_birth", "sex", "national_id"];

/// Non-critical patient fields that use last-write-wins.
const NON_CRITICAL_PATIENT_FIELDS: &[&str] = &["phone", "address", "given_name", "family_name"];

/// Detect a conflict between a local and remote operation on the same aggregate.
///
/// Returns `None` if no conflict exists (auto-merge succeeded or the operations
/// are compatible). Returns `Some(SyncConflict)` when manual resolution is needed.
pub fn detect_conflict(
    local_entry: &OpLogEntry,
    remote_entry: &OpLogEntry,
) -> Option<SyncConflict> {
    // Different aggregates never conflict
    if local_entry.aggregate_id != remote_entry.aggregate_id {
        return None;
    }

    // Same device authored both — no conflict (replication, not concurrent edit)
    if local_entry.device_id == remote_entry.device_id {
        return None;
    }

    let agg_type = local_entry.aggregate_type.as_str();

    match agg_type {
        "Patient" => detect_patient_conflict(local_entry, remote_entry),
        "Encounter" | "Observation" => detect_encounter_conflict(local_entry, remote_entry),
        "QueueEntry" => {
            // Queue entries use last-write-wins within a single facility.
            // Cross-facility conflicts shouldn't happen because queues are local.
            None
        }
        "Payment" | "Inventory" => {
            // Append-only / deduplicate by event ID — never conflict
            None
        }
        "ClaimCase" => detect_claim_conflict(local_entry, remote_entry),
        _ => {
            // Unknown aggregate types: default to conflict to be safe
            Some(build_conflict(
                local_entry,
                remote_entry,
                ConflictType::FieldConflict {
                    field_name: "unknown_aggregate".to_string(),
                },
            ))
        }
    }
}

/// Attempt an automatic merge of two payloads based on domain rules.
///
/// Returns the merged payload if auto-merge succeeds, or `None` if manual
/// resolution is required.
pub fn auto_merge(
    aggregate_type: &str,
    local_payload: &JsonValue,
    remote_payload: &JsonValue,
) -> Option<JsonValue> {
    match aggregate_type {
        "Patient" => auto_merge_patient(local_payload, remote_payload),
        "Encounter" | "Observation" => auto_merge_append_only(local_payload, remote_payload),
        "QueueEntry" => {
            // Last-write-wins: take the remote (it arrived later from upstream)
            Some(remote_payload.clone())
        }
        "Payment" | "Inventory" => {
            // Immutable append-only: take remote (deduplicated upstream)
            Some(remote_payload.clone())
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Patient demographics: field-aware merge
// ---------------------------------------------------------------------------

fn detect_patient_conflict(local: &OpLogEntry, remote: &OpLogEntry) -> Option<SyncConflict> {
    let local_obj = local.payload.as_object();
    let remote_obj = remote.payload.as_object();

    let (Some(l), Some(r)) = (local_obj, remote_obj) else {
        // If either payload is not an object, treat as conflict
        return Some(build_conflict(
            local,
            remote,
            ConflictType::FieldConflict {
                field_name: "payload_format".to_string(),
            },
        ));
    };

    // Check whether both modified the same critical field to different values
    for &field in CRITICAL_PATIENT_FIELDS {
        let l_val = l.get(field);
        let r_val = r.get(field);
        if l_val.is_some() && r_val.is_some() && l_val != r_val {
            return Some(build_conflict(
                local,
                remote,
                ConflictType::FieldConflict {
                    field_name: field.to_string(),
                },
            ));
        }
    }

    // No critical field conflicts — auto-merge handles the rest
    None
}

fn auto_merge_patient(local_payload: &JsonValue, remote_payload: &JsonValue) -> Option<JsonValue> {
    let (Some(local_obj), Some(remote_obj)) =
        (local_payload.as_object(), remote_payload.as_object())
    else {
        return None;
    };

    // Check critical fields for conflicts first
    for &field in CRITICAL_PATIENT_FIELDS {
        let l = local_obj.get(field);
        let r = remote_obj.get(field);
        if l.is_some() && r.is_some() && l != r {
            return None; // Cannot auto-merge
        }
    }

    // Field-aware merge: start from local, overlay non-critical from remote
    // (last-write-wins for non-critical), take whichever side has critical fields set.
    let mut merged = local_obj.clone();

    for &field in NON_CRITICAL_PATIENT_FIELDS {
        if let Some(val) = remote_obj.get(field) {
            merged.insert(field.to_string(), val.clone());
        }
    }

    // For critical fields, take whichever is set (they don't conflict per above check)
    for &field in CRITICAL_PATIENT_FIELDS {
        if let Some(val) = remote_obj.get(field) {
            merged.insert(field.to_string(), val.clone());
        }
    }

    Some(JsonValue::Object(merged))
}

// ---------------------------------------------------------------------------
// Encounters / observations: append-only auto-merge
// ---------------------------------------------------------------------------

fn detect_encounter_conflict(local: &OpLogEntry, remote: &OpLogEntry) -> Option<SyncConflict> {
    let local_obj = local.payload.as_object();
    let remote_obj = remote.payload.as_object();

    // Check for conflicting status transitions
    let local_status = local_obj.and_then(|o| o.get("status"));
    let remote_status = remote_obj.and_then(|o| o.get("status"));

    if let (Some(ls), Some(rs)) = (local_status, remote_status)
        && ls != rs
    {
        return Some(build_conflict(
            local,
            remote,
            ConflictType::StatusTransitionConflict,
        ));
    }

    // Concurrent appends (observations, notes) auto-merge
    None
}

fn auto_merge_append_only(
    local_payload: &JsonValue,
    remote_payload: &JsonValue,
) -> Option<JsonValue> {
    let (Some(local_obj), Some(remote_obj)) =
        (local_payload.as_object(), remote_payload.as_object())
    else {
        return None;
    };

    // If both change status to different values, cannot auto-merge
    let ls = local_obj.get("status");
    let rs = remote_obj.get("status");
    if ls.is_some() && rs.is_some() && ls != rs {
        return None;
    }

    // Merge: take local as base, overlay remote fields. For array fields
    // (observations, notes), concatenate.
    let mut merged = local_obj.clone();

    for (key, remote_val) in remote_obj {
        if let (Some(JsonValue::Array(local_arr)), JsonValue::Array(remote_arr)) =
            (merged.get(key).cloned(), remote_val)
        {
            // Concatenate arrays (append-only merge)
            let mut combined = local_arr;
            for item in remote_arr {
                if !combined.contains(item) {
                    combined.push(item.clone());
                }
            }
            merged.insert(key.clone(), JsonValue::Array(combined));
        } else {
            merged.insert(key.clone(), remote_val.clone());
        }
    }

    Some(JsonValue::Object(merged))
}

// ---------------------------------------------------------------------------
// Claims: ownership-based conflict detection
// ---------------------------------------------------------------------------

fn detect_claim_conflict(local: &OpLogEntry, remote: &OpLogEntry) -> Option<SyncConflict> {
    // If the local facility owns the claim and the remote is trying to modify it,
    // that's an ownership conflict.
    if local.facility_id != remote.facility_id {
        return Some(build_conflict(
            local,
            remote,
            ConflictType::OwnershipConflict,
        ));
    }
    None
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn build_conflict(
    local: &OpLogEntry,
    remote: &OpLogEntry,
    conflict_type: ConflictType,
) -> SyncConflict {
    SyncConflict {
        id: new_id(),
        facility_id: local.facility_id,
        aggregate_type: local.aggregate_type.clone(),
        aggregate_id: local.aggregate_id,
        local_entry_id: local.id,
        remote_entry_id: remote.id,
        conflict_type,
        status: ConflictStatus::Pending,
        created_at: Utc::now(),
        resolved_at: None,
        resolved_by: None,
        resolution: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    fn make_entry(
        aggregate_type: &str,
        aggregate_id: Uuid,
        device_id: Uuid,
        facility_id: Uuid,
        payload: JsonValue,
    ) -> OpLogEntry {
        OpLogEntry {
            id: Uuid::now_v7(),
            sequence: 1,
            facility_id,
            device_id,
            actor_id: Uuid::now_v7(),
            created_at: Utc::now(),
            aggregate_type: aggregate_type.to_string(),
            aggregate_id,
            payload,
            prev_hash: Vec::new(),
            entry_hash: Vec::new(),
        }
    }

    #[test]
    fn no_conflict_different_aggregates() {
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let fac = Uuid::now_v7();

        let local = make_entry(
            "Patient",
            Uuid::now_v7(),
            dev_a,
            fac,
            json!({"phone": "123"}),
        );
        let remote = make_entry(
            "Patient",
            Uuid::now_v7(),
            dev_b,
            fac,
            json!({"phone": "456"}),
        );

        assert!(detect_conflict(&local, &remote).is_none());
    }

    #[test]
    fn no_conflict_same_device() {
        let dev = Uuid::now_v7();
        let fac = Uuid::now_v7();
        let agg = Uuid::now_v7();

        let local = make_entry("Patient", agg, dev, fac, json!({"phone": "123"}));
        let remote = make_entry("Patient", agg, dev, fac, json!({"phone": "456"}));

        assert!(detect_conflict(&local, &remote).is_none());
    }

    #[test]
    fn patient_critical_field_conflict() {
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let fac = Uuid::now_v7();
        let agg = Uuid::now_v7();

        let local = make_entry(
            "Patient",
            agg,
            dev_a,
            fac,
            json!({"date_of_birth": "1990-01-01"}),
        );
        let remote = make_entry(
            "Patient",
            agg,
            dev_b,
            fac,
            json!({"date_of_birth": "1991-02-02"}),
        );

        let conflict = detect_conflict(&local, &remote);
        assert!(conflict.is_some());
        let c = conflict.expect("conflict expected");
        assert!(matches!(
            c.conflict_type,
            ConflictType::FieldConflict { .. }
        ));
    }

    #[test]
    fn patient_non_critical_no_conflict() {
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let fac = Uuid::now_v7();
        let agg = Uuid::now_v7();

        let local = make_entry("Patient", agg, dev_a, fac, json!({"phone": "111"}));
        let remote = make_entry("Patient", agg, dev_b, fac, json!({"phone": "222"}));

        assert!(detect_conflict(&local, &remote).is_none());
    }

    #[test]
    fn auto_merge_patient_non_critical_fields() {
        let local = json!({"given_name": "Alice", "phone": "111"});
        let remote = json!({"given_name": "Alice", "phone": "222", "address": "Kigali"});

        let merged = auto_merge("Patient", &local, &remote);
        assert!(merged.is_some());
        let m = merged.expect("merge expected");
        // Remote phone wins (last-write-wins for non-critical)
        assert_eq!(m.get("phone").and_then(|v| v.as_str()), Some("222"));
        assert_eq!(m.get("address").and_then(|v| v.as_str()), Some("Kigali"));
    }

    #[test]
    fn auto_merge_patient_critical_conflict_returns_none() {
        let local = json!({"sex": "Female"});
        let remote = json!({"sex": "Male"});

        assert!(auto_merge("Patient", &local, &remote).is_none());
    }

    #[test]
    fn encounter_status_transition_conflict() {
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let fac = Uuid::now_v7();
        let agg = Uuid::now_v7();

        let local = make_entry("Encounter", agg, dev_a, fac, json!({"status": "completed"}));
        let remote = make_entry("Encounter", agg, dev_b, fac, json!({"status": "cancelled"}));

        let conflict = detect_conflict(&local, &remote);
        assert!(conflict.is_some());
        let c = conflict.expect("conflict expected");
        assert!(matches!(
            c.conflict_type,
            ConflictType::StatusTransitionConflict
        ));
    }

    #[test]
    fn encounter_append_only_auto_merge() {
        let local = json!({"notes": ["note1"], "status": "in_progress"});
        let remote = json!({"notes": ["note2"], "status": "in_progress"});

        let merged = auto_merge("Encounter", &local, &remote);
        assert!(merged.is_some());
        let m = merged.expect("merge expected");
        let notes = m
            .get("notes")
            .and_then(|v| v.as_array())
            .expect("notes array");
        assert_eq!(notes.len(), 2);
    }

    #[test]
    fn claim_ownership_conflict_cross_facility() {
        let dev_a = Uuid::now_v7();
        let dev_b = Uuid::now_v7();
        let fac_a = Uuid::now_v7();
        let fac_b = Uuid::now_v7();
        let agg = Uuid::now_v7();

        let local = make_entry(
            "ClaimCase",
            agg,
            dev_a,
            fac_a,
            json!({"status": "submitted"}),
        );
        let remote = make_entry(
            "ClaimCase",
            agg,
            dev_b,
            fac_b,
            json!({"status": "approved"}),
        );

        let conflict = detect_conflict(&local, &remote);
        assert!(conflict.is_some());
        let c = conflict.expect("conflict expected");
        assert!(matches!(c.conflict_type, ConflictType::OwnershipConflict));
    }

    #[test]
    fn queue_entry_last_write_wins() {
        let local = json!({"position": 1});
        let remote = json!({"position": 3});

        let merged = auto_merge("QueueEntry", &local, &remote);
        assert!(merged.is_some());
        // Remote wins
        assert_eq!(
            merged
                .expect("merge")
                .get("position")
                .and_then(|v| v.as_u64()),
            Some(3)
        );
    }
}
