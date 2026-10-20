//! Oracle feed test fixture and unit tests for governed oracle configuration per market (Issue #19).
//!
//! Validates:
//! - Persisting selected feed configuration at market creation
//! - Supported feed identifier governance and validation
//! - Configuration bounds validation (freshness policy bounds)
//! - Event emission (`MarketOracleConfiguredEvent`, `SupportedFeedAddedEvent`, `SupportedFeedRemovedEvent`, etc.)
//! - Read methods (`get_market_oracle_config`, `is_feed_supported`, `get_supported_feeds`, `get_oracle_freshness_bounds`)
//! - Error conditions: unsupported feeds, invalid freshness values, unauthorized management/resolution, and configuration immutability after market creation.

#![cfg(test)]

use crate::test::{dummy_access_control, ROLE_ADMIN, ROLE_OPERATOR};
use crate::{
    MarketOracleConfig, MarketState, PoolConfig, PredifiContract, PredifiContractClient,
    PredifiError, DEFAULT_MAX_ORACLE_FRESHNESS, DEFAULT_MIN_ORACLE_FRESHNESS,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    vec, Address, Env, String, Symbol,
};

const ROLE_ORACLE: u32 = 3;

/// Test fixture providing pre-configured test accounts, token, access control,
/// and helper methods for governed oracle feed testing.
pub struct OracleFeedFixture<'a> {
    pub env: Env,
    pub admin: Address,
    pub operator: Address,
    pub oracle: Address,
    pub creator: Address,
    pub token_address: Address,
    pub client: PredifiContractClient<'a>,
    pub ac_client: dummy_access_control::DummyAccessControlClient<'a>,
}

impl<'a> OracleFeedFixture<'a> {
    /// Initialize a new test fixture with whitelisted token, whitelisted oracle,
    /// and standard supported feed identifiers (`BTC`, `ETH`).
    pub fn new(env: &'a Env) -> Self {
        env.mock_all_auths();

        // Advance ledger to a realistic timestamp
        env.ledger().with_mut(|l| {
            l.timestamp = 10_000;
        });

        let ac_id = env.register(dummy_access_control::DummyAccessControl, ());
        let ac_client = dummy_access_control::DummyAccessControlClient::new(env, &ac_id);

        let contract_id = env.register(PredifiContract, ());
        let client = PredifiContractClient::new(env, &contract_id);

        let admin = Address::generate(env);
        let operator = Address::generate(env);
        let oracle = Address::generate(env);
        let creator = Address::generate(env);
        let treasury = Address::generate(env);

        ac_client.grant_role(&admin, &ROLE_ADMIN);
        ac_client.grant_role(&operator, &ROLE_OPERATOR);
        ac_client.grant_role(&oracle, &ROLE_ORACLE);

        let token_admin = Address::generate(env);
        let token_contract = env.register_stellar_asset_contract(token_admin.clone());
        let token_address = token_contract;

        client.init(&ac_id, &treasury, &0u32, &0u64, &3600u64, &0u32);
        client.add_token_to_whitelist(&admin, &token_address);
        client.add_oracle(&admin, &oracle);

        // Add standard supported feeds
        client.add_supported_feed(&admin, &symbol_short!("BTC"));
        client.add_supported_feed(&admin, &symbol_short!("ETH"));

        Self {
            env: env.clone(),
            admin,
            operator,
            oracle,
            creator,
            token_address,
            client,
            ac_client,
        }
    }

    /// Construct a standard `PoolConfig` for binary test markets.
    pub fn default_pool_config(&self, description: &str) -> PoolConfig {
        PoolConfig {
            start_time: 0,
            description: String::from_str(&self.env, description),
            metadata_url: String::from_str(&self.env, "ipfs://oracle-market"),
            min_stake: 10,
            max_stake: 1_000_000,
            min_total_stake: 10,
            max_total_stake: 0,
            initial_liquidity: 0,
            required_resolutions: 1,
            private: false,
            whitelist_key: None,
            outcome_descriptions: vec![
                &self.env,
                String::from_str(&self.env, "No"),
                String::from_str(&self.env, "Yes"),
            ],
        }
    }

    /// Construct a `MarketOracleConfig` using the fixture's trusted oracle.
    pub fn default_oracle_config(&self, feed_id: Symbol, max_staleness: u64) -> MarketOracleConfig {
        MarketOracleConfig {
            oracle: self.oracle.clone(),
            feed_id,
            max_staleness_seconds: max_staleness,
        }
    }

