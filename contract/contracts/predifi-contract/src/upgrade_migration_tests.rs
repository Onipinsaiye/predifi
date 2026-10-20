//! Fixture-based upgrade-migration tests.
//!
//! These tests differ from the existing `boundary_tests.rs` upgrade tests:
//! they seed *representative deployed state* via the contract's own entry
//! points (pools, predictions, role grants, resolved pools with unclaimed
//! winnings), then exercise the full upgrade flow
//! (`upgrade_contract` + `migrate_state`) and assert that the post-upgrade
//! contract still serves reads and claims correctly.
//!
//! Covered acceptance criteria:
//!  - supported fixture migrates once and preserves balances + permissions
//!  - continued claims work against state written before the upgrade
//!  - unsupported schema versions are rejected safely
//!  - a second migration is idempotent

#![cfg(test)]

extern crate std;

use crate::{DataKey, MarketState, PredifiContract, PredifiContractClient, PoolConfig};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token, Address, BytesN, Env, String,
};

// Mirror the role constants used by `src/test.rs::setup`.
const ROLE_ADMIN: u32 = 0;
const ROLE_OPERATOR: u32 = 1;

/// A minimal access-control shim so the migration tests don't depend on the
/// dummy contract in `src/test.rs`. It just stores role grants in instance
/// storage under (address, role) keys and exposes `has_role` / `grant_role`.
mod ac {
    use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

    #[contract]
    pub struct StubAc;

    #[contractimpl]
    impl StubAc {
        pub fn grant_role(env: Env, user: Address, role: u32) {
            env.storage()
                .instance()
                .set(&(Symbol::new(&env, "role"), user, role), &true);
        }
        pub fn has_role(env: Env, user: Address, role: u32) -> bool {
            env.storage()
                .instance()
                .get(&(Symbol::new(&env, "role"), user, role))
                .unwrap_or(false)
        }
        pub fn get_admin(env: Env) -> Address {
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "admin"))
                .unwrap()
        }
        pub fn set_admin(env: Env, admin: Address) {
            env.storage()
                .instance()
                .set(&Symbol::new(&env, "admin"), &admin);
        }
    }
}

use ac::{StubAc, StubAcClient};

/// Everything the tests need to inspect the seeded "v1" state.
struct Fixture {
    env: Env,
    client: PredifiContractClient<'static>,
    ac: StubAcClient<'static>,
    admin: Address,
    operator: Address,
    user_winner: Address,
    user_loser: Address,
    token: token::Client<'static>,
    token_admin: token::StellarAssetClient<'static>,
    pool_active: u64,
    pool_resolved: u64,
    /// Payout the winner is owed on `pool_resolved`, computed at seed time
    /// from the pre-upgrade storage (i.e. this is what the fixture expects
    /// the new contract to still pay).
    expected_payout: i128,
}

fn make_pool_config(env: &Env, description: &str) -> PoolConfig {
    PoolConfig {
        start_time: 0,
        description: String::from_str(env, description),
        metadata_url: String::from_str(env, "ipfs://seed"),
        min_stake: 1,
        max_stake: 0,
        max_total_stake: 0,
        min_total_stake: 1,
        initial_liquidity: 0,
        required_resolutions: 1,
        private: false,
        whitelist_key: None,
        outcome_descriptions: soroban_sdk::vec![
            env,
            String::from_str(env, "Yes"),
            String::from_str(env, "No"),
        ],
    }
}

