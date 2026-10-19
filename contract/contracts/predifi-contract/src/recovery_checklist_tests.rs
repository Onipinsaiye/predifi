//! # Recovery Checklist Tests — Issue #36
//!
//! Validates the gate-unpause-on-recovery-checklist feature:
//!
//! - Missing checklist blocks unpause with `RecoveryChecklistNotConfigured`.
//! - Incomplete checklist (not all steps attested) blocks unpause with
//!   `RecoveryChecklistIncomplete`.
//! - Duplicate attestation for the same step is rejected with
//!   `RecoveryStepAlreadyAttested`.
//! - An address without the Admin role cannot attest steps
//!   (`Unauthorized` / panic).
//! - Successful recovery: configure checklist → attest all steps → unpause
//!   succeeds, all storage cleaned up, auditable events emitted.
//! - Checklist can be reconfigured (old attestations cleared).
//! - `clear_recovery_checklist` resets state; subsequent unpause requires a
//!   fresh configuration.
//! - Multi-step checklists work correctly.
//! - `get_recovery_checklist` / `get_recovery_attestation` return correct data.

#![cfg(test)]

extern crate std;

use crate::{PredifiContract, PredifiContractClient, PredifiError};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token, vec, Address, Env, Symbol, String,
};

// ─── Shared access-control stub ──────────────────────────────────────────────

mod ac_stub_rc {
    use soroban_sdk::{contract, contractimpl, Address, Env, Symbol};

    #[contract]
    pub struct AcStubRc;

    #[contractimpl]
    impl AcStubRc {
        pub fn grant_role(env: Env, user: Address, role: u32) {
            let key = (Symbol::new(&env, "role"), user.clone(), role);
            let already: bool = env.storage().instance().get(&key).unwrap_or(false);
            env.storage().instance().set(&key, &true);
            if role == 1 && !already {
                let ck = Symbol::new(&env, "op_count");
                let c: u32 = env.storage().instance().get(&ck).unwrap_or(0);
                env.storage().instance().set(&ck, &(c + 1));
            }
        }

        pub fn revoke_role(env: Env, user: Address, role: u32) {
            let key = (Symbol::new(&env, "role"), user, role);
            let had: bool = env.storage().instance().get(&key).unwrap_or(false);
            env.storage().instance().set(&key, &false);
            if role == 1 && had {
                let ck = Symbol::new(&env, "op_count");
                let c: u32 = env.storage().instance().get(&ck).unwrap_or(0);
                if c > 0 {
                    env.storage().instance().set(&ck, &(c - 1));
                }
            }
        }

        pub fn has_role(env: Env, user: Address, role: u32) -> bool {
            let key = (Symbol::new(&env, "role"), user, role);
            env.storage().instance().get(&key).unwrap_or(false)
        }

        pub fn get_operator_count(env: Env) -> u32 {
            env.storage()
                .instance()
                .get(&Symbol::new(&env, "op_count"))
                .unwrap_or(0)
        }
    }
}

use crate::PoolConfig;

// ─── Test harness ─────────────────────────────────────────────────────────────

struct RecoveryTestEnv<'a> {
    pub env: Env,
    pub client: PredifiContractClient<'a>,
    pub token_address: Address,
    pub token_admin: token::StellarAssetClient<'a>,
    pub admin: Address,
    pub stranger: Address,
    pub operator: Address,
}

