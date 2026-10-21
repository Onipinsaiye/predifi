# PrediFi Regional Disaster Recovery Plan

**Document Version:** 1.0  
**Last Updated:** October 19, 2024  
**Status:** For Review and Tabletop Exercise  

---

## Executive Summary

This document defines regional disaster recovery objectives, procedures, and targets for the PrediFi decentralized prediction protocol. It specifies Recovery Time Objectives (RTO), Recovery Point Objectives (RPO), failover capabilities, and sequencing for restoring services during a region-wide outage.

**Key Objectives:**
- Minimize service downtime during AWS region failures
- Define acceptable data loss windows and recovery priorities
- Establish clear runbooks for failover operations
- Document dependencies and failure modes
- Enable rapid recovery with measured objectives

---

## 1. Service Inventory and Classification

### Core Services

| Service | Type | Criticality | Location | Failover Capability |
|---------|------|------------|----------|-------------------|
| **Frontend** | Web UI | High | CloudFront / S3 / ALB | Multi-region capable (via CDN) |
| **Backend API** | REST API | Critical | ASG in AWS region | Single-region only (multi-AZ within region) |
| **PostgreSQL DB** | Data Store | Critical | RDS Multi-AZ | Automated failover (AZ-level), manual PITR |
| **Redis Cache** | Cache / State | Medium | ElastiCache | Automatic failover (replica replication) |
| **Stellar RPC** | Blockchain Gateway | Critical | External endpoint | Manual DNS failover to alternate RPC |
| **DNS (Route53)** | Service Discovery | Critical | Route53 (global) | Global service, no failover needed |
| **TLS Certificates** | Security | Critical | ACM (region-scoped) | Regional; requires re-issuance for new region |
| **Monitoring (Prometheus/Grafana)** | Observability | Low | EC2 in private subnet | Ephemeral; can restart on recovery |

### Dependency Graph

```
User Browser
    ↓ (HTTPS)
CloudFront / Route53 DNS
    ↓
AWS ALB (public subnet)
    ↓
ASG EC2 Instances (private subnet) ← Security Group ingress
    ├→ PostgreSQL RDS (Multi-AZ) ← Connection pool, backup snapshots
    ├→ ElastiCache Redis (replication group) ← TLS, at-rest encryption
    ├→ Stellar RPC endpoint (external) ← Health checks, retries
    └→ Monitoring EC2 (Prometheus, Grafana) ← Optional scraping
```

---

## 2. RTO/RPO Targets by Service

### Recovery Time Objective (RTO) — Maximum acceptable downtime

| Scenario | Service | RTO | Justification |
|----------|---------|-----|---------------|
| **Single AZ Failure** (within region) | All | 2–3 minutes | RDS Multi-AZ automatic failover; ASG auto-replacement |
| **Regional Outage** (complete AWS region unavailable) | Backend API | 4–6 hours | Manual intervention required; DNS failover to alternate region/setup |
| **Regional Outage** | PostgreSQL | 4–8 hours | RDS snapshot restore to alternate region; data integrity verification |
| **Regional Outage** | Redis Cache | 30 minutes | Cache data is non-critical; miss = database query; ephemeral loss acceptable |
| **Regional Outage** | Frontend UI | 15 minutes | Static content via S3 + CloudFront; reroute to cached/static version |
| **Stellar RPC Failure** | Blockchain queries | 10 minutes | Manual DNS update to alternate RPC endpoint |

### Recovery Point Objective (RPO) — Maximum acceptable data loss

| Service | RPO | Current Capability | Gap |
|---------|-----|-------------------|-----|
| **PostgreSQL** | 1 hour | 7-day automated backups + point-in-time recovery (PITR) | No cross-region replication; PITR within same region only |
| **Redis** | 0 (ephemeral) | Replication group with synchronous replication | Non-persistent cache; data loss acceptable |
| **Transaction Ledger** | 0 (immutable) | Stellar blockchain (on-chain state immutable) | Replicated across Stellar network; no action needed |
| **Configuration/Secrets** | 30 minutes | AWS Secrets Manager, versioned parameters | Not replicated across regions; manual restore needed |

---

## 3. Single AZ Failure (Within-Region High Availability)

### Current State

**Multi-AZ Architecture:**
- **Compute**: ASG spans 2+ availability zones
- **Database**: RDS Multi-AZ standby replica in separate AZ
- **Cache**: ElastiCache replication group (primary + replica in different AZs)

### Failover Process (Automatic)

| Component | Failure | Detection | Action | RTO |
|-----------|---------|-----------|--------|-----|
| **EC2 Instance (Compute)** | Instance crash / AZ loss | ALB health check (30s interval) | ASG launches replacement in healthy AZ | 1–2 min |
| **RDS Primary** | AZ failure or crash | AWS RDS monitoring | Automatic failover to standby replica (same region, different AZ) | 2–3 min |
| **ElastiCache Primary** | Node failure | Redis cluster monitoring | Automatic failover to replica; resync | 30–60 sec |

### Verification Steps

1. **Monitor CloudWatch metrics:**
   - `ASGGroupDesiredCapacity` vs. `GroupInServiceInstances` (should match after ~1–2 min)
   - `RDSFailover*` events in RDS console
   - `ElastiCache Failover*` events

2. **Test health checks:**
   ```bash
   curl -I https://api.predifi.example.com/health
   # Should return 200 OK within 30 seconds
   ```

