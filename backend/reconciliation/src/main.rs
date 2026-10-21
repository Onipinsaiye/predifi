use predifi_reconciliation::{
    rpc::SorobanReader, run_pass, store::PgProjectionStore, Error, Policy, INITIAL_CURSOR,
};
use sqlx::postgres::PgPoolOptions;
use std::{env, time::Duration};

fn number(key: &str, default: u64) -> Result<u64, Error> {
    match env::var(key) {
        Ok(value) => value.parse().map_err(|_| format!("invalid {key}").into()),
        Err(env::VarError::NotPresent) => Ok(default),
        Err(_) => Err(format!("invalid {key}").into()),
    }
}

#[tokio::main]
async fn main() -> Result<(), Error> {
    let args: Vec<_> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg != "--once") || args.len() > 1 {
        return Err("usage: predifi-reconciliation [--once]".into());
    }
    let once = !args.is_empty();
    let policy = Policy {
        sample_size: usize::try_from(number("RECONCILE_SAMPLE_SIZE", 25)?)?,
        interval: Duration::from_secs(number("RECONCILE_INTERVAL_SECS", 300)?),
        rpc_spacing: Duration::from_millis(number("RECONCILE_RPC_SPACING_MS", 500)?),
        rpc_timeout: Duration::from_secs(number("RECONCILE_RPC_TIMEOUT_SECS", 10)?),
    };
    policy.validate()?;
    let contract_id = env::var("RECONCILE_CONTRACT_ID")?;
    let mut reader =
        SorobanReader::new(&env::var("PREDIFI_STELLAR_RPC_URL")?, &contract_id, &policy)?;
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(Duration::from_secs(5))
        .connect(&env::var("PREDIFI_DATABASE_URL")?)
        .await?;
    let store = PgProjectionStore::new(pool, contract_id.clone());
    let mut cursor = INITIAL_CURSOR;
    // Cancellation drops in-flight DB/RPC futures. No background work survives.
    let worker = async {
        loop {
            match run_pass(&store, &mut reader, &contract_id, &mut cursor, &policy).await {
                Ok(report) => {
                    println!("{}", serde_json::to_string(&report)?);
                    if once && !report.findings.is_empty() {
                        return Err("reconciliation produced alerts; see JSON report".into());
                    }
                }
                Err(error) if once => return Err(error),
                Err(error) => eprintln!(
                    "{}",
                    serde_json::json!({
                        "event": "reconciliation_pass_failed",
                        "contract_id": contract_id,
                        "cursor": cursor,
                        "observed_at": chrono::Utc::now(),
                        "error": error.to_string()
                    })
                ),
            }
            if once {
                return Ok::<(), Error>(());
            }
            // Delay after completion prevents overlap and catch-up bursts.
            tokio::time::sleep(policy.interval).await;
        }
    };
    tokio::select! {
        result = worker => result,
        result = tokio::signal::ctrl_c() => result.map_err(Into::into),
    }
}
