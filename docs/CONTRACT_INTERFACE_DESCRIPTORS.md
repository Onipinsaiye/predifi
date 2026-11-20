# Contract Interface Descriptors

This document describes the release-pinned contract interface descriptors published with each PrediFi release.

## Overview

Every release publishes **contract interface descriptors** — machine-readable JSON files that capture the exact interface of each deployed contract, tied to the specific WASM artifact hash. These descriptors enable:

- **Type-safe client generation** — Generate TypeScript/Rust/Python clients from the spec
- **Interface verification** — Confirm a deployed contract matches the expected interface
- **Upgrade safety** — Detect breaking changes between contract versions
- **Audit trail** — Immutable record of what was deployed

## Descriptor Format

Each contract gets a `<contract>-descriptor.json` file with this structure.
The values below are illustrative; release descriptors contain the actual
version, commit, and artifact hashes:

```json
{
  "contract": "predifi_contract",
  "version": "0.0.0",
  "schema_version": "1",
  "git": {
    "commit": "abc123...",
    "ref": "refs/tags/v0.1.0",
    "tag": "v0.1.0",
    "repository": "https://github.com/Onipinsaiye/predifi"
  },
  "build": {
    "timestamp": "2026-08-28T12:00:00Z",
    "rust_version": "rustc 1.80.0",
    "cargo_version": "cargo 1.80.0",
    "target": "wasm32-unknown-unknown"
  },
  "artifacts": {
    "raw_wasm": {
      "sha256": "deadbeef...",
      "size_bytes": 123456,
      "path": "target/wasm32-unknown-unknown/release/predifi_contract.wasm"
    },
    "optimized_wasm": {
      "sha256": "cafebabe...",
      "size_bytes": 98765,
      "path": "target/wasm32-unknown-unknown/release/predifi_contract_optimized.wasm"
    }
  },
  "spec": {
    "format": "xdr-base64",
    "encoding": "base64",
    "data": "AAAAAQAAA...base64-encoded XDR..."
  },
  "network_independent": true,
  "generation_provenance": {
    "tool": "generate-contract-descriptors.sh",
    "generator_version": "1.0.0",
    "generated_at": "2026-08-28T12:00:00Z"
  }
}
```

## Fields

| Field                 | Description                                                      |
| --------------------- | ---------------------------------------------------------------- |
| `contract`            | Contract name (`predifi_contract` or `access_control`)           |
| `version`             | Semantic version from Cargo.toml                                 |
| `schema_version`      | Version of the descriptor schema                                 |
| `git.commit`          | Exact git commit SHA                                             |
| `git.tag`             | Release tag (e.g., `v0.1.0`)                                     |
| `build.timestamp`     | ISO 8601 build timestamp                                         |
| `artifacts.*.sha256`  | SHA-256 of WASM file — **verify this matches deployed contract** |
| `spec.data`           | Base64-encoded XDR contract spec — decode for type definitions   |
| `network_independent` | Always `true` — spec is the same across networks                 |

## Accessing Descriptors

### From GitHub Releases

Descriptors are attached to each GitHub release:

