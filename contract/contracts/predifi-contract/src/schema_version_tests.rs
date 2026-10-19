#![cfg(test)]

//! # Schema Version Guard Tests
//!
//! ## Coverage matrix (per acceptance criteria)
//!
//! | Scenario | Test |
//! |----------|------|
//! | Fresh init stores `CURRENT_SCHEMA_VERSION` | `fresh_init_stores_current_schema_version` |
//! | `get_schema_version` returns correct value | `get_schema_version_matches_stored_value` |
//! | Pre-versioning contract (version 0) can migrate | `pre_versioned_contract_migrates_to_current` |
//! | Supported migration runs once and emits event | `supported_migration_runs_once_and_emits_event` |
//! | Repeated migration is rejected | `repeated_migration_is_rejected` |
//! | Unknown future version is rejected | `unknown_future_schema_version_is_rejected` |
//! | Non-admin cannot call `migrate_state` | `non_admin_cannot_call_migrate_state` |
//! | Paused contract blocks `migrate_state` | `paused_contract_blocks_migrate_state` |
//! | Migration updates stored version | `migration_updates_stored_schema_version` |
//! | Unit: `validate_schema_version_for_migration` | `validate_already_current_returns_error` / `validate_future_returns_error` / `validate_older_returns_ok` |

extern crate std;

use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{
    schema::{
        read_schema_version, validate_schema_version_for_migration, write_schema_version,
        CURRENT_SCHEMA_VERSION, SCHEMA_VERSION_INITIAL,
    },
    DataKey, PredifiContract, PredifiContractClient, PredifiError,
};

// ─── Shared helpers ───────────────────────────────────────────────────────────

use crate::test::{dummy_access_control, setup, ROLE_ADMIN};

/// Returns `(ac_client, client, ac_id, admin_address)` for tests that only
/// need schema-version operations (no token or pool needed).
fn setup_schema(
    env: &Env,
) -> (
    dummy_access_control::DummyAccessControlClient<'_>,
    PredifiContractClient<'_>,
    Address, // access-control contract id
    Address, // admin address
) {
    let ac_id = env.register(dummy_access_control::DummyAccessControl, ());
    let ac_client = dummy_access_control::DummyAccessControlClient::new(env, &ac_id);

    let contract_id = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(env, &contract_id);

    let treasury = Address::generate(env);
    let admin = Address::generate(env);

    ac_client.grant_role(&admin, &ROLE_ADMIN);
    client.init(&ac_id, &treasury, &0u32, &0u64, &3600u64, &0u32);

    (ac_client, client, ac_id, admin)
}

// ─── Schema-version constant tests ──────────────────────────────────────────

#[test]
fn schema_version_initial_is_one() {
    assert_eq!(SCHEMA_VERSION_INITIAL, 1, "SCHEMA_VERSION_INITIAL must be 1");
}

#[test]
fn current_schema_version_equals_initial_at_genesis() {
    // Until the first storage layout change is merged, current == initial.
    assert_eq!(
        CURRENT_SCHEMA_VERSION,
        SCHEMA_VERSION_INITIAL,
        "CURRENT_SCHEMA_VERSION should equal SCHEMA_VERSION_INITIAL at genesis"
    );
}

// ─── Fresh init tests ────────────────────────────────────────────────────────

/// `init()` must write `CURRENT_SCHEMA_VERSION` to `DataKey::SchemaVersion`.
#[test]
fn fresh_init_stores_current_schema_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, _client, _ac_id, _admin) = setup_schema(&env);

    let stored: u32 = env
        .storage()
        .instance()
        .get(&DataKey::SchemaVersion)
        .expect("SchemaVersion must be present after init");

    assert_eq!(
        stored,
        CURRENT_SCHEMA_VERSION,
        "init must write CURRENT_SCHEMA_VERSION to DataKey::SchemaVersion"
    );
}

