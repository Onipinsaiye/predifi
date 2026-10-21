# Cache Runbooks

Redis (ElastiCache, `terraform/modules/redis`) backs response caching, rate
limiting, and sessions. With `PREDIFI_REDIS_REQUIRED=false` (the default) the
backend keeps serving when Redis is unavailable, with every read going to
Postgres.

---

## HighRedisErrors

**Owner:** infra-oncall · **Severity:** warning

**User impact:** Usually none at first. Cache misses push read load to Postgres,
so latency rises. Rate limiting may fail open or closed depending on the route.

**Dashboards & queries:**
- Errors by operation: `sum by (operation) (rate(app_redis_operations_total{result="error"}[5m]))`
- Cache hit rate: `sum by (cache_name, outcome) (rate(app_redis_cache_lookups_total[5m]))`
- ElastiCache console → *CPUUtilization*, *CurrConnections*, *Evictions*,
  *DatabaseMemoryUsagePercentage*.

**Diagnosis:**
1. Check ElastiCache events for failover or maintenance.
2. Memory near 100% with evictions → keys are being dropped; errors are likely
   `OOM command not allowed`.
3. Connection count at the limit → a connection leak or too many instances.
4. Check whether the errors are from one backend instance only (network
   problem on that host).

**Safe mitigation:**
1. **Failover:** wait for it to finish; clients reconnect automatically.
2. **Memory full:** scale the node type up, or flush only known-safe caches
   (price cache keys). Never `FLUSHALL` — it also drops sessions and rate-limit
   counters.
3. **One bad instance:** remove it from the target group and let the ASG
   replace it.
4. Watch Postgres while Redis is degraded; follow
   [database.md](database.md#databaseconnectionpoolsaturated) if the pool fills.

**Verification:** Redis error rate under 10/s for 10 minutes and cache hit rate
back to its normal level.

**Rollback:** Scale the node type back down after the incident if the larger
size is not needed.

**Escalation:** Not resolved in one business hour, or `HighP99Latency` /
`HighApiErrorRate` starts firing → secondary infra-oncall and backend-oncall.