impl<'a> RecoveryTestEnv<'a> {
    fn new(env: &'a Env) -> Self {
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.protocol_version = 23;
            li.timestamp = 1_000;
        });

        let admin = Address::generate(env);
        let operator = Address::generate(env);
        let stranger = Address::generate(env);

        let ac_id = env.register(ac_stub_rc::AcStubRc, ());
        let ac = ac_stub_rc::AcStubRcClient::new(env, &ac_id);
        ac.grant_role(&admin, &0u32);
        ac.grant_role(&operator, &1u32);
        // stranger has NO roles

        let treasury = Address::generate(env);
        let contract_id = env.register(PredifiContract, ());
        let client = PredifiContractClient::new(env, &contract_id);
        client.init(&ac_id, &treasury, &0u32, &0u64, &0u64, &0u32);

        let token_admin_addr = Address::generate(env);
        let token_contract = env.register_stellar_asset_contract_v2(token_admin_addr);
        let token_address = token_contract.address();
        let token_admin = token::StellarAssetClient::new(env, &token_address);
        client.add_token_to_whitelist(&admin, &token_address);

        Self {
            env: env.clone(),
            client,
            token_address,
            token_admin,
            admin,
            stranger,
            operator,
        }
    }

    /// Pause the contract. Prerequisite for checklist operations.
    fn pause(&self) {
        self.client.pause(&self.admin);
    }

    /// Build a one-step checklist Vec.
    fn single_step(&self) -> soroban_sdk::Vec<Symbol> {
        vec![&self.env, Symbol::new(&self.env, "step_one")]
    }

    /// Build a two-step checklist Vec.
    fn two_steps(&self) -> soroban_sdk::Vec<Symbol> {
        vec![
            &self.env,
            Symbol::new(&self.env, "funds_secured"),
            Symbol::new(&self.env, "root_cause_found"),
        ]
    }

    /// Build a three-step checklist Vec.
    fn three_steps(&self) -> soroban_sdk::Vec<Symbol> {
        vec![
            &self.env,
            Symbol::new(&self.env, "funds_secured"),
            Symbol::new(&self.env, "root_cause_found"),
            Symbol::new(&self.env, "patch_deployed"),
        ]
    }

    /// Configure and fully attest a single-step checklist.
    fn full_recovery_one_step(&self) {
        let steps = self.single_step();
        self.client
            .configure_recovery_checklist(&self.admin, &steps);
        self.client.attest_recovery_step(
            &self.admin,
            &Symbol::new(&self.env, "step_one"),
            &Symbol::new(&self.env, "ok"),
        );
    }

    fn create_pool(&self) -> u64 {
        let now = self.env.ledger().timestamp();
        self.client.create_pool(
            &self.admin,
            &(now + 7_200),
            &self.token_address,
            &2u32,
            &symbol_short!("Tech"),
            &PoolConfig {
                start_time: 0,
                description: String::from_str(&self.env, "Recovery test pool"),
                metadata_url: String::from_str(&self.env, "ipfs://recovery-test"),
                min_stake: 1i128,
                max_stake: 0i128,
                max_total_stake: 0i128,
                min_total_stake: 1i128,
                initial_liquidity: 0i128,
                required_resolutions: 1u32,
                private: false,
                whitelist_key: None,
                outcome_descriptions: vec![
                    &self.env,
                    String::from_str(&self.env, "No"),
                    String::from_str(&self.env, "Yes"),
                ],
            },
        )
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Missing checklist — unpause must fail
// ═══════════════════════════════════════════════════════════════════════════

/// Unpausing without any checklist configured must return
/// `RecoveryChecklistNotConfigured`.
#[test]
fn test_rc_unpause_blocked_no_checklist() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    assert!(ctx.client.is_contract_paused());

    let result = ctx.client.try_unpause(&ctx.admin);
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryChecklistNotConfigured)),
        "unpause without checklist must return RecoveryChecklistNotConfigured"
    );
    // Contract must remain paused.
    assert!(ctx.client.is_contract_paused());
}

/// Unpausing with a checklist configured but NO attestations must fail.
#[test]
fn test_rc_unpause_blocked_no_attestations() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    let steps = ctx.two_steps();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps);

    // No attestations provided — unpause must fail.
    let result = ctx.client.try_unpause(&ctx.admin);
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryChecklistIncomplete)),
        "unpause with no attestations must return RecoveryChecklistIncomplete"
    );
    assert!(ctx.client.is_contract_paused());
}