3. **Validate database:**
   - Query RDS replication status: Verify new primary has promoted and is accepting writes
   - Check application logs for connection errors

---

## 4. Regional Outage (Complete AWS Region Failure)

### Scope

Entire AWS region (us-east-1, eu-west-1, etc.) is unavailable:
- All AZs in the region are offline
- Route53 DNS is unaffected (global service)
- But all regional resources (EC2, RDS, ElastiCache, ALB, ACM cert) are lost

### Services Affected

- Backend API (ASG, ALB)
- PostgreSQL RDS
- ElastiCache Redis
- Monitoring EC2 (Prometheus/Grafana)

### Recovery Priority Order

1. **DNS Failover** (tier 0) — Enable traffic to alternate region
2. **PostgreSQL Restore** (tier 1) — Restore database to alternate region
3. **Redis Warm-up** (tier 2) — Repopulate cache (lower priority; can operate with cache miss)
4. **Backend API Scale** (tier 1) — Launch new ASG in alternate region and configure health checks
5. **Monitoring** (tier 3) — Re-setup Prometheus/Grafana for observability

### Detailed Failover Runbook

#### Phase 1: Detection & Alerting (~0–5 minutes)

1. **On-call engineer receives alert** (Grafana or third-party monitoring):
   - PrediFi API responds with 503 / timeouts
   - CloudWatch ALB target health shows unhealthy
   - Multiple backend services report connectivity failures

2. **Confirm regional outage**:
   ```bash
   # Check AWS status page
   https://status.aws.amazon.com/
   
   # Try direct health check (if not blocked by region failure)
   curl -I https://api.predifi.example.com/health
   # Expected: timeout or connection refused if region is down
   
   # Check DNS resolution (Route53 is global)
   nslookup api.predifi.example.com
   # Should resolve to ALB IP (but ALB is unreachable if region is down)
   ```

3. **Initiate incident response**:
   - Page senior engineer / incident commander
   - Document timestamp: "Outage detected at 2024-10-19T14:32:00Z"
   - Create incident channel (Slack, PagerDuty, etc.)

#### Phase 2: Database Recovery (~5–45 minutes)

**Goal**: Restore PostgreSQL to alternate region using automated backups or snapshot.

1. **List available RDS backups** (AWS Console or CLI):
   ```bash
   aws rds describe-db-snapshots \
     --region us-east-1 \
     --query "DBSnapshots[?DBInstanceIdentifier=='predifi-prod-db']" \
     --output table
   ```
   - Expected: Multiple snapshots; choose most recent within acceptable RPO (< 1 hour old)

2. **Restore snapshot to alternate region** (e.g., from us-east-1 to us-west-2):
   ```bash
   aws rds copy-db-snapshot \
     --source-db-snapshot-identifier arn:aws:rds:us-east-1:ACCOUNT:snapshot:predifi-prod-db-2024-10-19-14-00 \
     --target-db-snapshot-identifier predifi-prod-db-copy-2024-10-19 \
     --region us-west-2
   
   # Wait for copy to complete (5–15 minutes for large DB)
   aws rds describe-db-snapshots \
     --region us-west-2 \
     --db-snapshot-identifier predifi-prod-db-copy-2024-10-19 \
     --query "DBSnapshots[0].Status"
   # Poll until Status = "available"
   ```

3. **Restore to new RDS instance in alternate region**:
   ```bash
   aws rds restore-db-instance-from-db-snapshot \
     --db-instance-identifier predifi-prod-db-us-west-2 \
     --db-snapshot-identifier predifi-prod-db-copy-2024-10-19 \
     --region us-west-2 \
     --db-instance-class db.t4g.medium \
     --multi-az \
     --no-publicly-accessible
   
   # Wait for restore (5–10 minutes)
   ```

4. **Verify database connectivity**:
   ```bash
   # Get new RDS endpoint
   aws rds describe-db-instances \
     --region us-west-2 \
     --db-instance-identifier predifi-prod-db-us-west-2 \
     --query "DBInstances[0].Endpoint.Address"
   
   # Test connection (from EC2 or bastion in alternate region)
   psql -h <new-rds-endpoint> -U predifi_user -d predifi_db -c "SELECT version();"
   ```

5. **Perform data validation**:
   ```sql
   -- Connect to restored database
   SELECT COUNT(*) as pool_count FROM pools;
   SELECT COUNT(*) as prediction_count FROM predictions;
   SELECT MAX(updated_at) as latest_update FROM pools;
   
   -- Verify no corruption:
   SELECT * FROM pools WHERE pool_id < 0;  -- Should be empty
   SELECT * FROM predictions WHERE amount < 0;  -- Should be empty
   ```

**Timeline**: ~5–10 min (copy) + 5–10 min (restore) + 5 min (validation) = **15–25 minutes**

#### Phase 3: Backend Deployment to Alternate Region (~25–45 minutes)

**Goal**: Deploy backend API to alternate region and route traffic.

1. **Provision new infrastructure** (via Terraform or CloudFormation):
   ```bash
   cd terraform/environments/production
   # Update terraform.tfvars to target alternate region
   sed -i 's/us-east-1/us-west-2/g' terraform.tfvars
   
   # Initialize for alternate region (if not already done)
   terraform init \
     -backend-config="bucket=predifi-terraform-state" \
     -backend-config="key=production-us-west-2/terraform.tfstate" \
     -backend-config="region=us-west-2"
   
   # Verify plan
   terraform plan -out=failover.tfplan
   
   # Apply infrastructure (ALB, ASG, security groups, etc.)
   terraform apply failover.tfplan
   
   # Wait for resources: ~5–10 minutes
   ```

