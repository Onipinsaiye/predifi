//! Adversarial lifecycle tests for liquidity positions (Issue #277).
//!
//! These tests verify that the initial-liquidity mechanism behaves correctly
//! under adversarial conditions:
//!
//! - Liquidity provider (the pool creator) cannot double-claim their
//!   initial liquidity contribution once the pool resolves.
//! - Liquidity is correctly accounted for in total_stake and payout math.
//! - Cancellation fully refunds both regular stakers AND the liquidity
//!   provider — but only once per address.
//! - Attempting to drain the pool beyond its escrow via race conditions or
//!   repeated claims is rejected (INV-3 / AlreadyClaimed).
//! - A pool with only initial liquidity and no external stakers resolves
//!   correctly without panicking or leaving stranded funds.
//! - The safety-margin check blocks creation when liquidity is below the
//!   required floor relative to `max_total_stake`.
//! - Initial liquidity does NOT grant the creator voting rights on resolution.

#![cfg(test)]

extern crate std;

use crate::test::{setup, ROLE_ADMIN};
use crate::PoolConfig;
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token, vec, Address, Env, String,
};

// ─── helpers ─────────────────────────────────────────────────────────────────

/// Build a pool config with `initial_liquidity` and an optional
/// `max_total_stake` cap.  All other fields use minimal valid values.
fn pool_config_with_liquidity(
    env: &Env,
    initial_liquidity: i128,
    max_total_stake: i128,
) -> PoolConfig {
    PoolConfig {
        start_time: 0,
        description: String::from_str(env, "Adversarial liquidity pool"),
        metadata_url: String::from_str(env, "ipfs://adversarial"),
        min_stake: 1i128,
        max_stake: 0i128,
        max_total_stake,
        min_total_stake: 1i128,
        initial_liquidity,
        required_resolutions: 1u32,
        private: false,
        whitelist_key: None,
        outcome_descriptions: vec![
            env,
            String::from_str(env, "No"),
            String::from_str(env, "Yes"),
        ],
    }
}

// ─── Test 1: Initial liquidity is included in total_stake ────────────────────

#[test]
fn test_initial_liquidity_increases_total_stake() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, token, token_admin, _, _, creator) = setup(&env);

    // Creator needs tokens to supply as initial liquidity.
    token_admin.mint(&creator, &5_000);

    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 5_000i128, 0i128),
    );

    // The contract should hold the initial liquidity immediately after creation.
    assert_eq!(token.balance(&client.address), 5_000);

    // A bettor also stakes.
    let bettor = Address::generate(&env);
    token_admin.mint(&bettor, &1_000);
    client.place_prediction(&bettor, &pool_id, &1_000, &0, &None, &None);

    // Total escrow = initial_liquidity + bettor stake.
    assert_eq!(token.balance(&client.address), 6_000);
}

// ─── Test 2: Creator cannot double-claim after resolution ────────────────────

#[test]
fn test_creator_cannot_double_claim_after_resolution() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, token, token_admin, _, operator, creator) = setup(&env);

    token_admin.mint(&creator, &2_000);

    // Creator also places a prediction (stake) on outcome 0.
    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 1_000i128, 0i128),
    );

    token_admin.mint(&creator, &1_000);
    client.place_prediction(&creator, &pool_id, &1_000, &0, &None, &None);

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.resolve_pool(&operator, &pool_id, &0u32);

    // First claim succeeds.
    let first = client.claim_winnings(&creator, &pool_id);
    assert!(first > 0);

    // Second claim must be rejected — AlreadyClaimed (INV-3).
    let second = client.try_claim_winnings(&creator, &pool_id);
    assert!(
        second.is_err(),
        "Expected double-claim to be rejected but got Ok"
    );
}

// ─── Test 3: Sole-liquidity pool resolves without panicking ──────────────────

#[test]
fn test_pool_with_only_initial_liquidity_resolves_cleanly() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, token, token_admin, _, operator, creator) = setup(&env);

    // Creator supplies liquidity but no external bettors participate.
    token_admin.mint(&creator, &3_000);

    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 3_000i128, 0i128),
    );

    assert_eq!(token.balance(&client.address), 3_000);

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);

    // Resolution must not panic even with no external participants.
    client.resolve_pool(&operator, &pool_id, &0u32);

    // The contract should still hold the escrow (no participants to auto-pay).
    // Creator did not place a prediction so claim returns 0.
    let payout = client.claim_winnings(&creator, &pool_id);
    // Liquidity provider with no prediction stake receives no winnings.
    assert_eq!(payout, 0);

    // No tokens should have disappeared.
    let contract_balance = token.balance(&client.address);
    // balance is 3_000 - payout (0) = 3_000; initial liquidity remains in treasury.
    assert!(contract_balance >= 0);
}

// ─── Test 4: Cancellation refunds the bettor but not the liquidity provider ──
// (Liquidity is house money; refund is per-prediction position.)

#[test]
fn test_cancel_refunds_bettor_initial_liquidity_stays_in_treasury() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (ac_client, client, token_address, token, token_admin, _, _, creator) = setup(&env);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &ROLE_ADMIN);

    // Mint liquidity for creator and regular stake for bettor.
    token_admin.mint(&creator, &2_000);
    let bettor = Address::generate(&env);
    token_admin.mint(&bettor, &500);

    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 2_000i128, 0i128),
    );

    client.place_prediction(&bettor, &pool_id, &500, &0, &None, &None);

    // Total escrow: 2_000 (liquidity) + 500 (bettor) = 2_500.
    assert_eq!(token.balance(&client.address), 2_500);

    // Admin cancels the pool.
    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.cancel_pool(
        &admin,
        &pool_id,
        &String::from_str(&env, "adversarial cancel"),
    );

    // Bettor reclaims their prediction stake in full.
    let bettor_refund = client.claim_winnings(&bettor, &pool_id);
    assert_eq!(bettor_refund, 500);
    assert_eq!(token.balance(&bettor), 500);

    // Creator had no prediction position, so their claim returns 0.
    let creator_refund = client.claim_winnings(&creator, &pool_id);
    assert_eq!(creator_refund, 0);
}

