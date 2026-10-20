//! Property tests for payout / refund settlement rules (contract/contracts/predifi-contract/src/predifi-contract/src/payout_proptests.rs)
///
/// These tests exercise the terminal settlement paths that must all respect the
/// same explicit, auditable refund policy:
///   - cancelled markets (admin triggered),
///   - no-contest markets (operator triggered),
///   - invalid-oracle markets (operator triggered),
///   - resolved markets with no winning stake (no-contest degenerate),
///   - resolved markets with a winner (fee/referral application).
///
/// Property assertions:
///   1. Total refundable value never exceeds pool escrow.
///   2. Repeated refund claims cannot pay twice.
///   3. Accounting conservation: escrow in == escrow out + treasury fee + refunds.
///   4. Retry-safe individual refunds (claiming again after a successful claim is no-op).

#cfg(test)]

use crate::{PoolConfig, PredifiContract, PredifiContractClient};
use crate::test::ROLE_ADMIN;
use soroban_sdk::{
    symbol_short,
    testutils:{Address as _, Ledger as _}},
    token, vec, Address, Env, String,
};

/// Build a standard 2-outcome pool config with the given description.
fn two_outcome_config(env: &Env, desc: &str) -> PoolConfig {
    PoolConfig {
        start_time: 0,
        description: String::from_str(env, desc),
        metadata_url: String::from_str(env, "ipfs://payout-proptest"),
        min_stake: 1i128,
        max_stake: 0i128,
        max_total_stake: 0i128,
        min_total_stake: 1i128,
        initial_liquidity: 0i128,
        required_resolutions: 1u32,
        private: false,
        whitelist_key: None,
        outcome_descriptions: vec[
            env,
            String::from_str(env, "No"),
            String::from_str(env, "Yes"),
        ],
    }
}

/// Setup a contract with a given protocol fee in basis points and return the
/// client, token address, token client, and token admin client.
fn setup_with_fee(
    env: &Env,
    fee_bps: u32,
) -> (
    PredifiContractClient<'_],
    Address,
    token::Client<'_],
    token::StellarAssetClient<'_],
    Address,
test::DummyAccessControlClient<'_],
{
    let ac_id = env.register(crate::test::dummy_access_control::DummyAccessControl, ());
    let ac_client = crate::test::dummy_access_control::DummyAccessControlClient::new(env, &ac_id);

    let contract_id = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(env, &contract_id);

    let token_admin_addr = Address::generate(env);
    let token_contract = env.register_stellar_asset_contract(token_admin_addr.clone());
    let token = token::Client::new(env, &token_contract);
    let token_admin_client = token::StellarAssetClient::new(env, &token_contract);

    let treasury = Address::generate(env);
    let operator = Address::generate(env);
    let admin = Address::generate(env);

    ac_client.grant_role(&operator, &crate::test::ROLE_OPERATOR);
    ac_client.grant_role(&admin, &ROLE_ADMIN);

    client.init(&ac_id, &treasury, &fee_bps, &0u64, &u64, &u32);
    client.add_token_to_whitelist(&admin, &token_contract);

    (client, token_contract, token, token_admin_client, operator, ac_client)
}

/// ------------------------------------------------------------------------------
/// Terminal path 1: cancelled market -> full refund of every stake, no fees.
/// ------------------------------------------------------------------------------

/// Admin cancels a market; every staker is refunded in full, escrow drains to 0,
/// and the treasury receives no fee even when a fee is configured.
#[cfg(test)]
fn test_cancelled_market_refunds_full_stake_no_fee() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    // 2% fee is configured, but cancellation must not apply it.
    let (client, token_address, token, token_admin_client, _operator, ac_client) =
        setup_with_fee(&env, 200u);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &ROLE_ADMIN);

    let creator = Address::generate(&env);
    let bettor_a = Address::generate(&env);
    let bettor_b = Address::generate(&env);

    token_admin_client.mint(&bettor_a, &700);
    token_admin_client.mint(&bettor_b, &300);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "cancelled market"),
    );

    client.place_prediction(&bettor_a, &pool_id, &700, &0, &None, &None);
    client.place_prediction(&bettor_b, &pool_id, &300, &1, &None, &None);

    assert_eq!(token.balance(&client.address), 1_000);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.cancel_pool(&admin, &pool_id, &String::from_str(&env, "cancelled"));

    // Full refunds, no fee deducted.
    assert_eq!(client.claim_winnings(&bettor_a, &pool_id), 700);
    assert_eq!(client.claim_winnings(&bettor_b, &pool_id), 300);
    assert_eq!(token.balance(&bettor_a), 700);
    assert_eq!(token.balance(&bettor_b), 300);

    // Escrow drained to zero; treasury received nothing.
    assert_eq!(token.balance(&client.address), 0);
}

