//! # Event Schema Golden Tests (issue #32)
//!
//! These tests are the executable counterpart of `docs/EVENT_SCHEMAS.md`. They pin the
//! **public event schema** of the pool lifecycle, prediction, resolution, and payout
//! event families so external indexers can rely on it:
//!
//! * [`documented_topics_are_stable_symbols`] locks every documented `topics[0]` symbol.
//! * [`golden_full_lifecycle_flow`] runs `init → create_pool → place_prediction →
//!   close_staking → mark_pool_ready → resolve_pool → claim_winnings` and compares the
//!   ordered `topic[field:type,...]` snapshot emitted by each step against a golden value.
//! * [`golden_referral_payout_flow`] pins the payout family, including `referral_paid`.
//! * [`golden_cancel_and_refund_flow`] pins cancel/refund events.
//! * [`high_value_prediction_emits_documented_shape`] pins the prediction alert family.
//! * [`staking_closed_event_is_emitted_at_most_once`] pins the idempotency guarantee that
//!   makes `staking_closed` safe to index by `pool_id`.
//!
//! Because these are integration tests they exercise only the public ABI — exactly the
//! surface an external indexer consumes. If a topic name, a payload field name, or a field
//! type drifts away from the reference document, one of these tests fails.

use predifi_contract::{PoolConfig, PredifiContract, PredifiContractClient, EVENT_SCHEMA_VERSION};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token, vec, Address, Env, IntoVal, String, Symbol, TryFromVal, Val, Vec,
};
use std::string::String as StdString;
use std::string::ToString;
use std::vec::Vec as StdVec;

/// A permissive access-control stub: every role check succeeds. The event schema does not
/// depend on role enforcement, only on which events each successful call emits.
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

/// The documented event topics, grouped by family. Kept in sync with
/// `docs/EVENT_SCHEMAS.md` §1 and §6.
const LIFECYCLE_EVENT_TOPICS: &[&str] = &[
    "init",
    "pool_created",
    "initial_liquidity_provided",
    "staking_closed",
    "pool_ready",
    "pool_canceled",
    "pool_disputed",
    "stake_limits_updated",
    "pool_description_updated",
    "max_stake_increased",
    "outcome_stakes_updated",
];

const PREDICTION_EVENT_TOPICS: &[&str] = &[
    "prediction_placed",
    "high_value_prediction",
    "prediction_blocked_delisted",
    "referrer_updated",
];

const RESOLUTION_EVENT_TOPICS: &[&str] = &[
    "resolution_vote_cast",
    "resolution_conflict",
    "pool_resolved",
    "pool_resolved_diag",
    "oracle_resolved",
    "price_feed_updated",
    "price_condition_set",
    "price_resolved",
    "oracle_init",
];

const PAYOUT_EVENT_TOPICS: &[&str] = &[
    "winnings_claimed",
    "reward_claimed",
    "refund_claimed",
    "referral_paid",
    "treasury_withdrawn",
    "emergency_withdraw",
];

// ── Test harness ─────────────────────────────────────────────────────────────

struct Harness<'a> {
    client: PredifiContractClient<'a>,
    token: token::StellarAssetClient<'a>,
    token_address: Address,
    ac_id: Address,
    treasury: Address,
    admin: Address,
    operator: Address,
    creator: Address,
}

/// Register the contract under test, a permissive access-control stub, and a staking
/// token. Initialization is left to [`Harness::init`] so the `init` event can be captured.
fn harness(env: &Env) -> Harness<'_> {
    let ac_id = env.register(dummy_ac::DummyAC, ());
    let contract_id = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(env, &contract_id);

    let token_admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token_address = sac.address();
    let token = token::StellarAssetClient::new(env, &token_address);

    Harness {
        client,
        token,
        token_address,
        ac_id,
        treasury: Address::generate(env),
        admin: Address::generate(env),
        operator: Address::generate(env),
        creator: Address::generate(env),
    }
}

