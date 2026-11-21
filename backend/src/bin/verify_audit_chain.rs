//! Verify the integrity of the moderation audit chain.
//!
//! Usage:
//!   verify_audit_chain --input audit_chain.json [--expected-length N]
//!
//! The input file is a JSON array of `ModerationRecord` objects — the same
//! shape persisted by `ModerationAuditLog` (hash-chained: each record's
//! `record_hash` covers its own content plus the previous record's hash).
//!
//! On success the command prints a short report and exits 0. On any
//! integrity failure (broken hash chain, out-of-order sequence, missing
//! step-up proof, dangling correction, or a length mismatch) it prints the
//! failure and exits 1.

use predifi_backend::moderation_audit::{verify_records_with_expected_length, ModerationRecord};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
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
                input_path = Some(
                    args.next()
                        .ok_or_else(|| "--input requires a value".to_string())?,
                );
            }
            "--expected-length" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--expected-length requires a value".to_string())?;
                expected_length = Some(
                    value
                        .parse()
                        .map_err(|_| format!("invalid --expected-length value: {value}"))?,
                );
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

    let input_path = input_path.ok_or_else(|| {
        print_usage();
        "--input is required".to_string()
    })?;

    let data = std::fs::read_to_string(&input_path)
        .map_err(|error| format!("failed to read {input_path}: {error}"))?;
    let records: Vec<ModerationRecord> = serde_json::from_str(&data)
        .map_err(|error| format!("failed to parse audit chain JSON: {error}"))?;

    let report = verify_records_with_expected_length(&records, expected_length)
        .map_err(|error| format!("audit chain verification failed: {error:?}"))?;

    println!("ok: verified {} record(s)", report.checked_records);
    Ok(())
}

fn print_usage() {
    eprintln!("usage: verify_audit_chain --input <PATH> [--expected-length <N>]");
}
