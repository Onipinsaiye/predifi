# CI workflow validation matrix

The three component workflows use `paths` filters so that a documentation-only
change does not rebuild Rust, WASM and Next.js. The trade-off is that a filter
which omits a shared input silently skips a required check. This document and
`.github/ci/workflow-matrix.json` make those dependencies explicit, and
`.github/workflows/workflow-validation.yml` verifies them on every change to the
trigger configuration or a shared boundary.

Tracking issue: [#136 — [CI] Test path filters against cross-component changes](https://github.com/Onipinsaiye/predifi/issues/136).

## Components and shared boundaries

| Boundary | Shared paths | Dependent checks | Why |
| --- | --- | --- | --- |
| Shared errors | `contract/contracts/predifi-errors/**` | Backend CI, Contract CI/CD | `backend/Cargo.toml` declares `predifi-errors` as a path dependency, and every contract crate imports it. An error-code change must rebuild both consumers. |
| Contract interfaces | `contract/contracts/**`, `contract/Cargo.toml`, `contract/scripts/**` | Backend CI, Contract CI/CD | Contract entry points, types, wasm-size checks and deploy scripts define the interface the backend talks to. |
| Generated API schema | `backend/openapi.json`, `backend/src/openapi.rs`, `backend/src/bin/openapi_export.rs` | Backend CI, Frontend CI/CD | The frontend API types are generated from the committed OpenAPI schema. Stale types must surface as frontend build/type errors. |
| Build scripts | `docker-compose.yml` | Backend CI, Frontend CI/CD | Compose builds `backend/Dockerfile.dev` and `frontend/Dockerfile.dev`, and bind-mounts `contract/` for the backend path dependency. |
| Shared Rust tooling | `rustfmt.toml` | Backend CI, Contract CI/CD | rustfmt discovers the repository-level config for both workspaces; a change can affect either formatting gate. |

Paths that intentionally do **not** trigger builds: `docs/**`, `README.md`,
`CONTRIBUTING.md`, `.editorconfig`, and the observability configuration under
`docker/` (Loki, Promtail, Grafana). Those are validated by the docs-only and
`logging-config-only` fixtures.

## Branch conditions

Every component workflow triggers for `push` and `pull_request` on the same
integration branches:

```
main, develop, contract, frontend
```

Deployment remains deliberately narrower:

| Workflow | Job | Event | Branch |
| --- | --- | --- | --- |
| Backend CI | `deploy-staging` | `push` | `main` |
| Contract CI/CD | `deploy-testnet` | `push` | `develop` |

## Pull-request fixtures

`.github/ci/workflow-matrix.json` contains the fixture list evaluated by
`.github/scripts/validate_workflow_matrix.py`. A representative subset:

| Changed files | Branch | Expected workflows |
| --- | --- | --- |
| `contract/contracts/predifi-errors/src/errors.rs` | any integration branch | Backend CI, Contract CI/CD |
| `contract/contracts/predifi-contract/src/lib.rs` | any integration branch | Backend CI, Contract CI/CD |
| `backend/openapi.json` | any integration branch | Backend CI, Frontend CI/CD |
| `backend/src/bin/openapi_export.rs` | any integration branch | Backend CI, Frontend CI/CD |
| `docker-compose.yml` | any integration branch | Backend CI, Frontend CI/CD |
| `rustfmt.toml` | any integration branch | Backend CI, Contract CI/CD |
| `frontend/lib/api.ts` | any integration branch | Frontend CI/CD |
| `backend/src/routes/v1.rs` | any integration branch | Backend CI |
| `docs/faq.md`, `README.md` | any integration branch | none |

The evaluator re-implements GitHub's `*`, `**`, `**/`, `?` and character-class
path semantics, so the fixtures exercise the same matching rules GitHub uses
rather than a looser approximation.

## Running locally

```bash
python -m pip install pyyaml
python .github/scripts/validate_workflow_matrix.py
# or a single branch, matching one CI matrix job:
python .github/scripts/validate_workflow_matrix.py --branch develop
```

The script exits non-zero and prints every mismatch when:

- a workflow is missing an integration branch or declares an unexpected one;
- a boundary sample triggers fewer or more workflows than its `dependents`;
- a fixture triggers an unexpected set of workflows;
- a docs-only change would schedule a component workflow;
- a deployment job loses its branch/event gate.

## Adding a boundary

1. Add the shared path and its consumers to `.github/ci/workflow-matrix.json`
   under `boundaries`, with at least one concrete `samples` file.
2. Add the path to each dependent workflow (`paths` under both `push` and
   `pull_request`).
3. Add a fixture if the boundary has a realistic PR shape.
4. Run the validator locally and update this table.