2. **Configure environment variables for new region**:
   - Update **Backend Container** with new RDS endpoint:
     ```bash
     aws ssm put-parameter \
       --name /predifi/prod/database_url \
       --value "postgresql://user:pass@predifi-prod-db-us-west-2.xyz.us-west-2.rds.amazonaws.com:5432/predifi_db" \
       --region us-west-2
     ```
   - Update **Redis connection**:
     ```bash
     aws ssm put-parameter \
       --name /predifi/prod/redis_url \
       --value "rediss://predifi-prod-redis-us-west-2.xyz.us-west-2.cache.amazonaws.com:6379" \
       --region us-west-2
     ```
   - Ensure **Stellar RPC endpoint** is accessible from new region (likely no change; external service)

3. **Trigger deployment**:
   ```bash
   # Push new image or trigger redeploy
   aws ec2 create-auto-scaling-group \
     --region us-west-2 \
     --launch-template LaunchTemplateName=predifi-prod-us-west-2,Version='$Latest' \
     --availability-zones us-west-2a us-west-2b \
     --min-size 2 --max-size 4 --desired-capacity 2
   
   # Wait for instances to launch and pass health checks: ~5–10 minutes
   ```

4. **Verify backend health**:
   ```bash
   # Get ALB DNS
   aws elbv2 describe-load-balancers \
     --region us-west-2 \
     --query "LoadBalancers[?LoadBalancerName=='predifi-prod-alb-us-west-2'].DNSName" \
     --output text
   
   # Test health endpoint
   curl -I http://<alb-dns>/health
   # Expected: 200 OK
   
   # Test API endpoint
   curl -s http://<alb-dns>/api/v1/pools | jq .
   # Should return JSON pool data
   ```

**Timeline**: ~10 min (Terraform plan/apply) + 10 min (instance launch) + 5 min (health checks) = **~25 minutes**

#### Phase 4: DNS Failover (~45–50 minutes)

**Goal**: Point Route53 alias record to new ALB in alternate region.

1. **Update Route53 DNS record**:
   ```bash
   # Get new ALB details
   NEW_ALB_DNS=$(aws elbv2 describe-load-balancers \
     --region us-west-2 \
     --query "LoadBalancers[0].DNSName" \
     --output text)
   
   NEW_ALB_ZONE_ID=$(aws elbv2 describe-load-balancers \
     --region us-west-2 \
     --query "LoadBalancers[0].CanonicalHostedZoneId" \
     --output text)
   
   # Update Route53 record
   aws route53 change-resource-record-sets \
     --hosted-zone-id Z1234567890ABC \
     --change-batch '{
       "Changes": [{
         "Action": "UPSERT",
         "ResourceRecordSet": {
           "Name": "api.predifi.example.com",
           "Type": "A",
           "AliasTarget": {
             "HostedZoneId": "'${NEW_ALB_ZONE_ID}'",
             "DNSName": "'${NEW_ALB_DNS}'",
             "EvaluateTargetHealth": true
           }
         }
       }]
     }'
   ```

2. **Verify DNS propagation**:
   ```bash
   # Wait 30–60 seconds for DNS TTL to expire
   sleep 60
   
   nslookup api.predifi.example.com
   # Should resolve to new ALB IP in us-west-2
   
   dig api.predifi.example.com +short
   # Verify IP matches new ALB
   ```

3. **Monitor traffic shift**:
   ```bash
   # Watch ALB target health in us-west-2
   watch -n 5 'aws elbv2 describe-target-health \
     --region us-west-2 \
     --target-group-arn arn:aws:elasticloadbalancing:us-west-2:ACCOUNT:targetgroup/predifi-api/xyz'
   ```

**Timeline**: ~2–3 min (Route53 change) + ~1 min (DNS propagation) = **~5 minutes**

#### Phase 5: Validation and Smoke Tests (~50–70 minutes)

1. **Health checks**:
   ```bash
   curl -I https://api.predifi.example.com/health
   # Expected: 200 OK
   
   curl -I https://api.predifi.example.com/api/v1/health
   # Expected: 200 OK
   ```

2. **Functional testing**:
   ```bash
   # Fetch pools
   curl -s https://api.predifi.example.com/api/v1/pools \
     -H "Authorization: Bearer <test-jwt-token>" | jq .
   
   # Verify data consistency (should match pre-outage state within 1-hour RPO)
   SELECT COUNT(*) FROM pools;  -- Compare with production baseline
   ```

3. **Stellar RPC connectivity**:
   ```bash
   # Verify backend can reach Stellar RPC
   curl -s https://api.predifi.example.com/api/v1/health | jq .blockchain_rpc_status
   # Expected: "healthy"
   ```

4. **Monitor for errors**:
   ```bash
   # Check backend logs for exceptions
   aws logs tail /aws/ec2/predifi-prod-us-west-2 --follow
   # Should see normal request logging, no errors
   ```

**Timeline**: ~10–20 minutes (full testing cycle)

#### Phase 6: Secrets and TLS Recovery (~70–90 minutes, parallel)

