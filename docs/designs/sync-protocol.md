# Sync Protocol Design

## Context

Sync is Phase 3 of the RustyClinic build. The architecture defines the sync model at a conceptual level — operation log, replica cursors, conflict queues, three transport adapters. This design doc specifies the implementation-level details needed to build `rustyclinic-sync`.

## Operation-Log Entry Format

Each entry in the operation log is a self-describing record:

```rust
pub struct OpLogEntry {
    /// Globally unique, time-sortable identifier (UUIDv7)
    pub id: Uuid,
    /// Monotonically increasing per-facility sequence number
    pub sequence: u64,
    /// Facility that originated this operation
    pub facility_id: Uuid,
    /// Device that created this entry
    pub device_id: Uuid,
    /// User who initiated the operation
    pub actor_id: Uuid,
    /// UTC timestamp of creation
    pub created_at: DateTime<Utc>,
    /// Aggregate type (e.g., "Patient", "Encounter", "ClaimCase")
    pub aggregate_type: String,
    /// Aggregate instance ID
    pub aggregate_id: Uuid,
    /// Operation payload — the actual mutation
    pub payload: OpPayload,
    /// Package version active when this operation was created
    pub package_context: PackageContext,
    /// Hash of the previous entry in this facility's log (chain integrity)
    pub prev_hash: [u8; 32],
    /// SHA-256 hash of this entry (computed over all fields except this one)
    pub entry_hash: [u8; 32],
    /// Sync metadata
    pub sync_state: SyncState,
}

pub enum SyncState {
    /// Created locally, not yet pushed
    Pending,
    /// Pushed to upstream, awaiting ack
    Pushed,
    /// Acknowledged by upstream
    Acknowledged { acked_at: DateTime<Utc> },
}

pub struct PackageContext {
    /// Active deployment pack version
    pub deployment_version: String,
    /// Active program pack version (if applicable)
    pub program_version: Option<String>,
    /// Active form version (if form submission)
    pub form_version: Option<String>,
}
```

### Serialization

```text
WIRE FORMAT:

  OpLogEntry ──serialize──▶ CBOR bytes ──batch──▶ [entry1, entry2, ...] ──compress──▶ zstd frame

  Why CBOR: ~30% smaller than JSON, schema-flexible, well-supported in Rust (ciborium crate).
  Why zstd:  ~70% compression ratio on clinical data, streaming decompression, low memory.
```

On the wire, each sync message is:

```text
┌──────────────────────────────────────────┐
│ SYNC MESSAGE                             │
├──────────────────────────────────────────┤
│ version: u8          (protocol version)  │
│ message_type: u8     (push/pull/ack/...) │
│ facility_id: [u8;16] (sender facility)   │
│ device_id: [u8;16]   (sender device)     │
│ batch_count: u32     (entries in batch)   │
│ payload: zstd(cbor([OpLogEntry]))        │
│ checksum: [u8;32]    (SHA-256 of above)  │
└──────────────────────────────────────────┘
```

Protocol version starts at `1`. Incompatible changes increment the version. The receiver rejects messages with unknown versions and returns an error indicating the supported range.

## Sync Protocol — HTTPS Transport

### Endpoints

All sync endpoints are served by the `rustyclinic serve sync` role (or `serve all`).

```text
POST /sync/push          Push local operations to upstream
GET  /sync/pull          Pull remote operations since cursor
POST /sync/ack           Acknowledge received operations
GET  /sync/cursor        Get current cursor positions
GET  /sync/snapshot      Request a bootstrap snapshot
GET  /sync/packages      Get package update manifest
GET  /sync/health        Sync health status (for fleet dashboard)
```

### Authentication

Every sync request includes:
- Device certificate (mutual TLS or certificate in header)
- Facility-scoped auth token
- Device ID

The upstream validates:
1. Device is registered and not revoked
2. Device belongs to the claimed facility
3. Auth token is valid and not expired

### Push Flow

