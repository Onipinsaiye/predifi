# RFC: Conditional Market Semantics

- **Status:** Proposed
- **Issue:** #242
- **Scope:** Contract semantics before implementation
- **Applies to:** PrediFi pools whose outcome availability depends on another pool

## Summary

A conditional market is a pool that can accept and resolve predictions only
when the condition attached to a parent (upstream) pool is satisfied.

This RFC defines the protocol model, not a proposed ABI or implementation.
It preserves the existing lifecycle invariant that each pool has one-way
terminal state: `Active -> Resolved`, `Active -> Canceled`, or
`Active -> Disputed -> {Resolved, Canceled}`.

## Normative model

### Dependency graph

Each conditional market has exactly one direct upstream dependency:

```text
DependencyEdge {
    upstream_pool_id,
    downstream_pool_id,
    required_outcome,
}
```

- `upstream_pool_id` and `downstream_pool_id` must identify different pools.
- `required_outcome` must be a valid outcome index for the upstream pool.
- Dependencies may be added only while the downstream pool is `Active`.
- The graph is a directed acyclic graph (DAG). Direction is
  `upstream -> downstream`.
- An edge is immutable after the downstream pool is created.
- Transitive dependencies are allowed. A downstream market is conditional on
  every upstream dependency along the path.

For bounded on-chain traversal, the protocol will enforce a maximum dependency
depth and a maximum direct-child count. Initial implementation limits should be
proposed as protocol constants, with a default of depth 16 and 64 direct
downstream markets per pool.

### Logical state

A conditional market has one derived eligibility state in addition to its
normal pool state:

| Derived state | Meaning |
| --- | --- |
| `Blocked` | The upstream is unresolved or disputed, or its required outcome is not yet known. |
| `Eligible` | Every upstream in the transitive path is resolved and every required outcome matched. |
| `IneligibleCanceled` | An upstream resolved to a different outcome, or an upstream is canceled. |

`Blocked`, `Eligible`, and `IneligibleCanceled` are derived values. They must
not be a fourth terminal pool state.

### Creation and staking

1. Validate the direct dependency and outcome index.
2. Reject a cycle before storing the edge.
3. Reject dependencies beyond the configured depth or direct-child limit.
4. Store the dependency edge.
5. Mark the downstream pool `Blocked` if the upstream is not resolved to the
   required outcome.
6. Allow staking only while the pool is `Active` and `Eligible`.

The downstream pool's own `end_time` still controls staking. If a downstream
market becomes eligible after that cutoff, no new stake is accepted. If it
becomes eligible before the cutoff, normal staking rules apply.

### Cycle rejection

Before adding an edge `A -> B`, the contract must verify that no path exists
from `B` back to `A`.

The lookup is performed by following stored parent links with bounded
traversal. The operation must reject:

- a self-dependency;
- a direct cycle;
- an indirect cycle;
- a dependency at or beyond the configured depth limit.

Any rejection must leave graph storage unchanged and emit no successful
creation/resolution event.

### Upstream cancellation behavior

If an upstream pool enters `Canceled`, every transitive downstream pool must
enter `Canceled` and become eligible for the existing full-stake refund path.

This rule prevents an outcome-dependent pool from collecting stakes for an
invalid branch. A downstream pool cannot be resolved after an upstream
cancellation, and existing one-way claim/refund sentinels prevent replay.

Downstream terminalization is immediate for eligibility purposes: from the
moment the upstream is canceled, descendants derive `IneligibleCanceled` and
their staking and resolution paths are closed. If graph fan-out exceeds the gas
budget for a single transaction, the implementation may stage `Canceled`
terminal writes in a bounded queue. A descendant remains closed even while its
queued terminal write is pending. It becomes claimable for refund only after
its own terminal write has completed.

If an upstream pool is `Disputed`, downstream pools remain `Blocked`. They
may become eligible after a later resolved match or enter `Canceled` after a
later cancellation.

If an upstream pool resolves to an outcome other than the required outcome,
the downstream pool is canceled for outcome mismatch. It is not retained as
`Blocked` indefinitely.

### Resolution ordering

Resolution uses the following topological rule:

1. A pool cannot resolve before its own staking cutoff and resolution delay.
2. A pool with dependencies cannot resolve while any direct or transitive
   upstream is `Blocked`, `Disputed`, or `Canceled`.
3. A pool may resolve only after every upstream dependency has resolved to its
   required outcome.
4. Independent pools and different branches may resolve in any valid order.
5. Once the last upstream condition is satisfied, downstream resolution is
   enabled. It does not automatically resolve the downstream pool.

This ordering permits offline resolution of unrelated branches and forbids a
downstream pool from deriving an outcome from a stale or terminal-but-canceled
upstream branch.

## Payout model

Conditional markets retain the existing prediction payout model:

- Resolved eligible pools use the existing proportional winner calculation
  and protocol fee.
- Canceled downstream pools refund original principal for every successful
  prediction.
- A downstream pool never inherits the upstream pool's escrow or fee.
- Initial liquidity and protocol fees continue to follow existing pool rules.
- Claims remain write-once and scoped to `(user, pool_id)`.
- Rounding dust and treasury treatment remain unchanged by the dependency
  mechanism.

No payout is moved automatically at upstream resolution. Downstream users claim
or receive refunds only through the downstream pool's existing terminal path.

## Worked examples

### Example 1: Linear election branch

Three markets form this graph:

```text
Pool 10: Election winner
  required outcome 1: Candidate A
  -> Pool 20: Turnout above 60%
      required outcome 0: Yes
      -> Pool 30: A wins turnout market
```

1. Pool 20 is `Blocked` by Pool 10 until Pool 10 resolves to outcome 1.
2. Pool 30 remains `Blocked` until both Pool 10 and Pool 20 resolve:
   - Pool 10 must be outcome 1;
   - Pool 20 must be outcome 0.
