#!/usr/bin/env bash
set -Eeuo pipefail

: "${AWS_REGION:?Set AWS_REGION}"
: "${SOURCE_DB_INSTANCE_IDENTIFIER:?Set SOURCE_DB_INSTANCE_IDENTIFIER}"
: "${DRILL_DB_SUBNET_GROUP:?Set DRILL_DB_SUBNET_GROUP}"
: "${DRILL_VPC_SECURITY_GROUP_ID:?Set DRILL_VPC_SECURITY_GROUP_ID}"
: "${DRILL_DB_INSTANCE_CLASS:?Set DRILL_DB_INSTANCE_CLASS}"
: "${DRILL_DB_NAME:?Set DRILL_DB_NAME}"

for command in aws jq openssl psql; do
  command -v "$command" >/dev/null 2>&1 || {
    printf 'Required command not found: %s\n' "$command" >&2
    exit 1
  }
done

RPO_TARGET_SECONDS="${RPO_TARGET_SECONDS:-3600}"
RTO_TARGET_SECONDS="${RTO_TARGET_SECONDS:-14400}"
DRILL_INSTANCE="predifi-drill-${GITHUB_RUN_ID:-$(date -u +%s)}"
DRILL_EVIDENCE_PATH="${DRILL_EVIDENCE_PATH:-${RUNNER_TEMP:-/tmp}/postgres-restore-drill.json}"
RUN_STARTED_EPOCH="$(date -u +%s)"
RUN_STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RESTORE_POINT_TIME=""
RESTORE_AGE_SECONDS="null"
MASTER_USERNAME=""
DB_ENDPOINT=""
POOL_COUNT="null"
PREDICTION_COUNT="null"
ORPHAN_PREDICTION_COUNT="null"
INVALID_POOL_COUNT="null"
INVALID_PREDICTION_COUNT="null"
DB_PASSWORD=""
INSTANCE_CREATED="false"
CLEANUP_STATUS="not-created"

finish() {
  local exit_code="$?"
  local finished_epoch finished_at rto_seconds drill_status
  trap - EXIT
  finished_epoch="$(date -u +%s)"
  finished_at="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  rto_seconds=$((finished_epoch - RUN_STARTED_EPOCH))
  drill_status="passed"
  if [[ "$exit_code" -ne 0 ]]; then
    drill_status="failed"
  fi

  if [[ "$INSTANCE_CREATED" == "true" ]]; then
    aws rds delete-db-instance \
      --region "$AWS_REGION" \
      --db-instance-identifier "$DRILL_INSTANCE" \
      --skip-final-snapshot >/dev/null 2>&1 || true
    if aws rds wait db-instance-deleted \
      --region "$AWS_REGION" \
      --db-instance-identifier "$DRILL_INSTANCE" >/dev/null 2>&1; then
      CLEANUP_STATUS="deleted"
    else
      CLEANUP_STATUS="failed"
      drill_status="failed"
      exit_code=1
    fi
  fi

  mkdir -p "$(dirname "$DRILL_EVIDENCE_PATH")"
  jq -n \
    --arg status "$drill_status" \
    --arg region "$AWS_REGION" \
    --arg source_instance "$SOURCE_DB_INSTANCE_IDENTIFIER" \
    --arg restore_point_time "$RESTORE_POINT_TIME" \
    --arg started_at "$RUN_STARTED_AT" \
    --arg finished_at "$finished_at" \
    --arg cleanup_status "$CLEANUP_STATUS" \
    --argjson rpo_seconds "$RESTORE_AGE_SECONDS" \
    --argjson rto_seconds "$rto_seconds" \
    --argjson pool_count "$POOL_COUNT" \
    --argjson prediction_count "$PREDICTION_COUNT" \
    --argjson orphan_prediction_count "$ORPHAN_PREDICTION_COUNT" \
    --argjson invalid_pool_count "$INVALID_POOL_COUNT" \
    --argjson invalid_prediction_count "$INVALID_PREDICTION_COUNT" \
    '{status: $status, region: $region, source_instance: $source_instance,
      restore_point_time: $restore_point_time,
      started_at: $started_at, finished_at: $finished_at,
      achieved_rpo_seconds: $rpo_seconds, achieved_rto_seconds: $rto_seconds,
      temporary_instance_cleanup: $cleanup_status,
      application_checks: {pool_count: $pool_count,
        prediction_count: $prediction_count,
        orphan_prediction_count: $orphan_prediction_count,
        invalid_pool_count: $invalid_pool_count,
        invalid_prediction_count: $invalid_prediction_count}}' \
    >"$DRILL_EVIDENCE_PATH"

  if [[ -n "${GITHUB_STEP_SUMMARY:-}" ]]; then
    {
      printf '## PostgreSQL restore drill: %s\n\n' "$drill_status"
      printf -- '- Source: `%s` in `%s`\n' "$SOURCE_DB_INSTANCE_IDENTIFIER" "$AWS_REGION"
      printf -- '- Point-in-time recovery target: %s\n' "$RESTORE_POINT_TIME"
      printf -- '- Achieved RPO: %s seconds; target: %s seconds\n' "$RESTORE_AGE_SECONDS" "$RPO_TARGET_SECONDS"
      printf -- '- Achieved RTO: %s seconds; target: %s seconds\n' "$rto_seconds" "$RTO_TARGET_SECONDS"
      printf -- '- Temporary database cleanup: %s\n' "$CLEANUP_STATUS"
      printf -- '- Application rows: %s pools, %s predictions\n' "$POOL_COUNT" "$PREDICTION_COUNT"
      printf -- '- Consistency violations: %s orphan predictions, %s negative pool totals, %s negative prediction amounts\n' \
        "$ORPHAN_PREDICTION_COUNT" "$INVALID_POOL_COUNT" "$INVALID_PREDICTION_COUNT"
      printf -- '- Evidence: `%s`\n' "$DRILL_EVIDENCE_PATH"
    } >>"$GITHUB_STEP_SUMMARY"
  fi

  unset DB_PASSWORD
  exit "$exit_code"
}
trap finish EXIT