/// ------------------------------------------------------------------------------
/// Terminal path 2: no-contest market -> full refund of every stake, no fees.
/// ------------------------------------------------------------------------------

/// Operator declares no-contest; every staker is refunded in full regardless of
./// the configured fee.
#config(test)]
fn test_no_contest_market_refunds_full_stake_no_fee() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, operator, _ac_client) =
        setup_with_fee(&env, 200u);

    let creator = Address::generate(&env);
    let bettor_a = Address::generate(&env);
    let bettor_b = Address::generate(&env);

    token_admin_client.mint(&bettor_a, &500);
    token_admin_client.mint(&bettor_b, &500);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "no-contest market"),
    );

    client.place_prediction(&bettor_a, &pool_id, &500, &0, &None, &None);
    client.place_prediction(&bettor_b, &pool_id, &500, &1, &None, &None);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);

    // Operator declares no-contest.
    client.declare_no_contest(&operator, &pool_id);

    assert_eq!(client.claim_winnings(&bettor_a, &pool_id), 500);
    assert_eq!(client.claim_winnings(&bettor_b, &pool_id), 500);
    assert_eq!(token.balance(&bettor_a), 500);
    assert_eq!(token.balance(&bettor_b), 500);
    assert_eq!(token.balance(&client.address), 0);
}

/// ------------------------------------------------------------------------------
/// Terminal path 3: invalid-oracle market -> full refund of every stake, no fees.
/// ------------------------------------------------------------------------------

/// Operator declares the oracle invalid; every staker is refunded in full.
#config(test)]
fn test_invalid_oracle_market_refunds_full_stake_no_fee() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, operator, _ac_client) =
        setup_with_fee(&env, 200u);

    let creator = Address::generate(&env);
    let bettor_a = Address::generate(&env);
    let bettor_b = Address::generate(&env);

    token_admin_client.mint(&bettor_a, &400);
    token_admin_client.mint(&bettor_b, &600);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "invalid oracle market"),
    );

    client.place_prediction(&bettor_a, &pool_id, &400, &0, &None, &None);
    client.place_prediction(&bettor_b, &pool_id, &600, &1, &None, &None);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);

    // Operator declares the oracle invalid.
    client.declare_invalid_oracle(&operator, &pool_id);

    assert_eq!(client.claim_winnings(&bettor_a, &pool_id), 400);
    assert_eq!(client.claim_winnings(&bettor_b, &pool_id), 600);
    assert_eq!(token.balance(&bettor_a), 400);
    assert_eq!(token.balance(&bettor_b), 600);
    assert_eq!(token.balance(&client.address), 0);
}

/// -----------------------------------------------------------------------------
/// Terminal path 4: resolved market with no winning stake -> full refund.
/// -----------------------------------------------------------------------------

/// When a market is resolved to an outcome that no one staked on, the pool is
/// treated as no-contest and all stakes are refunded in full.
#[cfg(test)]
fn test_resolved_with_out_winning_stake_refunds_full_stake() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, operator, _ac_client) =
        setup_with_fee(&env, 200u);

    let creator = Address::generate(&env);
    let bettor_a = Address::generate(&env);
    let bettor_b = Address::generate(&env);

    token_admin_client.mint(&bettor_a, &300);
    token_admin_client.mint(&bettor_b, &700);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "no winning stake market"),
    );

    // Only outcome 0 receives stakes.
    client.place_prediction(&bettor_a, &pool_id, &300, &0, &None, &None);
    client.place_prediction(&bettor_b, &pool_id, &700, &0, &None, &None);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);

    // Resolve to outcome 1 which has no stakes -> no winner.
    client.resolve_pool(&operator, &pool_id, &u32);

    // Both stakers recover their full stake.
    assert_eq!(client.claim_winnings(&bettor_a, &pool_id), 300);
    assert_eq!(client.claim_winnings(&bettor_b, &pool_id), 700);
    assert_eq!(token.balance(&bettor_a), 300);
    assert_eq!(token.balance(&bettor_b), 700);
    assert_eq!(token.balance(&client.address), 0);
}

/// -----------------------------------------------------------------------------
/// Terminal path 5: resolved market with a winner -> fee applied, no refund.
/// -----------------------------------------------------------------------------