impl Harness<'_> {
    fn init(&self, fee_bps: u32) {
        self.client.init(
            &self.ac_id,
            &self.treasury,
            &fee_bps,
            &0u64,
            &3600u64,
            &0u32,
        );
    }

    fn whitelist_token(&self) {
        self.client
            .add_token_to_whitelist(&self.admin, &self.token_address);
    }

    fn create_pool(&self, env: &Env, end_time: u64, initial_liquidity: i128) -> u64 {
        self.client.create_pool(
            &self.creator,
            &end_time,
            &self.token_address,
            &2u32,
            &symbol_short!("Tech"),
            &PoolConfig {
                start_time: 0,
                description: String::from_str(env, "Schema test pool"),
                metadata_url: String::from_str(env, "ipfs://schema-test"),
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
            },
        )
    }
}

// ── Event decoding helpers ───────────────────────────────────────────────────

type EventFields = soroban_sdk::Map<Symbol, Val>;

/// Collect the events emitted by `contract` during the current top-level invocation,
/// preserving emission order.
fn predifi_events(env: &Env, contract: &Address) -> StdVec<(Symbol, EventFields)> {
    let mut out = StdVec::new();
    for (addr, topics, data) in env.events().all().iter() {
        if &addr != contract || topics.is_empty() {
            continue;
        }
        let topic = Symbol::try_from_val(env, &topics.get(0).unwrap()).unwrap();
        let fields: EventFields = data.into_val(env);
        out.push((topic, fields));
    }
    out
}

/// The Soroban type tag of a decoded field value, as documented in `EVENT_SCHEMAS.md` §5.
fn field_type(env: &Env, value: &Val) -> StdString {
    let name = if Address::try_from_val(env, value).is_ok() {
        "Address"
    } else if Symbol::try_from_val(env, value).is_ok() {
        "Symbol"
    } else if String::try_from_val(env, value).is_ok() {
        "String"
    } else if i128::try_from_val(env, value).is_ok() {
        "i128"
    } else if u64::try_from_val(env, value).is_ok() {
        "u64"
    } else if u32::try_from_val(env, value).is_ok() {
        "u32"
    } else if Vec::<Val>::try_from_val(env, value).is_ok() {
        "Vec"
    } else {
        "unknown"
    };
    StdString::from(name)
}

/// Canonical snapshot of a single event: `topic[field:type,...]` with fields sorted by
/// name. Sorting makes the snapshot independent of the data-map iteration order.
fn shape(env: &Env, topic: &Symbol, fields: &EventFields) -> StdString {
    let mut parts: StdVec<StdString> = StdVec::new();
    for key in fields.keys().iter() {
        let value = fields.get(key.clone()).unwrap();
        parts.push(std::format!(
            "{}:{}",
            key.to_string(),
            field_type(env, &value)
        ));
    }
    parts.sort();
    std::format!("{}[{}]", topic.to_string(), parts.join(","))
}

/// Ordered snapshot of every event emitted by `contract` in the current invocation.
fn snapshot(env: &Env, contract: &Address) -> StdString {
    let mut parts: StdVec<StdString> = StdVec::new();
    for (topic, fields) in predifi_events(env, contract).iter() {
        parts.push(shape(env, topic, fields));
    }
    parts.join(" | ")
}

fn field(env: &Env, fields: &EventFields, name: &str) -> Val {
    fields
        .get(Symbol::new(env, name))
        .expect("event is missing a documented field")
}

fn get_u64(env: &Env, fields: &EventFields, name: &str) -> u64 {
    u64::try_from_val(env, &field(env, fields, name)).expect("field is not a u64")
}

fn get_u32(env: &Env, fields: &EventFields, name: &str) -> u32 {
    u32::try_from_val(env, &field(env, fields, name)).expect("field is not a u32")
}

fn get_i128(env: &Env, fields: &EventFields, name: &str) -> i128 {
    i128::try_from_val(env, &field(env, fields, name)).expect("field is not an i128")
}

fn get_address(env: &Env, fields: &EventFields, name: &str) -> Address {
    Address::try_from_val(env, &field(env, fields, name)).expect("field is not an Address")
}

fn get_string(env: &Env, fields: &EventFields, name: &str) -> String {
    String::try_from_val(env, &field(env, fields, name)).expect("field is not a String")
}

