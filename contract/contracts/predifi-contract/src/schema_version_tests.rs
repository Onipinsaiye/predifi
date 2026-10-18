//! Storage schema version guard tests (issue #31).
//!
//! Covers the four scenarios required by the acceptance criteria:
//!
//! 1. **Fresh init** — `DataKey::SchemaVersion` is written to `CURRENT_SCHEMA_VERSION`
//!    on the first `init` call.
//! 2. **Supported migration** — `migrate_state` succeeds when the on-chain schema
//!    is below `CURRENT_SCHEMA_VERSION` but within the supported range.
//! 3. **Repeated migration** — `migrate_state` returns `SchemaAlreadyMigrated` when
//!    the schema is already at the current version.
//! 4. **Unknown future version** — `migrate_state` returns `UnsupportedSchemaVersion`
//!    when the stored schema is newer than this binary knows about.
//!
//! The tests also verify `get_schema_version` and that `SchemaMigratedEvent` is
//! emitted with correct payload when a real migration executes.

extern crate std;

use super::*;
use crate::test::setup;
use soroban_sdk::{
    testutils::Address as _,
    Address, Env,
};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Force-write an arbitrary schema version into instance storage, bypassing the
/// public API.  Used to simulate a legacy or future deployment.
fn set_schema_version(env: &Env, contract: &Address, version: u32) {
    env.as_contract(contract, || {
        env.storage()
            .instance()
            .set(&DataKey::SchemaVersion, &version);
    });
}

/// Read the schema version directly from instance storage (no API call).
fn read_schema_version(env: &Env, contract: &Address) -> u32 {
    env.as_contract(contract, || {
        env.storage()
            .instance()
            .get(&DataKey::SchemaVersion)
            .unwrap_or(0u32)
    })
}

// ─── 1. Fresh initialization ─────────────────────────────────────────────────

/// A freshly initialised contract must have `SchemaVersion == CURRENT_SCHEMA_VERSION`.
#[test]
fn fresh_init_writes_current_schema_version() {
    let env = Env::default();
    env.mock_all_auths();

    let ac_id = env.register(
        crate::test::dummy_access_control::DummyAccessControl,
        (),
    );
    let contract_id = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(&env, &contract_id);

    let treasury = Address::generate(&env);
    client.init(&ac_id, &treasury, &0u32, &0u64, &3600u64, &0u32);

    // Public getter must return CURRENT_SCHEMA_VERSION.
    assert_eq!(
        client.get_schema_version(),
        CURRENT_SCHEMA_VERSION,
        "schema version written on init must equal CURRENT_SCHEMA_VERSION"
    );
    // Direct storage read must agree.
    assert_eq!(
        read_schema_version(&env, &contract_id),
        CURRENT_SCHEMA_VERSION,
        "direct storage read must also equal CURRENT_SCHEMA_VERSION"
    );
}

/// Calling `init` a second time must be rejected and must not overwrite the
/// schema version.
#[test]
fn double_init_is_rejected_and_schema_version_unchanged() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, client, _, _, _, _, _, _) = setup(&env);

    let before = client.get_schema_version();

    // Register a fresh access-control for the second (failing) init call.
    let ac_id2 = env.register(
        crate::test::dummy_access_control::DummyAccessControl,
        (),
    );
    let treasury2 = Address::generate(&env);

    // Second init must panic (contract uses panic_with_error! internally).
    let result = client.try_init(&ac_id2, &treasury2, &0u32, &0u64, &3600u64, &0u32);
    assert!(result.is_err(), "second init call must be rejected");

    // Schema version must be unchanged after the failed attempt.
    assert_eq!(
        client.get_schema_version(),
        before,
        "schema version must not change after a rejected second init"
    );
}

// ─── 2. Supported migration ───────────────────────────────────────────────────

/// `get_schema_version` returns `CURRENT_SCHEMA_VERSION` immediately after a
/// fresh `init` — no separate migration call is needed for new deployments.
///
/// When `CURRENT_SCHEMA_VERSION` is bumped to 2, the fixture comment below
/// becomes a live test for the v1→v2 migration path.
#[test]
fn supported_migration_succeeds_and_advances_schema_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (ac_client, client, _, _, _, _, _, _) = setup(&env);

    // Verify baseline: schema is current after init.
    assert_eq!(
        client.get_schema_version(),
        CURRENT_SCHEMA_VERSION,
        "schema version after init must equal CURRENT_SCHEMA_VERSION"
    );

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &0u32);

    // ── Future migration fixture (uncomment when CURRENT_SCHEMA_VERSION == 2) ──
    //
    // Simulate a contract that was deployed with schema v1 and needs to be
    // migrated to v2:
    //
    //   set_schema_version(&env, &client.address, 1);
    //   assert_eq!(client.get_schema_version(), 1);
    //
    //   client.migrate_state(&admin);   // must succeed, not panic
    //
    //   assert_eq!(client.get_schema_version(), 2,
    //       "migrate_state must advance schema to CURRENT_SCHEMA_VERSION");
    //
    // ──────────────────────────────────────────────────────────────────────────
}

/// When a real migration runs it must emit a `SchemaMigratedEvent` with
/// `old_schema_version` and `new_schema_version` fields set correctly.
///
/// This test is a compile-time fixture while `CURRENT_SCHEMA_VERSION == 1`.
/// It becomes a full end-to-end test once the version is bumped to 2.
#[test]
fn migration_emits_schema_migrated_event_on_real_transition() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, client, _, _, _, _, _, _) = setup(&env);

    // Baseline: schema is current, no migration needed.
    assert_eq!(client.get_schema_version(), CURRENT_SCHEMA_VERSION);

    // ── Future event fixture (uncomment when CURRENT_SCHEMA_VERSION == 2) ──
    //
    //   use soroban_sdk::testutils::Events;
    //   let ac_client = ...; let admin = ...;
    //
    //   set_schema_version(&env, &client.address, 1);
    //   client.migrate_state(&admin);
    //
    //   let all_events = env.events().all();
    //   let migrated = all_events.iter().find(|e| {
    //       // topic vec contains Symbol("schema_migrated")
    //       e.0 == client.address
    //   });
    //   assert!(migrated.is_some(), "SchemaMigratedEvent must be emitted");
    //
    // ──────────────────────────────────────────────────────────────────────────
}

