//! Per-entry-point resource measurements for the core pool lifecycle (#42).
//!
//! Each test drives one public entry point through its worst-case path and
//! prints one `GAS_BUDGET {json}` line built from
//! `env.cost_estimate().resources()` (the host's metering of the last
//! top-level invocation). `scripts/check_gas_budgets.py` compares those lines
//! against `contract/gas-budgets.json`.
//!
//! The contract is registered natively, so VM instantiation and Wasm
//! execution costs are excluded; the figures are deterministic for a given
//! `soroban-sdk` version, which makes them comparable across runs.
//!
//! ```bash
//! cargo test -p predifi-contract --lib gas_budget_tests -- --nocapture --test-threads=1
//! ```

extern crate std;

use crate::{PoolConfig, PredifiContract, PredifiContractClient, MAX_OUTCOMES_PER_POOL};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token, Address, Env, String, Vec,
};

mod dummy_access_control {
    use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

    #[contract]
    pub struct DummyAccessControl;

    #[contractimpl]
    impl DummyAccessControl {
        pub fn grant_role(env: Env, user: Address, role: u32) {
            let key = (Symbol::new(&env, "role"), user, role);
            env.storage().instance().set(&key, &true);
        }

        pub fn has_role(env: Env, user: Address, role: u32) -> bool {
            let key = (Symbol::new(&env, "role"), user, role);
            env.storage().instance().get(&key).unwrap_or(false)
        }
    }
}

const ROLE_ADMIN: u32 = 0;
const ROLE_OPERATOR: u32 = 1;
const RESOLUTION_DELAY: u64 = 3600;
const POOL_DURATION: u64 = 10_000;
const STAKE: i128 = 1_000;

struct Harness<'a> {
    env: &'a Env,
    client: PredifiContractClient<'a>,
    admin: Address,
    token: Address,
    token_admin: token::StellarAssetClient<'a>,
}

impl<'a> Harness<'a> {
    fn new(env: &'a Env) -> Self {
        env.mock_all_auths();
        let ac_id = env.register(dummy_access_control::DummyAccessControl, ());
        let ac = dummy_access_control::DummyAccessControlClient::new(env, &ac_id);
        let client = PredifiContractClient::new(env, &env.register(PredifiContract, ()));
        let admin = Address::generate(env);
        ac.grant_role(&admin, &ROLE_ADMIN);
        ac.grant_role(&admin, &ROLE_OPERATOR);
        client.init(
            &ac_id,
            &Address::generate(env),
            &500,
            &RESOLUTION_DELAY,
            &RESOLUTION_DELAY,
            &0u32,
        );
        let token = env
            .register_stellar_asset_contract_v2(Address::generate(env))
            .address();
        client.add_token_to_whitelist(&admin, &token);
        let token_admin = token::StellarAssetClient::new(env, &token);
        Self {
            env,
            client,
            admin,
            token,
            token_admin,
        }
    }

    /// Worst case: the maximum number of outcomes, each with a description.
    fn create_pool(&self) -> u64 {
        let mut outcome_descriptions = Vec::new(self.env);
        for _ in 0..MAX_OUTCOMES_PER_POOL {
            outcome_descriptions.push_back(String::from_str(self.env, "Outcome"));
        }
        self.client.create_pool(
            &Address::generate(self.env),
            &(self.env.ledger().timestamp() + POOL_DURATION),
            &self.token,
            &MAX_OUTCOMES_PER_POOL,
            &symbol_short!("Tech"),
            &PoolConfig {
                start_time: 0,
                description: String::from_str(self.env, "Gas budget"),
                metadata_url: String::from_str(self.env, "ipfs://gas-budget"),
                min_stake: 1,
                max_stake: 0,
                max_total_stake: 0,
                min_total_stake: 1,
                initial_liquidity: 0,
                required_resolutions: 1,
                private: false,
                whitelist_key: None,
                outcome_descriptions,
            },
        )
    }

    /// First prediction by a new user (creates new storage entries).
    fn predict(&self, pool_id: u64, outcome: u32) -> Address {
        let user = Address::generate(self.env);
        self.token_admin.mint(&user, &STAKE);
        self.client
            .place_prediction(&user, &pool_id, &STAKE, &outcome, &None, &None);
        user
    }

    fn advance_past_resolution(&self) {
        self.env
            .ledger()
            .with_mut(|li| li.timestamp += POOL_DURATION + RESOLUTION_DELAY + 1);
    }

    fn report(&self, entry_point: &str) {
        let r = self.env.cost_estimate().resources();
        std::println!(
            "GAS_BUDGET {{\"entry_point\":\"{}\",\"cpu_insns\":{},\"mem_bytes\":{},\"read_entries\":{},\"write_entries\":{},\"read_bytes\":{},\"write_bytes\":{}}}",
            entry_point,
            r.instructions,
            r.mem_bytes,
            r.disk_read_entries + r.memory_read_entries,
            r.write_entries,
            r.disk_read_bytes,
            r.write_bytes,
        );
    }
}

#[test]
fn gas_budget_create_pool() {
    let env = Env::default();
    let h = Harness::new(&env);
    h.create_pool();
    h.report("create_pool");
}

#[test]
fn gas_budget_place_prediction() {
    let env = Env::default();
    let h = Harness::new(&env);
    let pool_id = h.create_pool();
    h.predict(pool_id, 0);
    h.predict(pool_id, MAX_OUTCOMES_PER_POOL - 1);
    h.report("place_prediction");
}

#[test]
fn gas_budget_resolve_pool() {
    let env = Env::default();
    let h = Harness::new(&env);
    let pool_id = h.create_pool();
    h.predict(pool_id, 0);
    h.predict(pool_id, 1);
    h.advance_past_resolution();
    h.client.resolve_pool(&h.admin, &pool_id, &0);
    h.report("resolve_pool");
}

#[test]
fn gas_budget_claim_winnings() {
    let env = Env::default();
    let h = Harness::new(&env);
    let pool_id = h.create_pool();
    let winner = h.predict(pool_id, 0);
    h.predict(pool_id, 1);
    h.advance_past_resolution();
    h.client.resolve_pool(&h.admin, &pool_id, &0);
    h.client.claim_winnings(&winner, &pool_id);
    h.report("claim_winnings");
}

#[test]
fn gas_budget_cancel_pool() {
    let env = Env::default();
    let h = Harness::new(&env);
    let pool_id = h.create_pool();
    h.predict(pool_id, 0);
    h.client
        .cancel_pool(&h.admin, &pool_id, &String::from_str(&env, "gas budget"));
    h.report("cancel_pool");
}

#[test]
fn gas_budget_claim_refund() {
    let env = Env::default();
    let h = Harness::new(&env);
    let pool_id = h.create_pool();
    let user = h.predict(pool_id, 0);
    h.client
        .cancel_pool(&h.admin, &pool_id, &String::from_str(&env, "gas budget"));
    h.client.claim_refund(&user, &pool_id);
    h.report("claim_refund");
}