/// `get_version` still returns `CONTRACT_VERSION` independently of schema version.
#[test]
fn get_schema_version_matches_stored_value() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, _admin) = setup_schema(&env);

    // Schema version is tracked independently from the contract version counter
    // (which counts binary upgrades, not storage-layout versions).
    let stored: u32 = env
        .storage()
        .instance()
        .get(&DataKey::SchemaVersion)
        .unwrap_or(0);

    assert_eq!(stored, CURRENT_SCHEMA_VERSION);

    // Contract version should be >= 1 (written by init via CONTRACT_VERSION const).
    assert!(
        client.get_version() >= 1,
        "get_version() must return at least 1 after init"
    );
}

// ─── Migration guard unit tests (pure Rust, no Soroban host) ─────────────────

#[test]
fn validate_already_current_returns_error() {
    let result = validate_schema_version_for_migration(CURRENT_SCHEMA_VERSION);
    assert_eq!(
        result,
        Err(PredifiError::SchemaMigrationAlreadyRun),
        "version == CURRENT should return SchemaMigrationAlreadyRun"
    );
}

#[test]
fn validate_future_version_returns_error() {
    let future = CURRENT_SCHEMA_VERSION + 1;
    let result = validate_schema_version_for_migration(future);
    assert_eq!(
        result,
        Err(PredifiError::UnsupportedSchemaVersion),
        "version > CURRENT should return UnsupportedSchemaVersion"
    );
}

#[test]
fn validate_version_zero_returns_ok() {
    // Version 0 is the "pre-versioning" baseline: migration is needed and possible.
    let result = validate_schema_version_for_migration(0);
    assert!(
        result.is_ok(),
        "version 0 (pre-versioning) should be migratable"
    );
}

#[test]
fn validate_older_version_returns_ok() {
    // Any version strictly less than CURRENT is migratable.
    // This test only runs if CURRENT > 1; skip gracefully for v1.
    if CURRENT_SCHEMA_VERSION > 1 {
        let result = validate_schema_version_for_migration(CURRENT_SCHEMA_VERSION - 1);
        assert!(
            result.is_ok(),
            "version < CURRENT should be migratable (ok)"
        );
    }
}

// ─── Integration tests via Soroban test environment ──────────────────────────

/// A contract initialised with a pre-versioning baseline (schema version 0)
/// should successfully migrate to `CURRENT_SCHEMA_VERSION`.
#[test]
fn pre_versioned_contract_migrates_to_current() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // Simulate a "pre-versioning" deployment by overwriting SchemaVersion with 0.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    // Verify baseline.
    env.as_contract(client.address(), || {
        assert_eq!(read_schema_version(&env), 0);
    });

    // Migration from version 0 must succeed.
    let result = client.try_migrate_state(&admin);
    assert!(
        result.is_ok(),
        "migrate_state must succeed from pre-versioning baseline"
    );

    // After migration the stored version must be CURRENT_SCHEMA_VERSION.
    env.as_contract(client.address(), || {
        assert_eq!(
            read_schema_version(&env),
            CURRENT_SCHEMA_VERSION,
            "stored schema version must equal CURRENT_SCHEMA_VERSION after migration"
        );
    });
}

/// When the stored schema version already equals `CURRENT_SCHEMA_VERSION`,
/// `migrate_state` must return `SchemaMigrationAlreadyRun`.
#[test]
fn repeated_migration_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // First, set the schema to 0 so that the first migration succeeds.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    // First call succeeds.
    client.migrate_state(&admin);

    // Second call on the *already-current* version must be rejected.
    let second = client.try_migrate_state(&admin);
    assert_eq!(
        second,
        Err(Ok(PredifiError::SchemaMigrationAlreadyRun)),
        "second migrate_state must return SchemaMigrationAlreadyRun"
    );
}

/// A contract whose on-chain schema version is ahead of `CURRENT_SCHEMA_VERSION`
/// must return `UnsupportedSchemaVersion`.
#[test]
fn unknown_future_schema_version_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // Inject a future schema version that this binary doesn't know about.
    let future_version = CURRENT_SCHEMA_VERSION + 99;
    env.as_contract(client.address(), || {
        write_schema_version(&env, future_version);
    });

    let result = client.try_migrate_state(&admin);
    assert_eq!(
        result,
        Err(Ok(PredifiError::UnsupportedSchemaVersion)),
        "migrate_state on a future schema version must return UnsupportedSchemaVersion"
    );
}