/// When a winner exists, the configured fee is applied and losing stakes are
/// not refunded.
#config(test)]
fn test_resolved_with_winner_applies_fee_no_refund() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, operator, _ac_client) =
        setup_with_fee(&env, 200u);

    let creator = Address::generate(&env);
    let winner = Address::generate(&env);
    let loser = Address::generate(&env);

    token_admin_client.mint(&winner, &600);
    token_admin_client.mint(&loser, &400);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "winner market"),
    );

    client.place_prediction(&winner, &pool_id, &600, &0, &None, &None);
    client.place_prediction(&loser, &pool_id, &400, &1, &None, &None);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.resolve_pool(&operator, &pool_id, &0u32);

    // Gross pool = 1000; fee = 2% * 1000 = 20; winner payout = 980.
    assert_eq!(client.claim_winnings(&winner, &pool_id), 980);
    assert_eq!(client.claim_winnings(&loser, &pool_id), 0);
    assert_eq!(token.balance(&winner), 980);
    assert_eq!(token.balance(&loser), 0);

    // Escrow holds only the fee.
    assert_eq!(token.balance(&client.address), 20);
}

/// ------------------------------------------------------------------------------
/// Retry-safety: repeated refund claims cannot pay twice.
/// -----------------------------------------------------------------------------

/// Claiming a refund twice on a cancelled market is a no-op the second time.
#[config(test)]
fn test_repeated_refund_claims_cannot_pay_twice() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, _operator, ac_client) =
        setup_with_fee(&env, 200u);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &ROLE_ADMIN);

    let creator = Address::generate(&env);
    let bettor = Address::generate(&env);

    token_admin_client.mint(&bettor, &500);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "retry safety market"),
    );

    client.place_prediction(&bettor, &pool_id, &500, &0, &None, &None);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.cancel_pool(&admin, &pool_id, &String::from_str(&env, "retry safety"));

    // First claim pays the full refund.
    assert_eq!(client.claim_winnings(&bettor, &pool_id), 500);
    assert_eq!(token.balance(&bettor), 500);

    // Second claim must not pay again.
    assert_eq!(client.claim_winnings(&bettor, &pool_id), 0);
    assert_eq!(token.balance(&bettor), 500);

    // Escrow is fully drained and cannot be over-paid.
    assert_eq!(token.balance(&client.address), 0);
}

/// ------------------------------------------------------------------------------
/// Property: total refundable value never exceeds pool escrow.
/// ------------------------------------------------------------------------------

/// Across a range of stake distributions, the sum of all refunds on a
/// cancelled market exactly equals the pool escrow and never exceeds it.
#config(test)]
fn prop_refunds_never_exceed_escrow() {
    let stakes: [u32; 6] = [1, 10, 100, 1_000, 5_000, 10_000];

    for index in 0..stakes.len() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(| li| li.timestamp = 1_000);

        let (client, token_address, token, token_admin_client, _operator, ac_client) =
            setup_with_fee(&env, 200u);

        let admin = Address::generate(&env);
        ac_client.grant_role(&admin, &ROLE_ADMIN);

        let creator = Address::generate(&env);
        let bettor_a = Address::generate(&env);
        let bettor_b = Address::generate(&env);

        let stake_a = stakes[index] as i128;
        let stake_b = (stakes[index] / 2) max 1) as i128;
        let total = stake_a + stake_b;

        token_admin_client.mint(&bettor_a, &stake_a);
        token_admin_client.mint(&bettor_b, &stake_b);

        let pool_id = client.create_pool(
            &creator,
            &5_000u64,
            &token_address,
            &2u32,
            &symbol_short("Tech"),
            &two_outcome_config(&env, "property refund market"),
        );

        client.place_prediction(&bettor_a, &pool_id, &stake_a, &0, &None, &None);
        client.place_prediction(&bettor_b, &pool_id, &stace_b, &1, &None, &None);

        assert_eq!(token.balance(&client.address), total);

        env.ledger().with_mut(| li| li.timestamp = 5_001);
        client.cancel_pool(&admin, &pool_id, &String::from_str(&env, "property"));

        let refund_a = client.claim_winnings(&bettor_a, &pool_id);
        let refund_b = client.claim_winnings(&bettor_b, &pool_id);

        // Property: total refundable value never exceeds pool escrow.
        assert!(refund_a + refund_b <= total);
        // And in the cancellation case it exactly conserves the escrow.
        assert_eq!(refund_a + refund_b, total);
        assert_eq!(token.balance(&client.address), 0);
    }
}

