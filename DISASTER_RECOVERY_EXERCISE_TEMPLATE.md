# PrediFi Disaster Recovery Exercise Report

**Exercise Date**: [DATE]  
**Exercise Type**: [Tabletop / Staged Failover / Full DR Test]  
**Scenario**: [DESCRIPTION]  
**Primary Region**: [us-east-1]  
**Alternate Region**: [us-west-2]  

## 0. PostgreSQL Backup Restore Drill Evidence

**Scheduled cadence**: Monthly (`PostgreSQL Restore Drill` workflow)
**Exercise owner**: [Database / Infrastructure owner]
**AWS account and restore region**: [NON-PRODUCTION ACCOUNT / REGION]
**Isolated subnet group and security group**: [IDENTIFIERS]
**Restore role**: [DEDICATED OIDC ROLE ARN]
**Evidence artifact**: [WORKFLOW RUN / ARTIFACT URL]
**Production application credentials used**: No

| Measurement | Target | Achieved |
|-------------|--------|----------|
| Latest restorable point age (RPO) | 3,600 seconds | [seconds] |
| Restore and validation duration (RTO) | 14,400 seconds | [seconds] |
| Pools / predictions restored | Informational | [counts] |
| Orphan predictions / negative totals / negative amounts | 0 / 0 / 0 | [counts] |
| Temporary instance cleanup | Required | [timestamp / status] |

Record failed drills as well as successful ones. Keep only the JSON evidence artifact; never include database passwords, Terraform state, or database dumps. An RPO/RTO miss requires an owner and remediation date before closing the exercise.

---

## 1. Exercise Participants

| Role | Name | Department | Status |
|------|------|-----------|--------|
| **Incident Commander** | | | Active / Backup |
| **Infrastructure Lead** | | | Active / Backup |
| **Database Lead** | | | Active / Backup |
| **Backend Lead** | | | Active / Backup |
| **Communications** | | | |
| **Observer/Facilitator** | | | |

---

## 2. Pre-Exercise Checklist

- [ ] Terraform prepared for alternate region (tested, reviewed)
- [ ] RDS backups confirmed available and recent
- [ ] Secrets Manager cross-region replication verified
- [ ] Route53 health checks configured
- [ ] Monitoring dashboards set up for both regions
- [ ] Communication channels established (Slack incident, bridge call info)
- [ ] Participant roles assigned and understood
- [ ] Success criteria defined and reviewed
- [ ] Backup/rollback plan reviewed
- [ ] Time-box set: **[DURATION]**

---

## 3. Exercise Timeline

### Detection Phase (Target: 0–5 min)

**Simulated Event**: [DESCRIBE FAILURE]

**Actions Performed**:
- [ ] Region failure simulated (Route53 health check, monitoring alert)
- [ ] Grafana/monitoring alert confirmed
- [ ] Incident channel created
- [ ] Team notified

**Timestamp**: 
- Outage detected: `[HH:MM:SS]`
- Team assembled: `[HH:MM:SS]`
- Roles assigned: `[HH:MM:SS]`

**Notes**:
```
[OBSERVATIONS / ISSUES]
```

---

### Database Recovery Phase (Target: 5–30 min)

**Assigned Owner**: [Name]

**Actions Performed**:
- [ ] Listed available RDS snapshots
- [ ] Selected snapshot: `[NAME]`
- [ ] Copied snapshot to alternate region
- [ ] Monitored snapshot copy progress
- [ ] Initiated restore from snapshot
- [ ] Waited for restore to complete
- [ ] Tested database connectivity
- [ ] Validated data integrity

**Timestamps**:
| Step | Time | Status |
|------|------|--------|
| Snapshot copy started | `[HH:MM:SS]` | ✓ |
| Snapshot copy completed | `[HH:MM:SS]` | ✓ / ✗ |
| Restore started | `[HH:MM:SS]` | ✓ |
| Restore completed | `[HH:MM:SS]` | ✓ / ✗ |
| Connectivity test | `[HH:MM:SS]` | PASS / FAIL |
| Data validation | `[HH:MM:SS]` | PASS / FAIL |

**Duration**: [__ min] (Target: 25 min)