fn event_by_topic(env: &Env, contract: &Address, topic: &str) -> EventFields {
    let want = Symbol::new(env, topic);
    for (t, fields) in predifi_events(env, contract).iter() {
        if t == &want {
            return fields.clone();
        }
    }
    panic!("expected event topic was not emitted");
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[test]
fn documented_topics_are_stable_symbols() {
    let env = Env::default();

    let mut all: StdVec<&str> = StdVec::new();
    all.extend_from_slice(LIFECYCLE_EVENT_TOPICS);
    all.extend_from_slice(PREDICTION_EVENT_TOPICS);
    all.extend_from_slice(RESOLUTION_EVENT_TOPICS);
    all.extend_from_slice(PAYOUT_EVENT_TOPICS);

    for name in all.iter() {
        // Symbols are limited to 32 chars and must round-trip exactly.
        assert!(
            name.len() <= 32,
            "topic {name} exceeds the Soroban Symbol length limit"
        );
        assert_eq!(Symbol::new(&env, name).to_string(), StdString::from(*name));
    }

    // The schema version that this reference pins is 1 (see EVENT_SCHEMAS.md §3).
    assert_eq!(EVENT_SCHEMA_VERSION, 1);
}

#[test]
fn golden_full_lifecycle_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let h = harness(&env);

    // ── init ────────────────────────────────────────────────────────────────
    h.init(200);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "init[access_control:Address,fee_bps:u32,max_predictions_per_user:u32,min_pool_duration:u64,resolution_delay:u64,treasury:Address]"
    );
    let init = event_by_topic(&env, &h.client.address, "init");
    assert_eq!(get_u32(&env, &init, "fee_bps"), 200);
    assert_eq!(get_u64(&env, &init, "min_pool_duration"), 3600);
    assert_eq!(get_address(&env, &init, "treasury"), h.treasury);

    // ── whitelist the staking token (admin/governance family) ───────────────
    h.whitelist_token();
    assert_eq!(
        snapshot(&env, &h.client.address),
        "token_whitelist_added[admin:Address,token:Address]"
    );

    // ── create pool with seed liquidity ─────────────────────────────────────
    h.token.mint(&h.creator, &1_000);
    let pool_id = h.create_pool(&env, 5_000, 100);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "pool_created[category:Symbol,creator:Address,end_time:u64,initial_liquidity:i128,max_total_stake:i128,metadata_url:String,options_count:u32,outcome_descriptions:Vec,pool_id:u64,required_resolutions:u32,token:Address] | initial_liquidity_provided[amount:i128,creator:Address,pool_id:u64]"
    );

    let created = event_by_topic(&env, &h.client.address, "pool_created");
    assert_eq!(get_u64(&env, &created, "pool_id"), pool_id);
    assert_eq!(get_address(&env, &created, "creator"), h.creator);
    assert_eq!(get_u64(&env, &created, "end_time"), 5_000);
    assert_eq!(get_i128(&env, &created, "initial_liquidity"), 100);
    assert_eq!(get_u32(&env, &created, "options_count"), 2);
    assert_eq!(
        get_string(&env, &created, "metadata_url"),
        String::from_str(&env, "ipfs://schema-test")
    );
    assert_eq!(get_address(&env, &created, "token"), h.token_address);

    // `outcome_descriptions` is a `Vec<String>`; every element decodes as a `String`.
    let descriptions: Vec<Val> =
        Vec::try_from_val(&env, &field(&env, &created, "outcome_descriptions")).unwrap();
    assert_eq!(descriptions.len(), 2);
    for element in descriptions.iter() {
        assert!(String::try_from_val(&env, &element).is_ok());
    }

    let liquidity = event_by_topic(&env, &h.client.address, "initial_liquidity_provided");
    assert_eq!(get_u64(&env, &liquidity, "pool_id"), pool_id);
    assert_eq!(get_i128(&env, &liquidity, "amount"), 100);

    // ── place prediction ────────────────────────────────────────────────────
    let bettor = Address::generate(&env);
    h.token.mint(&bettor, &1_000);
    h.client
        .place_prediction(&bettor, &pool_id, &400, &1, &None, &None);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "prediction_placed[amount:i128,outcome:u32,pool_id:u64,user:Address]"
    );
    let prediction = event_by_topic(&env, &h.client.address, "prediction_placed");
    assert_eq!(get_u64(&env, &prediction, "pool_id"), pool_id);
    assert_eq!(get_address(&env, &prediction, "user"), bettor);
    assert_eq!(get_i128(&env, &prediction, "amount"), 400);
    assert_eq!(get_u32(&env, &prediction, "outcome"), 1);

    // ── close staking ───────────────────────────────────────────────────────
    env.ledger().with_mut(|li| li.timestamp = 5_001);
    h.client.close_staking(&pool_id);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "staking_closed[end_time:u64,pool_id:u64,timestamp:u64,total_stake:i128]"
    );
    let closed = event_by_topic(&env, &h.client.address, "staking_closed");
    assert_eq!(get_u64(&env, &closed, "pool_id"), pool_id);
    assert_eq!(get_u64(&env, &closed, "end_time"), 5_000);
    assert_eq!(get_u64(&env, &closed, "timestamp"), 5_001);
    assert_eq!(get_i128(&env, &closed, "total_stake"), 500);

    // ── mark ready for resolution ───────────────────────────────────────────
    h.client.mark_pool_ready(&pool_id);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "pool_ready[pool_id:u64,timestamp:u64]"
    );

    // ── resolve ─────────────────────────────────────────────────────────────
    h.client.resolve_pool(&h.operator, &pool_id, &1u32);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "resolution_vote_cast[outcome:u32,pool_id:u64,required_resolutions:u32,vote_count:u32,voter:Address] | pool_resolved[operator:Address,outcome:u32,pool_id:u64] | pool_resolved_diag[outcome:u32,pool_id:u64,timestamp:u64,total_stake:i128,winning_stake:i128]"
    );
    let resolved = event_by_topic(&env, &h.client.address, "pool_resolved");
    assert_eq!(get_u64(&env, &resolved, "pool_id"), pool_id);
    assert_eq!(get_u32(&env, &resolved, "outcome"), 1);
    assert_eq!(get_address(&env, &resolved, "operator"), h.operator);

    let diag = event_by_topic(&env, &h.client.address, "pool_resolved_diag");
    assert_eq!(get_i128(&env, &diag, "total_stake"), 500);
    assert_eq!(get_i128(&env, &diag, "winning_stake"), 400);
    assert_eq!(get_u64(&env, &diag, "timestamp"), 5_001);

    // ── claim winnings (2% fee on 500 => payout pool 490) ───────────────────
    let winnings = h.client.claim_winnings(&bettor, &pool_id);
    assert_eq!(winnings, 490);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "winnings_claimed[amount:i128,pool_id:u64,user:Address] | reward_claimed[amount:i128,claim_type:String,pool_id:u64,user:Address]"
    );
    let claimed = event_by_topic(&env, &h.client.address, "winnings_claimed");
    assert_eq!(get_u64(&env, &claimed, "pool_id"), pool_id);
    assert_eq!(get_address(&env, &claimed, "user"), bettor);
    assert_eq!(get_i128(&env, &claimed, "amount"), 490);

    let reward = event_by_topic(&env, &h.client.address, "reward_claimed");
    assert_eq!(
        get_string(&env, &reward, "claim_type"),
        String::from_str(&env, "winnings")
    );
    assert_eq!(get_i128(&env, &reward, "amount"), 490);
}

