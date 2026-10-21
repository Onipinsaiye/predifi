# API Versioning and Deprecation Policy

This policy tells API clients how long a version is supported, how they are
warned before anything is removed, and which changes can ship without
warning. The backend enforces it through `backend/src/api_versioning.rs`,
which is the single source of truth for version and route status.

## Versions

The public API is versioned in the path: `/api/v1`, `/api/v2`, …

| Version | Status | Released | Deprecated | Sunset | Successor |
|---------|--------|----------|------------|--------|-----------|
| `v1` | Active | — | — | — | — |

A version is in one of three states:

| Status | Meaning |
|--------|---------|
| **Active** | Fully supported. Only backward-compatible changes. |
| **Deprecated** | Still served and supported (bug and security fixes), but has a sunset date. No new features. |
| **Sunset** | Past its sunset date. Requests receive `410 Gone` with code `API_VERSION_SUNSET`. |

Single operations can also be deprecated inside an active version. They
follow the same states.

## Support windows

| What | Minimum notice from `Deprecation` to `Sunset` |
|------|-----------------------------------------------|
| A whole version | 180 days, and at least 90 days after its successor is Active |
| A single operation in a supported version | 90 days |
| Security emergency | Shorter notice is allowed only when keeping the operation exposes users to harm. The reason goes in the release notes. |

Only one version older than the newest is supported at a time. A version
cannot be sunset while it is the newest Active version.

## How clients are told

Every response under `/api/<version>` carries:

```http
API-Version: v1
```

Once a version or operation is deprecated, its responses also carry:

```http
Deprecation: @1767225600
Sunset: Wed, 01 Jul 2026 00:00:00 GMT
Link: <https://github.com/Onipinsaiye/predifi/blob/main/docs/API_VERSIONING_POLICY.md>; rel="deprecation"; type="text/html", </api/v2>; rel="successor-version"
```

- `Deprecation` ([RFC 9745](https://www.rfc-editor.org/rfc/rfc9745)): Unix
  time the deprecation took effect.
- `Sunset` ([RFC 8594](https://www.rfc-editor.org/rfc/rfc8594)): the time
  after which the version or operation may stop working.
- `Link`: this policy (`rel="deprecation"`) and, when one exists, the
  replacement (`rel="successor-version"`).

The same information is available at runtime from `GET /api/v1`
(`versions` and `deprecated_routes`), in the OpenAPI spec description, in
the [API reference](API_REFERENCE.md#versioning-and-deprecation), and in
the release notes of the release that announces the deprecation.

Clients should log or alert on any response that has a `Deprecation`
header.

## Compatibility rules

Within a version, these changes are **backward compatible** and may ship at
any time:

- Adding an endpoint, an optional query parameter, an optional request
  field, or a response field.
- Adding an error code. Clients must handle unknown codes by HTTP status
  class.
- Adding a value to an enum-like string field that is documented as open
  (for example `status` values that clients must not exhaustively match).
- Relaxing validation (accepting input that used to be rejected).
- Adding response headers.

These changes are **breaking** and need a new version, or a deprecation of
the affected operation with the notice above:

- Removing or renaming an endpoint, field, query parameter or error code.
- Changing a field's type, format, units or meaning.
- Making an optional request field or parameter required.
- Tightening validation, or lowering a documented limit.
- Changing the HTTP status for an existing outcome.
- Changing the success or error envelope shape. The error envelope carries
  its own `version` field for this reason.
- Changing authentication requirements of an existing operation.

## Release checklist for version changes

Use this checklist in the PR that adds, deprecates or sunsets a version or
operation.

**Announcing a deprecation**

- [ ] Add or update the entry in `API_VERSIONS` or `DEPRECATED_ROUTES` in
      `backend/src/api_versioning.rs` with `deprecated_at`, `sunset_at`
      and the successor or replacement.
- [ ] Check the notice period against the support windows above.
- [ ] Mark the operation `deprecated` in its `#[utoipa::path]` and
      regenerate `backend/openapi.json`
      (`cargo run --bin predifi-openapi -- --out openapi.json`).
- [ ] Update the version table in this file and in `API_REFERENCE.md`.
- [ ] Add contract tests asserting the `Deprecation`, `Sunset` and `Link`
      headers for the affected routes.
- [ ] Add a release-notes entry with the dates, the replacement and a
      migration guide.
- [ ] Tell known integrators (frontend, indexer, partners) directly.

**Shipping a new version**

- [ ] New routes are mounted under `/api/<new>` and listed in
      `API_VERSIONS` as Active.
- [ ] The previous version has its successor set.
- [ ] OpenAPI spec and generated clients are updated.
- [ ] Contract tests cover both versions until the old one is sunset.

**Sunsetting**

- [ ] The sunset date has passed and usage metrics for the old version or
      operation are near zero, or integrators confirmed migration.
- [ ] Requests already get `410 API_VERSION_SUNSET` (automatic after
      `sunset_at`). Keep the entry for at least one more release so
      clients see a clear error instead of `404`.
- [ ] Then remove the routes, the registry entry and the OpenAPI
      operations, and update the tables in this file and the API reference.