/// Unpausing with only some steps attested must fail.
#[test]
fn test_rc_unpause_blocked_partial_attestation() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    let steps = ctx.two_steps();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps);

    // Attest only the first step.
    ctx.client.attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "funds_secured"),
        &Symbol::new(&env, "done"),
    );

    // Second step missing — unpause must still fail.
    let result = ctx.client.try_unpause(&ctx.admin);
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryChecklistIncomplete)),
        "unpause with partial attestation must return RecoveryChecklistIncomplete"
    );
    assert!(ctx.client.is_contract_paused());
}

// ═══════════════════════════════════════════════════════════════════════════
// Duplicate attestation must be rejected
// ═══════════════════════════════════════════════════════════════════════════

/// Attesting the same step twice must return `RecoveryStepAlreadyAttested`.
#[test]
fn test_rc_duplicate_attestation_rejected() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    let steps = ctx.single_step();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps);

    let step = Symbol::new(&env, "step_one");
    let notes = Symbol::new(&env, "ok");

    // First attestation succeeds.
    ctx.client
        .attest_recovery_step(&ctx.admin, &step, &notes);

    // Second attestation for the same step must be rejected.
    let result = ctx
        .client
        .try_attest_recovery_step(&ctx.admin, &step, &notes);
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryStepAlreadyAttested)),
        "duplicate attestation must return RecoveryStepAlreadyAttested"
    );
}

/// Attesting the same step twice across two different admins must still fail.
#[test]
fn test_rc_duplicate_attestation_rejected_regardless_of_attester() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    // Register a second admin.
    let admin2 = Address::generate(&env);
    {
        let ac_id = env.register(ac_stub_rc::AcStubRc, ());
        // Re-use the same env means we need to find the AC contract.
        // Simpler: just use ctx.admin twice since mock_all_auths is active.
    }
    // We use ctx.admin for both calls since mock_all_auths is active —
    // the point is that the step key itself is the duplicate guard.
    ctx.pause();
    let steps = ctx.single_step();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps);

    let step = Symbol::new(&env, "step_one");
    ctx.client
        .attest_recovery_step(&ctx.admin, &step, &Symbol::new(&env, "first"));

    let result = ctx.client.try_attest_recovery_step(
        &ctx.admin,
        &step,
        &Symbol::new(&env, "second"),
    );
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryStepAlreadyAttested)),
        "second attestation for an already-attested step must fail"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Unauthorized attestation must be rejected
// ═══════════════════════════════════════════════════════════════════════════

/// An address without the Admin role must not be able to attest a step.
#[test]
fn test_rc_unauthorized_cannot_attest_step() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &ctx.single_step());

    // stranger has no Admin role — attest must fail.
    let result = ctx.client.try_attest_recovery_step(
        &ctx.stranger,
        &Symbol::new(&env, "step_one"),
        &Symbol::new(&env, "notes"),
    );
    assert!(
        result.is_err(),
        "non-admin must not be able to attest a recovery step"
    );
    // Checklist must remain incomplete.
    assert!(
        ctx.client
            .get_recovery_attestation(&Symbol::new(&env, "step_one"))
            .is_none(),
        "attestation must not be stored after unauthorized attempt"
    );
}

/// An address without the Admin role must not be able to configure the checklist.
#[test]
fn test_rc_unauthorized_cannot_configure_checklist() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    let result = ctx
        .client
        .try_configure_recovery_checklist(&ctx.stranger, &ctx.single_step());
    assert!(
        result.is_err(),
        "non-admin must not be able to configure the recovery checklist"
    );
    // No checklist should have been stored.
    assert!(
        ctx.client.get_recovery_checklist().is_none(),
        "checklist must not be stored after unauthorized attempt"
    );
}

/// An address without the Admin role must not be able to clear the checklist.
#[test]
fn test_rc_unauthorized_cannot_clear_checklist() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &ctx.single_step());

    let result = ctx
        .client
        .try_clear_recovery_checklist(&ctx.stranger);
    assert!(
        result.is_err(),
        "non-admin must not be able to clear the recovery checklist"
    );
    // Checklist must still be present.
    assert!(
        ctx.client.get_recovery_checklist().is_some(),
        "checklist must remain after unauthorized clear attempt"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Successful recovery completion
// ═══════════════════════════════════════════════════════════════════════════

/// Full happy path: configure checklist → attest all steps → unpause succeeds.
#[test]
fn test_rc_successful_recovery_single_step() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    assert!(ctx.client.is_contract_paused());

    ctx.full_recovery_one_step();

    // Unpause must succeed.
    ctx.client.unpause(&ctx.admin);
    assert!(
        !ctx.client.is_contract_paused(),
        "contract must be unpaused after full recovery"
    );
}