fn seed_v1_fixture() -> Fixture {
    let env = Env::default();
    // Uploading the full WASM and performing the upgrade exceeds default
    // mainnet resource limits in the test host. Disable them: this test is
    // about storage continuity, not about proving the upgrade fits in a
    // single mainnet transaction budget.
    env.cost_estimate().budget().reset_unlimited();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    // Access control
    let ac_id = env.register(StubAc, ());
    let ac = StubAcClient::new(&env, &ac_id);

    // Contract
    let contract_id = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(&env, &contract_id);

    // Stellar asset (USDC-like)
    let token_admin_addr = Address::generate(&env);
    let token_id = env.register_stellar_asset_contract(token_admin_addr.clone());
    let token = token::Client::new(&env, &token_id);
    let token_admin = token::StellarAssetClient::new(&env, &token_id);

    // Actors
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let creator = Address::generate(&env);
    let user_winner = Address::generate(&env);
    let user_loser = Address::generate(&env);
    let treasury = Address::generate(&env);

    // Roles + init
    ac.set_admin(&admin);
    ac.grant_role(&admin, &ROLE_ADMIN);
    ac.grant_role(&operator, &ROLE_OPERATOR);

    client.init(&ac_id, &treasury, &0u32, &0u64, &3600u64, &0u32);
    client.add_token_to_whitelist(&admin, &token_id);

    // Fund users
    token_admin.mint(&user_winner, &1_000);
    token_admin.mint(&user_loser, &1_000);

    // Pool #1 — active, still open
    let pool_active = client.create_pool(
        &creator,
        &2_000_000u64,
        &token_id,
        &2u32,
        &symbol_short!("Tech"),
        &make_pool_config(&env, "Seed active pool"),
    );
    client.place_prediction(&user_winner, &pool_active, &50, &0u32, &None, &None);

    // Pool #2 — resolved with unclaimed winnings for user_winner
    let pool_resolved = client.create_pool(
        &creator,
        &1_500_000u64,
        &token_id,
        &2u32,
        &symbol_short!("Tech"),
        &make_pool_config(&env, "Seed resolved pool"),
    );
    client.place_prediction(&user_winner, &pool_resolved, &100, &0u32, &None, &None);
    client.place_prediction(&user_loser,  &pool_resolved, &100, &1u32, &None, &None);

    // Move past end_time + resolution delay, then resolve to outcome 0
    env.ledger().set_timestamp(1_600_000);
    client.resolve_pool(&operator, &pool_resolved, &0u32);

    // The resolved pool holds exactly the two users' stakes: 100 + 100.
    // With fee_bps = 0, the winning user receives the full pot. (Do not use
    // the contract's aggregate balance here — it also includes the active
    // pool's stake, which belongs to a different pool and is not claimable.)
    let expected_payout: i128 = 200;

    Fixture {
        env,
        client,
        ac,
        admin,
        operator,
        user_winner,
        user_loser,
        token,
        token_admin,
        pool_active,
        pool_resolved,
        expected_payout,
    }
}