**Commands Executed**:
```bash
# Snapshot copy
[COMMAND & OUTPUT]

# Restore
[COMMAND & OUTPUT]

# Connectivity test
[COMMAND & OUTPUT]

# Data validation
[COMMAND & OUTPUT]
```

**Issues Encountered**:
- [ ] None
- [ ] Issue #1: [DESCRIPTION]
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]
- [ ] Issue #2: [DESCRIPTION]
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]

**Success Criteria Met?**:
- [x] RDS snapshot available and recent (< 1 hour old)
- [x] Snapshot copy completed without errors
- [x] Restore completed successfully
- [x] Database accepting connections
- [x] Data consistency verified (no null/negative values)
- [ ] Actual time ≤ 35 min (planned 25 min + 40% contingency)

**Notes**:
```
[OBSERVATIONS / RECOMMENDATIONS]
```

---

### Backend Deployment Phase (Target: 30–55 min)

**Assigned Owner**: [Name]

**Actions Performed**:
- [ ] Updated Terraform tfvars for alternate region
- [ ] Initialized Terraform backend
- [ ] Generated Terraform plan
- [ ] Reviewed plan for correctness
- [ ] Applied Terraform infrastructure
- [ ] Configured environment variables (DB, Redis, Stellar RPC)
- [ ] Waited for ASG instances to launch
- [ ] Verified health checks passing
- [ ] Tested API endpoints

**Timestamps**:
| Step | Time | Status |
|------|------|--------|
| Terraform init started | `[HH:MM:SS]` | ✓ |
| Terraform plan generated | `[HH:MM:SS]` | ✓ / ✗ |
| Plan reviewed & approved | `[HH:MM:SS]` | ✓ / ✗ |
| Terraform apply started | `[HH:MM:SS]` | ✓ |
| Infrastructure deployed | `[HH:MM:SS]` | ✓ / ✗ |
| Env vars configured | `[HH:MM:SS]` | ✓ / ✗ |
| ASG instances healthy | `[HH:MM:SS]` | ✓ / ✗ |
| API health check | `[HH:MM:SS]` | PASS / FAIL |

**Duration**: [__ min] (Target: 25 min)

**Infrastructure Deployed**:
- [ ] Security Groups: [COUNT]
- [ ] Launch Template: [NAME]
- [ ] Auto Scaling Group: [NAME] (Capacity: [N])
- [ ] ALB: [NAME]
- [ ] Target Group: [NAME]
- [ ] Redis (if applicable): [NAME]

**Health Check Results**:
```bash
# Backend health
curl -I http://ALB-DNS/health
# HTTP [CODE] - [STATUS]

# API health
curl -I http://ALB-DNS/api/v1/health
# HTTP [CODE] - [STATUS]

# API functionality
curl -s http://ALB-DNS/api/v1/pools | jq '.data | length'
# Count: [N]
```

**Issues Encountered**:
- [ ] None
- [ ] Issue #1: [DESCRIPTION]
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]
- [ ] Issue #2: [DESCRIPTION]
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]

**Success Criteria Met?**:
- [ ] Terraform plan reviewed and no errors
- [ ] Infrastructure deployed without errors
- [ ] ALB created with correct configuration
- [ ] ASG scaling correctly (desired capacity met)
- [ ] All instances healthy (health checks passing)
- [ ] API endpoints returning 200 OK
- [ ] Database connection working from instances
- [ ] Actual time ≤ 40 min (planned 25 min + 60% contingency)

**Notes**:
```
[OBSERVATIONS / RECOMMENDATIONS]
```

---

### DNS Failover Phase (Target: 55–60 min)

**Assigned Owner**: [Name]

**Actions Performed**:
- [ ] Retrieved new ALB DNS and Zone ID
- [ ] Generated Route53 change batch
- [ ] Applied Route53 change
- [ ] Verified DNS propagation

**Timestamps**:
| Step | Time | Status |
|------|------|--------|
| ALB DNS retrieved | `[HH:MM:SS]` | ✓ |
| Route53 change applied | `[HH:MM:SS]` | ✓ / ✗ |
| DNS query verification | `[HH:MM:SS]` | PASS / FAIL |

**Duration**: [__ min] (Target: 5 min)

