# PrediFi Disaster Recovery Framework Overview

This directory contains comprehensive disaster recovery (DR) documentation for the PrediFi platform. This overview explains the structure and how to use the DR documentation.

---

## 📋 Documentation Structure

### 1. **DISASTER_RECOVERY_PLAN.md** (Primary Document)
**Purpose**: Comprehensive disaster recovery strategy document  
**Audience**: Engineering leads, infrastructure team, stakeholders  
**Content**:
- Service inventory and criticality classification
- RTO/RPO targets for all services
- Single AZ failure procedures (automatic)
- Regional outage detailed runbook (manual)
- Service dependencies and failover sequencing
- Secret management and TLS considerations
- Database recovery procedures (Multi-AZ, regional, PITR)
- Staged failover exercise procedures
- Post-failover validation steps
- Failure modes and mitigations
- Testing and validation plan
- Contact list and escalation procedures

**When to Use**:
- Planning and strategy discussions
- Understanding the complete DR architecture
- Training new team members
- Pre-incident preparation
- Post-incident review

**Reading Order**:
1. Executive Summary
2. Service Inventory (Section 1)
3. RTO/RPO Targets (Section 2)
4. Your Scenario (Single AZ or Regional, Sections 3–4)
5. Detailed Runbook (Section 4)

---

### 2. **DISASTER_RECOVERY_RUNBOOK_QUICK_REFERENCE.md** (Operational Playbook)
**Purpose**: Quick reference guide for use during active outages  
**Audience**: On-call engineers, incident commanders  
**Content**:
- Step-by-step commands for each recovery phase
- Copy-paste ready commands with inline comments
- Timeline expectations
- Go/no-go decision points
- Emergency contacts
- Escalation triggers

**When to Use**:
- **DURING AN ACTUAL OUTAGE** (primary reference)
- Quick lookup of specific commands
- Training scenarios
- Runbook dry-runs

**Key Features**:
- **Concise**: One page per phase
- **Executable**: Every command is tested and ready to run
- **Self-contained**: Minimal cross-referencing required
- **Color-coded**: ✓ for completion, ✗ for failures
- **Timing**: Expected duration for each phase

**How to Use During Outage**:
1. Confirm outage (Section 1)
2. Follow phases in order: Detection → DB → Backend → DNS → Validation
3. Copy commands directly into terminal (substitute placeholders)
4. Check off tasks as completed
5. Record actual times vs. planned targets
6. Escalate if timing exceeded

---

### 3. **DISASTER_RECOVERY_EXERCISE_TEMPLATE.md** (Evidence Document)
**Purpose**: Template for documenting tabletop and staged failover exercises  
**Audience**: Exercise participants, observers, leadership  
**Content**:
- Pre-exercise checklist
- Phase-by-phase observation sections
- Issue logging template
- Success criteria tracking
- Timeline comparison (planned vs. actual)
- Findings and action items
- RTO/RPO updates based on results
- Sign-off section

**When to Use**:
- Before conducting a tabletop or staged failover exercise
- During exercise to record observations in real-time
- After exercise to document findings and evidence
- For postmortem reviews

**How to Use**:
1. Copy template to new file (e.g., `DR_EXERCISE_2024_Q1.md`)
2. Fill in participant info and pre-exercise checklist
3. During exercise: Record timestamps and observations for each phase
4. After exercise: Complete findings and action items
5. Archive for audit trail and future reference

---

## 🎯 Disaster Recovery Objectives

### RTO (Recovery Time Objective) — Maximum acceptable downtime

| Scenario | Service | RTO |
|----------|---------|-----|
| **Single AZ Failure** | All | 2–3 minutes (automatic) |
| **Regional Outage** | Backend API | 4–6 hours (realistic) / 90 min (optimistic) |
| **Regional Outage** | Database | 4–8 hours (with data validation) |
| **Regional Outage** | Stellar RPC | 10 minutes (DNS failover) |
| **Regional Outage** | Frontend | 15 minutes (CloudFront serving cache) |

