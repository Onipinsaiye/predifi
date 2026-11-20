#!/usr/bin/env bash

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CONTRACT_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
REPO_ROOT="$(git -C "${SCRIPT_DIR}" rev-parse --show-toplevel)"
WASM_DIR="${CONTRACT_ROOT}/target/wasm32-unknown-unknown/release"
OUTPUT_DIR="${1:-${CONTRACT_ROOT}/releases/contract-descriptors}"
CONTRACTS=("predifi_contract:predifi-contract" "access_control:access-control")

for command in base64 cargo git jq rustc sha256sum wasm-opt; do
    if ! command -v "${command}" >/dev/null 2>&1; then
        echo "Error: required command '${command}' was not found." >&2
        exit 1
    fi
done

if command -v stellar >/dev/null 2>&1; then
    CLI="stellar"
elif command -v soroban >/dev/null 2>&1; then
    CLI="soroban"
else
    echo "Error: neither 'stellar' nor 'soroban' CLI was found." >&2
    exit 1
fi

for contract in "${CONTRACTS[@]}"; do
    wasm="${contract%%:*}"
    if [[ ! -f "${WASM_DIR}/${wasm}.wasm" ]]; then
        echo "Building contract WASM artifacts..."
        cargo build \
            --manifest-path "${CONTRACT_ROOT}/Cargo.toml" \
            --target wasm32-unknown-unknown \
            --release
        break
    fi
done

mkdir -p "${OUTPUT_DIR}"
GIT_COMMIT="$(git -C "${REPO_ROOT}" rev-parse HEAD)"
GIT_REF="$(git -C "${REPO_ROOT}" rev-parse --abbrev-ref HEAD)"
GIT_TAG="$(git -C "${REPO_ROOT}" describe --tags --exact-match 2>/dev/null || true)"
BUILD_TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

for contract in "${CONTRACTS[@]}"; do
    WASM="${contract%%:*}"
    PACKAGE="${contract#*:}"
    MANIFEST="${CONTRACT_ROOT}/contracts/${PACKAGE}/Cargo.toml"
    RAW_WASM="${WASM_DIR}/${WASM}.wasm"
    OPT_WASM="${WASM_DIR}/${WASM}_optimized.wasm"
    DESCRIPTOR_FILE="${OUTPUT_DIR}/${WASM}-descriptor.json"

    if [[ ! -f "${RAW_WASM}" ]]; then
        echo "Error: expected contract artifact not found: ${RAW_WASM}" >&2
        exit 1
    fi

    if [[ ! -f "${MANIFEST}" ]]; then
        echo "Error: expected contract manifest not found: ${MANIFEST}" >&2
        exit 1
    fi

    VERSION="$(awk -F '"' '$1 ~ /^version[[:space:]]*=/ { print $2; exit }' "${MANIFEST}")"
    if [[ -z "${VERSION}" ]]; then
        echo "Error: package version not found in ${MANIFEST}" >&2
        exit 1
    fi

    echo "Generating ${WASM} interface descriptor..."
    wasm-opt -Oz --strip-debug --enable-bulk-memory --enable-sign-ext \
        -o "${OPT_WASM}" "${RAW_WASM}"

    SPEC_BASE64="$("${CLI}" contract spec --wasm "${OPT_WASM}" --format base64)"
    if [[ -z "${SPEC_BASE64}" ]] ||
        ! printf '%s' "${SPEC_BASE64}" | base64 --decode >/dev/null 2>&1; then
        echo "Error: failed to generate a valid base64 contract spec for ${WASM}." >&2
        exit 1
    fi

    RAW_SHA256="$(sha256sum "${RAW_WASM}" | awk '{ print $1 }')"
    OPT_SHA256="$(sha256sum "${OPT_WASM}" | awk '{ print $1 }')"
    RAW_SIZE="$(wc -c < "${RAW_WASM}")"
    OPT_SIZE="$(wc -c < "${OPT_WASM}")"

    jq -n \
        --arg contract "${WASM}" \
        --arg version "${VERSION}" \
        --arg schema_version "1" \
        --arg commit "${GIT_COMMIT}" \
        --arg ref "${GIT_REF}" \
        --arg tag "${GIT_TAG}" \
        --arg timestamp "${BUILD_TIMESTAMP}" \
        --arg rust_version "$(rustc --version)" \
        --arg cargo_version "$(cargo --version)" \
        --arg raw_sha256 "${RAW_SHA256}" \
        --arg optimized_sha256 "${OPT_SHA256}" \
        --arg raw_path "target/wasm32-unknown-unknown/release/${WASM}.wasm" \
        --arg optimized_path "target/wasm32-unknown-unknown/release/${WASM}_optimized.wasm" \
        --arg spec "${SPEC_BASE64}" \
        --argjson raw_size "${RAW_SIZE}" \
        --argjson optimized_size "${OPT_SIZE}" \
        '{
          contract: $contract,
          version: $version,
          schema_version: $schema_version,
          git: {
            commit: $commit,
            ref: $ref,
            tag: $tag,
            repository: "https://github.com/Onipinsaiye/predifi"
          },
          build: {
            timestamp: $timestamp,
            rust_version: $rust_version,
            cargo_version: $cargo_version,
            target: "wasm32-unknown-unknown"
          },
          artifacts: {
            raw_wasm: {
              sha256: $raw_sha256,
              size_bytes: $raw_size,
              path: $raw_path
            },
            optimized_wasm: {
              sha256: $optimized_sha256,
              size_bytes: $optimized_size,
              path: $optimized_path
            }
          },
          spec: {
            format: "xdr-base64",
            encoding: "base64",
            data: $spec
          },
          network_independent: true,
          generation_provenance: {
            tool: "generate-contract-descriptors.sh",
            generator_version: "1.0.0",
            generated_at: $timestamp
          }
        }' > "${DESCRIPTOR_FILE}"
done

INDEX_FILE="${OUTPUT_DIR}/index.json"
jq -s \
    --arg generated_at "${BUILD_TIMESTAMP}" \
    --arg commit "${GIT_COMMIT}" \
    --arg ref "${GIT_REF}" \
    --arg tag "${GIT_TAG}" \
    '{
      generated_at: $generated_at,
      git: {
        commit: $commit,
        ref: $ref,
        tag: $tag,
        repository: "https://github.com/Onipinsaiye/predifi"
      },
      contracts: map({
        contract,
        version,
        schema_version,
        optimized_wasm_sha256: .artifacts.optimized_wasm.sha256,
        spec_generated: (.spec.data != "")
      })
    }' "${OUTPUT_DIR}"/*-descriptor.json > "${INDEX_FILE}"

echo "Contract descriptors written to ${OUTPUT_DIR}"