**Route53 Configuration**:
- Record Name: `api.predifi.example.com`
- Record Type: `A` (Alias)
- Target ALB DNS: `[NEW-ALB-DNS]`
- Target ALB Zone ID: `[ZONE-ID]`
- Evaluate Target Health: `true`

**DNS Propagation Test**:
```bash
# Before change
nslookup api.predifi.example.com
# Result: [OLD-ALB-IP]

# After change (wait 30-60 sec)
nslookup api.predifi.example.com
# Result: [NEW-ALB-IP]

# Verify
dig api.predifi.example.com +short
# Result: [NEW-ALB-IP]
```

**Issues Encountered**:
- [ ] None
- [ ] Issue #1: [DESCRIPTION]
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]

**Success Criteria Met?**:
- [ ] Route53 change applied successfully
- [ ] DNS record updated in all nameservers
- [ ] New ALB IP resolves correctly
- [ ] No DNS cache issues observed
- [ ] Actual time ≤ 10 min (planned 5 min + 100% contingency)

**Notes**:
```
[OBSERVATIONS / RECOMMENDATIONS]
```

---

### Validation Phase (Target: 60–90 min)

**Assigned Owner**: All

**Tests Performed**:

#### 1. Health Endpoints
- [ ] `/health` returning 200 OK
- [ ] `/api/v1/health` returning 200 OK
- [ ] Response time: [__ ms]

```bash
curl -I https://api.predifi.example.com/health
HTTP/1.1 [CODE]

curl -I https://api.predifi.example.com/api/v1/health
HTTP/1.1 [CODE]
```

#### 2. Functional API Tests
- [ ] Fetch pools: ✓ / ✗
- [ ] Fetch predictions: ✓ / ✗
- [ ] User stats: ✓ / ✗
- [ ] Create test prediction: ✓ / ✗

```bash
# Pools
curl -s https://api.predifi.example.com/api/v1/pools \
  -H "Authorization: Bearer [TOKEN]" | jq '.data | length'
Result: [N] pools

# Predictions
curl -s https://api.predifi.example.com/api/v1/predictions \
  -H "Authorization: Bearer [TOKEN]" | jq '.data | length'
Result: [N] predictions

# User stats
curl -s https://api.predifi.example.com/api/v1/user/stats \
  -H "Authorization: Bearer [TOKEN]" | jq '.total_wins'
Result: [N]
```

#### 3. Data Consistency Checks
- [ ] Row count matches pre-outage baseline
- [ ] No data corruption (negative values, null constraints)
- [ ] Recent transactions within RPO window (1 hour)

```bash
psql -h [NEW-DB] -U predifi_user -d predifi_db << EOF
SELECT COUNT(*) as pools FROM pools;
[N]

SELECT COUNT(*) as predictions FROM predictions;
[N]

SELECT COUNT(*) FROM predictions WHERE amount < 0;
[0]

SELECT COUNT(*) FROM pools WHERE pool_status NOT IN ('open','closed','resolved');
[0]

SELECT MAX(updated_at) FROM pools;
[TIMESTAMP - should be recent]
EOF
```

**Data Validation Results**:
| Check | Expected | Actual | Status |
|-------|----------|--------|--------|
| Pool count | [N] | [N] | ✓ / ✗ |
| Prediction count | [N] | [N] | ✓ / ✗ |
| Negative amounts | 0 | [N] | ✓ / ✗ |
| Invalid states | 0 | [N] | ✓ / ✗ |
| Latest update | < 1 hr ago | [TIME] | ✓ / ✗ |

#### 4. Blockchain Integration
- [ ] Stellar RPC reachable: ✓ / ✗
- [ ] RPC health check: `[HEALTHY / UNHEALTHY]`
- [ ] Can query Stellar ledger: ✓ / ✗

```bash
curl -s https://api.predifi.example.com/api/v1/health | jq '.blockchain_status'
Result: [HEALTHY / UNHEALTHY]
```

#### 5. Application Logs
- [ ] No ERROR level logs: ✓ / ✗
- [ ] No exception traces: ✓ / ✗
- [ ] Request latency normal: [__ ms avg]

