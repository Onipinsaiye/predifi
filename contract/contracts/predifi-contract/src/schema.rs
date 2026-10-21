//! # Storage Schema Version Guard
//!
//! This module provides the machinery for **detecting, validating, and evolving**
//! the on-chain storage layout of the PrediFi contract.
//!
//! ## Motivation
//!
//! Soroban smart contracts store state as XDR-encoded values.  When a contract
//! upgrade changes the shape of a stored struct (adding / removing / reordering
//! fields), existing ledger entries are still encoded in the *old* format.  If
//! the new binary reads them without any migration step it may:
//!
//! * silently deserialise garbage values (field type mismatch), or
//! * trap on an XDR decode error (corrupted state from the runtime's perspective).
//!
//! The schema version guard makes state-layout changes **detectable and fail-safe**:
//!
//! 1. `init()` writes `CURRENT_SCHEMA_VERSION` to instance storage under
//!    `DataKey::SchemaVersion`.
//! 2. After an upgrade the admin calls `migrate_state()`.  The guard reads the
//!    stored version, validates it against the known migration range, and runs the
//!    appropriate migration hook(s).
//! 3. If the stored version equals `CURRENT_SCHEMA_VERSION` the guard returns
//!    `SchemaMigrationAlreadyRun` to prevent accidental re-runs.
//! 4. If the stored version is *ahead* of `CURRENT_SCHEMA_VERSION` (i.e. a future
//!    schema that this binary doesn't know about) the guard returns
//!    `UnsupportedSchemaVersion` so that operators know to upgrade the binary first.
//!
//! ## Versioning policy
//!
//! | Constant                | Value | Meaning |
//! |-------------------------|-------|---------|
//! | `SCHEMA_VERSION_INITIAL`| 1     | First deployed layout |
//! | `CURRENT_SCHEMA_VERSION`| 1     | Layout expected by *this* binary |
//!
//! Every time the storage layout changes:
//! 1. Add a new `SCHEMA_VERSION_Vn` constant.
//! 2. Bump `CURRENT_SCHEMA_VERSION` to `n`.
//! 3. Add a `migrate_v{n-1}_to_v{n}` function.
//! 4. Register it inside `run_pending_migrations`.

use soroban_sdk::Env;

use crate::{DataKey, PredifiError};

// ═══════════════════════════════════════════════════════════════════════════
// SCHEMA VERSION CONSTANTS
// ═══════════════════════════════════════════════════════════════════════════

/// Schema version written by the very first deployment.
///
/// This corresponds to the original storage layout: `Config`, `Pool`, `Prediction`,
/// etc. as defined at contract genesis.  It must never change.
pub const SCHEMA_VERSION_INITIAL: u32 = 1;

/// The schema version that *this* binary understands.
///
/// `init()` stores this value in `DataKey::SchemaVersion` on fresh deployment.
/// `migrate_state()` migrates from the stored version up to this value.
///
/// **Bump this constant whenever the storage layout changes.**  A layout change
/// includes: adding/removing/reordering fields in any `#[contracttype]` struct
/// or enum that is persisted in storage.
pub const CURRENT_SCHEMA_VERSION: u32 = SCHEMA_VERSION_INITIAL;

// ═══════════════════════════════════════════════════════════════════════════
// PUBLIC HELPERS
// ═══════════════════════════════════════════════════════════════════════════

/// Read the schema version currently stored on-chain.
///
/// Returns `0` when the key is absent, which happens on contracts deployed
/// *before* schema versioning was introduced.  The migration system treats
/// version 0 as "pre-versioning" and will upgrade it to `CURRENT_SCHEMA_VERSION`
/// when `migrate_state` is called.
pub fn read_schema_version(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&DataKey::SchemaVersion)
        .unwrap_or(0u32)
}

/// Write `version` to `DataKey::SchemaVersion` in instance storage.
pub fn write_schema_version(env: &Env, version: u32) {
    env.storage()
        .instance()
        .set(&DataKey::SchemaVersion, &version);
}

/// Validate that the stored schema version is within the range this binary
/// can migrate.  Called from `migrate_state` before any migration logic runs.
///
/// # Errors
/// * `SchemaMigrationAlreadyRun` — stored version already equals
///   `CURRENT_SCHEMA_VERSION`; nothing to do.
/// * `UnsupportedSchemaVersion` — stored version is *greater than*
///   `CURRENT_SCHEMA_VERSION`; this binary is too old to handle the layout.
pub fn validate_schema_version_for_migration(
    stored_version: u32,
) -> Result<(), PredifiError> {
    if stored_version == CURRENT_SCHEMA_VERSION {
        return Err(PredifiError::SchemaMigrationAlreadyRun);
    }
    if stored_version > CURRENT_SCHEMA_VERSION {
        return Err(PredifiError::UnsupportedSchemaVersion);
    }
    Ok(())
}

/// Execute all pending migration steps, advancing from `stored_version` to
/// `CURRENT_SCHEMA_VERSION` one step at a time.
///
/// Each step is a small, focused function that transforms the on-chain state
/// from version `n` to `n+1`.  Steps run in order and are never skipped —
/// this ensures that a contract can be upgraded from *any* older schema to the
/// current one in a single `migrate_state` call, regardless of how many
/// intermediate versions were skipped.
///
/// After all steps succeed, `DataKey::SchemaVersion` is updated to
/// `CURRENT_SCHEMA_VERSION`.
///
/// # Panics / Errors
/// Individual migration steps should return `Result<(), PredifiError>` and
/// propagate errors upward so that the transaction is rolled back atomically.
pub fn run_pending_migrations(env: &Env, stored_version: u32) -> Result<(), PredifiError> {
    let mut version = stored_version;

    // ── Add migration steps below as the schema evolves ──────────────────
    //
    // Template:
    //
    //   if version < SCHEMA_VERSION_V2 {
    //       migrate_v1_to_v2(env)?;
    //       version = SCHEMA_VERSION_V2;
    //   }
    //
    // There are no pending migrations at schema version 1.
    //
    // ─────────────────────────────────────────────────────────────────────

    // Silence "unused variable" warning when no migrations are registered yet.
    let _ = &mut version;
    let _ = env;

    // Persist the final version so subsequent calls return
    // `SchemaMigrationAlreadyRun` instead of re-running.
    write_schema_version(env, CURRENT_SCHEMA_VERSION);
    Ok(())
}

// ═══════════════════════════════════════════════════════════════════════════
// MIGRATION STEP FUNCTIONS  (add one per schema version bump)
// ═══════════════════════════════════════════════════════════════════════════
//
// Convention: `migrate_v{from}_to_v{to}(env: &Env) -> Result<(), PredifiError>`
//
// Each function should:
//   1. Read the affected storage keys under the *old* layout.
//   2. Transform the data into the *new* layout.
//   3. Write the new values back.
//   4. Delete any keys that no longer exist in the new layout.
//
// Example (v1 → v2):
//
// fn migrate_v1_to_v2(env: &Env) -> Result<(), PredifiError> {
//     // Add new Config.some_new_field = default_value for all existing entries.
//     // This is typically a no-op for instance-storage structs because Soroban's
//     // XDR codec ignores extra trailing fields on read, so re-saving with the
//     // new field is all that's needed.
//     Ok(())
// }