#[test]
fn golden_referral_payout_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let h = harness(&env);
    h.init(200);
    h.whitelist_token();

    let pool_id = h.create_pool(&env, 5_000, 0);

    let bettor = Address::generate(&env);
    let referrer = Address::generate(&env);
    h.token.mint(&bettor, &1_000);
    h.client
        .place_prediction(&bettor, &pool_id, &400, &1, &Some(referrer.clone()), &None);

    env.ledger().with_mut(|li| li.timestamp = 5_001);
    h.client.close_staking(&pool_id);
    h.client.mark_pool_ready(&pool_id);
    h.client.resolve_pool(&h.operator, &pool_id, &1u32);

    let winnings = h.client.claim_winnings(&bettor, &pool_id);

    // total stake 400, 2% fee => fee 8, payout 392, referral cut defaults to 50% of fee => 4.
    assert_eq!(winnings, 392);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "referral_paid[amount:i128,pool_id:u64,referred_user:Address,referrer:Address] | winnings_claimed[amount:i128,pool_id:u64,user:Address] | reward_claimed[amount:i128,claim_type:String,pool_id:u64,user:Address]"
    );

    let referral = event_by_topic(&env, &h.client.address, "referral_paid");
    assert_eq!(get_u64(&env, &referral, "pool_id"), pool_id);
    assert_eq!(get_address(&env, &referral, "referrer"), referrer);
    assert_eq!(get_address(&env, &referral, "referred_user"), bettor);
    assert_eq!(get_i128(&env, &referral, "amount"), 4);
}

