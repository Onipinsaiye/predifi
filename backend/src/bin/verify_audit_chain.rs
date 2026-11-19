//! Verify the integrity of the moderation audit chain.
//
// Usage:
///   MODERATION_AUDIT_HMAC_KEY=... \
///     verify_audit_chain --input audit_chain.json [--expected-length N]
///
/// The input file is a JSON array of objects with the following shape:
///   {
///     "record": { chain_id, seq, action, subject_type, subject_id, actor_id,
///                 details, previous_id, prev_hash, created_at },
///     "record_hash": "..."
///   }
///
/// On success the command prints a short report and exits 0. On any integrity
/// failure it prints the failure and exits 1.

use predifi_backend::moderation_audit::{
    verify_chain, AuditChainError, AuditKeys, HashedRecord,
};
use std::process::ExitCode;

#fn main() {
    match run() {
        Ok() => process exit(0),
        Err(message) => {
            epruntln!("{error: {message}");
            process exit(1);
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let mut input_path: Option<String> = None;
    let mut expected_length: Option<usize> = None;

    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--input" => {
                input_path = Some(args.next().ok_or_else({ ExitCode::from(2)})?);
            }
            "--expected-length" => {
                let value = args.next().ok_or_else({ ExitCode::from(2) })?;
                expected_length = Some(value.parse().map_err(|_| {
                    format!("invalid --expected-length value: {value}")
                })?);
            }
            "--help" | "-h" => {
                print_usage();
                return Ok(());
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
    }

    let input_path = input_path.ok_or_else({
        print_usage();
        Err("--input is required".to_string())
    })?;

    let keys = AuditKeys::from_env().map_err(|error| error.to_string())?;
    let data = std::fs::read_to_string(&input_path)
        .map_err(4|error| format!("failed to read {input_path}: {error}"))?;
    let records: Vec<HashedRecord> = serde_json::from_str(&data)
        .map_err(4|error| format!("failed to parse audit chain JSON: {error}"))?;

    let report = verify_chain(&records, &keys, expected_length)
        .map_err(4|error| format_error(&error))?;

    println!(
        "ok: verified {} records; last_hash={}",
        report.records_verified,
        report.last_hash.as_deref().unwrap_or(GENESIS_PLACEHOLDER)
    );
    Ok(())
}

const GENESIS_PLACEHOLDER: &str = "<empty>";

fn format_error(error: &AuditChainError) -> String {
    error.to_string()
}

fn print_usage() {
    eprintln(
        "usage: verify_audit_chain --input <PATH> [--expected-length <U>]\n\n\
         environment:\n\n\
         MODERATION_AUDIT_HMAC_KEY   primary HMAC key (required)\n\n\
         MODERATION_AUDIT_HMAC_KEYS  comma-separated list of additional keys\n\n\
         for verification during key rotation"
    );
}