1. **JWT Secret Restoration**:
   - **Source**: Secrets Manager in failed region (snapshot/backup if available)
   - **Action**: Manually recreate or restore from encrypted backup
   - **Consideration**: Tokens signed with old key will remain valid during rotation period
   - **Command**:
     ```bash
     aws secretsmanager create-secret \
       --region us-west-2 \
       --name predifi/prod/jwt_secret \
       --secret-string <base64-encoded-secret>
     ```

2. **ACM Certificate for New Region** (if not reusing wildcard):
   - Request new ACM certificate: `*.predifi.example.com` in us-west-2
   - DNS validation (automatic, Route53 integration): ~5–10 min
   - Update ALB listener to use new certificate
   - Or: Use wildcard certificate already provisioned in us-west-2

3. **Credentials and API Keys**:
   - Stellar account / keypair: Restore from encrypted backup
   - Database credentials: Use AWS Secrets Manager

**Timeline**: ~10–20 minutes (parallel execution)

### Total Regional Failover Time

| Phase | Duration | Notes |
|-------|----------|-------|
| Detection | 5 min | Monitoring alert + confirmation |
| DB Restore | 25 min | Copy snapshot + restore + validate |
| Backend Deploy | 25 min | Terraform + ASG launch + health checks |
| DNS Failover | 5 min | Route53 update + propagation |
| Validation | 20 min | Smoke tests + monitoring |
| Secrets/TLS | 20 min | Parallel; may overlap with validation |
| **Total RTO** | **~90 minutes** (4–6 hours with manual steps) | See production considerations |

**Production Reality**: ~4–6 hours expected due to:
- Terraform state locks and backend re-initialization
- Manual review and approval gates
- Coordination across multiple teams
- Unexpected issues (e.g., database migration failures)

---

## 5. Service Dependencies and Failover Sequencing

### Dependency Tree

```
Route53 DNS (global)
  ↓ (no failover needed)
ALB (per-region) → EC2 ASG (per-region)
  ├→ RDS PostgreSQL (per-region, Multi-AZ)
  ├→ ElastiCache Redis (per-region)
  ├→ Stellar RPC (external; DNS failover)
  ├→ ACM Cert (per-region)
  └→ Secrets Manager (per-region)
```

### Failover Sequencing Rules

1. **Database must be available before backend boots**
   - Backend connection pool attempts to connect at startup
   - Failed DB connection = startup failure
   - Action: Restore DB first; verify connectivity before launching backend

2. **Redis is optional; cache miss is tolerable**
   - If Redis unavailable, backend queries database directly
   - Performance degradation but no data loss
   - Action: Restore Redis after backend is live; can be done asynchronously

3. **Stellar RPC is critical for blockchain queries**
   - Health checks in backend verify RPC connectivity
   - If RPC unreachable, blockchain operations fail
   - Action: Have alternate RPC endpoint DNS record ready for manual switch

4. **Secrets and TLS are prerequisites**
   - JWT secret required at backend startup
   - ACM cert required for ALB HTTPS listener
   - Action: Restore secrets before backend; ensure cert is in target region

5. **DNS is the final switch**
   - Route53 must point to new ALB after all services are healthy
   - Premature DNS switch = 503 errors from unhealthy targets
   - Action: Update DNS only after ASG reports healthy targets

### Traffic Rerouting (DNS)

**Current Setup:**
- Route53 hosted zone for `predifi.example.com`
- Alias record: `api.predifi.example.com` → ALB in `us-east-1`

**Failover Scenario:**
1. Primary region (us-east-1) is down
2. Alternate region (us-west-2) is provisioned with healthy ALB
3. Route53 alias record updated to point to us-west-2 ALB
4. DNS TTL: 60 seconds (default for ALB alias)
5. Clients resolve to new ALB within ~60 seconds; requests route to new backend

**Frontend Failover** (if applicable):
- Static assets (HTML, JS, CSS) served via CloudFront
- CloudFront is global; edge caches serve stale content if origin is down
- Backend API failures affect dynamic content (pool data, user state)
- Action: No special frontend failover needed; CloudFront continues serving cached content

---

## 6. Secret Availability and Rotation During Failover

### Secrets Inventory

| Secret | Storage | Rotation | Failover Plan |
|--------|---------|----------|---------------|
| **JWT Signing Key** | Secrets Manager (region) | 90-day rotation | Pre-replicate to alternate region; manual restore from encrypted backup |
| **Database Password** | RDS (auto-generated) + Secrets Manager | Manual or auto | Included in RDS restore; update connection string in backend config |
| **Redis Auth Token** | ElastiCache + Secrets Manager | Auto (per AWS) | Included in ElastiCache restore; update connection string |
| **Stellar Account Keypair** | Encrypted file / Secrets Manager | Manual | Store in Secrets Manager with cross-region replication enabled |
| **ACM Certificate** | ACM (per-region) | Auto (pre-renewal) | Request new cert in alternate region; allow 5–10 min for DNS validation |
| **OAuth / API Keys** | Secrets Manager | Per-provider | Replicate across regions or store with backup provider |

### Secrets Failover Actions

1. **Pre-failover (proactive)**:
   ```bash
   # Enable cross-region replication for critical secrets
   aws secretsmanager replicate-secret-to-regions \
     --secret-id predifi/prod/jwt_secret \
     --add-replica-regions Region=us-west-2,KmsKeyId=arn:aws:kms:us-west-2:...
   ```