3. If Pool 10 resolves to Candidate B (outcome 2), Pool 20 and Pool 30 are
   canceled and their participants receive refunds.
4. If Pool 10 resolves to Candidate A and Pool 20 later resolves to `Yes`,
   staking on Pool 30 becomes eligible, subject to its own cutoff.

### Example 2: Bracketed sports market

A playoff winner market branches into two downstream markets:

```text
Pool 40: League champion
  -> Pool 41: Champion's final margin > 10 points
  -> Pool 42: Champion's star is finals MVP
```

Both downstream pools depend on the same upstream outcome. If Pool 40 resolves
to Team C:

- Pool 41 and Pool 42 do not resolve automatically.
- Their own staking and resolution rules remain in effect.
- If the downstream requirement is Team C, both become eligible.
- If a branch requires Team D, that branch is canceled and refunded.

For Pool 41, if it resolves after becoming eligible, its winner payout is
calculated only from Pool 41's stakes. Pool 40 payouts remain independent.

### Example 3: Cancellation propagation

Pool 50 is upstream of Pool 60, and Pool 60 is upstream of Pool 70.

1. Pool 50 is canceled before resolution.
2. Pool 60 and Pool 70 immediately derive `IneligibleCanceled`.
3. Both downstream pools transition to `Canceled`.
4. Participants claim a full refund against each downstream pool.
5. Pool 70 cannot be resolved even if it already passed a staking cutoff.

## Storage and gas implications

### Proposed storage

The implementation will need to distinguish direct graph storage from existing
pool storage:

| Key or index | Proposed value |
| --- | --- |
| `Dependency(downstream_id)` | `(upstream_id, required_outcome)` |
| `Children(upstream_id)` | bounded list of downstream pool IDs |
| `DependencyDepth(pool_id)` | maximum stored depth in this DAG |
| `ConditionalState(pool_id)` | derived/cacheable eligibility state |

Persistent storage should favor parent lookups for cycle and eligibility
checks. A bounded child index supports cancellation propagation. Both indexes
require atomic updates.

`ConditionalState` may be stored for indexer consistency or derived only for
read paths. It must never be the source of truth for resolution; the
upstream pool state and edge records remain authoritative.

### Gas behavior

- Cycle validation is `O(depth)` and bounded by the depth limit.
- Eligibility evaluation is `O(depth)` in the read or resolution path.
- Cancellation propagation is `O(V + E)` for reachable descendants and must use
  bounded fan-out and depth limits.
- Direct-child iteration must be paginated or staged across invocations if
  implementation growth exceeds one transaction's gas budget.
- If atomic cascade is not safely bounded, use a one-step terminal transition
  for the upstream plus a downstream settlement queue rather than recursively
  mutating the graph.

No implementation is permitted to perform unbounded graph traversal, unbounded
recursive resolution, or child fan-out writes in a single unbounded invocation.

## Lifecycle invariants

1. **Acyclic graph:** every successful edge leaves a DAG.
2. **One-way pool state:** conditional behavior cannot reopen or change a
   terminal pool state.
3. **Cutoff:** the downstream `end_time` remains authoritative.
4. **Resolution threshold:** downstream resolution requires resolved, matched
   upstream dependencies.
5. **Cancellation conservation:** canceled downstream pools refund successful
   stake entries and create no protocol fees.
6. **Claim write-once:** conditional payout/refund replay uses the existing
   `(user, pool_id)` sentinel.
7. **No value creation:** downstream payouts use only downstream escrow.
8. **Bounded traversal:** all graph operations have explicit depth and
   fan-out limits.

The invariant review is a semantic gate, not an implementation contract:

- The graph has no retroactive effect on pools that are already terminal.
- `Resolved` and `Canceled` descendants must never return to `Active`.
- A `Disputed` upstream never grants downstream eligibility.
- A mismatched upstream resolution or upstream cancellation is terminal for
  the affected downstream branch.
- A staged cancellation queue cannot allow staking or resolution for a
  descendant that has derived `IneligibleCanceled`.
- The queue and pool state both use write-once terminal transitions so a
  retry cannot create a second refund or reopen a pool.

## Explicit non-goals

- No automatic downstream outcome inference or oracle reuse.
- No automatic payout or escrow transfer from upstream to downstream.
- No support for arbitrary boolean expressions in the initial protocol.
- No support for multiple direct upstream dependencies in one edge.
- No support for cyclic graphs or hot-swapping dependencies.
- No synthetic pool state that replaces existing `Active`, `Resolved`,
  `Canceled`, or `Disputed` states.
- No retroactive migration of existing pools.
- No generalized permissionless arbitrary market graph creation.
- No guarantee of transaction-wide cascading resolution; downstream resolution
  remains caller-triggered or keeper-triggered unless a later RFC defines
  automatic resolution.
- No frontend routing, wallet UX, or backend projection implementation in this
  RFC.

## Reference-model validation

The reference-model scenario suite must cover at least:

- a single dependency resolved to a matching outcome;
- a mismatching outcome causing downstream cancellation/refund;
- upstream cancellation propagating through two downstream levels;
- indirect cycle rejection;
- blocked resolution before upstream resolution;
- sibling branches receiving independent outcomes;
- replayed downstream claims/refunds;
- graph depth and direct-child limit rejection.

The scenarios are executable examples only; they do not imply a contract ABI
and must pass before implementation review begins.

Run the suite from the contract workspace:

```bash
cargo test -p predifi-reference-model conditional_markets
```

## Decision and next review

This RFC is accepted only as a semantic specification. It authorizes no contract
API, storage migration, or production behavior change. Implementations must
reference this document and add a separate implementation RFC or design review
before code is merged.