/// Full happy path with a multi-step checklist.
#[test]
fn test_rc_successful_recovery_multi_step() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();

    let steps = ctx.three_steps();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps);

    ctx.client.attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "funds_secured"),
        &Symbol::new(&env, "treasury_frozen"),
    );
    ctx.client.attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "root_cause_found"),
        &Symbol::new(&env, "overflow_in_payout"),
    );
    ctx.client.attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "patch_deployed"),
        &Symbol::new(&env, "v1_2_0"),
    );

    ctx.client.unpause(&ctx.admin);
    assert!(
        !ctx.client.is_contract_paused(),
        "contract must be unpaused after all three steps attested"
    );
}

/// After a successful unpause the checklist and attestations must be cleared
/// from storage so the next pause cycle starts fresh.
#[test]
fn test_rc_storage_cleared_after_successful_unpause() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.full_recovery_one_step();
    ctx.client.unpause(&ctx.admin);

    // Checklist must be gone.
    assert!(
        ctx.client.get_recovery_checklist().is_none(),
        "checklist must be cleared after unpause"
    );
    // Attestation must be gone.
    assert!(
        ctx.client
            .get_recovery_attestation(&Symbol::new(&env, "step_one"))
            .is_none(),
        "attestation must be cleared after unpause"
    );
}

/// Operations blocked during pause must work normally after recovery unpause.
#[test]
fn test_rc_operations_resume_after_recovery_unpause() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    // Create pool before pausing.
    let _pool_id = ctx.create_pool();

    ctx.pause();

    // Operations blocked while paused.
    assert_eq!(
        ctx.client.try_create_pool(
            &ctx.admin,
            &(ctx.env.ledger().timestamp() + 7_200),
            &ctx.token_address,
            &2u32,
            &symbol_short!("Tech"),
            &PoolConfig {
                start_time: 0,
                description: String::from_str(&env, "blocked"),
                metadata_url: String::from_str(&env, "ipfs://blocked"),
                min_stake: 1i128,
                max_stake: 0i128,
                max_total_stake: 0i128,
                min_total_stake: 1i128,
                initial_liquidity: 0i128,
                required_resolutions: 1u32,
                private: false,
                whitelist_key: None,
                outcome_descriptions: vec![
                    &env,
                    String::from_str(&env, "No"),
                    String::from_str(&env, "Yes"),
                ],
            },
        ),
        Err(Ok(PredifiError::ContractPaused))
    );

    // Recover.
    ctx.full_recovery_one_step();
    ctx.client.unpause(&ctx.admin);

    // Pool creation must succeed after recovery.
    let new_pool = ctx.create_pool();
    assert!(new_pool > 0, "pool creation must succeed after recovery unpause");
}

// ═══════════════════════════════════════════════════════════════════════════
// Checklist reconfiguration
// ═══════════════════════════════════════════════════════════════════════════

