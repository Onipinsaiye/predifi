//! `predifi-seed` — database seed binary for local development (#1189).
//!
//! Loads the runtime [`Config`] from `.env`/process environment, connects to
//! the configured Postgres instance, runs all migrations so the schema is
//! ready, and inserts deterministic sample data via [`predifi_backend::seed`].
//!
//! Fixture loading is isolated by environment (#142): the binary refuses to run
//! against production configuration by default, and destructive operations
//! (`--fresh`) require an explicit environment confirmation.
//!
//! See `--help` for supported flags.

use std::process::ExitCode;

use predifi_backend::config::Config;
use predifi_backend::seed::{run_seed, SeedConfig, SeedScenario, DEFAULT_NUM_POOLS};
use tracing::{error, info};

/// Environment variable that must be set to `1`/`true` to allow destructive
/// seed operations (`--fresh`) against a non-production database.
const CONFIRM_ENV_VAR: &str = "PREDIFI_SEED_CONFIRM";

/// Environment variable that must be set to `1`/`true` to allow fixture
/// loading against a production-configured database at all.
const ALLOW_PROD_ENV_VAR: &str = "PREDIFI_SEED_ALLOW_PROD";

#[tokio::main]
async fn main() -> ExitCode {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let raw_args: Vec<String> = std::env::args().skip(1).collect();
    let mut seed_config = SeedConfig::default();
    let mut show_help = false;

    let mut iter = raw_args.into_iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "-h" | "--help" => show_help = true,
            "--fresh" => seed_config.fresh = true,
            "--scenario" => match iter.next() {
                Some(value) => match SeedScenario::parse(&value) {
                    Some(scenario) => seed_config.scenario = scenario,
                    None => {
                        eprintln!(
                            "error: unknown scenario {value:?} (expected one of: {})",
                            SeedScenario::NAMES.join(", ")
                        );
                        return ExitCode::from(2);
                    }
                },
                None => {
                    eprintln!("error: --scenario requires a value");
                    return ExitCode::from(2);
                }
            },
            "--num-pools" => match iter.next() {
                Some(value) => match value.parse::<usize>() {
                    Ok(n) if n > 0 => seed_config.num_pools = n,
                    _ => {
                        eprintln!("error: --num-pools expects a positive integer, got {value:?}");
                        return ExitCode::from(2);
                    }
                },
                None => {
                    eprintln!("error: --num-pools requires a value");
                    return ExitCode::from(2);
                }
            },
            other => {
                eprintln!("error: unknown argument {other:?} (see --help)");
                return ExitCode::from(2);
            }
        }
    }

    if show_help {
        print_help();
        return ExitCode::SUCCESS;
    }

    let config = match Config::from_env() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load configuration: {e}");
            return ExitCode::FAILURE;
        }
    };

    // Production isolation: refuse fixture loading against production config
    // unless explicitly overridden.
    if is_production(&config) && !env_flag(ALLOW_PROD_ENV_VAR) {
        eprintln!(
            "error: refusing to load fixtures against production configuration; \
             set {ALLOW_PROD_ENV_VAR}=1 to override (not recommended)"
        );
        return ExitCode::FAILURE;
    }

    // Destructive operations require explicit environment confirmation.
    if seed_config.fresh && !env_flag(CONFIRM_ENV_VAR) {
        eprintln!(
            "error: --fresh is destructive and requires explicit confirmation; \
             set {CONFIRM_ENV_VAR}=1 to proceed"
        );
        return ExitCode::FAILURE;
    }

    info!(
        database_url = %mask_password(&config.database_url),
        "connecting to postgres"
    );

    let pool = match predifi_backend::db::create_pool(&config).await {
        Ok(p) => p,
        Err(e) => {
            error!(error = %e, "failed to create postgres pool");
            return ExitCode::FAILURE;
        }
    };

    info!("running migrations");
    if let Err(e) = sqlx::migrate!("./migrations").run(&pool).await {
        error!(error = %e, "failed to run migrations");
        return ExitCode::FAILURE;
    }

    info!(
        num_pools = seed_config.num_pools,
        fresh = seed_config.fresh,
        scenario = seed_config.scenario.as_str(),
        "seeding database"
    );

    match run_seed(&pool, &seed_config).await {
        Ok(summary) => {
            info!(
                pools = summary.pools,
                predictions = summary.predictions,
                referrals = summary.referrals,
                "seed complete"
            );
            ExitCode::SUCCESS
        }
        Err(e) => {
            error!(error = %e, "seed failed");
            ExitCode::FAILURE
        }
    }
}