### RPO (Recovery Point Objective) — Maximum acceptable data loss

| Service | RPO | Capability |
|---------|-----|-----------|
| **PostgreSQL** | 1 hour | 7-day automated backups + 35-day PITR |
| **Redis Cache** | 0 (ephemeral) | Non-persistent; cache miss acceptable |
| **Blockchain State** | 0 (immutable) | Replicated on Stellar network |
| **Configuration/Secrets** | 30 minutes | Secrets Manager with cross-region replication (goal) |

---

## 🚀 Quick Start: Using the Documentation

### Scenario 1: Pre-Incident Preparation
**Goal**: Understand DR capabilities and train team

1. **Week 1**: Read DISASTER_RECOVERY_PLAN.md (Executive Summary + Sections 1–2)
2. **Week 2**: Team reviews runbook (DISASTER_RECOVERY_RUNBOOK_QUICK_REFERENCE.md)
3. **Week 3**: Conduct tabletop exercise (use DISASTER_RECOVERY_EXERCISE_TEMPLATE.md)
4. **Week 4**: Review findings and update documentation

### Scenario 2: Active Outage Response
**Goal**: Recover to alternate region as quickly as possible

1. **Minute 0–5**: Use Quick Reference (Section 1) to confirm outage
2. **Minute 5–30**: Database Lead follows Section 2 of Quick Reference
3. **Minute 30–55**: Infrastructure Lead follows Section 3 of Quick Reference
4. **Minute 55–60**: DNS failover (Section 4)
5. **Minute 60–90**: Validation (Section 5)
6. **Post-Recovery**: Document timeline, issues, and updates

### Scenario 3: Exercise Planning
**Goal**: Conduct tabletop or staged failover exercise

1. Download DISASTER_RECOVERY_EXERCISE_TEMPLATE.md
2. Create exercise-specific copy (e.g., `DR_EXERCISE_2024_Q2_REGIONAL.md`)
3. **Week before**: Complete Pre-Exercise Checklist
4. **Exercise day**: Assign participants, follow phases, record observations
5. **Post-exercise**: Complete findings, action items, sign-off
6. Archive for audit and reference

---

## 📍 Service Dependencies

```
User Browser
    ↓ (HTTPS)
CloudFront / Route53 DNS
    ↓
AWS ALB (public subnet) ← Certificate (ACM)
    ↓
ASG EC2 (private subnet) ← Security Groups
    ├→ PostgreSQL RDS (Multi-AZ, backups)
    ├→ ElastiCache Redis (replication, TLS)
    ├→ Stellar RPC (external, failover via DNS)
    ├→ AWS Secrets Manager (JWT key, DB password)
    └→ Monitoring (Prometheus, Grafana - optional)
```

**Failover Sequencing**:
1. Database must be ready before backend boots
2. Redis is optional (cache miss = DB query)
3. Secrets and TLS must be available before ALB/backend starts
4. Stellar RPC failover via DNS (manual switch)
5. DNS is the final switch (Route53 to new ALB)

---

## ⏱️ Timeline Expectations

### Single AZ Failure (Automatic, ~2–3 min)
- ALB detects failed instances
- ASG launches replacements
- RDS Multi-AZ automatic failover
- No manual intervention needed

### Regional Outage (Manual, ~90 min ideal / 4–6 hours realistic)

| Phase | Duration | Type |
|-------|----------|------|
| Detection & Alerting | 5 min | Manual |
| Database Restore | 25 min | Manual (copy + restore) |
| Backend Deployment | 25 min | Manual (Terraform) |
| DNS Failover | 5 min | Manual (Route53) |
| Validation & Testing | 20 min | Manual (smoke tests) |
| **Total** | **80 min** | **Manual execution** |

