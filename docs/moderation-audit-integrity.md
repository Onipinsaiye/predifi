# Moderation Audit Integrity

Moderation history is stored in the `moderation_audit_log` table and is
designed to be tamper-evident: records are appended, chained, and signed with
HMAC-SHA256.

## Guarantees

- **Append-only.** The table rejects `UPDATE` and `DELETE` via database
  triggers. Corrections are appended as new records that reference the prior
  record via `previous_id`.
- **Hash chain.** Each record stores `prev_hash` (the predecessor's
  `record_hash`) and its own `record_hash`. The genesis record uses a
  zero hash as its predecessor.
- **Signed records.** `record_hash` is an HMAC-SHA256 over a canonical
  JSON serialization of the record fields (including `prev_hash`).
- **Monotonic sequence.** `seq` is unique per `chain_id` and increases by 1
  per record. Reordering is detected by the verifier.

## What the verifier detects

Run `verify_audit_chain` against a JSON dump of the chain. The verifier
rejects the chain if any of the following holds:

1. Any field of a record was modified (the HMAC no longer matches).
2. Records were reordered (`seq` is not monotonically increasing by 1, or
   the `prev_hash` link breaks).
3. Records were truncated from the end (the caller supplies the expected
   length via `--expected-length`).

Truncation from the end cannot be detected by the chain alone, so the
expected length must come from an external source of truth (e.g. a published
 checkpoint, monitoring system, or an independent log sink).

## Verification command

```
export MODERATION_AUDIT_HMAC_KEY="$(cat /run/secrets/moderation-audit-key)"
cargo run --bin verify_audit_chain -- \
  --input /var/lib/predifi/audit_chain.json \
  --expected-length 1234
```

The input file is a JSON array of objects of the form `record`/`record_hash` that
 matches the `HashedRecord` structure in `backend/src/moderation_audit.rs`.

## Key management

### Storage

The HMAC key is a high-entropy random secret of at least 32 bytes. It is
stored in the organization secret store (Vault, AWS Secrets Manager, G\CP SM
 etc.) and injected into the service as the `MODERATION_AUDIT_HMAC_KEY` for
environment variable. It must never be committed to source control or logged.

### Rotation

Rotation is supported without invalidating historical records:

1. Generate a new key (e.g. `openssl rand -base64 48`).
2. Set `MODERATION_AUDIT_HMAC_KEY` to the new key and
   `MODERATION_AUDIT_HMAC_KEYS` to the comma-separated list of all keys
   (new first, old keys after).
3. Restart the service. New records are signed with the new key;
   `verify_audit_chain` accepts any key in the list.
4. Once all records signed with the old key are outside the retention
   window, the old key can be removed from `MODERATION_AUDIT_HMAC_KEYS`.

### Compromise response

If an operator account or the active key is compromised:

1. Rotate to a new key immediately and remove the compromised key from
   `MODERATION_AUDIT_HMAC_KEYS`.
2. Run the verifier against the chain as stored in the database and
   against any external checkpoints to identify afficted records.
3. Append a correction record (`action = "moderation_correction"`) that
   references the affected record via `previous_id`. Never delete or
   rewrite the original record.

## Operational notes

- Take periodic checkpoints of the last record hash and sequence number and
  store them in an independent system. This is the only way to detect
  truncation of the tail of the chain.
- The verifier reads the chain from a JSON dump. Produce the dump with a
  read-only database role and include all records for the chain in `seq`
  order.
- Run the verifier on a schedule and alert on non-zero exit codes.
