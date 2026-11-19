# Performance budgets

Route-level performance, accessibility, and UX budgets for the market
discovery and wallet flows, enforced in CI by Lighthouse CI against the
production build (`pnpm run build && pnpm run start`).

## Routes covered

| Route | Why it's budgeted |
| --- | --- |
| `/` | Landing page — first impression, highest traffic. |
| `/markets` | Market discovery entry point. |
| `/user/pool-market` | Primary pool browsing/search surface. |
| `/user/wallet-token` | Wallet flow. |

Config: [`lighthouserc.json`](../lighthouserc.json). Add a route here by
adding its URL to `ci.collect.url`.

## Budgets (from `lighthouserc.json`)

| Metric | Threshold | Enforcement |
| --- | --- | --- |
| Performance score | ≥ 0.80 | **Blocks CI** |
| Accessibility score | ≥ 0.90 | Warns |
| Best practices score | ≥ 0.90 | Warns |
| SEO score | ≥ 0.90 | Warns |
| First Contentful Paint | ≤ 2000ms | Warns |
| Largest Contentful Paint | ≤ 2500ms | Warns |
| Cumulative Layout Shift | ≤ 0.1 | Warns |
| Total Blocking Time | ≤ 300ms | Warns |

These thresholds are the baseline captured from a production build of this
app at the time Lighthouse CI was wired into `frontend-ci.yml`. Only the
performance score blocks the build today; the others start as warnings so
regressions are visible in the job summary/artifact without destabilizing CI
while the team builds confidence in the measurements.

## Finding the largest contributors

When performance drops, pull the per-route report:

1. Download the `lighthouse-report` artifact from the failing `Frontend
   CI/CD` run (or run locally: `pnpm run build && pnpm dlx @lhci/cli@0.14.0
   autorun`).
2. Open the route's JSON report in `.lighthouseci/` and check
   `audits["diagnostics"].details.items` for main-thread/transfer-size
   breakdowns, or load it into https://googlechrome.github.io/lighthouse/viewer/
   for the full report UI.
3. `audits["network-requests"]` and `audits["resource-summary"]` identify the
   largest transferred resources (script/image/font) driving the regression.

## Changing a budget

A budget is only moved deliberately, not loosened to make a failing build
pass:

1. Open a PR that changes `lighthouserc.json` by itself (not bundled with the
   feature that needed the change).
2. The PR description states which metric/route is changing, the new
   threshold, and why the current one is no longer achievable or is now too
   loose (e.g. a new required third-party script, or a route that no longer
   does analytics — justify with a diff, not a shrug).
3. Requires review/approval from a frontend maintainer before merge, same as
   any other CI-gating config change.