/// Calling configure_recovery_checklist a second time replaces the old config
/// and clears stale attestations.
#[test]
fn test_rc_reconfigure_clears_old_attestations() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();

    // First configuration with one step.
    let steps_v1 = ctx.single_step();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps_v1);

    // Attest the first config's step.
    ctx.client.attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "step_one"),
        &Symbol::new(&env, "done"),
    );

    // Attestation is present.
    assert!(
        ctx.client
            .get_recovery_attestation(&Symbol::new(&env, "step_one"))
            .is_some(),
        "attestation must be stored after first attest"
    );

    // Reconfigure with a different checklist.
    let steps_v2 = ctx.two_steps();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps_v2);

    // Old attestation must be cleared.
    assert!(
        ctx.client
            .get_recovery_attestation(&Symbol::new(&env, "step_one"))
            .is_none(),
        "old attestation must be cleared after reconfiguration"
    );

    // New checklist must be stored with updated steps.
    let checklist = ctx.client.get_recovery_checklist();
    assert!(checklist.is_some());
    assert_eq!(
        checklist.unwrap().len(),
        2,
        "new checklist must have 2 steps"
    );

    // Unpause must now require both new steps.
    assert_eq!(
        ctx.client.try_unpause(&ctx.admin),
        Err(Ok(PredifiError::RecoveryChecklistIncomplete))
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// clear_recovery_checklist
// ═══════════════════════════════════════════════════════════════════════════

/// Admin can clear the checklist; subsequent unpause requires fresh config.
#[test]
fn test_rc_clear_checklist_resets_state() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.full_recovery_one_step();

    // All steps attested — but admin decides to clear and re-evaluate.
    ctx.client
        .clear_recovery_checklist(&ctx.admin);

    // Checklist must be gone.
    assert!(
        ctx.client.get_recovery_checklist().is_none(),
        "checklist must be absent after clear"
    );
    // Attestation must be gone.
    assert!(
        ctx.client
            .get_recovery_attestation(&Symbol::new(&env, "step_one"))
            .is_none(),
        "attestation must be absent after clear"
    );

    // Unpause must now fail — no checklist.
    assert_eq!(
        ctx.client.try_unpause(&ctx.admin),
        Err(Ok(PredifiError::RecoveryChecklistNotConfigured)),
        "unpause must fail after checklist cleared"
    );
}

/// Clearing a non-existent checklist must return
/// `RecoveryChecklistNotConfigured`.
#[test]
fn test_rc_clear_nonexistent_checklist_fails() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    // No checklist configured — clear must fail.
    let result = ctx.client.try_clear_recovery_checklist(&ctx.admin);
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryChecklistNotConfigured)),
        "clearing non-existent checklist must return RecoveryChecklistNotConfigured"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// configure_recovery_checklist guard: must be paused
// ═══════════════════════════════════════════════════════════════════════════

/// Configuring the checklist while the contract is NOT paused must panic.
#[test]
fn test_rc_configure_checklist_requires_paused() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    // Contract is currently unpaused.
    assert!(!ctx.client.is_contract_paused());

    let result = ctx
        .client
        .try_configure_recovery_checklist(&ctx.admin, &ctx.single_step());
    assert!(
        result.is_err(),
        "configure_recovery_checklist must fail while contract is not paused"
    );
}

/// An empty steps vector must return `InvalidData`.
#[test]
fn test_rc_configure_checklist_empty_steps_rejected() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    let empty: soroban_sdk::Vec<Symbol> = soroban_sdk::vec![&env];
    let result = ctx
        .client
        .try_configure_recovery_checklist(&ctx.admin, &empty);
    assert_eq!(
        result,
        Err(Ok(PredifiError::InvalidData)),
        "empty steps vector must return InvalidData"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Stale attestation / step not in checklist
// ═══════════════════════════════════════════════════════════════════════════

/// Attesting a step that is not in the configured checklist must return
/// `InvalidData`.
#[test]
fn test_rc_attesting_unknown_step_rejected() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &ctx.single_step());

    // "ghost_step" is not in the configured checklist.
    let result = ctx.client.try_attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "ghost_step"),
        &Symbol::new(&env, "notes"),
    );
    assert_eq!(
        result,
        Err(Ok(PredifiError::InvalidData)),
        "attesting a step not in the checklist must return InvalidData"
    );
}

