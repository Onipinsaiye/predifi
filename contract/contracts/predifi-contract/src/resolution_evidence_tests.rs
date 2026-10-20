//! Tests for the verifiable resolution evidence commitment recorded by
//! `resolve_pool_from_price` (issue #24).
//!
//! Acceptance criteria coverage:
//! - **Stable schema/preimage** — the stored `ResolutionEvidence` record
//!   carries feed, value, feed timestamp and the full comparison rule; the
//!   `resolution_evidence` event commits to it via a SHA-256 hash of the
//!   record's XDR encoding (golden-vector test below).
//! - **Atomic emission** — `resolution_evidence` and `pool_resolved` land in
//!   the same event batch of the resolution call.
//! - **Reconstruction** — the stored evidence re-derives `pool.outcome`
//!   off-chain using the documented bounded-match semantics.
//! - **Immutability** — the record is unchanged after resolution: a second
//!   resolution attempt fails and later feed updates cannot overwrite it.
//! - **Scope** — vote-based resolution (`resolve_pool` / `oracle_resolve`)
//!   carries no feed observation, so no evidence record exists there.

#![cfg(test)]

use crate::test::{setup, ROLE_ADMIN};
use crate::{DataKey, MarketState, PoolConfig, PredifiContractClient, ResolutionEvidence};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    xdr::ToXdr,
    Address, BytesN, Env, IntoVal, String, Symbol, TryFromVal,
};

// ─── Golden-vector constants ─────────────────────────────────────────────────

const FEED_PAIR: &str = "ETHUSD";
const TARGET_PRICE: i128 = 1_000;
const COMPARISON_OP: u32 = 1; // Greater
const TOLERANCE_BPS: u32 = 100;
const FEED_PRICE: i128 = 1_050; // > 1_000 + 10 tolerance ⇒ condition met ⇒ outcome 1
const FEED_TIMESTAMP: u64 = 999;
const FEED_EXPIRES_AT: u64 = 100_000;
const POOL_END_TIME: u64 = 5_000;
const EXPECTED_OUTCOME: u32 = 1;

/// SHA-256 over the XDR encoding of the golden `ResolutionEvidence` below.
/// Pinned to catch accidental schema or encoding drift — a change here means
/// the committed preimage changed and auditors' verifications must be re-pinned.
const GOLDEN_EVIDENCE_HASH: [u8; 32] = [
    185, 165, 227, 54, 112, 68, 54, 113, 191, 248, 100, 153, 243, 106, 232, 133, 124, 18, 92, 114,
    229, 138, 197, 182, 164, 59, 217, 202, 20, 133, 157, 236,
];

struct Ctx<'a> {
    client: PredifiContractClient<'a>,
    pool_id: u64,
    admin: Address,
    oracle: Address,
    feed_pair: Symbol,
}

/// Deploys the contract (resolution_delay = 0) with a 2-option pool and wires
/// a price condition + feed observation from the given vector values.
fn setup_price_pool<'a>(
    env: &'a Env,
    feed_pair_name: &str,
    target_price: i128,
    comparison_op: u32,
    tolerance_bps: u32,
    price: i128,
    feed_timestamp: u64,
) -> Ctx<'a> {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (ac_client, client, token_address, _, _, _, operator, creator) = setup(env);
    let admin = Address::generate(env);
    let oracle = Address::generate(env);
    ac_client.grant_role(&admin, &ROLE_ADMIN);
    client.add_oracle(&admin, &oracle);
    client.init_oracle(&admin, &Address::generate(env), &10_000u64, &100u32);

    let pool_id = client.create_pool(
        &creator,
        &POOL_END_TIME,
        &token_address,
        &2u32,
        &symbol_short!("Finance"),
        &PoolConfig {
            start_time: 1_000,
            description: String::from_str(env, "Evidence"),
            metadata_url: String::from_str(env, "ipfs://evidence"),
            min_stake: 1i128,
            max_stake: 0i128,
            max_total_stake: 0,
            min_total_stake: 1,
            initial_liquidity: 0i128,
            required_resolutions: 1u32,
            private: false,
            whitelist_key: None,
            outcome_descriptions: soroban_sdk::vec![
                env,
                String::from_str(env, "No"),
                String::from_str(env, "Yes"),
            ],
        },
    );

    let feed_pair = Symbol::new(env, feed_pair_name);
    client.set_price_condition(
        &operator,
        &pool_id,
        &feed_pair,
        &target_price,
        &comparison_op,
        &tolerance_bps,
    );
    client.update_price_feed(
        &oracle,
        &feed_pair,
        &price,
        &1i128,
        &feed_timestamp,
        &FEED_EXPIRES_AT,
    );

    Ctx {
        client,
        pool_id,
        admin,
        oracle,
        feed_pair,
    }
}

fn golden_ctx(env: &Env) -> Ctx<'_> {
    setup_price_pool(
        env,
        FEED_PAIR,
        TARGET_PRICE,
        COMPARISON_OP,
        TOLERANCE_BPS,
        FEED_PRICE,
        FEED_TIMESTAMP,
    )
}