    /// Helper to create an oracle market with default parameters.
    pub fn create_oracle_market(&self, feed_id: Symbol, max_staleness: u64) -> u64 {
        let pool_config = self.default_pool_config("Oracle Test Pool");
        let oracle_config = self.default_oracle_config(feed_id, max_staleness);
        let end_time = self.env.ledger().timestamp() + 7_200;

        self.client.create_oracle_pool(
            &self.creator,
            &end_time,
            &self.token_address,
            &2u32,
            &symbol_short!("Crypto"),
            &pool_config,
            &oracle_config,
        )
    }

    /// Push a fresh price feed update from the fixture's oracle.
    pub fn update_price_feed(
        &self,
        feed_id: Symbol,
        price: i128,
        timestamp: u64,
        expires_at: u64,
    ) {
        self.client
            .update_price_feed(&self.oracle, &feed_id, &price, &100i128, &timestamp, &expires_at);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// TESTS: GOVERNED ORACLE CONFIGURATION PER MARKET (ISSUE #19)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn test_create_oracle_pool_persists_config_and_emits_event() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let feed_id = symbol_short!("BTC");
    let max_staleness = 60u64;
    let pool_config = fixture.default_pool_config("Will BTC exceed $100k?");
    let oracle_config = fixture.default_oracle_config(feed_id.clone(), max_staleness);
    let end_time = env.ledger().timestamp() + 7_200;

    let pool_id = fixture.client.create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &oracle_config,
    );

    // Verify read method returns the exact persisted configuration
    let persisted_cfg = fixture.client.get_market_oracle_config(&pool_id);
    assert!(persisted_cfg.is_some());
    let cfg = persisted_cfg.unwrap();
    assert_eq!(cfg.oracle, fixture.oracle);
    assert_eq!(cfg.feed_id, feed_id);
    assert_eq!(cfg.max_staleness_seconds, max_staleness);

    // Verify regular pools without oracle config return None
    let regular_pool_id = fixture.client.create_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
    );
    assert_eq!(fixture.client.get_market_oracle_config(&regular_pool_id), None);
}

#[test]
fn test_unsupported_feed_rejected_at_creation() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let unsupported_feed = symbol_short!("SOL");
    let pool_config = fixture.default_pool_config("SOL market");
    let oracle_config = fixture.default_oracle_config(unsupported_feed.clone(), 60u64);
    let end_time = env.ledger().timestamp() + 7_200;

    // Creating pool with unsupported feed must fail with UnsupportedFeed
    let res = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &oracle_config,
    );
    assert_eq!(res, Err(Ok(PredifiError::UnsupportedFeed)));

    // Admin adds SOL to supported feeds
    fixture.client.add_supported_feed(&fixture.admin, &unsupported_feed);
    assert!(fixture.client.is_feed_supported(&unsupported_feed));

    // Now creating the pool succeeds
    let pool_id = fixture.client.create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &oracle_config,
    );
    assert_eq!(
        fixture.client.get_market_oracle_config(&pool_id),
        Some(oracle_config)
    );

    // Admin removes SOL from supported feeds
    fixture.client.remove_supported_feed(&fixture.admin, &unsupported_feed);
    assert!(!fixture.client.is_feed_supported(&unsupported_feed));

    // Creating another pool with SOL now fails again
    let res2 = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &fixture.default_oracle_config(unsupported_feed, 60u64),
    );
    assert_eq!(res2, Err(Ok(PredifiError::UnsupportedFeed)));
}

#[test]
fn test_invalid_freshness_values_rejected() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let feed_id = symbol_short!("BTC");
    let pool_config = fixture.default_pool_config("BTC market");
    let end_time = env.ledger().timestamp() + 7_200;

    // 1. Zero freshness must fail with InvalidFreshness
    let zero_cfg = fixture.default_oracle_config(feed_id.clone(), 0u64);
    let res_zero = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &zero_cfg,
    );
    assert_eq!(res_zero, Err(Ok(PredifiError::InvalidFreshness)));

    // 2. Freshness below minimum bound (default min: 5s)
    let too_low_cfg = fixture.default_oracle_config(feed_id.clone(), DEFAULT_MIN_ORACLE_FRESHNESS - 1);
    let res_low = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &too_low_cfg,
    );
    assert_eq!(res_low, Err(Ok(PredifiError::InvalidFreshness)));

    // 3. Freshness above maximum bound (default max: 86,400s)
    let too_high_cfg =
        fixture.default_oracle_config(feed_id.clone(), DEFAULT_MAX_ORACLE_FRESHNESS + 1);
    let res_high = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &too_high_cfg,
    );
    assert_eq!(res_high, Err(Ok(PredifiError::InvalidFreshness)));

    // 4. Freshness within bounds succeeds
    let valid_cfg = fixture.default_oracle_config(feed_id.clone(), 300u64);
    let pool_id = fixture.client.create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &valid_cfg,
    );
    assert_eq!(
        fixture.client.get_market_oracle_config(&pool_id),
        Some(valid_cfg)
    );

    // 5. Governance updates freshness bounds: min=10s, max=600s
    fixture.client.set_oracle_freshness_bounds(&fixture.admin, &10u64, &600u64);
    let (min_f, max_f) = fixture.client.get_oracle_freshness_bounds();
    assert_eq!(min_f, 10u64);
    assert_eq!(max_f, 600u64);

    // Value 700s now fails under new bound
    let res_new_high = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &fixture.default_oracle_config(feed_id, 700u64),
    );
    assert_eq!(res_new_high, Err(Ok(PredifiError::InvalidFreshness)));
}

