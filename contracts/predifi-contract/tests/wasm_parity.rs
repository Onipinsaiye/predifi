//! Native vs optimized WASM parity tests (issue #119).
//!
//! Runs identical deterministic lifecycle vectors against the native contract
//! and the optimized release WASM artifact inside a Soroban host, then compares
//! externally observable behavior (return values, errors, and emitted events).
//!
//! The WASM artifact is expected at `target/wasm32-unknown-unknown/release/`
//! (produced by `cargo build --release --target wasm32-unknown-unknown`).
//! Provenance is asserted via the embedded build metadata / commit SHA so the
//! artifact is traceable to the source commit.

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Events as _, Ledger as _},
    Address, Env, IntoVal, Symbol, Val, Vec as SdkVec,
};

use predifi_contract::{PredifiContract, PredifiContractClient};

/// Path to the optimized WASM artifact relative to the crate root.
const WASM_PATH: &str = "../target/wasm32-unknown-unknown/release/predifi_contract.wasm";

/// A single deterministic lifecycle vector: a named sequence of entry-point
/// invocations with fixed arguments.
struct Vector {
    name: &'static str,
    run: fn(&Env, &Address) -> Vec<StepResult>,
}

/// Observable result of a single entry-point invocation.
#[derive(Debug, PartialEq, Eq)]
enum StepResult {
    Ok(Val),
    Err(String),
}

/// Observable events captured after a step, rendered as comparable strings.
#[derive(Debug, PartialEq, Eq)]
struct StepObservation {
    entry: &'static str,
    result: StepResult,
    events: Vec<String>,
}

fn render_events(env: &Env) -> Vec<String> {
    env.events()
        .all()
        .iter()
        .map(|(contract, topics, data)| {
            format!("contract={:?} topics={:?} data={:?}", contract, topics, data)
        })
        .collect()
}

fn call_native(env: &Env, contract: &Address, entry: &str, args: SdkVec<Val>) -> StepResult {
    let result: Result<Val, _> = env.try_invoke_contract(contract, &Symbol::new(env, entry), args);
    match result {
        Ok(v) => StepResult::Ok(v),
        Err(e) => StepResult::Err(format!("{:?}", e)),
    }
}

fn call_wasm(env: &Env, contract: &Address, entry: &str, args: SdkVec<Val>) -> StepResult {
    let result: Result<Val, _> = env.try_invoke_contract(contract, &Symbol::new(env, entry), args);
    match result {
        Ok(v) => StepResult::Ok(v),
        Err(e) => StepResult::Err(format!("{:?}", e)),
    }
}

/// Deterministic lifecycle vector: init -> create -> predict -> resolve -> claim.
fn lifecycle_vector(env: &Env, contract: &Address) -> Vec<StepObservation> {
    let mut observations = Vec::new();

    let admin = Address::generate(env);
    let user = Address::generate(env);

    // init
    let mut args = SdkVec::new(env);
    args.push_back(admin.clone().into_val(env));
    let result = call_native(env, contract, "init", args);
    observations.push(StepObservation {
        entry: "init",
        result,
        events: render_events(env),
    });

    // create
    let mut args = SdkVec::new(env);
    args.push_back(user.clone().into_val(env));
    let result = call_native(env, contract, "create", args);
    observations.push(StepObservation {
        entry: "create",
        result,
        events: render_events(env),
    });

    // predict
    let mut args = SdkVec::new(env);
    args.push_back(user.clone().into_val(env));
    let result = call_native(env, contract, "predict", args);
    observations.push(StepObservation {
        entry: "predict",
        result,
        events: render_events(env),
    });

    // resolve
    let mut args = SdkVec::new(env);
    args.push_back(admin.clone().into_val(env));
    let result = call_native(env, contract, "resolve", args);
    observations.push(StepObservation {
        entry: "resolve",
        result,
        events: render_events(env),
    });

    // claim
    let mut args = SdkVec::new(env);
    args.push_back(user.clone().into_val(env));
    let result = call_native(env, contract, "claim", args);
    observations.push(StepObservation {
        entry: "claim",
        result,
        events: render_events(env),
    });

    observations
}

fn run_native() -> Vec<StepObservation> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);
    let contract = env.register_contract(None, PredifiContract);
    lifecycle_vector(&env, &contract)
}

fn run_wasm() -> Vec<StepObservation> {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_700_000_000);

    let wasm = std::fs::read(WASM_PATH).unwrap_or_else(|e| {
        panic!(
            "missing optimized WASM artifact at {}: {}. Build with `cargo build --release --target wasm32-unknown-unknown`.",
            WASM_PATH, e
        )
    });
    let contract = env.register_contract_wasm(None, wasm.as_slice());
    lifecycle_vector(&env, &contract)
}

#[test]
fn wasm_artifact_is_traceable_to_source_commit() {
    let wasm = std::fs::read(WASM_PATH).unwrap_or_else(|e| {
        panic!(
            "missing optimized WASM artifact at {}: {}. Build with `cargo build --release --target wasm32-unknown-unknown`.",
            WASM_PATH, e
        )
    });

    // The artifact must embed build metadata (commit SHA) so it is traceable
    // to the source commit. The build script writes this into the WASM custom
    // section; assert it is present and non-empty.
    let commit = option_env!("GIT_COMMIT_SHA").unwrap_or("unknown");
    let needle = commit.as_bytes();
    let found = needle.len() > 0 && wasm.windows(needle.len()).any(|w| w == needle);
    assert!(
        found,
        "WASM artifact at {} does not embed source commit SHA {:?}; provenance is not traceable",
        WASM_PATH, commit
    );
}

#[test]
fn native_and_wasm_lifecycle_parity() {
    let native = run_native();
    let wasm = run_wasm();

    assert_eq!(
        native.len(),
        wasm.len(),
        "native and WASM produced different number of lifecycle steps"
    );

    let mut mismatches = Vec::new();
    for (n, w) in native.iter().zip(wasm.iter()) {
        if n.entry != w.entry {
            mismatches.push(format!(
                "vector step order mismatch: native entry {:?} vs WASM entry {:?}",
                n.entry, w.entry
            ));
            continue;
        }
        if n.result != w.result {
            mismatches.push(format!(
                "entry {:?}: result mismatch\n  native: {:?}\n  wasm:   {:?}",
                n.entry, n.result, w.result
            ));
        }
        if n.events != w.events {
            mismatches.push(format!(
                "entry {:?}: event mismatch\n  native: {:?}\n  wasm:   {:?}",
                n.entry, n.events, w.events
            ));
        }
    }

    assert!(
        mismatches.is_empty(),
        "native/WASM behavior diverged:\n{}",
        mismatches.join("\n")
    );
}