fn expected_evidence(env: &Env) -> ResolutionEvidence {
    ResolutionEvidence {
        feed_pair: Symbol::new(env, FEED_PAIR),
        price: FEED_PRICE,
        feed_timestamp: FEED_TIMESTAMP,
        comparison_op: COMPARISON_OP,
        target_price: TARGET_PRICE,
        tolerance_bps: TOLERANCE_BPS,
        outcome: EXPECTED_OUTCOME,
    }
}

fn find_event(
    env: &Env,
    events: &soroban_sdk::Vec<(
        Address,
        soroban_sdk::Vec<soroban_sdk::Val>,
        soroban_sdk::Val,
    )>,
    topic: &str,
) -> Option<soroban_sdk::Map<Symbol, soroban_sdk::Val>> {
    let wanted = Symbol::new(env, topic);
    events.iter().find_map(|e| {
        let topic_val = e.1.get(0)?;
        let topic_sym = Symbol::try_from_val(env, &topic_val).ok()?;
        (topic_sym == wanted).then(|| e.2.clone().into_val(env))
    })
}

// ─── Golden vector ───────────────────────────────────────────────────────────

#[test]
fn test_resolution_evidence_golden_vector() {
    let env = Env::default();
    let ctx = golden_ctx(&env);

    env.ledger().with_mut(|li| li.timestamp = POOL_END_TIME);
    ctx.client.resolve_pool_from_price(&ctx.pool_id);

    // `events().all()` reflects the most recent contract invocation — capture
    // the resolution call's batch before any further client calls.
    let events = env.events().all();

    // The stored record matches the fixed schema exactly.
    let evidence = ctx
        .client
        .get_resolution_evidence(&ctx.pool_id)
        .expect("evidence must be persisted");
    assert_eq!(evidence, expected_evidence(&env));

    // The committed hash pins the XDR encoding of the record.
    let preimage = evidence.to_xdr(&env);
    let hash: BytesN<32> = env.crypto().sha256(&preimage).to_bytes();
    assert_eq!(hash, BytesN::from_array(&env, &GOLDEN_EVIDENCE_HASH));

    // The event carried the same commitment and preimage fields.
    let data = find_event(&env, &events, "resolution_evidence").expect("evidence event emitted");
    let get_i128 = |k: &str| -> i128 { data.get(Symbol::new(&env, k)).unwrap().into_val(&env) };
    let get_u64 = |k: &str| -> u64 { data.get(Symbol::new(&env, k)).unwrap().into_val(&env) };
    let get_u32 = |k: &str| -> u32 { data.get(Symbol::new(&env, k)).unwrap().into_val(&env) };
    assert_eq!(get_u64("pool_id"), ctx.pool_id);
    let feed_pair_val: Symbol = data
        .get(Symbol::new(&env, "feed_pair"))
        .unwrap()
        .into_val(&env);
    assert_eq!(feed_pair_val, Symbol::new(&env, FEED_PAIR));
    assert_eq!(get_i128("price"), FEED_PRICE);
    assert_eq!(get_u64("feed_timestamp"), FEED_TIMESTAMP);
    assert_eq!(get_u32("comparison_op"), COMPARISON_OP);
    assert_eq!(get_i128("target_price"), TARGET_PRICE);
    assert_eq!(get_u32("tolerance_bps"), TOLERANCE_BPS);
    assert_eq!(get_u32("outcome"), EXPECTED_OUTCOME);
    let emitted_hash: BytesN<32> = data
        .get(Symbol::new(&env, "evidence_hash"))
        .unwrap()
        .into_val(&env);
    assert_eq!(emitted_hash, hash);
}

// ─── Atomic emission with the resolution event ───────────────────────────────

#[test]
fn test_resolution_evidence_emitted_atomically_with_pool_resolved() {
    let env = Env::default();
    let ctx = golden_ctx(&env);

    env.ledger().with_mut(|li| li.timestamp = POOL_END_TIME);
    ctx.client.resolve_pool_from_price(&ctx.pool_id);
    // `events().all()` reflects the most recent contract invocation, so the
    // batch captured here is exactly the resolution call's emissions.
    let after = env.events().all();

    let evidence = find_event(&env, &after, "resolution_evidence");
    let resolved = find_event(&env, &after, "pool_resolved");
    assert!(
        evidence.is_some() && resolved.is_some(),
        "resolution must emit pool_resolved and resolution_evidence in the same call"
    );
}

// ─── Evidence reconstructs the decision ─────────────────────────────────────