SOURCE_DB="$(aws rds describe-db-instances \
  --region "$AWS_REGION" \
  --db-instance-identifier "$SOURCE_DB_INSTANCE_IDENTIFIER" \
  --query 'DBInstances[0]' \
  --output json)"
RESTORE_POINT_TIME="$(jq -r '.LatestRestorableTime // empty' <<<"$SOURCE_DB")"
MASTER_USERNAME="$(jq -r '.MasterUsername // empty' <<<"$SOURCE_DB")"
if [[ -z "$RESTORE_POINT_TIME" || -z "$MASTER_USERNAME" ]]; then
  printf 'No usable point-in-time recovery target found for %s\n' "$SOURCE_DB_INSTANCE_IDENTIFIER" >&2
  exit 1
fi

RESTORE_AGE_SECONDS=$((RUN_STARTED_EPOCH - $(date -u -d "$RESTORE_POINT_TIME" +%s)))
if [[ "$RESTORE_AGE_SECONDS" -lt 0 ]]; then
  printf 'Restore point is later than drill start time: %s\n' "$RESTORE_POINT_TIME" >&2
  exit 1
fi

aws rds restore-db-instance-to-point-in-time \
  --region "$AWS_REGION" \
  --target-db-instance-identifier "$DRILL_INSTANCE" \
  --source-db-instance-identifier "$SOURCE_DB_INSTANCE_IDENTIFIER" \
  --use-latest-restorable-time \
  --db-instance-class "$DRILL_DB_INSTANCE_CLASS" \
  --db-subnet-group-name "$DRILL_DB_SUBNET_GROUP" \
  --vpc-security-group-ids "$DRILL_VPC_SECURITY_GROUP_ID" \
  --no-publicly-accessible \
  --tags Key=ManagedBy,Value=postgres-restore-drill Key=ExpiresAfter,Value=24-hours >/dev/null
INSTANCE_CREATED="true"
aws rds wait db-instance-available \
  --region "$AWS_REGION" \
  --db-instance-identifier "$DRILL_INSTANCE"

DB_PASSWORD="$(openssl rand -hex 24)"
if [[ -n "${GITHUB_ACTIONS:-}" ]]; then
  printf '::add-mask::%s\n' "$DB_PASSWORD"
fi
aws rds modify-db-instance \
  --region "$AWS_REGION" \
  --db-instance-identifier "$DRILL_INSTANCE" \
  --master-user-password "$DB_PASSWORD" \
  --apply-immediately >/dev/null
aws rds wait db-instance-available \
  --region "$AWS_REGION" \
  --db-instance-identifier "$DRILL_INSTANCE"

DB_ENDPOINT="$(aws rds describe-db-instances \
  --region "$AWS_REGION" \
  --db-instance-identifier "$DRILL_INSTANCE" \
  --query 'DBInstances[0].Endpoint.Address' \
  --output text)"
QUERY_RESULT="$(PGPASSWORD="$DB_PASSWORD" PGSSLMODE=require psql \
  --no-psqlrc \
  --set=ON_ERROR_STOP=1 \
  --tuples-only \
  --no-align \
  --field-separator='|' \
  --host "$DB_ENDPOINT" \
  --username "$MASTER_USERNAME" \
  --dbname "$DRILL_DB_NAME" \
  --command 'SELECT
    (SELECT count(*) FROM pools),
    (SELECT count(*) FROM predictions),
    (SELECT count(*) FROM predictions p LEFT JOIN pools m USING (pool_id) WHERE m.pool_id IS NULL),
    (SELECT count(*) FROM pools WHERE total_stake < 0),
    (SELECT count(*) FROM predictions WHERE amount < 0);')"
IFS='|' read -r POOL_COUNT PREDICTION_COUNT ORPHAN_PREDICTION_COUNT INVALID_POOL_COUNT INVALID_PREDICTION_COUNT <<<"$QUERY_RESULT"

if [[ "$ORPHAN_PREDICTION_COUNT" -ne 0 || "$INVALID_POOL_COUNT" -ne 0 || "$INVALID_PREDICTION_COUNT" -ne 0 ]]; then
  printf 'Application-level PostgreSQL consistency checks failed.\n' >&2
  exit 1
fi
if [[ "$RESTORE_AGE_SECONDS" -gt "$RPO_TARGET_SECONDS" ]]; then
  printf 'Achieved RPO %s seconds exceeded target %s seconds.\n' "$RESTORE_AGE_SECONDS" "$RPO_TARGET_SECONDS" >&2
  exit 1
fi
if [[ $(( $(date -u +%s) - RUN_STARTED_EPOCH )) -gt "$RTO_TARGET_SECONDS" ]]; then
  printf 'Restore drill exceeded RTO target %s seconds.\n' "$RTO_TARGET_SECONDS" >&2
  exit 1
fi