```text
CLIENT                                          SERVER
  │                                               │
  ├── POST /sync/push                             │
  │   Body: zstd(cbor([OpLogEntry]))              │
  │   Headers: X-Device-Id, X-Facility-Id         │
  │   Query: since_sequence=N                     │
  │                                               │
  │                          ┌────────────────────┤
  │                          │ For each entry:     │
  │                          │ 1. Validate hash    │
  │                          │    chain             │
  │                          │ 2. Check sequence    │
  │                          │    (reject if gap)   │
  │                          │ 3. Deduplicate by    │
  │                          │    entry ID          │
  │                          │ 4. Apply domain      │
  │                          │    conflict rules    │
  │                          │ 5. Persist to        │
  │                          │    server op-log     │
  │                          └────────────────────┤
  │                                               │
  │ ◀── 200 OK                                    │
  │     Body: { accepted: N, rejected: [...],     │
  │             conflicts: [...],                  │
  │             server_sequence: M }               │
  │                                               │
  ├── POST /sync/ack                              │
  │   Body: { acked_through: M }                  │
  │                                               │
  │ ◀── 200 OK                                    │
```

### Pull Flow

```text
CLIENT                                          SERVER
  │                                               │
  ├── GET /sync/pull?since=M&limit=1000           │
  │                                               │
  │ ◀── 200 OK                                    │
  │     Body: zstd(cbor([OpLogEntry]))            │
  │     Headers: X-Total-Remaining: N             │
  │              X-Server-Sequence: M+K           │
  │                                               │
  │   Client applies entries locally:             │
  │   1. Validate hash chain                      │
  │   2. Apply domain conflict rules              │
  │   3. Route conflicts to conflict queue        │
  │   4. Update local cursor                      │
  │   5. Trigger affected projection rebuilds     │
  │                                               │
  ├── POST /sync/ack                              │
  │   Body: { pulled_through: M+K }              │
  │                                               │
  │ ◀── 200 OK                                    │
```

### Pagination

Pull requests return at most `limit` entries (default 1000, max 5000). The `X-Total-Remaining` header tells the client how many more entries exist, enabling progress reporting in the UI.

### Resumability

Every push and pull is resumable:
- Push: the client tracks which local sequence was last accepted. On failure, it resends from that point. The server deduplicates by entry ID.
- Pull: the client tracks its cursor. On failure, it re-requests from the same cursor. Entries are idempotent to apply.

Network flap handling: if a connection drops mid-transfer, the client retries with exponential backoff (1s, 2s, 4s, 8s, max 60s). No data is lost because neither side commits until the full batch is validated.

## Conflict Resolution

### Conflict Detection

A conflict occurs when two facilities create operations that cannot be automatically merged for the same aggregate. Detection happens during pull (client-side) or push (server-side).

```text
CONFLICT DETECTION RULES:

  For each incoming operation on aggregate X:
    1. Check if local op-log has an unacknowledged operation on X
       from a DIFFERENT facility
    2. Apply domain-specific merge rules (see below)
    3. If auto-merge succeeds → apply both, no conflict
    4. If auto-merge fails → route to conflict queue
```

### Domain-Specific Merge Rules

```text
AGGREGATE TYPE          │ AUTO-MERGE RULE                     │ CONFLICT TRIGGER
─────────────────────── │ ──────────────────────────────────── │ ────────────────────
Patient demographics    │ Field-aware merge:                  │ Both changed same
                        │ - Non-critical fields (phone,       │ critical field (DOB,
                        │   address): last-write-wins         │ sex, primary ID)
                        │ - Critical fields: conflict         │
─────────────────────── │ ──────────────────────────────────── │ ────────────────────
Encounters/observations │ Append-only: concurrent appends     │ Conflicting status
                        │ merge automatically                 │ transitions on same
                        │                                     │ encounter
─────────────────────── │ ──────────────────────────────────── │ ────────────────────
Claims                  │ Single-writer per owning facility   │ Remote transition on
                        │                                     │ locally-owned claim
─────────────────────── │ ──────────────────────────────────── │ ────────────────────
Payments/inventory      │ Deduplicate by event ID             │ Never (immutable
                        │                                     │ append-only)
─────────────────────── │ ──────────────────────────────────── │ ────────────────────
Bed occupancy           │ First committed lease wins          │ Competing lease on
                        │                                     │ same bed
─────────────────────── │ ──────────────────────────────────── │ ────────────────────
Packages/terminology    │ Replace by version + checksum       │ Never (versioned)
```

