//! `predifi-openapi` — dump the OpenAPI spec to stdout or a file (#1381).

/// Lets tooling (TypeScript client generation, Postman, linters) read the spec

/// without booting the server and its dependencies:

///

/// ```text

/// cargo run --bin predifi-openapi                       # stdout

/// cargo run --bin predifi-openapi -- --out openapi.json # write to a file

/// cargo run --bin predifi-openapi -- --check openapi.json # verify spec is up to date

/// ```

///

/// ### Contract validation (#1381)

///

/// The `predifi-openapi` binary is the single source of truth for the published

/// spec. Two modes guarantee the generated client artifacts stay compatible with

/// the live Axum handlers:

///

/// - `--check PATH` regenerates the spec in-memory and compares it to the committed

///   artifact. A non-zero exit code means the checked-in spec (and thus any

///   client generated from it) is stale.

/// - `--check-contract PATH` additionally validates the spec structurally:

///   every operation must declare at least one response, and every response

///   code must resolve to a defined schema or a content type. This is the

///   clean-diff check that CI runs against the committed artifact.

///

/// The runtime contract cases that exercise the Axum handlers (error

/// responses, optional fields) live in the integration test suite and call

/// this binary's `--check` mode as part of the contract job.

use std::process::ExitCode;

use predifi_backend::openapi::ApiDoc;

use utoipa::OpenApi;

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

                if let Some(next_arg) = args.peek() {

                    if !next_arg.starts_with('-') && next_arg != "--check-contract" {

                        check_path = Some(args.next().unwrap());

                    } else {

                        check_path = Some("openapi.json".to_string());

                    }

                } else {

                    check_path = Some("openapi.json".to_string());

                }

            }

            "--check-contract" => {

                check_contract = true;

                if check_path.is_none() {

                    check_path = Some("openapi.json".to_string());

                }

            }

            other => {

                eprintln!("error: unknown argument {other:?} (see --help)");

                return ExitCode::from(2);

            }

        }

    }

    let doc = ApiDoc::openapi();

    let spec = match serde_json::to_string_pretty(&doc) {

        Ok(json) => json,

        Err(e) => {

            eprintln!("failed to serialize OpenAPI spec: {e}");

            return ExitCode::FAILURE;

        }

    };

    if check_contract {

        if let Err(msg) = validate_contract(&doc) {

            eprintln!("error: OpenAPI contract validation failed: {msg}");

            return ExitCode::FAILURE;

        }

        eprintln!("OpenAPI contract validation passed.");

    }

    if let Some(path) = check_path {

        let expected = format!("{spec}\n");

        let existing = match std::fs::read_to_string(&path) {

            Ok(content) => content,

            Err(e) => {

                eprintln!("error: could not read OpenAPI spec file at '{path}': {e}");

                eprintln!("Run `cargo run --bin predifi-openapi -- --out {path}` to generate it.");

                return ExitCode::FAILURE;

            }

        };

        let normalized_existing = existing.replace("\r\n", "\n");

        let normalized_expected = expected.replace("\r\n", "\n");

        if normalized_existing != normalized_expected {

            eprintln!("error: OpenAPI spec file at '{path}' is out of date.");

            eprintln!("Run `cargo run --bin predifi-openapi -- --out {path}` to regenerate it.");

            return ExitCode::FAILURE;

        }

        eprintln!("OpenAPI spec at '{path}' is up to date.");

        return ExitCode::SUCCESS;

    }

    match out_path {

        Some(path) => {

            if let Err(e) = std::fs::write(&path, format!("{spec}\n")) {

                eprintln!("failed to write {path}: {e}");

                return ExitCode::FAILURE;

            }

            eprintln!("wrote OpenAPI spec to {path}");

        }

        None => println!("{spec}"),

    }

    ExitCode::SUCCESS

}

/// Structural validation of the generated spec. Ensures that every operation

/// declares at least one response and that each response has a resolvable

/// schema or content type. This is the contract check that CI runs against the

/// committed artifact so stale or broken specs fail before a client is generated.

fn validate_contract(doc: &utopa::openapi::OpenApi) => Result<(), String> {

    let mut errors: Vec<String> = Vec::new();

    for (path, item) in &doc.paths.paths {

        for (method, op)  in operations_of(item) {

            if op.responses.responses.is_empty() {

                errors.push(format!("{method} {path} declares no responses"));

                continue;

            }

            for (code, resp) in &op.responses.responses {

                if resp.content.is_empty() {

                    errors.push(format!("{method} {path} response {code} has no content schema"));

                }

            }

        }

    }

    if errors.is_empty() {

        Ok(())

    } else {

        Err(()

            .chain(errors.iter().map(|s| s.as_str()))

            .collect::String>())

    }

}

/// Returns the HTTP method name and operation for each method defined on a

/// path item. Utoipa stores operations in a struct with optional fields, so

/// we iterate the known HTTP verbs explicitly.

fn operations_of(item: &utoipa::openapi::path::PathItem) -> Vec<(&'str, &utopa::openapi::path::Operation)> {

    let mut out = Vec::new();

    if let Some(op) = &item.get {

        out.push(("GET", op));

    }

    if let Some(op) = &item.put {

        out.push(("PUT", op));

    }

    if let Some(op) = &item.post {

        out.push(("POST", op));

    }

    if let Some(op) = &item.delete {

        out.push(("DELETE", op));

    }

    if let Some(op) = &item.options {

        out.push(("OPTIONS", op));

    }

    if let Some(op) = &item.head {

        out.push(("HEAD", op));

    }

    if let Some(op) = &item.patch {

        out.push(("PATCH", op));

    }

    if let Some(op) = &item.trace {

        out.push(("TRACE", op));

    }

    out

}

fn print_help() {

    println!("predifi-openapi — print or check the PrediFi OpenAPI 3.x specification as JSON");

    println!();

    println!("USAGE:");

    println!("    cargo run --bin predifi-openapi -- [OPTIONS]");

    println!();

    println!("OPTIONS:");

    println!("    -o, --out PATH              Write the spec to PATH instead of stdout");

    println!("    -c, --check [PATH]      Verify that PATH (default: openapi.json) matches the current spec");

    println!("        --check-contract     Also validate the spec structurally (responses and schemas)");

    println!("    -h, --help              Print this help message and exit");

}
