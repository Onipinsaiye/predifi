//! Deterministic LP fee attribution (issue #276).
//!
//! Property/fuzz tests over provider counts, contribution ratios and repeated
//! accrual, plus an on-chain test that exercises the exposed claimable amounts
//! and total-fee conservation.

use predifi_contract::{
    allocate_fees, claimable_for, total_allocated, FeeAllocationError, PoolConfig, PredifiContract,
    PredifiContractClient, ProviderInput,
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

fn inputs(pairs: &[(u64, i128)]) -> std::vec::Vec<ProviderInput<u64>> {
    pairs
        .iter()
        .map(|&(key, shares)| ProviderInput { key, shares })
        .collect()
}

// ── Determinism / conservation ──────────────────────────────────────────────

#[test]
fn equal_shares_split_evenly_and_conserve_every_unit() {
    let result = allocate_fees(10, &inputs(&[(1, 100), (2, 100), (3, 100)])).unwrap();
    assert_eq!(total_allocated(&result), 10);
    assert_eq!(claimable_for(&result, &1), 4);
    assert_eq!(claimable_for(&result, &2), 3);
    assert_eq!(claimable_for(&result, &3), 3);
}

#[test]
fn remainder_assignment_is_independent_of_iteration_order() {
    let a = allocate_fees(100, &inputs(&[(1, 1), (2, 1), (3, 1)])).unwrap();
    let b = allocate_fees(100, &inputs(&[(3, 1), (1, 1), (2, 1)])).unwrap();
    let c = allocate_fees(100, &inputs(&[(2, 1), (3, 1), (1, 1)])).unwrap();
    assert_eq!(a, b);
    assert_eq!(b, c);
    assert_eq!(total_allocated(&a), 100);
}

#[test]
fn proportional_ratios_and_largest_remainder() {
    let result = allocate_fees(1_000, &inputs(&[(1, 1), (2, 2), (3, 7)])).unwrap();
    assert_eq!(total_allocated(&result), 1_000);
    assert_eq!(claimable_for(&result, &1), 100);
    assert_eq!(claimable_for(&result, &2), 200);
    assert_eq!(claimable_for(&result, &3), 700);
}

#[test]
fn zero_fees_or_zero_shares_are_rejected_or_empty() {
    let zero = allocate_fees(0, &inputs(&[(1, 10)])).unwrap();
    assert_eq!(total_allocated(&zero), 0);
    assert!(zero.iter().all(|a| a.amount == 0));
    assert_eq!(
        allocate_fees(10, &inputs(&[(1, 0), (2, 0)])),
        Err(FeeAllocationError::NoShares)
    );
    assert_eq!(
        allocate_fees(-1, &inputs(&[(1, 10)])),
        Err(FeeAllocationError::InvalidAmount)
    );
    assert_eq!(
        allocate_fees(10, &inputs(&[(1, -1)])),
        Err(FeeAllocationError::InvalidAmount)
    );
}

#[test]
fn fuzz_provider_counts_and_ratios_never_leak_fees() {
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for _ in 0..500 {
        let count = 1 + (next() % 20) as u64;
        let total_fees = (next() % 1_000_000) as i128;
        let mut pairs = std::vec::Vec::new();
        for key in 1..=count {
            pairs.push((key, 1 + (next() % 1_000_000) as i128));
        }

        let allocations = allocate_fees(total_fees, &inputs(&pairs)).unwrap();
        assert_eq!(total_allocated(&allocations), total_fees);
        assert!(allocations.iter().all(|a| a.amount >= 0));

        let shuffled: std::vec::Vec<_> = pairs.iter().rev().cloned().collect();
        assert_eq!(allocations, allocate_fees(total_fees, &inputs(&shuffled)).unwrap());
    }
}

#[test]
fn repeated_accrual_never_allocates_more_than_collected() {
    let pairs = [(1, 3), (2, 5), (3, 11), (4, 1)];
    let mut collected = 0i128;
    for round in 1..=50 {
        collected += round * 7;
        let allocations = allocate_fees(collected, &inputs(&pairs)).unwrap();
        let allocated = total_allocated(&allocations);
        assert_eq!(allocated, collected);
        assert!(allocated <= collected);
    }
}

// ── On-chain exposure ───────────────────────────────────────────────────────

fn harness(env: &Env) -> (PredifiContractClient<'_>, Address, Address) {
    env.mock_all_auths();
    env.ledger().with_mut(|l| l.timestamp = 1_000);
    let ac = env.register(dummy_ac::DummyAC, ());
    let contract = env.register(PredifiContract, ());
    let client = PredifiContractClient::new(env, &contract);
    let admin = Address::generate(env);
    let treasury = Address::generate(env);
    let token_admin = Address::generate(env);
    let sac = env.register_stellar_asset_contract_v2(token_admin);
    let token = sac.address();

    client.init(&ac, &treasury, &0u32, &0u64, &3_600u64, &0u32);
    client.add_token_to_whitelist(&admin, &token);
    (client, token, admin)
}

fn create_pool(env: &Env, client: &PredifiContractClient<'_>, token: &Address) -> u64 {
    client.create_pool(
        &Address::generate(env),
        &(env.ledger().timestamp() + 10_000),
        token,
        &2u32,
        &symbol_short!("Tech"),
        &PoolConfig {
            start_time: 0,
            description: String::from_str(env, "Fee allocation test"),
            metadata_url: String::from_str(env, "ipfs://fees"),
            min_stake: 1,
            max_stake: 0,
            max_total_stake: 0,
            min_total_stake: 1,
            initial_liquidity: 0,
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

#[test]
fn on_chain_allocation_exposes_conserved_claimable_amounts() {
    let env = Env::default();
    let (client, token, _) = harness(&env);
    let sac = token::StellarAssetClient::new(&env, &token);
    let pool_id = create_pool(&env, &client, &token);

    let providers = [Address::generate(&env), Address::generate(&env), Address::generate(&env)];
    for (index, provider) in providers.iter().enumerate() {
        sac.mint(provider, &1_000);
        client.deposit_liquidity(provider, &pool_id, &((index as i128 + 1) * 100));
    }

    let fees = 1_000i128;
    client.accrue_lp_fees(&pool_id, &fees);

    let allocated = client.get_allocated_lp_fees(&pool_id);
    assert_eq!(allocated, fees);

    let claimable: std::vec::Vec<i128> = providers
        .iter()
        .map(|p| client.get_lp_fee_claimable(&pool_id, p))
        .collect();
    assert_eq!(claimable.iter().sum::<i128>(), fees);
    assert!(claimable.iter().all(|&amount| amount >= 0));

    // Idempotent: re-running allocation must not change any claimable amount.
    client.allocate_lp_fees(&pool_id);
    let again: std::vec::Vec<i128> = providers
        .iter()
        .map(|p| client.get_lp_fee_claimable(&pool_id, p))
        .collect();
    assert_eq!(claimable, again);
    assert_eq!(client.get_allocated_lp_fees(&pool_id), fees);
}
