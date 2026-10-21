//! `predifi-openapi` — dump the OpenAPI spec to stdout or a file (#1381).
//!
//! Lets tooling (TypeScript client generation, Postman, linters) read the spec
//! without booting the server and its dependencies:
//!
//! ```text
//! cargo run --bin predifi-openapi                          # stdout
//! cargo run --bin predifi-openapi -- --out openapi.json    # write to a file
//! cargo run --bin predifi-openapi -- --check openapi.json  # verify spec is up to date
//! ```
//!
//! ### Contract validation (#52)
//!
//! Output is the canonical artifact from
//! [`predifi_backend::openapi::openapi_artifact`]: keys sorted at every level,
//! so regeneration is byte-for-byte reproducible.
//!
//! - `--check [PATH]` regenerates the spec in memory and compares it to the
//!   committed artifact. A non-zero exit code means the checked-in spec (and
//!   any client generated from it) is stale.
//! - `--check-contract` additionally validates the spec structurally: every
//!   operation declares at least one response, and every error response
//!   declares a body schema.
//!
//! Route coverage and live response shapes are checked by
//! `tests/openapi_drift.rs`.

use std::process::ExitCode;

use predifi_backend::openapi::{openapi_artifact, openapi_json, OPENAPI_ARTIFACT_PATH};

fn main() -> ExitCode {
    let mut out_path: Option<String> = None;
    let mut check_path: Option<String> = None;
    let mut check_contract = false;

    let mut args = std::env::args().skip(1).peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print_help();
                return ExitCode::SUCCESS;
            }
            "-o" | "--out" => match args.next() {
                Some(path) => out_path = Some(path),
                None => {
                    eprintln!("error: --out requires a path");
                    return ExitCode::from(2);
                }
            },
            "-c" | "--check" => {
                let path = match args.peek() {
                    Some(next) if !next.starts_with('-') => args.next(),
                    _ => None,
                };
                check_path = Some(path.unwrap_or_else(|| OPENAPI_ARTIFACT_PATH.to_string()));
            }
            "--check-contract" => {
                check_contract = true;
                let path = match args.peek() {
                    Some(next) if !next.starts_with('-') => args.next(),
                    _ => None,
                };
                if let Some(path) = path {
                    check_path = Some(path);
                } else if check_path.is_none() {
                    check_path = Some(OPENAPI_ARTIFACT_PATH.to_string());
                }
            }
            other => {
                eprintln!("error: unknown argument {other:?} (see --help)");
                return ExitCode::from(2);
            }
        }
    }

    let spec = openapi_artifact();

    if check_contract {
        if let Err(problems) = validate_contract(&openapi_json()) {
            eprintln!("error: OpenAPI contract validation failed:");
            for problem in problems {
                eprintln!("  - {problem}");
            }
            return ExitCode::FAILURE;
        }
        eprintln!("OpenAPI contract validation passed.");
    }

    if let Some(path) = check_path {
        let existing = match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                eprintln!("error: could not read OpenAPI spec file at '{path}': {e}");
                eprintln!("Run `cargo run --bin predifi-openapi -- --out {path}` to generate it.");
                return ExitCode::FAILURE;
            }
        };
        if existing.replace("\r\n", "\n") != spec {
            eprintln!("error: OpenAPI spec file at '{path}' is out of date.");
            eprintln!("Run `cargo run --bin predifi-openapi -- --out {path}` to regenerate it.");
            return ExitCode::FAILURE;
        }
        eprintln!("OpenAPI spec at '{path}' is up to date.");
        return ExitCode::SUCCESS;
    }

    match out_path {
        Some(path) => {
            if let Err(e) = std::fs::write(&path, &spec) {
                eprintln!("failed to write {path}: {e}");
                return ExitCode::FAILURE;
            }
            eprintln!("wrote OpenAPI spec to {path}");
        }
        None => print!("{spec}"),
    }
    ExitCode::SUCCESS
}

/// Structural validation: every operation declares responses and every error
/// response declares a body schema, so generated clients get typed errors.
fn validate_contract(spec: &serde_json::Value) -> Result<(), Vec<String>> {
    const METHODS: [&str; 8] = [
        "get", "put", "post", "delete", "options", "head", "patch", "trace",
    ];
    let mut problems = Vec::new();
    let Some(paths) = spec["paths"].as_object() else {
        return Err(vec!["spec has no paths".to_string()]);
    };
    for (path, item) in paths {
        for method in METHODS {
            let Some(op) = item.get(method) else { continue };
            let label = format!("{} {path}", method.to_uppercase());
            let Some(responses) = op["responses"].as_object().filter(|r| !r.is_empty()) else {
                problems.push(format!("{label} declares no responses"));
                continue;
            };
            for (code, response) in responses {
                let has_body = response["content"].as_object().is_some_and(|c| !c.is_empty());
                if code.starts_with(['4', '5']) && !has_body {
                    problems.push(format!("{label} response {code} has no body schema"));
                }
            }
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems)
    }
}

fn print_help() {
    println!("predifi-openapi — print or check the PrediFi OpenAPI 3.x specification as JSON");
    println!();
    println!("USAGE:");
    println!("    cargo run --bin predifi-openapi -- [OPTIONS]");
    println!();
    println!("OPTIONS:");
    println!("    -o, --out PATH          Write the spec to PATH instead of stdout");
    println!("    -c, --check [PATH]      Verify that PATH (default: openapi.json) matches the current spec");
    println!("        --check-contract [PATH]  Also validate the spec structurally (responses and schemas)");
    println!("    -h, --help              Print this help message and exit");
}