2. **During regional failover**:
   - Secrets Manager secrets with cross-region replication: automatically available in alternate region
   - Manually restore from backup if replication not configured
   - Database password: update in backend environment variables (Secrets Manager parameter)

3. **Post-failover**:
   - Trigger key rotation for JWT secret (existing keys remain valid for grace period)
   - Update ACM certificate if needed
   - Audit access logs (CloudTrail) for any unauthorized secret access

---

## 7. Database Recovery Sequencing

### PostgreSQL Multi-AZ Failover (Single AZ Failure)

| Step | Action | Duration | Status |
|------|--------|----------|--------|
| 1 | Primary fails (detected by AWS) | Immediate | Automatic |
| 2 | AWS initiates failover | 30–120 sec | Automatic |
| 3 | Standby replica promoted to primary | 30–120 sec | Automatic |
| 4 | DNS (internal) updated to new primary | 10–30 sec | Automatic |
| 5 | Application reconnects | <5 sec | Automatic |
| **Total** | | **1–3 min** | **Automatic** |

**Data Loss**: None (synchronous replication)

### PostgreSQL Regional Failover (Region Failure)

| Step | Action | Duration | RTO Impact |
|------|--------|----------|------------|
| 1 | Detect region down | 2–5 min | Alert delay |
| 2 | Identify latest automated backup | 2–5 min | Manual |
| 3 | Copy snapshot to alternate region | 5–15 min | Network + DB size |
| 4 | Restore from snapshot | 5–10 min | DB size |
| 5 | Run migration checks | 2–5 min | Manual verification |
| 6 | Update backend connection string | 1–2 min | Configuration |
| 7 | Application starts and connects | 1–2 min | Startup |
| **Total** | | **15–45 min** | **15–45 min** |

**Data Loss**: Up to 1 hour (depending on last automated backup time; RPO = 1 hour)

### Point-in-Time Recovery (PITR) Steps

If a backup is corrupted or data loss is detected:

1. **Identify corruption**:
   ```bash
   SELECT * FROM pools WHERE created_at > NOW() - INTERVAL '30 minutes';
   -- Look for anomalies (e.g., invalid amounts, missing entries)
   ```

2. **Use PITR to recover to pre-corruption state**:
   ```bash
   aws rds restore-db-instance-to-point-in-time \
     --source-db-instance-identifier predifi-prod-db \
     --target-db-instance-identifier predifi-prod-db-pitr-2024-10-19-1400 \
     --restore-time 2024-10-19T14:00:00Z \
     --region us-east-1
   ```

3. **Verify restored data**:
   ```bash
   psql -h predifi-prod-db-pitr-2024-10-19-1400.xyz.us-east-1.rds.amazonaws.com \
     -U predifi_user -d predifi_db -c "SELECT COUNT(*) FROM pools;"
   ```

4. **Promote recovered instance as primary** (if main instance is corrupt):
   ```bash
   # Verify data integrity
   # ... tests ...
   
   # Update application connection string to PITR instance
   aws ssm put-parameter \
     --name /predifi/prod/database_url \
     --value "postgresql://user:pass@predifi-prod-db-pitr-2024-10-19-1400.xyz.us-east-1.rds.amazonaws.com/predifi_db"
   ```

**PITR Window**: 35 days (AWS default for automated backups)

---

## 8. Runbook: Staged Failover Exercise

### Exercise Objectives

1. Verify RTO/RPO targets are achievable
2. Identify bottlenecks and single points of failure
3. Train team on failover procedures
4. Document actual recovery times vs. planned targets
5. Update runbooks with lessons learned

### Pre-Exercise Setup

**Timeline**: 1 week before exercise

1. **Prepare alternate region**:
   - Reserve AWS resources (estimate costs)
   - Create Terraform templates for rapid deployment
   - Pre-stage secrets in Secrets Manager (cross-region)

2. **Prepare monitoring**:
   - Set up CloudWatch dashboards for both regions
   - Configure alerting for alternate region
   - Document health check URLs and thresholds

3. **Communicate schedule**:
   - Notify all stakeholders (engineering, product, support)
   - Document communication channels (Slack incident thread, bridge call)
   - Assign roles: Incident Commander, Database Lead, Infrastructure Lead, Communicator

4. **Backup current state**:
   - Snapshot primary region RDS and backup configuration
   - Export Terraform state as reference
   - Screenshot DNS and ALB config

### Exercise Execution

**Duration**: 2–3 hours (includes testing, communication, analysis)

**Scenario**: "Primary region (us-east-1) becomes unavailable at 2:00 PM UTC. Recover to alternate region (us-west-2) within planned RTO."

#### Stage 1: Detection (0–5 minutes)

**Actions**:
1. Simulate region failure by updating Route53 health check to report "unhealthy" for us-east-1
   ```bash
   aws route53 update-health-check \
     --health-check-id abc123 \
     --health-check-version 1 \
     --alarm-identifier Name=PrediFi-Simulated-Outage,Region=us-east-1
   ```

2. Trigger Grafana alert manually (simulate APM alert)

3. Incident Commander: Create incident thread and assign roles

4. **Record timestamp**: Outage detection time = 14:00:00 UTC

#### Stage 2: Database Recovery (5–30 minutes)

**Database Lead**:

1. List RDS snapshots:
   ```bash
   aws rds describe-db-snapshots --region us-east-1 \
     --query "DBSnapshots[?DBInstanceIdentifier=='predifi-prod-db']"
   ```

2. Copy latest snapshot to alternate region:
   ```bash
   aws rds copy-db-snapshot \
     --source-db-snapshot-identifier arn:aws:rds:us-east-1:ACCOUNT:snapshot:predifi-prod-db-2024-10-19-1359 \
     --target-db-snapshot-identifier predifi-prod-db-us-west-2-exercise \
     --region us-west-2
   ```

3. Monitor copy progress:
   ```bash
   watch -n 10 'aws rds describe-db-snapshots --region us-west-2 \
     --db-snapshot-identifier predifi-prod-db-us-west-2-exercise \
     --query "DBSnapshots[0].Status"'
   ```

4. Restore from snapshot:
   ```bash
   aws rds restore-db-instance-from-db-snapshot \
     --db-instance-identifier predifi-prod-db-us-west-2-exercise \
     --db-snapshot-identifier predifi-prod-db-us-west-2-exercise \
     --region us-west-2 \
     --db-instance-class db.t4g.medium \
     --multi-az
   ```

5. Wait for restore and test connectivity

6. **Record timestamp**: DB ready time

#### Stage 3: Backend Deployment (30–55 minutes)

**Infrastructure Lead**:

1. Deploy backend to us-west-2 (using pre-prepared Terraform):
   ```bash
   cd terraform/environments/production
   sed -i 's/us-east-1/us-west-2/g' terraform.tfvars
   terraform init -backend-config="key=production-us-west-2/terraform.tfstate"
   terraform plan -out=exercise.tfplan
   terraform apply exercise.tfplan
   ```

2. Configure backend environment variables to point to restored DB

3. Wait for ASG instances to launch and pass health checks

4. **Record timestamp**: Backend ready time

#### Stage 4: DNS Failover (55–60 minutes)

**Infrastructure Lead**:

1. Update Route53 to point to new ALB:
   ```bash
   aws route53 change-resource-record-sets \
     --hosted-zone-id Z1234567890ABC \
     --change-batch '{
       "Changes": [{
         "Action": "UPSERT",
         "ResourceRecordSet": {
           "Name": "api.predifi.example.com",
           "Type": "A",
           "AliasTarget": {
             "HostedZoneId": "<new-alb-zone>",
             "DNSName": "<new-alb-dns>",
             "EvaluateTargetHealth": true
           }
         }
       }]
     }'
   ```

2. Verify DNS propagation

3. **Record timestamp**: DNS updated time

#### Stage 5: Validation and Testing (60–90 minutes)

**All teams**:

1. **Health checks**:
   ```bash
   curl -I https://api.predifi.example.com/health
   curl -I https://api.predifi.example.com/api/v1/health
   ```

2. **Functional tests**:
   ```bash
   # Fetch pools
   curl -s https://api.predifi.example.com/api/v1/pools \
     -H "Authorization: Bearer <test-token>"
   
   # Verify data consistency (count rows)
   psql -h <new-db> -U user -d db -c "SELECT COUNT(*) FROM pools;"
   ```

3. **Load test** (optional):
   ```bash
   # Simulate production traffic
   ab -n 1000 -c 10 https://api.predifi.example.com/api/v1/pools
   ```

4. **Monitor application logs**:
   ```bash
   tail -f /var/log/predifi-backend.log  # From EC2 or CloudWatch
   ```

5. **Record observations**:
   - Any errors or timeouts?
   - Response times and latency?
   - Database connection pool health?

#### Stage 6: Rollback (final ~10 minutes)

1. Switch DNS back to primary region (simulating issue resolution)
2. Verify traffic returns to us-east-1
3. Terminate exercise resources in us-west-2

### Exercise Reporting

**Document findings**:

1. **Timeline**:
   | Phase | Planned RTO | Actual Time | Delta | Notes |
   |-------|------------|-------------|-------|-------|
   | DB Restore | 25 min | 28 min | +3 min | Snapshot copy slower than expected |
   | Backend Deploy | 25 min | 32 min | +7 min | Terraform state lock delays |
   | DNS Failover | 5 min | 3 min | -2 min | DNS propagation faster than expected |
   | Validation | 20 min | 18 min | -2 min | All health checks passed on first try |
   | **Total** | **90 min** | **103 min** | **+13 min** | See recommendations |

2. **Issues Found**:
   - [ ] Issue: Terraform S3 backend not configured; had to initialize new backend in alternate region
     - Impact: +7 minutes
     - Fix: Pre-initialize alternate region terraform backend
   
   - [ ] Issue: Secrets Manager secret (JWT key) not replicated; had to manually copy
     - Impact: +5 minutes (parallel, during validation)
     - Fix: Enable automatic cross-region replication for critical secrets
   
   - [ ] Issue: ALB health checks initially failed due to security group misconfiguration
     - Impact: +4 minutes (manual SG rule fix)
     - Fix: Document security group requirements; add Terraform validation

3. **Successes**:
   - Database snapshot copy and restore completed without issues
   - RDS data integrity verified; no data corruption
   - Application connected successfully to new database
   - All API endpoints returned 200 OK

