# PrediFi Disaster Recovery Quick Reference

**For use during active regional outages. Full details in `DISASTER_RECOVERY_PLAN.md`**

---

## 1. Initial Detection (0–5 min)

### Confirm Outage
```bash
# Check AWS status
https://status.aws.amazon.com/

# DNS test (Route53 is global)
nslookup api.predifi.example.com

# Direct API health check
curl -I https://api.predifi.example.com/health
# Expected if down: timeout or connection refused
```

### Alert Team
- [ ] Page Incident Commander
- [ ] Create incident channel (Slack/PagerDuty)
- [ ] Record timestamp: **`[OUTAGE TIME]`**
- [ ] Assign roles:
  - **Incident Commander**: Overall coordination
  - **Database Lead**: RDS restore
  - **Infrastructure Lead**: Backend deploy + DNS
  - **Communicator**: Status updates

---

## 2. Database Recovery (5–30 min)

**Owner: Database Lead**

### Step 1: List Backups
```bash
aws rds describe-db-snapshots \
  --region us-east-1 \
  --query "DBSnapshots[?DBInstanceIdentifier=='predifi-prod-db']" \
  --output table
```
✓ Choose most recent snapshot (within 1-hour RPO)

### Step 2: Copy to Alternate Region
```bash
SNAPSHOT_ID="predifi-prod-db-2024-10-19-1359"  # CHANGE ME
REGION_TO="us-west-2"  # CHANGE ME to target region
COPY_ID="predifi-prod-db-copy-$(date +%s)"

aws rds copy-db-snapshot \
  --source-db-snapshot-identifier arn:aws:rds:us-east-1:ACCOUNT:snapshot:${SNAPSHOT_ID} \
  --target-db-snapshot-identifier ${COPY_ID} \
  --region ${REGION_TO}

# Monitor (poll every 30 sec)
aws rds describe-db-snapshots \
  --region ${REGION_TO} \
  --db-snapshot-identifier ${COPY_ID} \
  --query "DBSnapshots[0].Status"
# Wait until: available
```

### Step 3: Restore from Snapshot
```bash
DB_ID="predifi-prod-db-${REGION_TO}-dr"

aws rds restore-db-instance-from-db-snapshot \
  --db-instance-identifier ${DB_ID} \
  --db-snapshot-identifier ${COPY_ID} \
  --region ${REGION_TO} \
  --db-instance-class db.t4g.medium \
  --multi-az \
  --no-publicly-accessible

# Wait for restore (5–10 min)
aws rds describe-db-instances \
  --region ${REGION_TO} \
  --db-instance-identifier ${DB_ID} \
  --query "DBInstances[0].DBInstanceStatus"
# Wait until: available
```

### Step 4: Get Endpoint
```bash
NEW_DB_ENDPOINT=$(aws rds describe-db-instances \
  --region ${REGION_TO} \
  --db-instance-identifier ${DB_ID} \
  --query "DBInstances[0].Endpoint.Address" \
  --output text)

echo "New DB endpoint: ${NEW_DB_ENDPOINT}"
# Save this for next steps
```

### Step 5: Test Connection
```bash
psql -h ${NEW_DB_ENDPOINT} \
  -U predifi_user -d predifi_db \
  -c "SELECT COUNT(*) FROM pools; SELECT MAX(updated_at) FROM pools;"
```
✓ Should see pool count and recent timestamp

### Step 6: Data Validation
```bash
psql -h ${NEW_DB_ENDPOINT} -U predifi_user -d predifi_db << EOF
SELECT COUNT(*) as pools FROM pools;
SELECT COUNT(*) as predictions FROM predictions;
SELECT COUNT(*) FROM predictions WHERE amount < 0;  -- Should be 0
EOF
```

✓ **Record timestamp: DB Ready at `[TIME]`**

---

## 3. Backend Deployment (30–55 min)

**Owner: Infrastructure Lead**

### Step 1: Update Terraform
```bash
cd terraform/environments/production

# Backup original tfvars
cp terraform.tfvars terraform.tfvars.backup

# Update region in tfvars
sed -i 's/us-east-1/us-west-2/g' terraform.tfvars

# Verify change
grep "aws_region\|region" terraform.tfvars | head -5
```

### Step 2: Initialize Backend
```bash
terraform init \
  -backend-config="bucket=predifi-terraform-state-dr" \
  -backend-config="key=production-us-west-2-dr/terraform.tfstate" \
  -backend-config="region=us-west-2"

# Confirm: Type 'yes' when prompted
```

### Step 3: Plan and Approve
```bash
terraform plan -out=failover.tfplan

# Review plan:
# - ALB, ASG, security groups should be created
# - Database should NOT be created (already done)
# - Redis should be created

# If looks good, proceed
```

### Step 4: Apply Infra
```bash
terraform apply failover.tfplan

# Wait ~10 minutes for resources
```