#[test]
fn test_authorization_governance_and_oracle() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let non_admin = Address::generate(&env);
    let feed_id = symbol_short!("XLM");

    // 1. Non-admin cannot add supported feed
    let res_add = fixture.client.try_add_supported_feed(&non_admin, &feed_id);
    assert_eq!(res_add, Err(Ok(PredifiError::Unauthorized)));

    // 2. Non-admin cannot remove supported feed
    let res_rem = fixture.client.try_remove_supported_feed(&non_admin, &symbol_short!("BTC"));
    assert_eq!(res_rem, Err(Ok(PredifiError::Unauthorized)));

    // 3. Non-admin cannot set freshness bounds
    let res_bounds = fixture.client.try_set_oracle_freshness_bounds(&non_admin, &10u64, &100u64);
    assert_eq!(res_bounds, Err(Ok(PredifiError::Unauthorized)));

    // 4. Non-whitelisted oracle identity rejected at market creation
    let untrusted_oracle = Address::generate(&env);
    let unwhitelisted_cfg = MarketOracleConfig {
        oracle: untrusted_oracle,
        feed_id: symbol_short!("BTC"),
        max_staleness_seconds: 60u64,
    };
    let pool_config = fixture.default_pool_config("Untrusted oracle market");
    let end_time = env.ledger().timestamp() + 7_200;

    let res_oracle = fixture.client.try_create_oracle_pool(
        &fixture.creator,
        &end_time,
        &fixture.token_address,
        &2u32,
        &symbol_short!("Crypto"),
        &pool_config,
        &unwhitelisted_cfg,
    );
    assert_eq!(res_oracle, Err(Ok(PredifiError::Unauthorized)));
}

#[test]
fn test_configuration_immutability_after_market_creation() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let feed_id = symbol_short!("BTC");
    let max_staleness = 60u64;
    let pool_id = fixture.create_oracle_market(feed_id.clone(), max_staleness);

    // Initial config is persisted
    let initial_cfg = fixture.client.get_market_oracle_config(&pool_id).unwrap();
    assert_eq!(initial_cfg.feed_id, feed_id);
    assert_eq!(initial_cfg.max_staleness_seconds, max_staleness);

    // Attempting to overwrite or mutate the oracle config via set_market_oracle_config must fail
    let new_oracle = Address::generate(&env);
    fixture.client.add_oracle(&fixture.admin, &new_oracle);

    let mutated_cfg = MarketOracleConfig {
        oracle: new_oracle,
        feed_id: symbol_short!("ETH"),
        max_staleness_seconds: 120u64,
    };

    let res_mutate = fixture.client.try_set_market_oracle_config(
        &fixture.operator,
        &pool_id,
        &mutated_cfg,
    );
    assert_eq!(res_mutate, Err(Ok(PredifiError::ConfigurationImmutable)));

    // Also fails if admin tries to overwrite it
    let res_admin = fixture.client.try_set_market_oracle_config(
        &fixture.admin,
        &pool_id,
        &mutated_cfg,
    );
    assert_eq!(res_admin, Err(Ok(PredifiError::ConfigurationImmutable)));

    // Verify config is completely unchanged
    let current_cfg = fixture.client.get_market_oracle_config(&pool_id).unwrap();
    assert_eq!(current_cfg, initial_cfg);
}

