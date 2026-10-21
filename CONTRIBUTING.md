# Contributing to PrediFi

Welcome to PrediFi! We appreciate your interest in contributing to the project.

This root guide provides an overview of how to contribute across the entire repository. PrediFi consists of multiple components, including the Rust backend and Soroban smart contracts.

---

## 🍴 How to Fork and Clone

1. **Fork the Repository**: Click the **Fork** button at the top-right of the repository page on GitHub.
2. **Clone your Fork**:
   ```bash
   git clone https://github.com/YOUR-USERNAME/predifi.git
   cd predifi
   ```
3. **Set Up Remote**:
   ```bash
   git remote add upstream https://github.com/predifi/predifi.git
   ```

---

## 🌿 Branch Naming

Create a feature or bugfix branch off `main` before making your changes:

- `feat/feature-name` (e.g. `feat/batch-claim`)
- `fix/issue-description` (e.g. `fix/issue-1648`)
- `docs/topic-name` (e.g. `docs/issue-templates`)

---

## 💬 Commit Message Style

We follow Conventional Commits standard practices. Scope your commit message appropriately based on the area of code modified:

- **General**: `fix: resolve race condition in worker`
- **Contract**: `feat(contract): implement batch prediction claiming` or `docs(contract): document create_pool`
- **Backend**: `feat(backend): add redis cache layer`
- **Frontend**: `fix(frontend): adjust loading skeleton component`
- **Docs**: `docs: add comprehensive API guide`

---

## 📬 How to Open a Pull Request

1. **Keep Branches Updated**: Rebase or update your branch against upstream `main`:
   ```bash
   git fetch upstream
   git rebase upstream/main
   ```
2. **Run Tests & Linters**: Make sure all checks pass locally before opening a PR.
3. **Push to Your Fork**:
   ```bash
   git push origin feat/your-branch-name
   ```
4. **Create Pull Request**: Open a PR against the `main` branch. Fill out the pull request template completely and link any relevant issues.

The template includes a **Deployment Impact** section. Tick anything your change
needs — a migration, a new environment variable, a Terraform apply, a contract
redeploy — and say what has to happen and in what order. A reviewer cannot infer
a deployment step from a diff, and finding out after merge is the expensive way.

---

## 🧭 Architecture Decision Records

Read the [Architecture Decision Records](docs/adr/README.md) before making a
cross-component change. Add or update an ADR when a change affects contract
compatibility (calls, events, errors, storage, settlement, or upgrades), oracle
trust or resolution authority, consistency between on-chain state and backend
or frontend projections, or deployment, migration, rollback, or operational
boundaries. The ADR must explain the context, decision, alternatives,
consequences, and linked implementation issues. Keep the ADR after the work is
merged; supersede it with a new ADR if the decision changes.

An ADR is not needed for an isolated implementation detail, a documentation
correction, or a change that follows an existing decision without changing its
tradeoffs. When uncertain, open a proposed ADR and ask the affected area owners
to review it.

---

## 👥 Code Review and Ownership

Reviewers are assigned automatically from [`.github/CODEOWNERS`](.github/CODEOWNERS),
which maps each top-level area to the people responsible for it:

| Path | Reviewers |
|---|---|
| `/contract/` | Contract reviewers |
| `/backend/` | Backend reviewers |
| `/frontend/` | Frontend reviewers |
| `/terraform/`, `/docker/` | Infrastructure reviewers |
| `/.github/` | Maintainers |
| everything else | Maintainers |

A pull request touching more than one area requests a review from each owner,
so a change spanning the contract and the backend needs both to sign off.

Two things worth knowing if you are editing that file:

- **The last matching rule wins**, not the most specific one. This is the
  opposite of `.gitignore`, and it is why the catch-all `*` sits at the top.
- **An owner without write access is silently skipped.** GitHub reports no
  error, so a rule naming a team that does not exist assigns nobody while
  making the repository look covered. That is the first thing to check if a
  review is not being requested.

---

## 📚 Per-Area Contributing Guides

For specific setup, building, testing, and linting instructions for individual sub-projects, please refer to:

- ⚙️ **Backend Guide**: [backend/README.md](backend/README.md)
- 📜 **Smart Contracts Guide**: [contract/CONTRIBUTING.md](contract/CONTRIBUTING.md)