### Step 5: Set Environment Variables
```bash
REGION="us-west-2"

# Database connection
aws ssm put-parameter \
  --region ${REGION} \
  --name /predifi/prod/database_url \
  --value "postgresql://predifi_user:DBPASS@${NEW_DB_ENDPOINT}:5432/predifi_db" \
  --overwrite

# Redis (if available)
REDIS_ENDPOINT=$(aws elasticache describe-cache-clusters \
  --region ${REGION} \
  --cache-cluster-id predifi-prod-redis \
  --show-cache-node-info \
  --query "CacheClusters[0].CacheNodes[0].Endpoint.Address" \
  --output text 2>/dev/null || echo "pending")

if [ "${REDIS_ENDPOINT}" != "pending" ]; then
  aws ssm put-parameter \
    --region ${REGION} \
    --name /predifi/prod/redis_url \
    --value "rediss://${REDIS_ENDPOINT}:6379" \
    --overwrite
fi
```

### Step 6: Wait for Backend to Boot
```bash
ALB_DNS=$(aws elbv2 describe-load-balancers \
  --region us-west-2 \
  --query "LoadBalancers[0].DNSName" \
  --output text)

echo "ALB DNS: ${ALB_DNS}"

# Wait 2–3 min for instances to launch
sleep 120

# Check health
curl -I http://${ALB_DNS}/health
# Expected: 200 OK (or 503 if still booting)

# Poll every 10 sec until 200
for i in {1..30}; do
  HTTP_CODE=$(curl -s -o /dev/null -w "%{http_code}" http://${ALB_DNS}/health)
  echo "Attempt $i: HTTP $HTTP_CODE"
  [ "${HTTP_CODE}" = "200" ] && break
  sleep 10
done
```

✓ **Record timestamp: Backend Ready at `[TIME]`**

---

## 4. DNS Failover (50–55 min)

**Owner: Infrastructure Lead**

### Step 1: Get New ALB Details
```bash
NEW_ALB_DNS=$(aws elbv2 describe-load-balancers \
  --region us-west-2 \
  --query "LoadBalancers[0].DNSName" \
  --output text)

NEW_ALB_ZONE_ID=$(aws elbv2 describe-load-balancers \
  --region us-west-2 \
  --query "LoadBalancers[0].CanonicalHostedZoneId" \
  --output text)

echo "ALB DNS: ${NEW_ALB_DNS}"
echo "ALB Zone ID: ${NEW_ALB_ZONE_ID}"
```

### Step 2: Create Route53 Change
```bash
cat > /tmp/dns-change.json << EOF
{
  "Changes": [{
    "Action": "UPSERT",
    "ResourceRecordSet": {
      "Name": "api.predifi.example.com",
      "Type": "A",
      "AliasTarget": {
        "HostedZoneId": "${NEW_ALB_ZONE_ID}",
        "DNSName": "${NEW_ALB_DNS}",
        "EvaluateTargetHealth": true
      }
    }
  }]
}
EOF

cat /tmp/dns-change.json
```

### Step 3: Apply DNS Change
```bash
aws route53 change-resource-record-sets \
  --hosted-zone-id Z1234567890ABC \
  --change-batch file:///tmp/dns-change.json
```
✓ DNS updated (takes 30–60 sec to propagate)

### Step 4: Verify Propagation
```bash
# Wait 30 sec
sleep 30

# Verify new DNS
nslookup api.predifi.example.com
dig api.predifi.example.com +short

# Should point to new ALB in us-west-2
```

✓ **Record timestamp: DNS Switched at `[TIME]`**

---

## 5. Validation (55–75 min)

**Owner: All**

### Health Checks
```bash
# Basic health
curl -I https://api.predifi.example.com/health
curl -I https://api.predifi.example.com/api/v1/health

# Should both return 200 OK
```

### Functional Tests
```bash
# Fetch pools (requires valid JWT)
curl -s https://api.predifi.example.com/api/v1/pools \
  -H "Authorization: Bearer $JWT_TOKEN" | jq '.data | length'

# Create test prediction (if safe)
curl -X POST https://api.predifi.example.com/api/v1/predictions \
  -H "Authorization: Bearer $JWT_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"pool_id":"test-pool","outcome":"yes","amount":100}'
```

### Data Consistency
```bash
# Connect to new DB
psql -h ${NEW_DB_ENDPOINT} -U predifi_user -d predifi_db << EOF
SELECT COUNT(*) as total_pools FROM pools;
SELECT COUNT(*) as total_predictions FROM predictions;
SELECT MAX(updated_at) as latest_pool FROM pools;

-- Verify no anomalies
SELECT COUNT(*) FROM predictions WHERE amount < 0;
SELECT COUNT(*) FROM pools WHERE pool_status NOT IN ('open','closed','resolved');
EOF

# Compare row counts to pre-outage baseline
```

### Blockchain RPC
```bash
curl -s https://api.predifi.example.com/api/v1/health | jq '.blockchain_status'
# Should show "healthy"
```

