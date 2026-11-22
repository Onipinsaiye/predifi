//! Bootstrap depth limits for new pools (issue #275).
//!
//! Covers both the pure validation rule (identical on native and optimised
//! WASM builds) and the typed errors surfaced by every `create_pool_*` entry
//! point.

use predifi_contract::{
    validate_bootstrap_depth, BootstrapError, PoolConfig, PredifiContract, PredifiContractClient,
    PredifiError, MAX_POOL_BOOTSTRAP_DEPTH, MIN_POOL_BOOTSTRAP_DEPTH,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token, vec, Address, Env, String,
};

mod dummy_ac {
    use soroban_sdk::{contract, contractimpl, Address, Env};

    #[contract]
    pub struct DummyAC;

    #[contractimpl]
    impl DummyAC {
        pub fn grant_role(_env: Env, _user: Address, _role: u32) {}
        pub fn revoke_role(_env: Env, _user: Address, _role: u32) {}
        pub fn has_role(_env: Env, _user: Address, _role: u32) -> bool {
            true
        }
        pub fn get_operator_count(_env: Env) -> u32 {
            1
        }
    }
}

fn pool_config(env: &Env, initial_liquidity: i128) -> PoolConfig {
    PoolConfig {
        start_time: 0,
        description: String::from_str(env, "Bootstrap limit test pool"),
        metadata_url: String::from_str(env, "ipfs://bootstrap-limits"),
        min_stake: 1,
        max_stake: 0,
        max_total_stake: 0,
        min_total_stake: 1,
        initial_liquidity,
        required_resolutions: 1,
        private: false,
        whitelist_key: None,
        outcome_descriptions: vec![
            env,
            String::from_str(env, "No"),
            String::from_str(env, "Yes"),
        ],
    }
}

struct Harness<'a> {
    client: PredifiContractClient<'a>,
    token: Address,
    creator: Address,
}

fn harness(env: &Env) -> Harness<'_> {
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let ac = env.register(dummy_ac::DummyAC, ());
    let contract = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(env, &contract);
    let admin = Address::generate(env);
    let creator = Address::generate(env);
    let treasury = Address::generate(env);
    let token_admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token = sac.address();
    token::StellarAssetClient::new(env, &token).mint(&creator, &MAX_POOL_BOOTSTRAP_DEPTH);

    client.init(&ac, &treasury, &0u32, &0u64, &3600u64, &0u32);
    client.add_token_to_whitelist(&admin, &token);

    Harness {
        client,
        token,
        creator,
    }
}

fn create(env: &Env, h: &Harness<'_>, liquidity: i128) -> bool {
    h.client
        .try_create_pool(
            &h.creator,
            &(env.ledger().timestamp() + 3_600),
            &h.token,
            &2u32,
            &symbol_short!("Tech"),
            &pool_config(env, liquidity),
        )
        .is_ok()
}

// ── Pure rule: native + WASM share these exact boundaries ───────────────────

#[test]
fn smallest_accepted_depth_and_zero_are_valid() {
    assert_eq!(validate_bootstrap_depth(0), Ok(()));
    assert_eq!(
        validate_bootstrap_depth(MIN_POOL_BOOTSTRAP_DEPTH),
        Ok(())
    );
    assert_eq!(validate_bootstrap_depth(MAX_POOL_BOOTSTRAP_DEPTH), Ok(()));
}

#[test]
fn just_below_boundary_is_rejected_with_stable_error() {
    // Zero is a distinct "no bootstrap" state; the smallest invalid depth is
    // one base unit below the minimum, i.e. negative.
    assert_eq!(validate_bootstrap_depth(0), Ok(()));
    assert_eq!(
        validate_bootstrap_depth(MIN_POOL_BOOTSTRAP_DEPTH - 2),
        Err(BootstrapError::DepthTooLow)
    );
    assert_eq!(
        validate_bootstrap_depth(-1),
        Err(BootstrapError::DepthTooLow)
    );
}

#[test]
fn just_above_maximum_is_rejected_with_stable_error() {
    assert_eq!(
        validate_bootstrap_depth(MAX_POOL_BOOTSTRAP_DEPTH + 1),
        Err(BootstrapError::DepthTooHigh)
    );
}

#[test]
fn bounds_are_ordered_at_compile_time() {
    const _: () = assert!(MIN_POOL_BOOTSTRAP_DEPTH >= 0);
    const _: () = assert!(MIN_POOL_BOOTSTRAP_DEPTH <= MAX_POOL_BOOTSTRAP_DEPTH);
}

// ── Contract entry point: typed errors ──────────────────────────────────────

#[test]
fn create_pool_accepts_boundary_values() {
    for liquidity in [0, MIN_POOL_BOOTSTRAP_DEPTH, MAX_POOL_BOOTSTRAP_DEPTH] {
        let env = Env::default();
        let h = harness(&env);
        assert!(create(&env, &h, liquidity), "depth {liquidity} must be accepted");
    }
}

#[test]
fn create_pool_rejects_negative_depth_with_typed_error() {
    let env = Env::default();
    let h = harness(&env);
    let result = h.client.try_create_pool(
        &h.creator,
        &(env.ledger().timestamp() + 3_600),
        &h.token,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config(&env, -1),
    );
    assert_eq!(result, Err(Ok(PredifiError::BootstrapDepthTooLow)));
}

#[test]
fn create_pool_rejects_excessive_depth_with_typed_error() {
    let env = Env::default();
    let h = harness(&env);
    let result = h.client.try_create_pool(
        &h.creator,
        &(env.ledger().timestamp() + 3_600),
        &h.token,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config(&env, MAX_POOL_BOOTSTRAP_DEPTH + 1),
    );
    assert_eq!(result, Err(Ok(PredifiError::BootstrapDepthTooHigh)));
}