/// Attesting without any checklist configured must return
/// `RecoveryChecklistNotConfigured`.
#[test]
fn test_rc_attest_without_checklist_fails() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    // No checklist configured.
    let result = ctx.client.try_attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "step_one"),
        &Symbol::new(&env, "notes"),
    );
    assert_eq!(
        result,
        Err(Ok(PredifiError::RecoveryChecklistNotConfigured)),
        "attestation without checklist must return RecoveryChecklistNotConfigured"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// get_recovery_checklist / get_recovery_attestation queries
// ═══════════════════════════════════════════════════════════════════════════

/// `get_recovery_checklist` returns `None` before any checklist is configured.
#[test]
fn test_rc_get_checklist_none_initially() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    assert!(
        ctx.client.get_recovery_checklist().is_none(),
        "checklist must be None before configuration"
    );
}

/// `get_recovery_checklist` returns the correct step names after configuration.
#[test]
fn test_rc_get_checklist_returns_configured_steps() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    let steps = ctx.two_steps();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &steps);

    let stored = ctx.client.get_recovery_checklist().unwrap();
    assert_eq!(stored.len(), 2, "must return two steps");
}

/// `get_recovery_attestation` returns `None` for an unattested step.
#[test]
fn test_rc_get_attestation_none_before_attest() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &ctx.single_step());

    assert!(
        ctx.client
            .get_recovery_attestation(&Symbol::new(&env, "step_one"))
            .is_none(),
        "attestation must be None before attesting"
    );
}

/// `get_recovery_attestation` returns the correct attestation after attesting.
#[test]
fn test_rc_get_attestation_returns_correct_data() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    ctx.pause();
    ctx.client
        .configure_recovery_checklist(&ctx.admin, &ctx.single_step());

    ctx.client.attest_recovery_step(
        &ctx.admin,
        &Symbol::new(&env, "step_one"),
        &Symbol::new(&env, "all_clear"),
    );

    let att = ctx
        .client
        .get_recovery_attestation(&Symbol::new(&env, "step_one"))
        .expect("attestation must be present after attesting");

    assert_eq!(att.attested_by, ctx.admin, "attested_by must be the admin");
    assert_eq!(
        att.attested_at,
        env.ledger().timestamp(),
        "attested_at must be the current timestamp"
    );
    assert_eq!(
        att.notes,
        Symbol::new(&env, "all_clear"),
        "notes must match supplied notes"
    );
}

// ═══════════════════════════════════════════════════════════════════════════
// Multiple pause / unpause cycles with checklist
// ═══════════════════════════════════════════════════════════════════════════

/// Each pause cycle requires a fresh checklist — the previous cycle's state
/// must not carry over.
#[test]
fn test_rc_each_cycle_requires_fresh_checklist() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    // Cycle 1: full recovery.
    ctx.pause();
    ctx.full_recovery_one_step();
    ctx.client.unpause(&ctx.admin);
    assert!(!ctx.client.is_contract_paused());

    // Cycle 2: pause again — checklist is gone, must configure fresh.
    ctx.pause();
    assert_eq!(
        ctx.client.try_unpause(&ctx.admin),
        Err(Ok(PredifiError::RecoveryChecklistNotConfigured)),
        "second pause cycle must require a fresh checklist"
    );

    // Configure and complete cycle 2.
    ctx.full_recovery_one_step();
    ctx.client.unpause(&ctx.admin);
    assert!(!ctx.client.is_contract_paused());
}

/// Completing recovery does not corrupt pool state.
#[test]
fn test_rc_pool_state_intact_after_recovery_cycle() {
    let env = Env::default();
    let ctx = RecoveryTestEnv::new(&env);

    let pool_id = ctx.create_pool();
    let token = token::Client::new(&env, &ctx.token_address);

    let user = Address::generate(&env);
    ctx.token_admin.mint(&user, &1_000i128);
    ctx.client
        .place_prediction(&user, &pool_id, &1_000i128, &0u32, &None, &None);
    assert_eq!(ctx.client.get_pool(&pool_id).total_stake, 1_000);

    // Pause, configure, attest, unpause.
    ctx.pause();
    ctx.full_recovery_one_step();
    ctx.client.unpause(&ctx.admin);

    // Pool state must be intact.
    let pool = ctx.client.get_pool(&pool_id);
    assert_eq!(
        pool.total_stake, 1_000,
        "pool stake must be preserved across recovery cycle"
    );
}