### Check Application Logs
```bash
aws logs tail /aws/ec2/predifi-prod-us-west-2 --follow --since 10m
# Watch for any ERROR or WARN messages
```

✓ **All checks passing? Continue. Otherwise, debug and fix.**

---

## 6. Secrets & TLS (parallel with validation)

**Owner: Infrastructure Lead (or Backend Lead)**

### JWT Secret
```bash
# If not pre-replicated, restore from backup/Secrets Manager
aws secretsmanager create-secret \
  --region us-west-2 \
  --name predifi/prod/jwt_secret \
  --secret-string file:///path/to/jwt-secret-backup

# Or manually copy if backup accessible
```

### ACM Certificate
```bash
# Check if certificate exists in us-west-2
aws acm list-certificates --region us-west-2

# If not, request new certificate
aws acm request-certificate \
  --domain-name "*.predifi.example.com" \
  --region us-west-2 \
  --domain-validation-options DomainName=predifi.example.com,ValidationDomain=predifi.example.com

# DNS validation should auto-complete via Route53 (~5–10 min)
```

### Update ALB Listener (if new cert)
```bash
ALB_ARN=$(aws elbv2 describe-load-balancers \
  --region us-west-2 \
  --query "LoadBalancers[0].LoadBalancerArn" \
  --output text)

NEW_CERT_ARN=$(aws acm list-certificates \
  --region us-west-2 \
  --query "CertificateSummaryList[0].CertificateArn" \
  --output text)

aws elbv2 modify-listener \
  --listener-arn "arn:aws:elasticloadbalancing:us-west-2:ACCOUNT:listener/app/predifi/..." \
  --certificates CertificateArn=${NEW_CERT_ARN}
```

---

## 7. Post-Recovery Checklist

- [ ] All health checks passing
- [ ] API endpoints responding
- [ ] Database verified for consistency
- [ ] No error logs
- [ ] Blockchain RPC healthy
- [ ] JWT secret restored
- [ ] TLS certificate valid
- [ ] Monitoring dashboard up
- [ ] Team notified of recovery
- [ ] RTO/RPO targets met? Document findings
- [ ] Secrets/logs secured? Check CloudTrail

---

## 8. Timeline Summary

| Phase | Est. Duration | Cumulative |
|-------|---|---|
| Detection | 5 min | 5 min |
| DB Restore | 25 min | 30 min |
| Backend Deploy | 25 min | 55 min |
| DNS Failover | 5 min | 60 min |
| Validation | 20 min | 80 min |
| **Total RTO** | | **~90 min (ideal)** |
| | | **~4–6 hours (realistic)** |

---

## 9. Emergency Contacts

| Role | Name | Contact | Status |
|------|------|---------|--------|
| Incident Commander | | | |
| Infrastructure Lead | | | |
| Database Lead | | | |
| Backend Lead | | | |
| Communications | | | |

---

## 10. Important Commands Reference

```bash
# DATABASES
aws rds describe-db-snapshots --region us-east-1 --query "DBSnapshots[?DBInstanceIdentifier=='predifi-prod-db']"
aws rds copy-db-snapshot --source-db-snapshot-identifier ARN --target-db-snapshot-identifier NAME --region us-west-2
aws rds restore-db-instance-from-db-snapshot --db-instance-identifier NAME --db-snapshot-identifier COPY_ID --region us-west-2 --db-instance-class db.t4g.medium --multi-az
psql -h ENDPOINT -U predifi_user -d predifi_db -c "SELECT COUNT(*) FROM pools;"

# TERRAFORM
cd terraform/environments/production
terraform init -backend-config="bucket=STATE_BUCKET" -backend-config="key=PROD_KEY" -backend-config="region=REGION"
terraform plan -out=failover.tfplan
terraform apply failover.tfplan

# ALB & DNS
aws elbv2 describe-load-balancers --region us-west-2 --query "LoadBalancers[0].DNSName"
aws route53 change-resource-record-sets --hosted-zone-id ZONE_ID --change-batch file://dns-change.json

# HEALTH CHECKS
curl -I https://api.predifi.example.com/health
curl -s https://api.predifi.example.com/api/v1/pools -H "Authorization: Bearer TOKEN"
```

---

## 11. When to Escalate

**If RTO exceeded by >15 min:**
- Escalate to VP Engineering
- Page all Infrastructure team
- Establish war room (Zoom/Slack)

**If data loss exceeds RPO:**
- Escalate to CTO
- Notify Legal/Compliance
- Initiate customer communication

**If database restore fails:**
- Use point-in-time recovery (PITR)
- Restore to time T-30min
- Verify data integrity before promoting

---

## 12. Post-Incident

1. Document actual times vs. planned RTO/RPO
2. Identify what slowed recovery (bottlenecks)
3. Update runbook with findings
4. Schedule postmortem (within 48 hours)
5. Create follow-up action items

---

**Last Updated: October 19, 2024**  
**Next Review: January 2025**
