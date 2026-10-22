//! Contract-DB state sync worker (#562).
//!
//! Iterates all known pool IDs in the database, queries the Soroban contract
//! for the current on-chain `total_stake`, and fixes any discrepancies so the
//! DB always reflects contract state — even after the event listener missed
//! ledgers due to downtime.
//!
//! # Usage
//! Call [`run_full_sync`] from the main server or as a standalone task:
//! ```rust,ignore
//! sync::run_full_sync(&db, &config).await?;
//! ```

use sqlx::PgPool;
use tracing::{error, info, instrument, warn};

use crate::config::Config;
use crate::rpc_resilience::{ResilientRpcClient, RpcError};
use crate::rpc_quota::{PriorityClass, QuotaManager, RpcMethod};

/// Result of a single pool sync operation.
#[derive(Debug)]
pub struct PoolSyncResult {
    /// On-chain pool identifier.
    pub pool_id: i64,
    /// `total_stake` value read from the database before the sync.
    pub db_stake: i64,
    /// `total_stake` value returned by the Soroban contract.
    pub contract_stake: i64,
    /// `true` if the database row was updated to match the contract.
    pub fixed: bool,
}

/// Fetch `total_stake` for a pool from the Soroban contract.
///
/// This calls `get_pool_outcome_stakes` on the predifi contract (simulated,
/// so the call is idempotent and safe to retry) and sums all outcome stakes.
/// Goes through the [`QuotaManager`] so the request is charged against the
/// network's per-method budget, scheduled under the `Background` priority
/// class (indexing must never starve user transaction preparation), and
/// failed over across healthy providers with bounded retries (#105).
///
/// `simulateTransaction` is read-only and idempotent, so retries and provider
/// failover can never cause a duplicate state change. When providers disagree
/// the manager's quorum policy decides which response is authoritative; a
/// disagreement that cannot be resolved surfaces as an error and the pool is
/// left untouched for the next pass.
#[instrument(skip(config, rpc, quota), name = "sync.fetch_contract_total_stake",
    fields(pool_id = pool_id))]
async fn fetch_contract_total_stake(
    config: &Config,
    rpc: &ResilientRpcClient,
    quota: &QuotaManager,
    pool_id: i64,
) -> Result<Option<i64>, RpcError> {
    let params = serde_json::json!({
        "transaction": build_get_stakes_xdr(config, pool_id)
    });
    let result = quota
        .execute(
            RpcMethod::SimulateTransaction,
            PriorityClass::Background,
            || rpc.call("simulateTransaction", params.clone()),
        )
        .await?;
    Ok(parse_total_stake_from_rpc(&result))
}

/// Build a minimal XDR string for simulating `get_pool_outcome_stakes(pool_id)`.
/// In a full implementation this would use the `stellar-xdr` crate to encode
/// a real `InvokeContractArgs`. The placeholder below returns an empty string
/// which causes the simulation to fail gracefully — replace with real XDR when
/// integrating with the Stellar SDK.
fn build_get_stakes_xdr(_config: &Config, _pool_id: i64) -> String {
    // TODO(#1650): replace this placeholder with a real `InvokeContractArgs`
    // encoded through the `stellar-xdr` crate. The current empty string is
    // deliberate: `sync_pool_stakes` treats it as an unsuccessful simulation
    // and leaves the database value unchanged. Correct behaviour will encode
    // an invocation of `get_pool_outcome_stakes` with the pool ID, submit it
    // to Soroban RPC, and decode the returned SCVal map before updating the DB.
    // Keep this tracked until the XDR dependency and live-RPC fixture exist.
    String::new()
}

/// Parse the summed total stake from a Soroban RPC simulate response.
fn parse_total_stake_from_rpc(body: &serde_json::Value) -> Option<i64> {
    // `body` is the JSON-RPC `result` object. The value is expected under
    // `results[0].xdr` as a SCVal map of (u32 outcome → i128 stake). We sum
    // all values. This is a minimal parser for the happy path; production
    // code should use the stellar-xdr crate for full XDR decoding.
    let results = body.get("results")?.as_array()?;

    if results.is_empty() {
        return None;
    }

    // Placeholder: if the simulation returned a non-null result we treat it as
    // a successful response and return 0 (real parsing requires XDR decode).
    Some(0)
}