/// Re-derives the outcome from a stored evidence record using the same
/// bounded-match semantics as `price_resolution_outcome`:
/// operators 0=Equal, 1=Greater, 2=Less; outcome 1 = condition met else 0.
fn derive_outcome(e: &ResolutionEvidence) -> u32 {
    let tolerance = e.target_price * (e.tolerance_bps as i128) / 10_000i128;
    let (lower, upper) = (e.target_price - tolerance, e.target_price + tolerance);
    let met = match e.comparison_op {
        0 => e.price >= lower && e.price <= upper,
        1 => e.price > upper,
        2 => e.price < lower,
        _ => panic!("unsupported comparison_op in evidence"),
    };
    if met {
        1
    } else {
        0
    }
}

#[test]
fn test_resolution_evidence_reconstructs_decision() {
    for (price, op, expected) in [
        (1_050i128, 1u32, 1u32), // Greater, above band -> met
        (1_005i128, 0u32, 1u32), // Equal, inside band -> met
        (985i128, 2u32, 1u32),   // Less, below band -> met
        (1_005i128, 1u32, 0u32), // Greater, inside band -> not met
        (1_100i128, 0u32, 0u32), // Equal, outside band -> not met
    ] {
        let env = Env::default();
        let ctx = setup_price_pool(
            &env,
            "VEC1",
            TARGET_PRICE,
            op,
            TOLERANCE_BPS,
            price,
            FEED_TIMESTAMP,
        );

        env.ledger().with_mut(|li| li.timestamp = POOL_END_TIME + 1);
        ctx.client.resolve_pool_from_price(&ctx.pool_id);

        let evidence = ctx.client.get_resolution_evidence(&ctx.pool_id).unwrap();
        let pool = ctx.client.get_pool(&ctx.pool_id);
        assert_eq!(evidence.outcome, expected, "stored outcome mismatch");
        assert_eq!(pool.outcome, expected, "pool outcome mismatch");
        assert_eq!(
            derive_outcome(&evidence),
            pool.outcome,
            "stored evidence must re-derive the recorded outcome"
        );
    }
}

// ─── Immutability after resolution ──────────────────────────────────────────

#[test]
fn test_resolution_evidence_unchanged_after_resolution() {
    let env = Env::default();
    let ctx = golden_ctx(&env);

    env.ledger().with_mut(|li| li.timestamp = POOL_END_TIME);
    ctx.client.resolve_pool_from_price(&ctx.pool_id);
    let committed = ctx.client.get_resolution_evidence(&ctx.pool_id).unwrap();

    // A second resolution attempt fails and cannot rewrite the record.
    assert!(ctx
        .client
        .try_resolve_pool_from_price(&ctx.pool_id)
        .is_err());
    assert_eq!(
        ctx.client.get_resolution_evidence(&ctx.pool_id),
        Some(committed.clone())
    );

    // A later feed update does not touch the committed evidence.
    // (`update_price_feed` requires feed_timestamp < ledger timestamp and a
    // bounded deviation from the previous price.)
    env.ledger()
        .with_mut(|li| li.timestamp = POOL_END_TIME + 100);
    let new_oracle = Address::generate(&env);
    ctx.client.add_oracle(&ctx.admin, &new_oracle);
    ctx.client.update_price_feed(
        &new_oracle,
        &ctx.feed_pair,
        &1_060i128,
        &1i128,
        &(POOL_END_TIME + 50),
        &FEED_EXPIRES_AT,
    );
    assert_eq!(
        ctx.client.get_resolution_evidence(&ctx.pool_id),
        Some(committed)
    );
}

// ─── Absence where no feed observation exists ────────────────────────────────

#[test]
fn test_resolution_evidence_absent_for_unresolved_and_vote_paths() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    let (_ac, client, token_address, _, _, _, operator, creator) = setup(&env);

    // Unresolved pool: no evidence.
    let pool_id = client.create_pool(
        &creator,
        &POOL_END_TIME,
        &token_address,
        &2u32,
        &symbol_short!("Finance"),
        &PoolConfig {
            start_time: 1_000,
            description: String::from_str(&env, "Vote"),
            metadata_url: String::from_str(&env, "ipfs://vote"),
            min_stake: 1i128,
            max_stake: 0i128,
            max_total_stake: 0,
            min_total_stake: 1,
            initial_liquidity: 0i128,
            required_resolutions: 1u32,
            private: false,
            whitelist_key: None,
            outcome_descriptions: soroban_sdk::vec![
                &env,
                String::from_str(&env, "No"),
                String::from_str(&env, "Yes"),
            ],
        },
    );
    assert_eq!(client.get_resolution_evidence(&pool_id), None);

    // Vote-path resolution records no feed evidence.
    env.ledger().with_mut(|li| li.timestamp = POOL_END_TIME + 1);
    client.resolve_pool(&operator, &pool_id, &1u32);
    assert_eq!(client.get_pool(&pool_id).state, MarketState::Resolved);
    assert_eq!(client.get_resolution_evidence(&pool_id), None);

    // The pool itself carries no dangling evidence key.
    assert!(!env.as_contract(&client.address, || {
        env.storage()
            .persistent()
            .has(&DataKey::ResolutionEvidence(pool_id))
    }));
}