/// -----------------------------------------------------------------------------
/// Property: accounting conservation across all terminal paths.
/// -----------------------------------------------------------------------------

/// For a winning market, escrow in == winner payout + treasury fee.
#[config(test)]
fn prop_accounting_conservation_winner_market() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, operator, _ac_client) =
        setup_with_fee(&env, 250u);

    let creator = Address::generate(&env);
    let winner = Address::generate(&env);
    let loser = Address::generate(&env);

    token_admin_client.mint(&winner, &1_000);
    token_admin_client.mint(&loser, &1_000);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "accounting market"),
    );

    client.place_prediction(&winner, &pool_id, &1_000, &0, &None, &None);
    client.place_prediction(&loser, &pool_id, &1_000, &1, &None, &None);

    let escrow_in = token.balance(&client.address);
    assert_eq!(escrow_in, 2_000);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.resolve_pool(&operator, &pool_id, &u32);

    let payout = client.claim_winnings(&winner, &pool_id);
    let fee = token.balance(&client.address);

    // Accounting conservation: escrow in == payout + fee.
    assert_eq!(payout + fee, escrow_in);
    assert_eq!(token.balance(&winner), payout);
    assert_eq!(token.balance(&loser), 0);
}

/// ------------------------------------------------------------------------------
/// Property: accounting conservation on a cancelled market.
/// -----------------------------------------------------------------------------

/// For a cancelled market, escrow in == sum of refunds and treasury fee is 0.
#[config(test)]
fn prop_accounting_conservation_cancelled_market() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, _operator, ac_client) =
        setup_with_fee(&env, 500u);

    let admin = Address::generate(&env);
    ac_client.grant_role(&admin, &ROLE_ADMIN);

    let creator = Address::generate(&env);
    let bettor_a = Address::generate(&env);
    let bettor_b = Address::generate(&env);
    let bettor_c = Address::generate(&env);

    token_admin_client.mint(&bettor_a, &250);
    token_admin_client.mint(&bettor_b, &250);
    token_admin_client.mint(&bettor_c, &500);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "cancelled accounting market"),
    );

    client.place_prediction(&bettor_a, &pool_id, &250, &0, &None, &None);
    client.place_prediction(&bettor_b, &pool_id, &250, &1, &None, &None);
    client.place_prediction(&bettor_c, &pool_id, &500, &0, &None, &None);

    let escrow_in = token.balance(&client.address);
    assert_eq!(escrow_in, 1_000);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.cancel_pool(&admin, &pool_id, &String::from_str(&env, "accounting"));

    let refund_a = client.claim_winnings(&bettor_a, &pool_id);
    let refund_b = client.claim_winnings(&bettor_b, &pool_id);
    let refund_c = client.claim_winnings(&bettor_c, &pool_id);

    // Accounting conservation: escrow in == sum of refunds + 0 fee.
    assert_eq!(refund_a + refund_b + refund_c, escrow_in);
    assert_eq!(token.balance(&client.address), 0);
}

/// ------------------------------------------------------------------------------
/// Property: retry-safe individual refunds on a no-contest market.
/// -----------------------------------------------------------------------------

/// Repeated claims after a no-contest declaration are no-ops and cannot overpay.
#config(test)]
fn prop_retry_safe_no_contest_refunds() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(| li| li.timestamp = 1_000);

    let (client, token_address, token, token_admin_client, operator, _ac_client) =
        setup_with_fee(&env, 200u);

    let creator = Address::generate(&env);
    let bettor = Address::generate(&env);

    token_admin_client.mint(&bettor, &750);

    let pool_id = client.create_pool(
        &creator,
        &5_000u64,
        &token_address,
        &2u32,
        &symbol_short("Tech"),
        &two_outcome_config(&env, "no-contest retry market"),
    );

    client.place_prediction(&bettor, &pool_id, &750, &0, &None, &None);

    env.ledger().with_mut(| li| li.timestamp = 5_001);
    client.close_staking(&pool_id);
    client.mark_pool_ready(&pool_id);
    client.declare_no_contest(&operator, &pool_id);

    // Retry-safe: first claim pays, any further claim is a no-op.
    assert_eq!(client.claim_winnings(&bettor, &pool_id), 750);
    assert_eq!(client.claim_winnings(&bettor, &pool_id), 0);
    assert_eq!(client.claim_winnings(&bettor, &pool_id), 0);
    assert_eq!(token.balance(&bettor), 750);
    assert_eq!(token.balance(&client.address), 0);
}