```bash
aws logs tail /aws/ec2/predifi-prod-us-west-2 --follow --since 30m --filter-pattern "ERROR"
Result: [NONE / N ERRORS]
```

#### 6. Load Test (Optional)
```bash
# Simulate light traffic
ab -n 100 -c 5 https://api.predifi.example.com/api/v1/health

# Results:
# Requests/sec: [N]
# Failed requests: [N]
# Avg response time: [N ms]
```

**Validation Timeline**:
| Test | Started | Completed | Result |
|------|---------|-----------|--------|
| Health endpoints | `[HH:MM:SS]` | `[HH:MM:SS]` | PASS / FAIL |
| API tests | `[HH:MM:SS]` | `[HH:MM:SS]` | PASS / FAIL |
| Data consistency | `[HH:MM:SS]` | `[HH:MM:SS]` | PASS / FAIL |
| Blockchain | `[HH:MM:SS]` | `[HH:MM:SS]` | PASS / FAIL |
| Logs review | `[HH:MM:SS]` | `[HH:MM:SS]` | PASS / FAIL |

**Issues Encountered**:
- [ ] None
- [ ] Issue #1: [DESCRIPTION]
  - Severity: Critical / Major / Minor
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]

**Success Criteria Met?**:
- [ ] All health checks passing
- [ ] API endpoints functional
- [ ] Data consistency verified (no loss beyond RPO)
- [ ] No critical errors in logs
- [ ] Blockchain integration working
- [ ] Actual time ≤ 30 min (planned 20 min + 50% contingency)

**Notes**:
```
[OBSERVATIONS / RECOMMENDATIONS]
```

---

### Secrets & TLS Phase (Parallel, Target: 60–90 min)

**Assigned Owner**: [Name]

**Actions Performed**:
- [ ] JWT secret restored / replicated
- [ ] ACM certificate verified in new region
- [ ] ALB HTTPS listener updated (if new cert)
- [ ] Database credentials updated
- [ ] Redis auth token verified

**Results**:
- [ ] JWT secret available: ✓ / ✗
- [ ] ACM certificate valid: ✓ / ✗
- [ ] HTTPS connections working: ✓ / ✗
- [ ] DB credentials correct: ✓ / ✗

**Issues Encountered**:
- [ ] None
- [ ] Issue #1: [DESCRIPTION]
  - Root Cause: [CAUSE]
  - Resolution: [WHAT WAS DONE]
  - Time Impact: [+X min]

**Notes**:
```
[OBSERVATIONS / RECOMMENDATIONS]
```

---

## 4. Overall Timeline Summary

| Phase | Target Duration | Actual Duration | Variance | Status |
|-------|-----------------|-----------------|----------|--------|
| **Detection** | 5 min | [__] min | [+/- __] min | ✓ / ✗ |
| **DB Restore** | 25 min | [__] min | [+/- __] min | ✓ / ✗ |
| **Backend Deploy** | 25 min | [__] min | [+/- __] min | ✓ / ✗ |
| **DNS Failover** | 5 min | [__] min | [+/- __] min | ✓ / ✗ |
| **Validation** | 20 min | [__] min | [+/- __] min | ✓ / ✗ |
| **Secrets/TLS** | 20 min (parallel) | [__] min | [+/- __] min | ✓ / ✗ |
| **TOTAL RTO** | **90 min** | **[__] min** | **[+/- __] min** | **✓ / ✗** |

**RTO Met?**: ✓ YES / ✗ NO  
**RPO Met?**: ✓ YES / ✗ NO (Data loss: [__ minutes])

---

## 5. Detailed Issues Log

### Issue #1

**Title**: [ISSUE TITLE]  
**Severity**: ⚠️ Critical / 🔴 Major / 🟡 Minor / 🟢 Informational

**Timeline**:
- Discovered: `[HH:MM:SS]`
- Phase: [PHASE NAME]
- Blocked: [YES / NO]

**Description**:
```
[DETAILED DESCRIPTION OF ISSUE]
```

**Root Cause**:
```
[ROOT CAUSE ANALYSIS]
```

**Immediate Resolution**:
```
[WHAT WAS DONE TO FIX]
```

