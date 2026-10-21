//! Bounded, read-only reconciliation of indexed pools with Soroban storage.

pub mod rpc;
pub mod store;

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{future::Future, time::Duration};
use uuid::Uuid;

pub type Error = Box<dyn std::error::Error + Send + Sync>;

/// Pool IDs start at zero in the contract; -1 is the beginning of a sweep.
pub const INITIAL_CURSOR: i64 = -1;

/// Hard limits apply even when constructing the worker outside the binary.
#[derive(Clone, Debug)]
pub struct Policy {
    pub sample_size: usize,
    pub interval: Duration,
    pub rpc_spacing: Duration,
    pub rpc_timeout: Duration,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            sample_size: 25,
            interval: Duration::from_secs(300),
            rpc_spacing: Duration::from_millis(500),
            rpc_timeout: Duration::from_secs(10),
        }
    }
}

impl Policy {
    pub fn validate(&self) -> Result<(), Error> {
        if !(1..=100).contains(&self.sample_size)
            || !(Duration::from_secs(60)..=Duration::from_secs(86400)).contains(&self.interval)
            || !(Duration::from_millis(500)..=Duration::from_secs(10)).contains(&self.rpc_spacing)
            || !(Duration::from_millis(1)..=Duration::from_secs(10)).contains(&self.rpc_timeout)
        {
            return Err("invalid reconciliation limits; see reconciliation/README.md".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, sqlx::FromRow)]
pub struct IndexedPool {
    pub pool_id: i64,
    /// Raw integer token base units, selected as text to preserve NUMERIC precision.
    pub total_stake: String,
    pub state: String,
    pub result: Option<String>,
    pub observed_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ContractPool {
    pub total_stake: String,
    /// The database lifecycle representation (active / settled / closed / disputed).
    pub state: String,
    pub result: Option<String>,
    pub latest_ledger: u32,
    pub last_modified_ledger: u32,
}

#[derive(Clone, Debug)]
pub enum ContractRead {
    Found(ContractPool),
    /// Absent live storage may mean archival, not deletion.
    Unavailable {
        latest_ledger: u32,
    },
}

pub trait ProjectionStore {
    fn sample(
        &self,
        after: i64,
        limit: usize,
    ) -> impl Future<Output = Result<Vec<IndexedPool>, Error>> + Send;
}

pub trait ContractReader {
    fn read_pool(
        &mut self,
        pool_id: i64,
    ) -> impl Future<Output = Result<ContractRead, Error>> + Send;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    TotalStake,
    Lifecycle,
    Resolution,
    InvalidProjection,
    ContractUnavailable,
    ContractReadFailed,
}

/// Every alert retains both observations; there is intentionally no repair API.
#[derive(Debug, Serialize)]
pub struct Finding {
    pub category: Category,
    pub indexed: IndexedPool,
    pub contract: Option<ContractPool>,
    pub latest_ledger: Option<u32>,
    pub detail: String,
}

#[derive(Debug, Serialize)]
pub struct Report {
    pub run_id: Uuid,
    pub contract_id: String,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub cursor_before: i64,
    pub cursor_after: i64,
    pub sampled: usize,
    pub matched: usize,
    pub policy: &'static str,
    pub findings: Vec<Finding>,
}

/// Runs one bounded pass. The cursor advances even on failed reads, so one bad
/// pool cannot starve the rest. An empty page resets it for the following pass.
/// The caller must serialize passes and wait the policy interval between them.
pub async fn run_pass(
    store: &impl ProjectionStore,
    reader: &mut impl ContractReader,
    contract_id: &str,
    cursor: &mut i64,
    policy: &Policy,
) -> Result<Report, Error> {
    policy.validate()?;
    let started_at = Utc::now();
    let rows = store.sample(*cursor, policy.sample_size).await?;
    if rows.len() > policy.sample_size
        || rows.iter().any(|row| row.pool_id <= *cursor)
        || rows
            .windows(2)
            .any(|pair| pair[0].pool_id >= pair[1].pool_id)
    {
        return Err("projection store returned an invalid bounded keyset page".into());
    }
    let mut report = Report {
        run_id: Uuid::new_v4(),
        contract_id: contract_id.to_owned(),
        started_at,
        finished_at: started_at,
        cursor_before: *cursor,
        cursor_after: rows.last().map_or(INITIAL_CURSOR, |row| row.pool_id),
        sampled: rows.len(),
        matched: 0,
        policy: "alert_only; operator-reviewed event replay required for repair",
        findings: Vec::new(),
    };
    for indexed in rows {
        let findings_before = report.findings.len();
        match reader.read_pool(indexed.pool_id).await {
            Ok(ContractRead::Found(contract)) => {
                report.findings.extend(compare(&indexed, &contract));
                if report.findings.len() == findings_before {
                    report.matched += 1;
                }
            }
            Ok(ContractRead::Unavailable { latest_ledger }) => {
                report.findings.push(Finding {
                    category: Category::ContractUnavailable,
                    indexed,
                    contract: None,
                    latest_ledger: Some(latest_ledger),
                    detail: "live Pool key absent; investigate network, contract ID, or archival; never delete indexed history".into(),
                });
            }
            Err(error) => report.findings.push(Finding {
                category: Category::ContractReadFailed,
                indexed,
                contract: None,
                latest_ledger: None,
                detail: error.to_string(),
            }),
        }
    }
    report.finished_at = Utc::now();
    *cursor = report.cursor_after;
    Ok(report)
}

fn compare(indexed: &IndexedPool, contract: &ContractPool) -> Vec<Finding> {
    let mut findings = Vec::new();
    let mut add = |category, detail: &str| {
        findings.push(Finding {
            category,
            indexed: indexed.clone(),
            contract: Some(contract.clone()),
            latest_ledger: Some(contract.latest_ledger),
            detail: detail.to_owned(),
        });
    };
    match integer_stake(&indexed.total_stake) {
        Some(stake) if stake.to_string() == contract.total_stake => {}
        Some(_) => add(
            Category::TotalStake,
            "total_stake differs in raw token base units",
        ),
        None => add(
            Category::InvalidProjection,
            "indexed total_stake is not a nonnegative i128 integer in raw token base units",
        ),
    }
    if indexed.state != contract.state {
        add(
            Category::Lifecycle,
            "indexed lifecycle differs from contract state",
        );
    }
    if indexed.result != contract.result {
        add(
            Category::Resolution,
            "indexed winning outcome differs from contract resolution",
        );
    }
    findings
}

fn integer_stake(value: &str) -> Option<i128> {
    let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
    if !fraction.bytes().all(|b| b == b'0') {
        return None;
    }
    integer.parse::<i128>().ok().filter(|amount| *amount >= 0)
}
