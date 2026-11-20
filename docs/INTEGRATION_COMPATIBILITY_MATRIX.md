# Integration Compatibility Matrix

**Matrix Version:** 1.0.0

This matrix records combinations that have reproducible validation evidence.
An unlisted or planned combination is not a support commitment.

## Versioning

| Component          | Source of version                                                  |
| ------------------ | ------------------------------------------------------------------ |
| Frontend           | `frontend/package.json`                                            |
| API                | `backend/openapi.json` and the `/api/v1` route                     |
| Contracts          | Each contract crate's `Cargo.toml`                                 |
| Wallet             | Freighter browser API; its extension version is managed externally |
| Contract interface | `schema_version` in the release descriptor                         |

## Compatibility Matrix

### Current source snapshot (unreleased)

| Frontend | API                 | Contracts | Wallet                | Network | Status    | Validation Evidence                                                                                                                                                                                                                                                                                                                              |
| -------- | ------------------- | --------- | --------------------- | ------- | --------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `0.1.0`  | `1.0.0` (`/api/v1`) | `0.0.0`   | Freighter browser API | Testnet | Supported | [Frontend CI](../.github/workflows/frontend-ci.yml), [Backend CI](../.github/workflows/backend.yml), [Contract CI](../.github/workflows/contract-ci.yml), [Wallet conformance tests](../frontend/__tests__/wallet-capability-conformance.test.tsx), [Contract integration tests](../contract/contracts/predifi-contract/src/integration_test.rs) |
| `0.1.0`  | `1.0.0` (`/api/v1`) | `0.0.0`   | Freighter browser API | Mainnet | Planned   | Not applicable                                                                                                                                                                                                                                                                                                                                   |

There are no published tagged releases yet. When a release is published, its
compatibility entry must link to the release and the validation evidence used
to support it.

### Component boundaries

| Boundary             | Supported combination                                                 | Validation evidence                                                                                                                               |
| -------------------- | --------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| Frontend ↔ API       | Frontend `0.1.0` with API `1.0.0` at `/api/v1`                        | [Frontend CI](../.github/workflows/frontend-ci.yml), [Backend CI](../.github/workflows/backend.yml)                                               |
| Frontend ↔ contracts | Frontend `0.1.0` with contract crates `0.0.0`                         | [Contract CI](../.github/workflows/contract-ci.yml), [Contract integration tests](../contract/contracts/predifi-contract/src/integration_test.rs) |
| Wallet ↔ network     | Freighter with the configured Stellar network; Testnet is the default | [Wallet conformance tests](../frontend/__tests__/wallet-capability-conformance.test.tsx)                                                          |
| Contract spec ↔ WASM | Schema version `1`, generated from the release's contract WASM        | [Descriptor publication workflow](../.github/workflows/contract-descriptors.yml)                                                                  |

## Network Requirements

| Network   | Soroban RPC                                 | Support status                                   |
| --------- | ------------------------------------------- | ------------------------------------------------ |
| Testnet   | `https://soroban-testnet.stellar.org:443`   | Active; default frontend network                 |
| Futurenet | `https://soroban-futurenet.stellar.org:443` | Supported by the wallet network check            |
| Mainnet   | `https://soroban.stellar.org:443`           | Planned; not yet a supported release combination |

The wallet must report the network configured by
`NEXT_PUBLIC_STELLAR_NETWORK`. Network names and passphrases are checked by the
frontend's wallet connection layer.

## Deprecation Windows

No component version is currently marked deprecated. When a version is
deprecated, its entry must include the deprecation date, sunset date, and
migration guide. Maintain at least a 90-day migration window between
deprecation and sunset; security issues may require a shorter window and an
explicit maintainer decision.

## Validation Evidence Requirements

Every **Supported** or **Deprecated** matrix entry must link to at least one
existing test file, workflow, or published GitHub release. Planned entries do
not claim compatibility and are not treated as supported combinations by the
validator.

Run the same check locally or in CI:

```bash
./scripts/validate-compatibility-matrix.sh
```

The check rejects missing, placeholder, broken, or out-of-repository evidence
links. The [validation workflow](../.github/workflows/validate-compatibility-matrix.yml)
runs it whenever this matrix changes.

## Updating the Matrix

1. Update the relevant component versions, network requirements, and lifecycle
   dates.
2. Run the linked tests and workflows for the combination.
3. Add evidence links to the row before marking it Supported or Deprecated.
4. For a release, link the release page and retain the prior release entry and
   its descriptors.

## Quick Reference

| Component                  | Current source value  |
| -------------------------- | --------------------- |
| Frontend                   | `0.1.0`               |
| API                        | `1.0.0` (`/api/v1`)   |
| Contract crates            | `0.0.0`               |
| Wallet                     | Freighter browser API |
| Default network            | Stellar Testnet       |
| Contract descriptor schema | `1`                   |
