# Auth & Session Design

## Context

Authentication and session management in RustyClinic must handle a unique set of constraints: offline operation with cached credentials, shared devices in clinic settings, multiple auth modes (password, PIN, biometric, smart card), and a device security lifecycle for stolen/lost devices. This design doc specifies the implementation details for `rustyclinic-auth`.

## Auth Modes

```text
AUTH MODE SELECTION:

  ┌──────────────┐     ┌──────────────┐     ┌──────────────┐
  │   Password   │     │     PIN      │     │  Biometric   │
  │  (primary)   │     │  (fast swap) │     │ (optional)   │
  └──────┬───────┘     └──────┬───────┘     └──────┬───────┘
         │                    │                    │
         └────────────┬───────┘                    │
                      │                            │
              ┌───────▼────────┐          ┌────────▼───────┐
              │ Credential     │          │ Biometric SDK  │
              │ Verification   │          │ (device-native)│
              └───────┬────────┘          └────────┬───────┘
                      │                            │
                      └────────────┬───────────────┘
                                   │
                           ┌───────▼────────┐
                           │ Session Created │
                           │ (JWT or local)  │
                           └────────────────┘
```

### Password

- Primary authentication method for all users
- Minimum 8 characters, no maximum (hashed, length doesn't matter)
- Hashed with Argon2id (memory: 64MB, iterations: 3, parallelism: 4)
- Online: verified against the auth service
- Offline: verified against cached Argon2id hash

### PIN

- 4-6 digit numeric PIN for fast user switching on shared devices
- NOT a replacement for password — supplements it
- PIN is set after initial password authentication
- PIN has a shorter offline lifetime than password (default: 7 days vs 14 days)
- After 5 failed PIN attempts, PIN is locked and password is required
- PIN is hashed with Argon2id (same parameters, but the input is shorter — the hash parameters compensate)

### Biometric

- Optional, device-dependent (fingerprint scanner, facial recognition)
- Enrollment requires password authentication first
- Biometric templates are stored encrypted on the device
- Biometric match produces a device-local attestation that maps to a user session
- Falls back to PIN or password if biometric hardware is unavailable

### Smart Card / NFC

- Optional, available where health worker ID cards have NFC/smart card chips
- Card presents a certificate or signed challenge
- Maps to a user via the identifier registry
- Falls back to PIN or password if reader is unavailable

## Token Format

### Online Sessions (Standard/Enterprise Tiers)

```text
JWT STRUCTURE:

  Header:
    { "alg": "EdDSA", "typ": "JWT", "kid": "facility-key-id" }

  Payload:
    {
      "sub": "user-uuid",
      "iss": "rustyclinic",
      "aud": "rustyclinic",
      "iat": 1710000000,
      "exp": 1710003600,       // 1 hour
      "facility_id": "uuid",
      "device_id": "uuid",
      "roles": ["nurse", "queue_manager"],
      "purpose": "clinical_care",
      "session_id": "uuid",
      "auth_method": "password"  // or "pin", "biometric", "smartcard"
    }

  Signature:
    Ed25519 using the facility's signing key
```

- **Access token lifetime:** 1 hour
- **Refresh token lifetime:** 24 hours (stored as httpOnly cookie)
- **Signing algorithm:** Ed25519 (fast, compact signatures, no RSA overhead)

### Offline Sessions (Nano/Micro Tiers)

On SQLite tiers, sessions are local database rows instead of JWTs:

```rust
pub struct LocalSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub facility_id: Uuid,
    pub device_id: Uuid,
    pub roles: Vec<String>,
    pub purpose: String,
    pub auth_method: AuthMethod,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub state: SessionState,
}

pub enum SessionState {
    Active,
    Locked { locked_at: DateTime<Utc> },
    Expired,
    Revoked { reason: String },
}
```

The same expiry and scope semantics apply. The difference is storage (SQLite row vs JWT), not behavior.

## Offline Credential Cache

### Storage

```rust
pub struct CachedCredential {
    pub user_id: Uuid,
    pub credential_type: CredentialType, // Password or PIN
    pub hash: String,                    // Argon2id hash
    pub cached_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub max_lifetime_days: u16,          // deployment-configured
    pub roles_snapshot: Vec<String>,     // roles at cache time
    pub permissions_snapshot: Vec<String>,
}

pub enum CredentialType {
    Password,
    Pin,
}
```

### Lifetime Rules

| Credential | Default Lifetime | Min | Max | Rationale |
|-----------|-----------------|-----|-----|-----------|
| Password | 14 days | 1 day | 90 days | Balance between security and offline usability |
| PIN | 7 days | 1 day | 30 days | Shorter because PINs are weaker |

### Clock Manipulation Defense (Critical Gap from Reviews)

The reviews identified clock manipulation as a critical gap — a user could set the device clock backward to extend credential lifetime.

**Defense mechanism:**

```text
MONOTONIC CLOCK CHECK:

  1. On every authentication attempt, record the system clock reading
  2. Compare with the LAST recorded clock reading (stored in SQLite)
  3. If current time < last recorded time - 5 minutes:
     a. Log a clock regression event in audit
     b. Display warning: "Device clock appears incorrect"
     c. Fall back to credential creation time + max_lifetime check
        using a monotonic counter (boot count + uptime)
     d. If the device has been rebooted AND the clock is wrong,
        require online re-authentication (cannot extend offline)
  4. On nano/micro devices with RTC batteries, the 5-minute tolerance
     accounts for NTP drift and power-loss clock skew
```

This doesn't prevent all clock attacks (a sophisticated attacker with root access could manipulate the monotonic counter), but it catches accidental and casual clock changes — which is the realistic threat model for a clinic tablet.

## Offline-to-Online Transition

```text
RECONNECTION FLOW:

  Device regains connectivity
       │
       ▼
  ┌─────────────────────┐
  │ Present device cert  │
  │ + current session    │
  │ token to upstream    │
  └──────────┬──────────┘
             │
     ┌───────▼────────┐
     │ Fetch revocation│
     │ delta since     │
     │ last sync cursor│
     └───────┬────────┘
             │
     ┌───────▼────────────────────────────────────┐
     │ REVOCATION DELTA contains:                  │
     │ - Disabled user IDs                         │
     │ - Users with credential resets              │
     │ - Role changes (added/removed roles)        │
     │ - Permission policy updates                 │
     │ - Facility-level policy changes             │
     └───────┬────────────────────────────────────┘
             │
     ┌───────▼────────┐     ┌─────────────────────────┐
     │ For each        │────▶│ User disabled?           │
     │ revocation item │     │ → Terminate session NOW  │
     │                 │     │ → Lock screen displayed  │
     │                 │     ├─────────────────────────┤
     │                 │     │ Credential reset?        │
     │                 │     │ → Expire cached cred     │
     │                 │     │ → Require new login      │
     │                 │     ├─────────────────────────┤
     │                 │     │ Role change?             │
     │                 │     │ → Update local role cache│
     │                 │     │ → Session continues      │
     │                 │     ├─────────────────────────┤
     │                 │     │ Policy update?           │
     │                 │     │ → Update local policy    │
     │                 │     │ → Session continues      │
     └────────────────┘     └─────────────────────────┘
             │
     ┌───────▼────────┐
     │ Refresh cached  │
     │ credentials     │
     │ (new hash +     │
     │  new expiry)    │
     └───────┬────────┘
             │
     ┌───────▼────────┐
     │ Resume sync     │
     └────────────────┘

  If revocation delta fetch FAILS:
    → Continue with last-known policy
    → Display staleness warning in UI header
    → Log staleness event in audit
    → Retry on next sync cycle
```

## Shared-Device Session Management

### Session Isolation

```text
SHARED DEVICE — 3 CONCURRENT SESSIONS:

  ┌─────────────────────────────────────────────────┐
  │ DEVICE: clinic-tablet-01                         │
  ├─────────────────────────────────────────────────┤
  │                                                  │
  │  Session 1: Nurse Mukamana (ACTIVE)             │
  │  ├── Form drafts: ANC visit for patient #312    │
  │  ├── Clipboard: lab results copied              │
  │  └── Queue position: viewing encounter          │
  │                                                  │
  │  Session 2: Dr. Habimana (LOCKED — idle 15min)  │
  │  ├── Form drafts: prescription for patient #298 │
  │  └── Last screen: pharmacy dispense             │
  │                                                  │
  │  Session 3: (available for new user)             │
  │                                                  │
  │  Max concurrent locked sessions: 3 (configurable)│
  │  If a 4th user logs in, the oldest locked        │
  │  session is terminated (drafts preserved in       │
  │  form_draft table, associated with user_id)      │
  └─────────────────────────────────────────────────┘
```

### Session Lifecycle

```text
SESSION STATE MACHINE:

  [Login] ──▶ ACTIVE ──idle timeout──▶ LOCKED ──re-auth──▶ ACTIVE
                │                        │
                │                        ├──session expired──▶ EXPIRED
                │                        │
                │                        └──user disabled ──▶ REVOKED
                │
                ├──logout──▶ TERMINATED
                │
                └──session expired──▶ EXPIRED
```

- **Idle timeout:** Configurable per facility (default: 15 minutes)
- **Lock screen:** Shows user name + facility. Requires PIN or password to unlock.
- **Different user login:** Creates a new session without destroying locked sessions
- **Session storage:** Form drafts, clipboard state, and workflow position are stored in SQLite keyed by `(user_id, session_id)`. They survive lock/unlock and even session expiry (drafts are recoverable for 7 days).

## Device Security Lifecycle (from CEO Review)

### Device Registration

```rust
pub struct Device {
    pub id: Uuid,
    pub facility_id: Uuid,
    pub device_name: String,           // "clinic-tablet-01"
    pub device_type: DeviceType,       // Tablet, Laptop, RaspberryPi, Desktop
    pub certificate_fingerprint: String,
    pub registered_at: DateTime<Utc>,
    pub registered_by: Uuid,
    pub status: DeviceStatus,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub last_sync_at: Option<DateTime<Utc>>,
}

pub enum DeviceStatus {
    Active,
    Suspended { reason: String, suspended_at: DateTime<Utc> },
    Revoked { reason: String, revoked_at: DateTime<Utc> },
    Lost { reported_at: DateTime<Utc>, reported_by: Uuid },
}
```

### Stolen/Lost Device Response

```text
DEVICE REPORTED STOLEN/LOST:

  1. Admin marks device as Lost in control plane
     └── DeviceStatus::Lost { reported_at, reported_by }

  2. Immediate effects (server-side):
     ├── All sync requests from this device are rejected
     ├── Device certificate is added to revocation list
     └── Alert sent to facility admin (SMS if configured)

  3. When device reconnects (if ever):
     ├── Sync endpoint returns 403 with "device_revoked" error
     ├── Device displays: "This device has been reported lost.
     │   Contact your facility administrator."
     └── No data sync occurs — local data remains but cannot
         be sent to or received from upstream

  4. Local data protection:
     ├── SQLite database is encrypted at rest (SQLCipher)
     │   └── Key derived from facility key + device-specific salt
     ├── Cached credentials expire per normal lifetime rules
     │   └── After expiry, no login is possible without connectivity
     ├── If the thief cannot authenticate, they see only the lock screen
     └── PHI is protected by encryption at rest even if storage is
         physically extracted

  5. Remote wipe (when device reconnects):
     ├── Server sends wipe command alongside the 403
     ├── Device erases: all patient data, cached credentials,
     │   form drafts, sync state, and package cache
     ├── Device retains: the executable itself, device certificate
     │   (for audit trail), and the wipe confirmation log
     └── Device displays: "This device has been wiped.
         Contact your facility administrator to re-provision."
```

### Encryption at Rest

- **SQLite databases:** Encrypted using SQLCipher (AES-256-CBC with HMAC-SHA512)
- **Key derivation:** Facility key (from envelope encryption) + device-specific salt → HKDF → SQLCipher key
- **On nano/micro tiers:** The SQLCipher key is derived at startup from the facility key stored in the device's secure element (or, on devices without a secure element, from a key file with filesystem permissions)
- **On standard/enterprise tiers:** PostgreSQL handles encryption at rest via Transparent Data Encryption (TDE) or volume-level encryption

## Break-Glass Access

```rust
pub struct BreakGlassSession {
    pub id: Uuid,
    pub user_id: Uuid,
    pub facility_id: Uuid,
    pub reason_code: BreakGlassReason,
    pub reason_text: String,           // free-text justification
    pub granted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,     // max 4 hours
    pub elevated_permissions: Vec<String>,
    pub reviewed: bool,
    pub reviewed_by: Option<Uuid>,
    pub reviewed_at: Option<DateTime<Utc>>,
}

pub enum BreakGlassReason {
    EmergencyCare,
    SystemRecovery,
    PatientRequest,
    RegulatoryAudit,
    Other,
}
```

### Break-Glass Rules

1. Break-glass grants temporary elevated permissions (e.g., access patient records outside normal scope)
2. Maximum duration: 4 hours (configurable, max 24 hours)
3. Every action during break-glass is tagged in audit with the break-glass session ID
4. Break-glass sessions are queued for mandatory review by a supervisor
5. **Rate limiting:** If the same user triggers break-glass more than 3 times in 7 days, an alert is sent to the facility admin and the reason for frequent use is flagged for review

## RBAC Model

### Role Hierarchy

```text
ROLES (not exhaustive — roles are deployment-configurable via packages):

  system_admin         Full system access, user management, device management
  facility_admin       Facility-level admin, user management within facility
  physician            Full clinical access, prescribing, diagnosis
  nurse                Clinical access, triage, vitals, nursing notes
  midwife              ANC/delivery access, maternal health workflows
  lab_technician       Lab orders, specimen management, results
  pharmacist           Pharmacy access, dispensing, stock management
  billing_clerk        Financial access, claims, payments
  receptionist         Registration, queue management, appointments
  chw                  Community health worker — limited clinical, home visits
  data_officer         Reports, data quality, DHIS2 exports
  auditor              Read-only access to audit logs and reports
```

### Permission Model

Permissions are contextual — they combine role, facility scope, and purpose-of-use:

```text
PERMISSION CHECK:

  Can user U perform action A on resource R?

  1. Does U have a role that grants A? (RBAC check)
  2. Is R within U's facility scope? (tenant isolation)
  3. Is U's purpose-of-use valid for A? (policy check)
  4. Is there a workflow-state constraint? (state machine check)
     e.g., can only dispense if prescription is verified
  5. Is there a co-sign requirement? (confirmation check)
     e.g., pharmacy dispense requires pharmacist co-sign

  All 5 must pass. Any failure → 403 with specific denial reason.
```

## Open Questions Resolved

1. **Should OAuth2/OIDC be supported from day one?** No. Nano and micro tiers don't have internet to reach an external IdP. The built-in auth service is the primary auth mechanism. OIDC support is Phase 8+ (interop) for standard/enterprise tiers that want to integrate with hospital SSO.

2. **Should biometric templates be synced?** No. Biometric templates are device-local and never leave the device. If a user enrolls biometrics on Device A, they must re-enroll on Device B. This avoids syncing sensitive biometric data and simplifies the trust model.

3. **What about SMART on FHIR?** SMART on FHIR is an authorization framework for FHIR APIs. It's relevant for standard/enterprise tiers where external apps access the FHIR API. It's not relevant for nano/micro tiers or internal auth. Implementation is Phase 8+ alongside OIDC.
