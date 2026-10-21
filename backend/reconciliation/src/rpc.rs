//! Reads the same persistent `DataKey::Pool(u64)` entry used by `get_pool`.
//! No simulation placeholder, signing key, transaction submission, or TTL write.

use crate::{ContractPool, ContractRead, ContractReader, Error, Policy};
use serde::Deserialize;
use std::time::Duration;
use stellar_xdr::curr::{
    ContractDataDurability, ContractId, Hash, LedgerEntryData, LedgerKey, LedgerKeyContractData,
    Limits, ReadXdr, ScAddress, ScSymbol, ScVal, WriteXdr,
};
use tokio::time::{sleep_until, Instant};

const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

fn limits() -> Limits {
    Limits {
        depth: 64,
        len: MAX_RESPONSE_BYTES,
    }
}

/// Encoding is tied to this repository's contract schema, not an RPC JSON shape.
pub fn pool_key(contract_id: &str, pool_id: i64) -> Result<LedgerKey, Error> {
    let id = stellar_strkey::Contract::from_string(contract_id)
        .map_err(|_| "invalid Soroban contract ID")?;
    let pool_id = u64::try_from(pool_id).map_err(|_| "negative pool ID")?;
    Ok(LedgerKey::ContractData(LedgerKeyContractData {
        contract: ScAddress::Contract(ContractId(Hash(id.0))),
        key: ScVal::Vec(Some(
            vec![
                ScVal::Symbol(ScSymbol("Pool".try_into()?)),
                ScVal::U64(pool_id),
            ]
            .try_into()?,
        )),
        durability: ContractDataDurability::Persistent,
    }))
}

pub struct SorobanReader {
    client: reqwest::Client,
    url: reqwest::Url,
    contract_id: String,
    spacing: Duration,
    timeout: Duration,
    next_request: Instant,
}

impl SorobanReader {
    pub fn new(url: &str, contract_id: &str, policy: &Policy) -> Result<Self, Error> {
        policy.validate()?;
        pool_key(contract_id, 0)?;
        let url = reqwest::Url::parse(url)?;
        if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
            return Err("RPC URL must have an HTTP(S) scheme and host".into());
        }
        Ok(Self {
            client: reqwest::Client::builder()
                .timeout(policy.rpc_timeout)
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
            url,
            contract_id: contract_id.to_owned(),
            spacing: policy.rpc_spacing,
            timeout: policy.rpc_timeout,
            next_request: Instant::now(),
        })
    }

    async fn fetch(&self, key: &str) -> Result<RpcResult, Error> {
        let mut response = self
            .client
            .post(self.url.clone())
            .json(&serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "getLedgerEntries",
                "params": {"keys": [key]}
            }))
            .send()
            .await
            .map_err(|_| "RPC transport failure or timeout")?;
        if !response.status().is_success() {
            return Err(format!("RPC HTTP {}", response.status().as_u16()).into());
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| "RPC body read failed")? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err("RPC response exceeds 1 MiB".into());
            }
            body.extend_from_slice(&chunk);
        }
        let response: RpcResponse =
            serde_json::from_slice(&body).map_err(|_| "malformed RPC response")?;
        if response.jsonrpc != "2.0" || response.id != 1 {
            return Err("RPC envelope identity mismatch".into());
        }
        if let Some(error) = response.error {
            // Do not log server-controlled messages or endpoint credentials.
            return Err(format!("RPC application error {}", error.code).into());
        }
        response.result.ok_or_else(|| "RPC result missing".into())
    }
}

impl ContractReader for SorobanReader {
    async fn read_pool(&mut self, pool_id: i64) -> Result<ContractRead, Error> {
        let key = pool_key(&self.contract_id, pool_id)?;
        let encoded = key.to_xdr_base64(limits())?;
        sleep_until(self.next_request).await;
        self.next_request = Instant::now() + self.spacing;
        let result = tokio::time::timeout(self.timeout, self.fetch(&encoded))
            .await
            .map_err(|_| "RPC request timed out")??;
        if result.latest_ledger == 0 {
            return Err("RPC latestLedger must be positive".into());
        }
        if result.entries.is_empty() {
            return Ok(ContractRead::Unavailable {
                latest_ledger: result.latest_ledger,
            });
        }
        if result.entries.len() != 1 {
            return Err("RPC returned unexpected number of entries".into());
        }
        let entry = &result.entries[0];
        if LedgerKey::from_xdr_base64(&entry.key, limits())? != key
            || entry.last_modified_ledger_seq > result.latest_ledger
        {
            return Err("RPC entry key or ledger mismatch".into());
        }
        let data = LedgerEntryData::from_xdr_base64(&entry.xdr, limits())?;
        let (LedgerKey::ContractData(expected), LedgerEntryData::ContractData(data)) = (key, data)
        else {
            return Err("RPC entry is not contract data".into());
        };
        if data.contract != expected.contract
            || data.key != expected.key
            || data.durability != expected.durability
        {
            return Err("RPC contract data identity mismatch".into());
        }
        let mut pool = decode_pool(&data.val)?;
        pool.latest_ledger = result.latest_ledger;
        pool.last_modified_ledger = entry.last_modified_ledger_seq;
        Ok(ContractRead::Found(pool))
    }
}

#[derive(Deserialize)]
struct RpcResponse {
    jsonrpc: String,
    id: u64,
    result: Option<RpcResult>,
    error: Option<RpcError>,
}

#[derive(Deserialize)]
struct RpcError {
    code: i64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcResult {
    latest_ledger: u32,
    entries: Vec<RpcEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RpcEntry {
    key: String,
    xdr: String,
    last_modified_ledger_seq: u32,
}

fn decode_pool(value: &ScVal) -> Result<ContractPool, Error> {
    let ScVal::Map(Some(fields)) = value else {
        return Err("Pool value is not an SCMap".into());
    };
    let field = |name: &str| -> Result<&ScVal, Error> {
        let key = ScVal::Symbol(ScSymbol(name.try_into()?));
        let mut entries = fields.iter().filter(|entry| entry.key == key);
        let value = &entries.next().ok_or("Pool field missing")?.val;
        if entries.next().is_some() {
            return Err("duplicate Pool field".into());
        }
        Ok(value)
    };
    let ScVal::I128(stake) = field("total_stake")? else {
        return Err("Pool total_stake is not i128".into());
    };
    let stake = (i128::from(stake.hi) << 64) | i128::from(stake.lo);
    if stake < 0 {
        return Err("negative contract stake".into());
    }
    // MarketState is a repr(u32) contract enum and is encoded as SCV_U32.
    let state = match field("state")? {
        ScVal::U32(0) => "active",
        ScVal::U32(1) => "settled",
        ScVal::U32(2) => "closed",
        ScVal::U32(3) => "disputed",
        _ => return Err("unknown contract MarketState encoding".into()),
    };
    let ScVal::U32(outcome) = field("outcome")? else {
        return Err("Pool outcome is not u32".into());
    };
    let result = if state == "settled" {
        if *outcome == u32::MAX {
            return Err("resolved Pool has unresolved outcome".into());
        }
        Some(outcome.to_string())
    } else {
        None
    };
    Ok(ContractPool {
        total_stake: stake.to_string(),
        state: state.into(),
        result,
        latest_ledger: 0,
        last_modified_ledger: 0,
    })
}