1. Go to [Releases](https://github.com/Onipinsaiye/predifi/releases)
2. Download `index.json` and the `*-descriptor.json` files for the release

### From Release Assets API

```bash
# Get latest release descriptors
curl -L -H "Accept: application/vnd.github+json" \
  https://api.github.com/repos/Onipinsaiye/predifi/releases/latest \
  | jq -r '.assets[] | select(.name | endswith("descriptor.json")) | .browser_download_url' \
  | xargs -I {} curl -L -O {}
```

### Verify a Deployed Contract

```bash
# 1. Download descriptor from release
curl -L -O https://github.com/Onipinsaiye/predifi/releases/download/v0.1.0/predifi_contract-descriptor.json

# 2. Extract expected hash
EXPECTED_HASH=$(jq -r '.artifacts.optimized_wasm.sha256' predifi_contract-descriptor.json)

# 3. Get deployed contract WASM hash
stellar contract fetch --id <CONTRACT_ID> --network testnet --output deployed.wasm
ACTUAL_HASH=$(sha256sum deployed.wasm | awk '{print $1}')

# 4. Compare
if [[ "$EXPECTED_HASH" == "$ACTUAL_HASH" ]]; then
  echo "✅ Contract matches release v0.1.0"
else
  echo "❌ MISMATCH! Expected: $EXPECTED_HASH, Got: $ACTUAL_HASH"
fi
```

## Using the Spec for Client Generation

### Decode Spec to JSON

```bash
# Extract and decode spec
jq -r '.spec.data' predifi_contract-descriptor.json | base64 -d > spec.xdr
# Or use stellar CLI to pretty-print
stellar contract spec --wasm predifi_contract-optimized.wasm --format json
```

### Generate TypeScript Types

```bash
# Using the descriptor spec directly
jq -r '.spec.data' predifi_contract-descriptor.json | base64 -d | \
  stellar contract spec --format json > spec.json

# Generate TypeScript types (requires @stellar/stellar-sdk)
npx @stellar/stellar-sdk-contract-types spec.json --output ./types/predifi-contract.ts
```

### Generate Rust Client

```rust
// In your Cargo.toml
[dependencies]
soroban-sdk = "23"
predifi-contract = { git = "https://github.com/Onipinsaiye/predifi", package = "predifi-contract" }

// Use the generated client
use predifi_contract::{PredifiContractClient, Pool, Prediction};

let client = PredifiContractClient::new(&env, &contract_id);
let pool: Pool = client.get_pool(&pool_id);
```

## CI Integration

Add to your CI to verify contract interfaces haven't changed unexpectedly:

```yaml
- name: Verify contract descriptor
  run: |
    # Download expected descriptor
    curl -L -o expected.json https://github.com/Onipinsaiye/predifi/releases/download/v0.1.0/predifi_contract-descriptor.json

    # Generate current spec
    cargo build --target wasm32-unknown-unknown --release
    wasm-opt -Oz --strip-debug --enable-bulk-memory --enable-sign-ext \
      -o target/wasm32-unknown-unknown/release/predifi_contract_optimized.wasm \
      target/wasm32-unknown-unknown/release/predifi_contract.wasm
    stellar contract spec --wasm target/wasm32-unknown-unknown/release/predifi_contract_optimized.wasm --format base64 > current_spec.b64

    # Compare
    EXPECTED_SPEC=$(jq -r '.spec.data' expected.json)
    CURRENT_SPEC=$(cat current_spec.b64)

    if [[ "$EXPECTED_SPEC" != "$CURRENT_SPEC" ]]; then
      echo "❌ Contract interface has changed!"
      exit 1
    fi
    echo "✅ Interface matches release descriptor"
```

## Versioning Policy

| Version Component  | When It Changes                                             |
| ------------------ | ----------------------------------------------------------- |
| **Major** (X.0.0)  | Breaking interface changes (removed/changed methods, types) |
| **Minor** (0.X.0)  | New methods/events added, backward compatible               |
| **Patch** (0.0.X)  | Internal fixes, no interface change                         |
| **Schema Version** | Storage layout changes (requires migration)                 |

## Previous Descriptors

All previous release descriptors remain available in the [releases](https://github.com/Onipinsaiye/predifi/releases) page. They are never deleted or modified.

## Security

- Descriptors are **signed with Sigstore** (GitHub Actions attestation)
- Verify attestation: `gh attestation verify <descriptor.json> --repo Onipinsaiye/predifi`
- WASM hashes in descriptors match the [WASM build provenance](../.github/workflows/contract-ci.yml#artifact-provenance)

## Related Documents

- [Smart Contract Deployment Guide](./SMART_CONTRACT_DEPLOYMENT_GUIDE.md)
- [Contract Reference](./contract-reference.md)
- [Event Schemas](./EVENT_SCHEMAS.md)
- [Integration Compatibility Matrix](./INTEGRATION_COMPATIBILITY_MATRIX.md)