**Reality Check**: 
- Realistic RTO: 4–6 hours
- Includes: decision delays, approval gates, unexpected issues
- Can be improved by: pre-staging infrastructure, automation, team readiness

---

## 🧪 Exercise Schedule

| Quarter | Exercise Type | Focus | Evidence |
|---------|---|---|---|
| **Q1** | Tabletop | RDS Multi-AZ failover | Template + Recording |
| **Q2** | Staged Failover | Full regional DR | Template + Runbook Evidence |
| **Q3** | Tabletop | Secrets & TLS recovery | Template + Findings |
| **Q4** | Full DR Test | End-to-end production dry-run | Template + Full Timeline |

---

## ✅ Success Criteria

### Exercise Success
- [ ] Database restored within planned RTO (or documented deviation)
- [ ] Backend deployed and health checks passing
- [ ] DNS failover completed with no data loss
- [ ] All API endpoints functional
- [ ] Data consistency verified
- [ ] Actual times documented vs. planned targets
- [ ] Issues identified and action items created

### Runbook Success (During Actual Incident)
- [ ] RTO met (or documented and justified)
- [ ] RPO met (data loss within 1 hour)
- [ ] Zero critical errors post-recovery
- [ ] All systems validated
- [ ] Team safely returned to primary (if applicable)

---

## 📞 Incident Response

### Contacts
See DISASTER_RECOVERY_PLAN.md Appendix B for:
- Incident Commander
- Infrastructure Lead
- Database Lead
- Backend Lead
- Communications Lead

### Escalation Triggers
1. **RTO exceeded by >15 min**: Escalate to VP Engineering
2. **Data loss exceeds RPO**: Escalate to CTO, notify Legal/Compliance
3. **Database restore fails**: Attempt PITR or escalate to database vendor

### Communication During Incident
- Create Slack incident channel: #predifi-outage-[DATE]
- Send initial status update within 10 minutes
- Update every 15 minutes during recovery
- Post recovery summary within 1 hour

---

## 🔄 Document Maintenance

### Review Schedule
- **Monthly**: Verify contacts and escalation procedures
- **Quarterly**: Review against latest infrastructure changes
- **After Each Exercise**: Update with findings and lessons learned
- **Annually**: Full audit and major revisions

### Updates Required When
- Infrastructure changes (new region, new services)
- Team changes (new contact persons)
- RTO/RPO targets change
- Exercise reveals gaps or bottlenecks
- New AWS features become available

### Versioning
- Tracked in document header (Version X.Y)
- Git history for detailed change tracking
- Archive old versions: `DISASTER_RECOVERY_PLAN_v1.0.md`

---

## 🎓 Training

### For New Team Members
1. Read Executive Summary of DISASTER_RECOVERY_PLAN.md
2. Review your specific role in DISASTER_RECOVERY_RUNBOOK_QUICK_REFERENCE.md
3. Attend live exercise or observe previous exercise recording
4. Get hands-on walkthrough of Terraform + AWS CLI commands
5. Sign off on understanding

### For On-Call Engineers
- **Before shift**: Review DISASTER_RECOVERY_RUNBOOK_QUICK_REFERENCE.md
- **During shift**: Have Quick Reference open and accessible
- **If incident**: Follow runbook step-by-step
- **After incident**: Contribute to findings and postmortem

### For Leadership
- Understand RTO/RPO targets (DISASTER_RECOVERY_PLAN.md Section 2)
- Know escalation procedures (Section 12, Appendix C)
- Review exercise results quarterly
- Approve action items and budget for improvements

---

## 📊 Metrics to Track

### Recovery Performance
- **Actual RTO vs. Planned RTO**: Target ±10%
- **Data Loss (vs. RPO)**: Should be zero or within 1 hour
- **MTTR (Mean Time to Recovery)**: Target trending downward
- **Exercise Pass Rate**: Target 100%