#[test]
fn golden_cancel_and_refund_flow() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let h = harness(&env);
    h.init(0);
    h.whitelist_token();

    let pool_id = h.create_pool(&env, 5_000, 0);

    let bettor = Address::generate(&env);
    h.token.mint(&bettor, &1_000);
    h.client
        .place_prediction(&bettor, &pool_id, &300, &0, &None, &None);

    h.client.cancel_pool(
        &h.operator,
        &pool_id,
        &String::from_str(&env, "delisted source"),
    );
    assert_eq!(
        snapshot(&env, &h.client.address),
        "pool_canceled[caller:Address,operator:Address,pool_id:u64,reason:String]"
    );
    let canceled = event_by_topic(&env, &h.client.address, "pool_canceled");
    assert_eq!(get_u64(&env, &canceled, "pool_id"), pool_id);
    assert_eq!(get_address(&env, &canceled, "operator"), h.operator);
    assert_eq!(
        get_string(&env, &canceled, "reason"),
        String::from_str(&env, "delisted source")
    );

    let refund = h.client.claim_refund(&bettor, &pool_id);
    assert_eq!(refund, 300);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "refund_claimed[amount:i128,pool_id:u64,user:Address] | reward_claimed[amount:i128,claim_type:String,pool_id:u64,user:Address]"
    );
    let refunded = event_by_topic(&env, &h.client.address, "refund_claimed");
    assert_eq!(get_u64(&env, &refunded, "pool_id"), pool_id);
    assert_eq!(get_i128(&env, &refunded, "amount"), 300);
    let reward = event_by_topic(&env, &h.client.address, "reward_claimed");
    assert_eq!(
        get_string(&env, &reward, "claim_type"),
        String::from_str(&env, "refund")
    );
}

#[test]
fn high_value_prediction_emits_documented_shape() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let h = harness(&env);
    h.init(0);
    h.whitelist_token();

    let pool_id = h.create_pool(&env, 5_000, 0);

    let whale = Address::generate(&env);
    h.token.mint(&whale, &2_000_000_000);
    h.client
        .place_prediction(&whale, &pool_id, &1_000_000_000, &0, &None, &None);

    assert_eq!(
        snapshot(&env, &h.client.address),
        "prediction_placed[amount:i128,outcome:u32,pool_id:u64,user:Address] | high_value_prediction[amount:i128,outcome:u32,pool_id:u64,threshold:i128,user:Address]"
    );

    let alert = event_by_topic(&env, &h.client.address, "high_value_prediction");
    assert_eq!(get_u64(&env, &alert, "pool_id"), pool_id);
    assert_eq!(get_address(&env, &alert, "user"), whale);
    assert_eq!(get_i128(&env, &alert, "amount"), 1_000_000_000);
    assert_eq!(get_i128(&env, &alert, "threshold"), 1_000_000_000);
}

#[test]
fn staking_closed_event_is_emitted_at_most_once() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let h = harness(&env);
    h.init(0);
    h.whitelist_token();

    let pool_id = h.create_pool(&env, 5_000, 0);

    env.ledger().with_mut(|li| li.timestamp = 5_001);

    // First close: emits exactly the documented event.
    h.client.close_staking(&pool_id);
    assert_eq!(
        snapshot(&env, &h.client.address),
        "staking_closed[end_time:u64,pool_id:u64,timestamp:u64,total_stake:i128]"
    );

    // Second close is a no-op: indexers keyed on `pool_id` must not see a duplicate.
    h.client.close_staking(&pool_id);
    assert_eq!(snapshot(&env, &h.client.address), "");
}