4. **Recommendations**:
   - [ ] Enable Terraform backend S3 configuration before next exercise
   - [ ] Pre-replicate JWT key and other secrets to alternate region
   - [ ] Automate database snapshot listing and restore steps (create wrapper script)
   - [ ] Add integration tests to validate data after restore
   - [ ] Establish on-call rotation and communication protocol
   - [ ] Create CloudFormation/CDK templates for faster infrastructure deployment

### Lessons Learned Template

**Date**: October 19, 2024  
**Facilitator**: [Name]  
**Participants**: [List]

**What went well**:
- [ ] X
- [ ] Y

**What could be improved**:
- [ ] X
- [ ] Y

**Action items**:
- [ ] X (Owner, Due Date)
- [ ] Y (Owner, Due Date)

**Updated RTO/RPO targets**:
- Database: Planned 25 min → Actual 28 min (revised: 35 min for contingency)
- Backend: Planned 25 min → Actual 32 min (revised: 40 min for contingency)

---

## 9. Post-Failover Procedures

### Health Check and Validation

After failover to alternate region:

1. **Verify database**:
   ```bash
   psql -h <new-db> -U predifi_user -d predifi_db << EOF
   SELECT COUNT(*) as pools FROM pools;
   SELECT COUNT(*) as predictions FROM predictions;
   SELECT MAX(updated_at) as latest_pool_update FROM pools;
   SELECT MAX(updated_at) as latest_prediction_update FROM predictions;
   
   -- Spot check: Verify no negative amounts or invalid states
   SELECT COUNT(*) FROM predictions WHERE amount < 0;  -- Should be 0
   SELECT COUNT(*) FROM pools WHERE pool_status NOT IN ('open', 'closed', 'resolved');  -- Should be 0
   EOF
   ```

2. **Test API endpoints**:
   ```bash
   # Test pool retrieval
   curl -s https://api.predifi.example.com/api/v1/pools \
     -H "Authorization: Bearer <token>" | jq '.data | length'
   
   # Test user endpoints
   curl -s https://api.predifi.example.com/api/v1/user/stats \
     -H "Authorization: Bearer <token>" | jq '.user_id'
   ```

3. **Monitor for errors**:
   ```bash
   # Watch application logs for errors
   aws logs tail /aws/ec2/predifi-prod-us-west-2 --follow --filter-pattern "ERROR"
   
   # Check Grafana dashboards for anomalies
   # Navigate to: https://grafana.predifi.example.com/d/predifi-backend
   ```

4. **Test blockchain interactions**:
   ```bash
   # Verify Stellar RPC connectivity
   curl -s https://api.predifi.example.com/api/v1/health | jq '.blockchain_status'
   
   # Create a test prediction (if safe in production)
   curl -X POST https://api.predifi.example.com/api/v1/predictions \
     -H "Authorization: Bearer <token>" \
     -H "Content-Type: application/json" \
     -d '{"pool_id":"test-pool","outcome":"yes","amount":100}'
   ```

### Data Consistency Verification

1. **Compare row counts** (before vs. after failover):
   ```bash
   # Original database (if accessible)
   SELECT COUNT(*) FROM pools;  -- Expected: 5432
   
   # New database
   SELECT COUNT(*) FROM pools;  -- Should match
   ```

2. **Verify recent transactions**:
   ```bash
   SELECT * FROM pools WHERE created_at > NOW() - INTERVAL '2 hours' ORDER BY created_at DESC LIMIT 5;
   -- Verify the most recent pools are within RPO window (1 hour)
   ```

3. **Audit trail**:
   - Check CloudTrail logs for any unauthorized access
   - Review database access logs (if enabled)

### Transition to Long-Term Operations

1. **Monitor for issues**:
   - Set up continuous monitoring dashboard
   - Increase alert sensitivity temporarily
   - Have senior engineer on-call for 24 hours

2. **Plan return to primary region** (if outage is resolved):
   - Do not immediately failback; let primary region stabilize
   - Plan controlled migration back (could take hours/days)
   - Test primary region thoroughly before traffic switch

3. **Update documentation**:
   - Record lessons learned
   - Update runbooks with timeline data
   - Identify improvements for next exercise

---

## 10. Failure Modes and Mitigations

### Single Points of Failure

| Component | Failure Mode | Impact | Mitigation | RTO |
|-----------|--------------|--------|-----------|-----|
| **Route53** | Apex domain unavailable | Cannot resolve API DNS | N/A (AWS global, no failover) | N/A |
| **Stellar RPC** | Single endpoint unreachable | Blockchain queries fail | Implement RPC failover chain with health checks | 10 min |
| **Primary DB (Multi-AZ)** | Primary and standby both fail | Data unavailable | Cross-region snapshot backup; RTO ~30 min | 30 min |
| **ACM Certificate** | Cert expires or is revoked | HTTPS fails | Auto-renewal enabled; manual renewal before expiry | 5 min |
| **Application Secrets** | JWT key or DB password leak | Unauthorized access | Rotate keys immediately; audit logs | Immediate |
| **Terraform State** | Local state deleted | Cannot manage infrastructure | Enable S3 backend with versioning | Varies |

### Mitigations

1. **Stellar RPC Failover**:
   ```rust
   // In backend config
   PREDIFI_STELLAR_RPC_URLS = [
     "https://rpc.stellar.org",      // Primary
     "https://rpc-fallover.stellar.org", // Fallback 1
     "https://private-rpc.example.com"   // Fallback 2
   ]
   
   // Try each URL until one succeeds
   for rpc_url in PREDIFI_STELLAR_RPC_URLS:
     match health_check(rpc_url):
       if OK: use this RPC
       if FAIL: try next
   ```

