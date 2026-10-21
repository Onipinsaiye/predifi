# Host Runbooks

Backend instances run in an auto scaling group behind the application load
balancer (`terraform/modules/compute`, `terraform/modules/loadbalancer`).

---

## BackendMemoryHigh

**Owner:** infra-oncall · **Severity:** critical

**User impact:** None until the process is killed. An OOM kill drops every
in-flight request and WebSocket connection on that instance; clients reconnect
to another instance after a few seconds.

**Dashboards & queries:**
- Per instance: `(app_memory_used_bytes / app_memory_total_bytes) * 100`
- Growth over time: `deriv(app_memory_used_bytes[30m])`
- Open WebSockets: `app_ws_connections_active`

**Diagnosis:**
1. One instance or all? One instance with steady growth since start is a leak;
   all instances at once is load.
2. Compare with `app_ws_connections_active`. Many open sockets use memory
   legitimately.
3. Check the price and pool caches in recent changes — an unbounded in-process
   cache is the usual leak.
4. Check whether a deploy happened when growth started.

**Safe mitigation:**
1. **One instance:** drain it (deregister from the target group, wait for
   connections to close, up to `PREDIFI_SHUTDOWN_TIMEOUT_SECS`) and terminate
   it; the ASG starts a fresh one.
2. **All instances, load-driven:** raise the ASG desired capacity.
3. **All instances after a deploy:** roll back the release.

**Verification:** Memory below 80% on every instance for 30 minutes.

**Rollback:** Return desired capacity to its previous value once load falls.

**Escalation:** Memory climbs back within hours of a restart (a leak) → open a
bug for backend-oncall with the growth graph. Not mitigated in 30 minutes →
secondary infra-oncall.