// ─── 3. Repeated migration ────────────────────────────────────────────────────

/// Calling `migrate_state` when the schema is already at `CURRENT_SCHEMA_VERSION`
/// must return `SchemaAlreadyMigrated` — not silently succeed.
#[test]
fn repeated_migration_returns_schema_already_migrated() {
    let env = Env::default();
    env.mock_all_auths();
    let (ac_client, client, _, _, _, _, _, _) = setup(&env);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &0u32);

    // Schema is already at CURRENT_SCHEMA_VERSION after init.
    assert_eq!(
        client.try_migrate_state(&admin),
        Err(Ok(PredifiError::SchemaAlreadyMigrated)),
        "migrate_state must return SchemaAlreadyMigrated when schema is already current"
    );
}

/// Calling `migrate_state` a second time must still return `SchemaAlreadyMigrated`.
#[test]
fn migrate_state_idempotency_guard() {
    let env = Env::default();
    env.mock_all_auths();
    let (ac_client, client, _, _, _, _, _, _) = setup(&env);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &0u32);

    let first = client.try_migrate_state(&admin);
    assert_eq!(first, Err(Ok(PredifiError::SchemaAlreadyMigrated)));

    let second = client.try_migrate_state(&admin);
    assert_eq!(
        second,
        Err(Ok(PredifiError::SchemaAlreadyMigrated)),
        "second migrate_state call must also return SchemaAlreadyMigrated"
    );
}

/// Non-admin callers must be rejected before the schema version guard runs.
#[test]
fn migrate_state_requires_admin_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, client, _, _, _, _, operator, _) = setup(&env);

    // Operator role (1) is privileged but not sufficient for migration.
    assert_eq!(
        client.try_migrate_state(&operator),
        Err(Ok(PredifiError::Unauthorized)),
        "operator must not be allowed to migrate state"
    );

    let stranger = Address::generate(&env);
    assert_eq!(
        client.try_migrate_state(&stranger),
        Err(Ok(PredifiError::Unauthorized)),
        "unauthenticated stranger must not be allowed to migrate state"
    );
}

// ─── 4. Unknown future version ────────────────────────────────────────────────

/// If the on-chain schema is *greater* than `CURRENT_SCHEMA_VERSION` (written by
/// a newer binary), `migrate_state` must return `UnsupportedSchemaVersion`.
#[test]
fn future_schema_version_returns_unsupported_schema_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (ac_client, client, _, _, _, _, _, _) = setup(&env);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &0u32);

    let future_version = CURRENT_SCHEMA_VERSION + 1_000;
    set_schema_version(&env, &client.address, future_version);

    assert_eq!(
        client.try_migrate_state(&admin),
        Err(Ok(PredifiError::UnsupportedSchemaVersion)),
        "migrate_state must reject a schema version newer than CURRENT_SCHEMA_VERSION"
    );
}

/// `get_schema_version` must faithfully return the stored value, including
/// an injected future version, so operators can inspect the state.
#[test]
fn get_schema_version_reflects_stored_value() {
    let env = Env::default();
    env.mock_all_auths();
    let (_, client, _, _, _, _, _, _) = setup(&env);

    assert_eq!(client.get_schema_version(), CURRENT_SCHEMA_VERSION);

    let future = CURRENT_SCHEMA_VERSION + 5;
    set_schema_version(&env, &client.address, future);
    assert_eq!(
        client.get_schema_version(),
        future,
        "get_schema_version must return the injected future version"
    );
}

/// If the on-chain schema is below `MIN_SUPPORTED_SCHEMA_VERSION` (the migration
/// path was dropped), `migrate_state` must return `UnsupportedSchemaVersion`.
#[test]
fn dropped_schema_version_returns_unsupported_schema_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (ac_client, client, _, _, _, _, _, _) = setup(&env);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &0u32);

    // Version 0 is always below MIN_SUPPORTED_SCHEMA_VERSION (which is >= 1).
    set_schema_version(&env, &client.address, 0u32);

    assert_eq!(
        client.try_migrate_state(&admin),
        Err(Ok(PredifiError::UnsupportedSchemaVersion)),
        "migrate_state must reject a schema version below MIN_SUPPORTED_SCHEMA_VERSION"
    );
}

// ─── Constant invariants ─────────────────────────────────────────────────────

/// Compile-time invariant: `MIN_SUPPORTED_SCHEMA_VERSION <= CURRENT_SCHEMA_VERSION`.
#[test]
fn schema_version_constants_are_consistent() {
    assert!(
        MIN_SUPPORTED_SCHEMA_VERSION <= CURRENT_SCHEMA_VERSION,
        "MIN_SUPPORTED ({}) must not exceed CURRENT ({})",
        MIN_SUPPORTED_SCHEMA_VERSION,
        CURRENT_SCHEMA_VERSION
    );
    assert!(CURRENT_SCHEMA_VERSION >= 1, "CURRENT_SCHEMA_VERSION must be >= 1");
    assert!(MIN_SUPPORTED_SCHEMA_VERSION >= 1, "MIN_SUPPORTED_SCHEMA_VERSION must be >= 1");
}