2. **Database Backup Strategy**:
   - Enable automated daily snapshots → 7-day retention (current)
   - Consider: Point-in-time recovery (35-day window)
   - Consider: Weekly cross-region snapshot copies (cold standby)

3. **Secrets Replication**:
   - Enable cross-region replication for all Secrets Manager entries
   - Automate rotation every 90 days
   - Audit access logs monthly

4. **Terraform State**:
   - Migrate to S3 backend with versioning
   - Enable MFA delete for protection
   - Backup state files daily to separate storage

---

## 11. Testing and Validation Plan

### Quarterly Exercises

1. **Q1 (Jan–Mar)**: Database failover (RDS Multi-AZ within region)
2. **Q2 (Apr–Jun)**: Regional failover (full scenario)
3. **Q3 (Jul–Sep)**: Secrets recovery (key rotation, access audit)
4. **Q4 (Oct–Dec)**: End-to-end failover (all components, production dry-run)

### Continuous Validation

- **Monthly**: Health check validation (automated)
- **Weekly**: Backup snapshot verification (automated)
- **Daily**: Monitoring dashboard review (manual, morning standup)

### Success Criteria for Each Exercise

- [ ] Database restored and validated within planned RTO
- [ ] Backend deployed and health checks passing within planned RTO
- [ ] DNS failover completed with zero data loss
- [ ] API endpoints respond correctly from alternate region
- [ ] All data verified for consistency
- [ ] Actual RTO within ±10% of planned target (or documented extension)
- [ ] No unplanned incidents or security issues during exercise

---

## 12. Appendices

### A. Critical Commands Cheat Sheet

```bash
# List RDS snapshots
aws rds describe-db-snapshots --region us-east-1 \
  --query "DBSnapshots[?DBInstanceIdentifier=='predifi-prod-db']" \
  --output table

# Copy snapshot to alternate region
aws rds copy-db-snapshot \
  --source-db-snapshot-identifier arn:aws:rds:us-east-1:ACCOUNT:snapshot:NAME \
  --target-db-snapshot-identifier NAME-copy \
  --region us-west-2

# Restore from snapshot
aws rds restore-db-instance-from-db-snapshot \
  --db-instance-identifier predifi-prod-db-us-west-2 \
  --db-snapshot-identifier NAME-copy \
  --region us-west-2 \
  --db-instance-class db.t4g.medium \
  --multi-az

# Get RDS endpoint
aws rds describe-db-instances --region us-west-2 \
  --query "DBInstances[0].Endpoint.Address"

# Deploy via Terraform
cd terraform/environments/production
sed -i 's/us-east-1/us-west-2/g' terraform.tfvars
terraform init -backend-config="key=production-us-west-2/terraform.tfstate"
terraform plan -out=failover.tfplan
terraform apply failover.tfplan

# Update Route53
aws route53 change-resource-record-sets \
  --hosted-zone-id Z_ZONE_ID \
  --change-batch file://dns-change.json

# Verify DNS
nslookup api.predifi.example.com
dig api.predifi.example.com +short

# Test API health
curl -I https://api.predifi.example.com/health
curl -I https://api.predifi.example.com/api/v1/health
```

### B. Contact List

| Role | Name | Phone | Email | Backup |
|------|------|-------|-------|--------|
| Incident Commander | [TBD] | [TBD] | [TBD] | [TBD] |
| Infrastructure Lead | [TBD] | [TBD] | [TBD] | [TBD] |
| Database Lead | [TBD] | [TBD] | [TBD] | [TBD] |
| Backend Lead | [TBD] | [TBD] | [TBD] | [TBD] |
| Communications | [TBD] | [TBD] | [TBD] | [TBD] |

### C. Escalation Procedure

**If RTO is exceeded by >15 minutes:**
1. Incident Commander escalates to VP Engineering
2. Page all Infrastructure team members
3. Establish war room (Zoom + Slack)
4. Review decision: Continue recovery vs. declare extended outage

**If data loss exceeds RPO:**
1. Incident Commander escalates to CTO
2. Legal/Compliance notified (if applicable)
3. Customer communication initiated
4. Postmortem scheduled

### D. References

- [AWS Disaster Recovery Documentation](https://docs.aws.amazon.com/disaster-recovery/)
- [RDS Backup & Restore](https://docs.aws.amazon.com/AmazonRDS/latest/UserGuide/BackupRestoreGuide.html)
- [Route53 Failover Routing](https://docs.aws.amazon.com/Route53/latest/DeveloperGuide/routing-policy-failover.html)
- [ElastiCache Failover](https://docs.aws.amazon.com/AmazonElastiCache/latest/red-ug/AutomaticFailover.html)
- [Terraform Best Practices](https://www.terraform.io/docs/cloud/guides/recommended-practices.html)

---

## 13. Document Control

| Version | Date | Author | Changes | Status |
|---------|------|--------|---------|--------|
| 1.0 | Oct 19, 2024 | Infrastructure Team | Initial draft | Draft for Review |
| 1.1 | *[Pending]* | *[TBD]* | Tabletop exercise results | Pending |
| 2.0 | *[Pending]* | *[TBD]* | Final, post-exercise | Pending Approval |

**Next Review Date**: January 2025 (post-Q1 exercise)