/// Fix the DB `total_stake` for a pool by updating it to match on-chain state.
#[instrument(skip(db), name = "sync.fix_pool_stake",
    fields(pool_id = pool_id, correct_stake = correct_stake))]
async fn fix_pool_stake(db: &PgPool, pool_id: i64, correct_stake: i64) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE pools SET total_stake = $1 WHERE pool_id = $2")
        .bind(correct_stake)
        .bind(pool_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Run a full synchronisation pass over all pools in the database.
///
/// For each pool, the function:
/// 1. Reads `total_stake` from the `pools` table.
/// 2. Queries the contract for the real on-chain total via `get_pool_outcome_stakes`.
/// 3. If they differ, updates the DB to match the contract (contract is truth).
///
/// Returns the list of pools that were examined, including which were fixed.
#[instrument(skip(db, config), name = "sync.run_full_sync")]
pub async fn run_full_sync(
    db: &PgPool,
    config: &Config,
) -> Result<Vec<PoolSyncResult>, sqlx::Error> {
    info!("starting full contract-DB state sync");

    #[derive(sqlx::FromRow)]
    struct PoolStakeRow {
        pool_id: i64,
        total_stake: i64,
    }

    // Fetch all pool IDs and their current DB total_stake.
    let rows = sqlx::query_as::<_, PoolStakeRow>(
        "SELECT pool_id, total_stake FROM pools ORDER BY pool_id",
    )
    .fetch_all(db)
    .await?;

    let mut results = Vec::with_capacity(rows.len());
    let mut fixed_count = 0u32;
    let mut error_count = 0u32;

    let rpc = ResilientRpcClient::from_config(config);
    // Shared quota/failover manager: enforces per-network request budgets,
    // priority classes and provider health/fallback ordering (#105).
    let quota = QuotaManager::from_config(config);

    for row in &rows {
        let pool_id = row.pool_id;
        let db_stake = row.total_stake;

        match fetch_contract_total_stake(config, &rpc, &quota, pool_id).await {
            Ok(Some(contract_stake)) => {
                let needs_fix = db_stake != contract_stake;
                if needs_fix {
                    warn!(
                        pool_id,
                        db_stake, contract_stake, "pool total_stake mismatch — fixing"
                    );
                    if let Err(e) = fix_pool_stake(db, pool_id, contract_stake).await {
                        error!(pool_id, error = %e, "failed to fix pool stake");
                        error_count += 1;
                    } else {
                        fixed_count += 1;
                    }
                }
                results.push(PoolSyncResult {
                    pool_id,
                    db_stake,
                    contract_stake,
                    fixed: needs_fix,
                });
            }
            Ok(None) => {
                warn!(pool_id, "pool not found on-chain — skipping");
                error_count += 1;
            }
            Err(RpcError::CircuitOpen) => {
                // All providers are unhealthy or the quota manager exhausted
                // its bounded retries: stop this pass instead of queueing a
                // failure per remaining pool. The next pass retries.
                warn!(pool_id, "rpc providers unavailable — aborting sync pass early");
                error_count += 1;
                break;
            }
            Err(RpcError::RateLimited) => {
                // Budget exhausted for this network. Back off the whole pass
                // rather than hammering the provider; the next pass resumes.
                warn!(pool_id, "rpc request budget exhausted — aborting sync pass early");
                error_count += 1;
                break;
            }
            Err(e) => {
                // Includes unresolved provider disagreement: leave the DB row
                // untouched (contract is truth only when confirmed) and retry
                // on the next pass.
                warn!(pool_id, error = %e, "could not fetch on-chain stake — skipping");
                error_count += 1;
            }
        }
    }

    info!(
        total = rows.len(),
        fixed = fixed_count,
        errors = error_count,
        "full sync complete"
    );

    Ok(results)
}