### Conflict Queue

```rust
pub struct SyncConflict {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub aggregate_type: String,
    pub aggregate_id: Uuid,
    pub local_operation: OpLogEntry,
    pub remote_operation: OpLogEntry,
    pub conflict_type: ConflictType,
    pub status: ConflictStatus,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Uuid>,
    pub resolution: Option<ConflictResolution>,
}

pub enum ConflictType {
    FieldConflict { field_name: String },
    StatusTransitionConflict,
    LeaseConflict,
    OwnershipConflict,
}

pub enum ConflictStatus {
    Pending,
    Resolved,
    Escalated,
}

pub enum ConflictResolution {
    AcceptLocal,
    AcceptRemote,
    ManualMerge { merged_payload: OpPayload },
}
```

### Conflict UX

The conflict resolution UI shows:
1. What the local change was (who, when, what changed)
2. What the remote change was (who, when, what changed)
3. The current state of the aggregate
4. Options: Accept local, Accept remote, or Manual merge (side-by-side field editor)

For clinical data conflicts, the UI highlights which fields differ and shows both values. The resolver can pick field-by-field for manual merge.

## Cursor Gap Detection (Critical Gap from Reviews)

The CEO and eng reviews identified cursor gaps as a critical gap — if the op-log has a gap (entries skipped), the client could silently miss data.

### Detection

On every pull response, the client validates:
1. The first entry's sequence number equals `cursor + 1`
2. Each subsequent entry's sequence number is exactly previous + 1
3. Each entry's `prev_hash` matches the hash of the previous entry

If any check fails:

```text
GAP DETECTED:
  1. Halt sync immediately
  2. Log the gap: expected sequence N, got M
  3. Display UI warning: "Sync integrity error — contact support"
  4. Attempt recovery:
     a. Re-request from the gap point with explicit sequence
     b. If server confirms the gap exists (entries were lost):
        - Mark the gap in the local sync log
        - Offer re-bootstrap as the safe recovery path
        - Never silently skip data
  5. The gap is recorded in the audit log as a sync integrity event
```

### Prevention

- The server never deletes op-log entries that haven't been acknowledged by all registered devices
- Sequence numbers are monotonic and gapless per facility
- Hash chaining provides tamper detection independent of sequence numbers

## Bootstrap Protocol

```text
NEW DEVICE BOOTSTRAP:

  1. Device registers with upstream (presents device certificate)
  2. Server generates point-in-time snapshot:
     ┌─────────────────────────────────────────┐
     │ SNAPSHOT MANIFEST                        │
     │ facility_id: UUID                        │
     │ snapshot_at: DateTime                    │
     │ op_log_position: u64 (sequence number)   │
     │ tables: [                                │
     │   { name: "patient", row_count: N },     │
     │   { name: "encounter", row_count: N },   │
     │   ...                                    │
     │ ]                                        │
     │ packages: [                              │
     │   { id: UUID, version: "1.2.0" },        │
     │   ...                                    │
     │ ]                                        │
     │ total_size_bytes: N                       │
     │ checksum: SHA-256                         │
     └─────────────────────────────────────────┘

  3. Snapshot is transferred as a series of chunked CBOR+zstd batches
     (same format as sync messages, but with snapshot_chunk message type)

  4. Client applies snapshot:
     a. Create all tables and indexes
     b. Insert all rows
     c. Install and activate all packages
     d. Set cursor to snapshot's op_log_position
     e. Verify checksum

  5. Incremental sync proceeds from the cursor
```