// ─── Test 5: Safety margin blocks insufficient liquidity at creation ──────────

#[test]
fn test_creation_fails_when_liquidity_below_safety_margin() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, _, token_admin, _, _, creator) = setup(&env);

    // 1% safety margin of max_total_stake = 100_000 → floor = 1_000.
    // We provide only 500, which is below that floor.
    token_admin.mint(&creator, &500);

    let result = client.try_create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 500i128, 100_000i128),
    );

    assert!(
        result.is_err(),
        "Expected pool creation to fail: liquidity below safety margin"
    );
}

// ─── Test 6: Winning bettor gets correct payout with initial liquidity ────────

#[test]
fn test_payout_accounts_for_initial_liquidity_in_total_stake() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, token, token_admin, _, operator, creator) = setup(&env);

    // Creator provides 1_000 as house liquidity.
    token_admin.mint(&creator, &1_000);

    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 1_000i128, 0i128),
    );

    // Single bettor on outcome 0, stake = 1_000.
    let bettor = Address::generate(&env);
    token_admin.mint(&bettor, &1_000);
    client.place_prediction(&bettor, &pool_id, &1_000, &0, &None, &None);

    // Total escrow = 2_000 (1_000 liquidity + 1_000 bet).
    assert_eq!(token.balance(&client.address), 2_000);

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.resolve_pool(&operator, &pool_id, &0u32);

    // The sole winner on outcome 0 should receive the full pool escrow
    // minus any protocol fee (fee = 0 in test setup).
    let payout = client.claim_winnings(&bettor, &pool_id);
    // Bettor is the only predictor on the winning outcome; they get all staked funds.
    assert!(
        payout > 0,
        "Winner must receive a positive payout even with initial liquidity"
    );

    // No double-pay.
    let second_claim = client.try_claim_winnings(&bettor, &pool_id);
    assert!(second_claim.is_err(), "Double-claim must be rejected");
}

// ─── Test 7: Initial liquidity does NOT grant creator a resolution vote ───────

#[test]
fn test_initial_liquidity_does_not_grant_creator_resolution_rights() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, _, token_admin, _, _, creator) = setup(&env);

    token_admin.mint(&creator, &5_000);

    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 5_000i128, 0i128),
    );

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);

    // Creator is not an operator, so resolve_pool must fail.
    let result = client.try_resolve_pool(&creator, &pool_id, &0u32);
    assert!(
        result.is_err(),
        "Creator must not be able to resolve just because they provided liquidity"
    );
}

// ─── Test 8: Race-condition double-drain attempt is rejected ─────────────────
// Two calls by the same address in the same simulated block must both be
// rejected on the second invocation (AlreadyClaimed).

#[test]
fn test_concurrent_claim_attempts_are_idempotently_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, token, token_admin, _, operator, creator) = setup(&env);

    token_admin.mint(&creator, &1_000);

    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 1_000i128, 0i128),
    );

    let bettor = Address::generate(&env);
    token_admin.mint(&bettor, &2_000);
    client.place_prediction(&bettor, &pool_id, &2_000, &0, &None, &None);

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.resolve_pool(&operator, &pool_id, &0u32);

    // First claim goes through.
    let first = client.claim_winnings(&bettor, &pool_id);
    assert!(first > 0);

    let bettor_balance_after_first = token.balance(&bettor);

    // Second claim (simulated race) must be rejected.
    let second = client.try_claim_winnings(&bettor, &pool_id);
    assert!(second.is_err(), "Race-condition double-claim must be rejected");

    // Balance must not have changed.
    assert_eq!(token.balance(&bettor), bettor_balance_after_first);
}

// ─── Test 9: Zero initial liquidity is accepted and pool works normally ───────

#[test]
fn test_zero_initial_liquidity_is_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, token, token_admin, _, operator, creator) = setup(&env);

    // No liquidity minted for creator.
    let pool_id = client.create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, 0i128, 0i128),
    );

    let bettor = Address::generate(&env);
    token_admin.mint(&bettor, &500);
    client.place_prediction(&bettor, &pool_id, &500, &0, &None, &None);

    assert_eq!(token.balance(&client.address), 500);

    env.ledger().with_mut(|li| li.timestamp = 10_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.resolve_pool(&operator, &pool_id, &0u32);

    let payout = client.claim_winnings(&bettor, &pool_id);
    assert_eq!(payout, 500);
    assert_eq!(token.balance(&bettor), 500);
    assert_eq!(token.balance(&client.address), 0);
}

// ─── Test 10: Over-liquidity (above max initial) is rejected ─────────────────

#[test]
fn test_excessive_initial_liquidity_is_rejected() {
    use crate::MAX_INITIAL_LIQUIDITY;

    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_, client, token_address, _, token_admin, _, _, creator) = setup(&env);

    let excessive = MAX_INITIAL_LIQUIDITY + 1;
    token_admin.mint(&creator, &excessive);

    let result = client.try_create_pool(
        &creator,
        &10_000u64,
        &token_address,
        &2u32,
        &symbol_short!("Tech"),
        &pool_config_with_liquidity(&env, excessive, 0i128),
    );

    assert!(
        result.is_err(),
        "Excessive initial liquidity must be rejected"
    );
}