/// Returns `true` when the environment variable is set to a truthy value.
fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name).ok().as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes")
    )
}

/// Best-effort detection of a production-configured environment.
///
/// Considers `PREDIFI_ENV`/`APP_ENV`/`ENVIRONMENT` set to `production`/`prod`.
fn is_production(config: &Config) -> bool {
    let from_config = config.environment.as_deref();
    let from_env = std::env::var("PREDIFI_ENV")
        .ok()
        .or_else(|| std::env::var("APP_ENV").ok())
        .or_else(|| std::env::var("ENVIRONMENT").ok());
    let value = from_config
        .map(|s| s.to_string())
        .or(from_env)
        .unwrap_or_default();
    matches!(value.to_ascii_lowercase().as_str(), "production" | "prod")
}

/// Replace the password portion of a postgres URL with `***` for logging.
fn mask_password(url: &str) -> String {
    if let Some(after_scheme) = url.split("://").nth(1) {
        if let Some(at_idx) = after_scheme.find('@') {
            let (creds, rest) = after_scheme.split_at(at_idx);
            if let Some(colon_idx) = creds.find(':') {
                let user = &creds[..colon_idx];
                return format!(
                    "{}://{}:***{}",
                    &url[..url.len() - after_scheme.len() - 3],
                    user,
                    rest
                );
            }
        }
    }
    url.to_string()
}

fn print_help() {
    println!("predifi-seed — populate the local PrediFi database with deterministic sample data");
    println!();
    println!("USAGE:");
    println!("    cargo run --bin predifi-seed -- [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    --fresh           Truncate seed tables before inserting (destructive; requires {CONFIRM_ENV_VAR}=1)");
    println!("    --num-pools N     Number of pools to generate (default: {DEFAULT_NUM_POOLS})");
    println!("    --scenario NAME   Seed a targeted scenario (default: all)");
    println!("    -h, --help        Print this help message and exit");
    println!();
    println!("SCENARIOS:");
    for name in SeedScenario::NAMES {
        if let Some(scenario) = SeedScenario::parse(name) {
            println!("    {:<16}{}", name, scenario.description());
        }
    }
    println!();
    println!("ENVIRONMENT:");
    println!("    PREDIFI_DATABASE_URL   Postgres connection string");
    println!("    {CONFIRM_ENV_VAR}   Set to 1 to allow destructive --fresh operations");
    println!("    {ALLOW_PROD_ENV_VAR}   Set to 1 to allow fixture loading against production (refused by default)");
    println!("    RUST_LOG               Tracing filter (default: info)");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_password_redacts_postgres_credentials() {
        let masked = mask_password("postgres://postgres:secret@localhost:5432/predifi");
        assert!(masked.contains("***"));
        assert!(!masked.contains("secret"));
        assert!(masked.contains("postgres://postgres:***@localhost:5432/predifi"));
    }

    #[test]
    fn mask_password_passes_through_url_without_password() {
        let url = "postgres://localhost:5432/predifi";
        assert_eq!(mask_password(url), url);
    }

    #[test]
    fn env_flag_recognizes_truthy_values() {
        std::env::set_var("PREDIFI_SEED_TEST_FLAG", "1");
        assert!(env_flag("PREDIFI_SEED_TEST_FLAG"));
        std::env::set_var("PREDIFI_SEED_TEST_FLAG", "true");
        assert!(env_flag("PREDIFI_SEED_TEST_FLAG"));
        std::env::set_var("PREDIFI_SEED_TEST_FLAG", "0");
        assert!(!env_flag("PREDIFI_SEED_TEST_FLAG"));
        std::env::remove_var("PREDIFI_SEED_TEST_FLAG");
        assert!(!env_flag("PREDIFI_SEED_TEST_FLAG"));
    }
}