/// Helper: build a fake "new WASM hash" that the test environment will accept.
/// In `testutils`, `update_current_contract_wasm` just records the hash; it
/// does not require the WASM to actually exist.
fn dummy_wasm_hash(env: &Env) -> BytesN<32> {
    // Upload a real WASM artifact into the test host so `update_current_contract_wasm`
    // has a hash to swap to. In production this is the operator-supplied new
    // contract; in the test we reuse the current build, because the assertions
    // here are about storage continuity across the upgrade boundary, not about
    // a specific code delta.
    //
    // Prerequisite: `cd ~/predifi/contract && cargo build --target wasm32-unknown-unknown --release -p predifi-contract`
    let wasm = include_bytes!(
        "../../../target/wasm32v1-none/release/predifi_contract.wasm"
    );
    let bytes = soroban_sdk::Bytes::from_slice(env, wasm);
    env.deployer().upload_contract_wasm(bytes)
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 1: upgrade preserves balances and permissions
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn fixture_upgrade_preserves_balances_and_permissions() {
    let fx = seed_v1_fixture();

    // Snapshot pre-upgrade state
    let version_before = fx.client.get_version();
    let admin_still_admin = fx.ac.has_role(&fx.admin, &ROLE_ADMIN);
    let op_still_operator = fx.ac.has_role(&fx.operator, &ROLE_OPERATOR);
    let contract_balance_before = fx.token.balance(&fx.client.address);
    let winner_balance_before = fx.token.balance(&fx.user_winner);
    let loser_balance_before = fx.token.balance(&fx.user_loser);

    // ---- Perform upgrade ----
    let new_wasm = dummy_wasm_hash(&fx.env);
    fx.client.upgrade_contract(&fx.admin, &new_wasm);
    fx.client.migrate_state(&fx.admin);

    // ---- Assertions ----
    assert_eq!(
        fx.client.get_version(),
        version_before + 1,
        "upgrade_contract must bump version by exactly 1"
    );
    assert_eq!(
        fx.ac.has_role(&fx.admin, &ROLE_ADMIN),
        admin_still_admin,
        "admin role must survive upgrade"
    );
    assert_eq!(
        fx.ac.has_role(&fx.operator, &ROLE_OPERATOR),
        op_still_operator,
        "operator role must survive upgrade"
    );
    assert_eq!(
        fx.token.balance(&fx.client.address),
        contract_balance_before,
        "contract-held token balance must be unchanged by upgrade"
    );
    assert_eq!(
        fx.token.balance(&fx.user_winner),
        winner_balance_before,
        "user balance must be unchanged by upgrade"
    );
    assert_eq!(
        fx.token.balance(&fx.user_loser),
        loser_balance_before,
        "user balance must be unchanged by upgrade"
    );

    // Read-through of pool state must still work after upgrade
    let active_pool = fx.client.get_pool(&fx.pool_active);
    assert_eq!(active_pool.state, MarketState::Active);

    let resolved_pool = fx.client.get_pool(&fx.pool_resolved);
    assert_eq!(resolved_pool.state, MarketState::Resolved);
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 2: continued claims against pre-upgrade state
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn fixture_continued_claims_after_upgrade() {
    let fx = seed_v1_fixture();

    // Upgrade first
    let new_wasm = dummy_wasm_hash(&fx.env);
    fx.client.upgrade_contract(&fx.admin, &new_wasm);
    fx.client.migrate_state(&fx.admin);

    // Winner's claim must pay exactly what was owed pre-upgrade
    let claimed = fx.client.claim_winnings(&fx.user_winner, &fx.pool_resolved);
    assert_eq!(
        claimed, fx.expected_payout,
        "claim after upgrade must pay the same amount the pre-upgrade state owed"
    );

    // Loser's claim must yield 0 (no winnings)
    let loser_claimed = fx.client.claim_winnings(&fx.user_loser, &fx.pool_resolved);
    assert_eq!(loser_claimed, 0, "loser must not be paid");

    // Double-claim must fail — the Claimed sentinel must survive the upgrade
    let double = fx.client.try_claim_winnings(&fx.user_winner, &fx.pool_resolved);
    assert!(
        double.is_err(),
        "second claim after upgrade must be rejected (Claimed sentinel preserved)"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 3: migrate_state rejects unsupported (future) schema versions
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn fixture_migrate_rejects_unsupported_version() {
    let fx = seed_v1_fixture();

    // Simulate "storage was written by a future contract we don't understand":
    // directly poke DataKey::Version beyond what this WASM supports.
    let contract_id = fx.client.address.clone();
    fx.env.as_contract(&contract_id, || {
        fx.env
            .storage()
            .instance()
            .set(&DataKey::Version, &99u32);
    });

    let result = fx.client.try_migrate_state(&fx.admin);
    assert!(
        result.is_err(),
        "migrate_state must reject unsupported future schema versions"
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Test 4: migration is idempotent (second call succeeds, state unchanged)
// ─────────────────────────────────────────────────────────────────────────────
#[test]
fn fixture_double_migration_is_idempotent() {
    let fx = seed_v1_fixture();

    let new_wasm = dummy_wasm_hash(&fx.env);
    fx.client.upgrade_contract(&fx.admin, &new_wasm);

    let balance_before = fx.token.balance(&fx.client.address);
    let version_after_upgrade = fx.client.get_version();

    // Two migrations in a row must both succeed and not corrupt state.
    fx.client.migrate_state(&fx.admin);
    fx.client.migrate_state(&fx.admin);

    assert_eq!(
        fx.client.get_version(),
        version_after_upgrade,
        "migrate_state must not touch the version counter"
    );
    assert_eq!(
        fx.token.balance(&fx.client.address),
        balance_before,
        "second migration must not move funds"
    );

    // Claims still work after double migration
    let claimed = fx.client.claim_winnings(&fx.user_winner, &fx.pool_resolved);
    assert_eq!(claimed, fx.expected_payout);
}