### Improvement Areas
- Phase durations (which is slowest?)
- Issues encountered (frequency, severity)
- Automation opportunities (manual steps to automate)
- Skill gaps (training needed)

---

## 🛠️ Continuous Improvement

### From Exercise Results
1. Document actual timeline vs. planned
2. Identify slowest phases
3. Log issues and root causes
4. Create action items with owners and due dates
5. Update targets and procedures

### From Operational Feedback
- Collect feedback from on-call engineers
- Monitor for process gaps or ambiguities
- Incorporate AWS best practices updates
- Test new tools or automation approaches

### Automation Opportunities
- Snapshot copy/restore (bash script)
- Terraform state initialization (Python script)
- Health check monitoring (CloudWatch alarm)
- Route53 DNS failover (Lambda trigger)
- Notification system (SNS/Lambda)

---

## 📚 Related Documentation

- **Backend README**: `backend/README.md` — Architecture, deployment, local dev
- **Infrastructure README**: `terraform/README.md` — AWS modules, sizing, configuration
- **Security Hardening**: `backend/JWT_SECURITY_HARDENING.md` — Key rotation, token revocation
- **GitHub Workflows**: `.github/workflows/` — CI/CD, deployment automation
- **Monitoring**: `backend/grafana/ALERTS.md` — Alert rules, dashboards

---

## ❓ FAQ

### Q: What's the difference between RTO and RPO?
**A**: 
- **RTO** = How fast can we get the system back up?
- **RPO** = How much data can we afford to lose?

Example: RTO 4 hours, RPO 1 hour = System down for up to 4 hours, but data lost up to 1 hour old.

### Q: Why is realistic RTO 4–6 hours but planned is 90 min?
**A**: Planned RTO assumes:
- No unexpected issues
- Immediate team availability
- Pre-staged infrastructure
- No approvals needed

Realistic RTO includes:
- Decision delays (confirm it's a real outage)
- Approval gates (notify leadership, customers)
- Unexpected issues (terraform locks, security groups, etc.)
- Team coordination overhead

### Q: Can we auto-failover to another region?
**A**: Not currently. Regional failover requires manual DNS switch via Route53. This is intentional to prevent cascading failures. Single-AZ failover within a region is automatic (RDS Multi-AZ, ASG, ElastiCache).

### Q: What if the database backup is corrupted?
**A**: Use Point-in-Time Recovery (PITR) to restore to an earlier state (35-day window). See DISASTER_RECOVERY_PLAN.md Section 7 for steps.

### Q: How often should we exercise?
**A**: Quarterly (4x per year). See Exercise Schedule above. Each exercise focuses on different components.

### Q: Who decides to invoke disaster recovery?
**A**: Incident Commander (or VP Engineering if Commander unavailable). Trigger: Entire AWS region confirmed unavailable OR >15 min service outage with RTO risk.

---

## 📄 Document Checklist

- [x] DISASTER_RECOVERY_PLAN.md — Comprehensive strategy (13 sections, appendices)
- [x] DISASTER_RECOVERY_RUNBOOK_QUICK_REFERENCE.md — Operational playbook (6 phases)
- [x] DISASTER_RECOVERY_EXERCISE_TEMPLATE.md — Evidence tracking (12 sections)
- [x] DISASTER_RECOVERY_OVERVIEW.md — This guide

---

## 🚨 Quick Links

**During Outage**: Open `DISASTER_RECOVERY_RUNBOOK_QUICK_REFERENCE.md`

**Planning Exercise**: Use `DISASTER_RECOVERY_EXERCISE_TEMPLATE.md`

**Understanding Strategy**: Read `DISASTER_RECOVERY_PLAN.md`

**Team Training**: Share this `DISASTER_RECOVERY_OVERVIEW.md`

---

**Last Updated**: October 19, 2024  
**Next Review**: January 2025 (post-Q1 exercise)  
**Owner**: Infrastructure Team  
**Status**: Ready for Review & Tabletop Exercise