### Snapshot Transfer on Slow Links

For a 1 Mbps link (common in rural settings):
- 500 MB snapshot / 1 Mbps = ~67 minutes raw transfer
- With zstd compression (~70%): ~150 MB / 1 Mbps = ~20 minutes
- The transfer is chunked (1 MB chunks) and resumable — if interrupted, it resumes from the last acknowledged chunk

### USB Bootstrap

Same snapshot format, written to an encrypted signed file:
```text
rustyclinic-snapshot-{facility_id}-{date}.rcsnap
```

The `.rcsnap` file contains:
1. Header with facility ID, date, and op-log position
2. Signature (Ed25519 using facility key)
3. Encrypted payload (ChaCha20-Poly1305 using a derived key from the facility key)

## LAN Peer Sync

For clinic-to-clinic sync when both are offline from the district:

```text
DEVICE A                                    DEVICE B
   │                                           │
   ├── mDNS discovery: _rustyclinic._tcp       │
   │                                           │
   ├── mutual TLS handshake ──────────────────▶│
   │   (both present device certificates)      │
   │                                           │
   ├── exchange facility IDs + cursors ────────▶│
   │                                           │
   │◀── push: B's ops since A's cursor ────────┤
   ├── push: A's ops since B's cursor ────────▶│
   │                                           │
   ├── ack ───────────────────────────────────▶│
   │◀── ack ───────────────────────────────────┤
```

LAN sync uses the same message format and conflict rules as HTTPS sync. The only difference is discovery (mDNS) and transport (direct TCP with mutual TLS instead of HTTPS to an upstream server).

## Op-Log Pruning

```text
PRUNING DECISION TREE:

  For each op-log entry:
    ├── Is it acknowledged by upstream?
    │   ├── NO → Never prune. Keep until synced.
    │   └── YES → Check retention window
    │       ├── Within retention window (default 30 days)? → Keep
    │       └── Beyond retention window? → Eligible for pruning
    │           └── Prune during idle/charging (nano) or maintenance window
    │
    └── Is this device the ONLY copy?
        └── YES → Never prune (this IS the upstream for this data)
```

### Storage Estimates

| Scenario | Ops/day | Monthly growth (pre-pruning) | Post-pruning (30-day window) |
|----------|---------|------------------------------|------------------------------|
| CHW tablet (nano) | 20-50 | 5-15 MB | 5-15 MB (rolling) |
| Rural clinic (micro) | 100-300 | 30-90 MB | 30-90 MB (rolling) |
| District hospital (standard) | 1000+ | 300+ MB | Not pruned (PostgreSQL, unlimited retention) |

## Sync Health Metrics

The `SyncHealthProjection` exposes:

| Metric | Source | Alert Threshold |
|--------|--------|-----------------|
| `pending_ops_count` | Count of SyncState::Pending entries | > 1000 |
| `last_push_at` | Timestamp of last successful push | > 24 hours ago |
| `last_pull_at` | Timestamp of last successful pull | > 24 hours ago |
| `conflict_queue_depth` | Unresolved conflicts | > 10 |
| `cursor_lag` | Server sequence - local cursor | > 5000 |
| `device_online` | Current connectivity state | offline > 48h |
| `package_drift` | Local vs upstream package versions | Any mismatch |

These metrics feed both the local sync status UI and the fleet health dashboard (from the CEO review fleet ops expansion).

## Open Questions Resolved

1. **What happens during sync if care is actively happening?** Sync runs in background. It never blocks clinical operations. If a write happens during pull, the write completes normally and the entry is added to the local op-log. The next push will include it.

2. **Can sync run during clinic hours?** Yes — sync is designed to be lightweight. Push/pull operations are batched and compressed. On nano devices, large sync backlogs are deferred to idle/charging periods, but incremental sync (small batches) runs continuously.

3. **What if the upstream server is also offline?** LAN peer sync or USB media. The architecture supports three transports precisely because upstream availability is not guaranteed.