**Permanent Fix (Action Item)**:
```
[RECOMMENDED FIX FOR FUTURE]
```

**Time Impact**: [+X min]

**Owner**: [NAME]

---

### Issue #2

[REPEAT ABOVE TEMPLATE FOR EACH ISSUE]

---

## 6. Success Criteria Evaluation

| Criterion | Target | Actual | Met? |
|-----------|--------|--------|------|
| Database restored within RTO | 25 min | [__] min | ✓ / ✗ |
| Backend deployed within RTO | 25 min | [__] min | ✓ / ✗ |
| DNS failover completed | 5 min | [__] min | ✓ / ✗ |
| All API health checks passing | 100% | [_]% | ✓ / ✗ |
| Data consistency verified | Yes | [YES/NO] | ✓ / ✗ |
| Zero data loss | Yes | [YES/NO] | ✓ / ✗ |
| No critical errors in logs | Yes | [YES/NO] | ✓ / ✗ |
| Blockchain RPC healthy | Yes | [YES/NO] | ✓ / ✗ |
| Actual RTO ≤ 4 hours | Yes | [YES/NO] | ✓ / ✗ |
| Overall exercise successful | Yes | [YES/NO] | ✓ / ✗ |

---

## 7. Key Findings

### What Went Well ✓

1. [OBSERVATION]
2. [OBSERVATION]
3. [OBSERVATION]

### What Could Be Improved 🔧

1. [AREA FOR IMPROVEMENT]
   - Current state: [DESCRIPTION]
   - Recommendation: [RECOMMENDATION]
   - Priority: High / Medium / Low
   - Owner: [NAME]

2. [AREA FOR IMPROVEMENT]
   - Current state: [DESCRIPTION]
   - Recommendation: [RECOMMENDATION]
   - Priority: High / Medium / Low
   - Owner: [NAME]

3. [AREA FOR IMPROVEMENT]
   - Current state: [DESCRIPTION]
   - Recommendation: [RECOMMENDATION]
   - Priority: High / Medium / Low
   - Owner: [NAME]

---

## 8. Action Items (from Exercise)

| # | Action | Priority | Owner | Due Date | Status |
|---|--------|----------|-------|----------|--------|
| 1 | [ACTION] | High / Med / Low | [NAME] | [DATE] | ☐ |
| 2 | [ACTION] | High / Med / Low | [NAME] | [DATE] | ☐ |
| 3 | [ACTION] | High / Med / Low | [NAME] | [DATE] | ☐ |
| 4 | [ACTION] | High / Med / Low | [NAME] | [DATE] | ☐ |

---

## 9. Updated RTO/RPO Targets

**Based on Exercise Results:**

| Metric | Previous Target | Updated Target | Rationale |
|--------|-----------------|-----------------|-----------|
| Database Recovery RTO | 25 min | [__] min | [RATIONALE] |
| Backend Deploy RTO | 25 min | [__] min | [RATIONALE] |
| Overall Regional Failover RTO | 90 min | [__] min | [RATIONALE] |
| Database RPO | 1 hour | [__] min | [RATIONALE] |

---

## 10. Recommendations for Next Exercise

1. [RECOMMENDATION]
2. [RECOMMENDATION]
3. [RECOMMENDATION]

---

## 11. Document Sign-Off

| Role | Name | Signature | Date |
|------|------|-----------|------|
| Exercise Facilitator | | | |
| Incident Commander | | | |
| Infrastructure Lead | | | |
| Database Lead | | | |
| VP Engineering / CTO | | | |

---

## 12. Appendices

### A. Test Logs

**Backend Deploy Log**:
```
[TERRAFORM OUTPUT]
```

**Database Restore Log**:
```
[RDS OUTPUT]
```

**Validation Log**:
```
[HEALTH CHECK OUTPUT]
```

### B. Screenshots

[ATTACH SCREENSHOTS OF DASHBOARDS, LOGS, ETC.]

### C. Incident Channel Transcript

[ATTACH SLACK THREAD OR MEETING TRANSCRIPT IF APPLICABLE]

---

**Exercise Report Completed**: [DATE] [TIME]  
**Next Exercise Scheduled**: [DATE] [TYPE]  
**Next Review Date**: [DATE]
