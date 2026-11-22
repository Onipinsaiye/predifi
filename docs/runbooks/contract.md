# Contract Runbooks

The backend reads Soroban contract state through `PREDIFI_STELLAR_RPC_URL` and
records those calls under `query_type` values containing `contract`. The
contract itself is authoritative ([ADR 0001](../adr/0001-contract-state-authoritative.md)).

For the schema and contract change workflow (including how contract interface
changes are proposed, reviewed, and rolled out) and for release sign-off
requirements, see the maintainer handbook:
[`docs/maintainer-handbook.md`](../maintainer-handbook.md). Any contract
interface change must follow that workflow before it reaches production.

Liquidity-provider accounting (shares, deposits, withdrawals, fee accrual,
rounding, loss allocation, and lifecycle edge cases) is specified in
[RFC 0001: Liquidity-Provider Accounting](../rfcs/0001-liquidity-provider-accounting.md).
That RFC is the normative reference for LP behaviour; implementation approval
requires contract maintainer review and passing conservation property tests as
described there.

---

## ContractInteractionFailures

**Owner:** contract-oncall · **Severity:** critical

**User impact:** The backend cannot read or verify contract state. Pool totals,
resolutions, and claim status in the app may not match the chain. Users can
still sign transactions with their wallet, but the app may show wrong numbers
before and after.

**Dashboards & queries:**
- Failures by call: `sum by (query_type) (rate(app_db_queries_total{query_type=~".*contract.*", result="error"}[5m]))`
- RPC health: `GET /api/v1/health` (Soroban `getHealth` result).
- Logs (Loki): `{job="predifi", service="backend"} |~ "(?i)(soroban|rpc|contract)" | level="ERROR"`

**Diagnosis:**
1. Check RPC health and the provider status page. Timeouts or 429s point at the
   RPC, not the contract.
2. Errors that decode to a contract error code → look up the code in
   [`docs/ERROR_HANDLING_REFERENCE.md`](../ERROR_HANDLING_REFERENCE.md).
3. Check whether the contract was upgraded or the configured contract ID changed
   recently ([`docs/contract-upgrade-procedure.md`](../contract-upgrade-procedure.md)).
   A changed interface breaks the backend's calls.
4. Check whether the contract is paused (admin action) — reads that require an
   active contract will fail.

**Safe mitigation:**
1. **RPC problem:** switch `PREDIFI_STELLAR_RPC_URL` to the backup provider and
   restart instances one at a time.
2. **Wrong contract ID after a deploy:** roll back the backend release or
   restore the previous contract ID configuration.
3. **Contract interface change:** roll back the backend to the last release
   compatible with the deployed contract. Do **not** call admin or upgrade
   functions on the contract as a mitigation without maintainer approval.
   Follow the schema and contract change workflow in the
   [maintainer handbook](../maintainer-handbook.md) for the forward fix, and
   obtain release sign-off as described there before shipping it.

**Verification:** Contract error rate is 0 for 10 minutes and
`GET /api/v1/pools/<id>` for a recently active pool matches the chain.

**Rollback:** Restore the original RPC URL or contract ID and restart.

**Escalation:** Immediately page `@Web3Novalabs/maintainers` if you suspect
the contract itself is misbehaving (unexpected state, failed invariant, paused
without notice). Otherwise, not mitigated in 30 minutes → secondary
contract-oncall and backend-oncall.