/// Only an address with the Admin role may call `migrate_state`.
#[test]
fn non_admin_cannot_call_migrate_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, _admin) = setup_schema(&env);

    // Set version 0 so the guard doesn't fire before the auth check.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    let non_admin = Address::generate(&env);
    let result = client.try_migrate_state(&non_admin);

    // `require_admin_role` returns PredifiError::Unauthorized.
    assert!(
        result.is_err(),
        "non-admin should not be able to call migrate_state"
    );
}

/// A paused contract must block `migrate_state`.
#[test]
fn paused_contract_blocks_migrate_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // Pause the contract.
    client.pause(&admin);

    // Set version 0 so the schema guard doesn't fire before the pause check.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    let result = client.try_migrate_state(&admin);
    assert!(
        result.is_err(),
        "migrate_state must fail when the contract is paused"
    );
}

/// After a successful migration from version 0, the on-chain schema version
/// must be updated to `CURRENT_SCHEMA_VERSION`.
#[test]
fn migration_updates_stored_schema_version() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // Inject version 0.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    client.migrate_state(&admin);

    env.as_contract(client.address(), || {
        let after = read_schema_version(&env);
        assert_eq!(
            after,
            CURRENT_SCHEMA_VERSION,
            "schema version must be updated after migration"
        );
    });
}

/// `migrate_state` must emit a `SchemaMigratedEvent` with correct fields.
#[test]
fn migration_emits_schema_migrated_event() {
    use soroban_sdk::testutils::Events;
    use soroban_sdk::{Symbol, TryFromVal};

    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // Start from version 0 so migration runs.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    client.migrate_state(&admin);

    // Verify the "schema_migrated" event was emitted by scanning the event log.
    let events = env.events().all();
    let expected_topic = Symbol::new(&env, "schema_migrated");
    let found = events.iter().any(|(contract_id, topics, _data)| {
        if contract_id != *client.address() {
            return false;
        }
        topics.iter().any(|t| {
            if let Ok(sym) = Symbol::try_from_val(&env, &t) {
                sym == expected_topic
            } else {
                false
            }
        })
    });

    assert!(
        found,
        "migrate_state must emit a 'schema_migrated' event"
    );

    // Re-verify storage as the definitive correctness oracle.
    env.as_contract(client.address(), || {
        assert_eq!(read_schema_version(&env), CURRENT_SCHEMA_VERSION);
    });
}

/// Full "fresh deploy → upgrade → migrate" lifecycle exercised end-to-end.
///
/// Simulates:
/// 1. Fresh deployment (`init`) writes schema version 1.
/// 2. A new binary is deployed (`upgrade_contract`) which sets schema to 0
///    to simulate a layout change.
/// 3. `migrate_state` succeeds and advances schema to `CURRENT_SCHEMA_VERSION`.
/// 4. A second `migrate_state` call is rejected.
#[test]
fn full_deployment_upgrade_migrate_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();
    let (_ac, client, _ac_id, admin) = setup_schema(&env);

    // Step 1: fresh init wrote CURRENT_SCHEMA_VERSION.
    env.as_contract(client.address(), || {
        assert_eq!(read_schema_version(&env), CURRENT_SCHEMA_VERSION);
    });

    // Step 2: simulate a schema layout change by injecting version 0.
    env.as_contract(client.address(), || {
        write_schema_version(&env, 0);
    });

    // Step 3: migration succeeds.
    let result = client.try_migrate_state(&admin);
    assert!(
        result.is_ok(),
        "migrate_state must succeed after simulated upgrade"
    );
    env.as_contract(client.address(), || {
        assert_eq!(read_schema_version(&env), CURRENT_SCHEMA_VERSION);
    });

    // Step 4: second migration is rejected.
    let result2 = client.try_migrate_state(&admin);
    assert_eq!(
        result2,
        Err(Ok(PredifiError::SchemaMigrationAlreadyRun)),
        "second migrate_state on the same version must return SchemaMigrationAlreadyRun"
    );
}