#[test]
fn test_price_condition_mismatched_feed_rejected() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    // Create pool bound to BTC
    let pool_id = fixture.create_oracle_market(symbol_short!("BTC"), 60u64);

    // Operator attempts to set a price condition using ETH feed (mismatch)
    let res_mismatch = fixture.client.try_set_price_condition(
        &fixture.operator,
        &pool_id,
        &symbol_short!("ETH"),
        &3_000_00000000i128,
        &1u32, // Greater
        &100u32,
    );
    assert_eq!(res_mismatch, Err(Ok(PredifiError::InvalidData)));

    // Setting price condition with the matching configured feed (BTC) succeeds
    fixture.client.set_price_condition(
        &fixture.operator,
        &pool_id,
        &symbol_short!("BTC"),
        &100_000_00000000i128,
        &1u32,
        &100u32,
    );
}

#[test]
fn test_oracle_resolve_requires_market_bound_oracle() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let pool_id = fixture.create_oracle_market(symbol_short!("BTC"), 60u64);

    // Register a second oracle and whitelist it globally
    let second_oracle = Address::generate(&env);
    fixture.ac_client.grant_role(&second_oracle, &ROLE_ORACLE);
    fixture.client.add_oracle(&fixture.admin, &second_oracle);

    // Fast-forward past end_time + resolution_delay
    env.ledger().with_mut(|l| {
        l.timestamp += 7_200 + 10;
    });

    // Second oracle (globally whitelisted but not bound to this market) attempts resolution
    let res_unbound = fixture.client.try_oracle_resolve(
        &second_oracle,
        &pool_id,
        &1u32,
        &String::from_str(&env, "proof"),
    );
    assert_eq!(res_unbound, Err(Ok(PredifiError::Unauthorized)));

    // Bound oracle resolves successfully
    fixture.client.oracle_resolve(
        &fixture.oracle,
        &pool_id,
        &1u32,
        &String::from_str(&env, "proof"),
    );

    let pool = fixture.client.get_pool(&pool_id);
    assert_eq!(pool.state, MarketState::Resolved);
    assert_eq!(pool.outcome, 1u32);
}

#[test]
fn test_market_specific_freshness_policy_enforced_during_resolution() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    // Initialize global oracle with generous max_price_age = 300 seconds
    fixture.client.init_oracle(
        &fixture.admin,
        &Address::generate(&env),
        &300u64,
        &100u32,
    );

    // Create market with a STRICT freshness policy: 30 seconds
    let pool_id = fixture.create_oracle_market(symbol_short!("BTC"), 30u64);

    // Set matching price condition: BTC > $50,000
    fixture.client.set_price_condition(
        &fixture.operator,
        &pool_id,
        &symbol_short!("BTC"),
        &50_000_00000000i128,
        &1u32,
        &0u32,
    );

    // Push price update at timestamp 9,990 (strictly before ledger timestamp 10,000)
    fixture.update_price_feed(
        symbol_short!("BTC"),
        60_000_00000000i128,
        9_990,
        20_000,
    );

    // Fast-forward past betting deadline (end_time was 10_000 + 7200 = 17_200)
    // Ledger time is now 17_210.
    // Price at 10_000 is 7,210 seconds old — way older than 30s.
    env.ledger().with_mut(|l| {
        l.timestamp = 17_210;
    });

    // Resolution must fail with PriceStale because price is older than 30s
    let res_stale = fixture.client.try_resolve_pool_from_price(&pool_id);
    assert_eq!(res_stale, Err(Ok(PredifiError::PriceStale)));

    // Push a fresh price update at current_time - 15s (17_195)
    fixture.update_price_feed(
        symbol_short!("BTC"),
        60_000_00000000i128,
        17_195,
        20_000,
    );

    // Now resolution succeeds because age (15s) <= market freshness policy (30s)
    fixture.client.resolve_pool_from_price(&pool_id);

    let pool = fixture.client.get_pool(&pool_id);
    assert_eq!(pool.state, MarketState::Resolved);
    assert_eq!(pool.outcome, 1u32);
}

#[test]
fn test_supported_feeds_query_and_bounds_methods() {
    let env = Env::default();
    let fixture = OracleFeedFixture::new(&env);

    let feeds = fixture.client.get_supported_feeds();
    assert!(feeds.contains(symbol_short!("BTC")));
    assert!(feeds.contains(symbol_short!("ETH")));
    assert_eq!(fixture.client.is_feed_supported(&symbol_short!("BTC")), true);
    assert_eq!(fixture.client.is_feed_supported(&symbol_short!("XYZ")), false);

    let (min_f, max_f) = fixture.client.get_oracle_freshness_bounds();
    assert_eq!(min_f, DEFAULT_MIN_ORACLE_FRESHNESS);
    assert_eq!(max_f, DEFAULT_MAX_ORACLE_FRESHNESS);
}
