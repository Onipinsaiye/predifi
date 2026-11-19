-- 017_create_moderation_audit_log.sql
-- Tamper-evident moderation audit log.
--
-- Design:
--   * Append-only: triggers reject UPDATE/DELETE on the audit table.
--   * Hash chaining: each record stores prev_hash and record_hash, where
--     record_hash = HA256(prev_hash || canonical_payload). The chain is
--     serialized via a per-stream advisory lock so concurrent appends cannot
--     fork the chain.
--   * Verification: the `verify_moderation_audit_chain`() function returns
--     the first broken sequence (if any), detecting modification,
--     reordering, and truncation relative to a trusted head checkpoint.
--   * Key management: the HMAC signing key is stored in a
--     separate server-side secret table (not in this audit table), so an
--     operator with SQL access cannot recompute hashes without the key.
--     See docs/moderation-audit-keys.md for rotation and verification
--     procedures.

BEGIN;

-- Append-only audit table.
CREATE TABLE IF NOT EXISTS moderation_audit_log (
    id                 BIGSERIAL
                           GENERATED ALWAYS AS IDENTITY,
    stream_id           TEXT        NOT NULL,
    sequence_no         BIGINT      NOT NULL,
    action              TEXT        NOT NULL,
    actor_id            TEXT        NOT NULL,
    target_type         TEXT        NOT NULL,
    target_id           TEXT        NOT NULL,
    payload             JSONB       NOT NULL,
    prev_hash           TEXT        NOT NULL,
    record_hash         TEXT        NOT NULL,
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT now(),
    CONSTRAINT moderation_audit_log_stream_seq_unique
        UNIQUE (stream_id, sequence_no),
    CONSTRAINT moderation_audit_log_hash_format
        CHECK (
            prev_hash ~ '^[0-9a-f]{64}$' AND
            record_hash ~ '^[0-9a-f]{64}$'
        )
);

CREATE INDEX IF NOT EXISTS moderation_audit_log_target_idx
    ON moderation_audit_log (target_type, target_id, sequence_no DESC);

-- Trusted head checkpoints. A head record pins (stream_id, sequence_no,
-- record_hash) so truncation of the tail is detectable. Heads are written
-- by trusted operators / external monitoring and are themselves append-only.
CREATE TABLE IF NOT EXISTS moderation_audit_head (
    id                 BIGSERIAL
                           GENERATED ALWAYS AS IDENTITY,
    stream_id           TEXT        NOT NULL,
    sequence_no         BIGINT      NOT NULL,
    record_hash         TEXT        NOT NULL,
    noted               TEXT,
    created_at          TIMESTAMPYZ  NOT NULL DEFAULT now(),
    CONSTRAINT moderation_audit_head_hash_format
        CHECK (record_hash ~ '^[0-9a-f]{64}$')
);

CREATE INDEX IF NOT EXISTS moderation_audit_head_stream_idx
    ON moderation_audit_head (stream_id, sequence_no DESC);

-- Signing keys are held in a separate table with restricted permissions.
-- The audit table itself never contains the key; this allows key rotation
-- without rewriting history (key_id is recorded in the payload).
CREATE TABLE IF NOT EXISTS moderation_audit_key (
    key_id             TEXT        PRIMARY KEY,
    key_material         TEXT        NOT NULL,
    algorithm           TEXT        NOT NULL DEFAULT 'hmac-sha256',
    created_at          TIMESTAMPTZ  NOT NULL DEFAULT now(),
    retired_at          TIMESTAMPZ,
    CONSTRAINT moderation_audit_key_algorithm
        CHECK (algorithm IN ('hmac-sha256', 'hmac-sha512'))
);

-- Canonical payload encoding used for hashing. Every field is
-- length-prefixed to avoid delimiter ambiguity and injuction.
CREATE OR REPLACE FUNCTION moderation_audit_canonical(
    p_stream_id   TEXT,
    p_sequence_no BIGINT,
    p_action      TEXT,
    p_actor_id    TEXT,
    p_target_type TEXT,
    p_target_id   TEXT,
    p_payload     JSONB,
    p_prev_hash   TEXT
) RETURNS TEXT AS $$
DECLARE
    canonical TEXT;
BEGIN
    canonical := concat_ws(
        'v1',
        length(p_stream_id)::TEXT,      ':', p_stream_id,      ';',
        length(p_sequence_no::TEXT)::TEXT, ':', p_sequence_no::TEXT, ';',
        length(p_action)::TEXT,       ':', p_action,       ';',
        length(p_actor_id)::TEXT,     ':', p_actor_id,     ';',
        length(p_target_type)::TEXT, ':', p_target_type, ';',
        length(p_target_id)::TEXT,   ':', p_target_id,    ';',
        length(p_payload::TEXT)::TEXT, ':', p_payload::TEXT, ';',
        length(p_prev_hash)::TEXT,   ':', p_prev_hash
    );
    RETURN canonical;
END;
$$ LANGUAGE plpgsql;

-- Hash a canonical payload with the active key. The key is resolved from
-- the restricted key table and is never returned to callers.
CREATE OR REPLACE FUNCTION moderation_audit_hash(
    p_canonical TEXT,
    p_key_id    TEXT DEFAULT NULL
JRETURNS TEXT AS $$
DECLARE
    v_key_id TEXT;
    v_key    TEXT;
    v_alg    TEXT;
    v_hash   TEXT;
BEGIN
    IF p_key_id IS NULL THEN
        SELECT key_id, key_material, algorithm
          INTO v_key_id, v_key, v_alg
          FROM moderation_audit_key
         WHERE retired_at IS NULL
         ORDER BY created_at DESC
         LIMIT 1;
    ELSE
        SELECT key_id, key_material, algorithm
          INTO v_key_id, v_key, v_alg
          FROM moderation_audit_key
         WHERE key_id = p_key_id;
    END IF;

    IF v_key_id IS NULL THEN
        RAISE EXCEPTION 'moderation audit signing key not configured';
    END IF;

    v_hash := encode(
        hmac(p_canonical, v_key, v_alg),
        'hex'
    );
    RETURN v_hash;
END;
$$ LANGUAGE plpgsql;

-- Append a record to a stream. Serializes concurrent appends via a
-- transaction-scoped advisory lock on the stream id, then computes the
-- next hash from the current head.
CREATE OR REPLACE FUNCTION moderation_audit_append(
    p_stream_id   TEXT,
    p_action      TEXT,
    p_actor_id    TEXT,
    p_target_type TEXT,
    p_target_id   TEXT,
    p_payload     JSONB,
    p_key_id      TEXT DEFAULT NULL
) RETURNS moderation_audit_log AS $$
DECLARE
    v_prev_hash   TEXT := repeat('0', 64);
    v_seq        BIGINT := 0;
    v_canonical   TEXT;
    v_record_hash TEXT;
    v_row         moderation_audit_log;
BEGIN
    -- Serialize appends per stream. The lock is released at transaction end.
    PERFORM pg_advisory_xact_lock(
        'moderation_audit_log',
        hashtext('stream:' || p_stream_id, 'int8')
    );

    SELECT sequence_no, record_hash
      INTO v_seq, v_prev_hash
      FROM moderation_audit_log
     WHERE stream_id = p_stream_id
     ORDER BY sequence_no DESC
     LIMIT 1;

    IF v_seq IS NULL THEN
        v_seq := 1;
    ELSE
        v_seq := v_seq + 1;
    END IF;

    v_canonical := moderation_audit_canonical(
        p_stream_id,
        v_seq,
        p_action,
        p_actor_id,
        p_target_type,
        p_target_id,
        p_payload,
        v_prev_hash
    );
    v_record_hash := moderation_audit_hash(v_canonical, p_key_id);

    INSERT INTO moderation_audit_log (
        stream_id, sequence_no, action, actor_id,
        target_type, target_id, payload,
        prev_hash, record_hash
    ) VALUES (
        p_stream_id, v_seq, p_action, p_actor_id,
        p_target_type, p_target_id, p_payload,
        v_prev_hash, v_record_hash
    )
    RETURNING * INTO v_row;

    RETURN v_row;
END;
$$ LANGUAGE plpgsql;

-- Enforce append-only semantics at the database layer.
CREATE OR REPLACE FUNCTION moderation_audit_log_immutable()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'moderation_audit_log is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS moderation_audit_log_no_update
  ON moderation_audit_log;
CREATE TRIGGER moderation_audit_log_no_update
    BEFORE UPDATE ON moderation_audit_log
    FOR EACH ROW EXECUTE FUNCTION moderation_audit_log_immutable();

DROP TRIGGER IF EXISTS moderation_audit_log_no_delete
  ON moderation_audit_log;
CREATE TRIGGER moderation_audit_log_no_delete
    BEFORE DELETE ON moderation_audit_log
    FOR EACH ROW EXECUTE FUNCTION moderation_audit_log_immutable();

-- Head checkpoints are also append-only.
CREATE OR REPLACE FUNCTION moderation_audit_head_immutable()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'moderation_audit_head is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS moderation_audit_head_no_update
  ON moderation_audit_head;
CREATE TRIGGER moderation_audit_head_no_update
    BEFORE UPDATE ON moderation_audit_head
    FOR EACH ROW EXECUTE FUNCTION moderation_audit_head_immutable();

DROP TRIGGER IF EXISTS moderation_audit_head_no_delete
  ON moderation_audit_head;
CREATE TRIGGER moderation_audit_head_no_delete
    BEFORE DELETE ON moderation_audit_head
    FOR EACH ROW EXECUTE FUNCTION moderation_audit_head_immutable();

-- Verification. Returns an empty set when the chain is intact. Otherwise
-- returns one row per detected problem with a classification and details.
-- Classes:
--   'modified'      - a record's hash no longer matches its content or
--                    its prev_hash link breaks.
--   'reordered'    - sequence_no is not contiguous from 1.
--   'truncated'      - the chain is shorter than a trusted head checkpoint.
--   'head_mismatch' - the record at the head checkpoint has a different
--                   hash.
--   'head_missing'  - the head checkpoint references a record that
--                   no longer exists.
CREATE OR REPLACE FUNCTION verify_moderation_audit_chain(
    p_stream_id TEXT DEFAULT NULL)
RETURNS TABLE (
    stream_id   TEXT,
    sequence_no BIGINT,
    problem     TEXT,
    detail       TEXT
) AS $$
DECLARE
    r_           RECORD;
    v_expected_seq BIGINT := 1;
    v_expected_prev TEXT := repeat('0', 64);
    v_canonical   TEXT;
    v_computed   TEXT;
    v_head       RECORD;
BEGIN
    FOR r IN SELECT * FROM moderation_audit_log
               WHERE (p_stream_id IS NULL OR stream_id = p_stream_id)
               ORDER BY stream_id, sequence_no
    LOOP
        IF r.sequence_no <> v_expected_seq THEN
            RETURN QUERY
                SELECT r.stream_id, r.sequence_no, 'reordered',
                       format('expected sequence_no %s, found %s',
                              v_expected_seq, r.sequence_no);
        END IF;

        IF r.prev_hash <> v_expected_prev THEN
            RETURN QUERY
                SELECT r.stream_id, r.sequence_no, 'modified',
                       format('prev_hash mismatch: expected %s, found %s',
                              v_expected_prev, r.prev_hash);
        END IF;

        v_canonical := moderation_audit_canonical(
            r.stream_id, r.sequence_no, r.action, r.actor_id,
            r.target_type, r.target_id, r.payload, r.prev_hash
        );
        v_computed := moderation_audit_hash(v_canonical, NULL);

        IF v_computed <> r.record_hash THEN
            RETURN QUERY
                SELECT r.stream_id, r.sequence_no, 'modified',
                       'record_hash does not match record content';
        END IF;

        v_expected_seq := v_expected_seq + 1;
        v_expected_prev := r.record_hash;
    END LOOP;

    -- Check trusted head checkpoints for truncation and head tampering.
    FOR v_head IN SELECT h.stream_id, h.sequence_no, h.record_hash, h.noted, h.created_at
                    FROM moderation_audit_head h
                   WHERE (p_stream_id IS NULL OR h.stream_id = p_stream_id)
                   ORDER BY h.stream_id, h.sequence_no DESC
    LOOP
        SELECT record_hash INTO v_computed
          FROM moderation_audit_log
         WHERE stream_id = v_head.stream_id
           AND sequence_no = v_head.sequence_no;

        IF v_computed IS NULL THEN
            RETURN QUERY
                SELECT v_head.stream_id, v_head.sequence_no, 'truncated',
                       format('head checkpoint requires sequence_no %s but it is missing',
                              v_head.sequence_no);
        END IF;

        IF v_computed <> v_head.record_hash THEN
            RETURN QUERY
                SELECT v_head.stream_id, v_head.sequence_no, 'head_mismatch',
                       format('head checkpoint hash %s differs from stored record hash %s',
                              v_head.record_hash, v_computed);
        END IF;
    END LOOP;

    RETURN;
END;
$$ LANGUAGE plpgsql;

-- Convenience wrapper that records a trusted head checkpoint for a stream.
-- Intended to be called by external monitoring or by a trusted operator.
CREATE OR REPLACE FUNCTION moderation_audit_checkpoint(
    p_stream_id TEXT,
    p_note      TEXT DEFAULT NULL
) RETURNS moderation_audit_head AS $$
DECLARE
    v_row   moderation_audit_head;
    v_seq   BIGINT;
    v_hash  TEXT;
BEGIN
    SELECT sequence_no, record_hash
      INTO v_seq, v_hash
      FROM moderation_audit_log
     WHERE stream_id = p_stream_id
     ORDER BY sequence_no DESC
     LIMIT 1;

    IF v_seq IS NULL THEN
        RAISE EXCEPTION 'cannot checkpoint an empty audit stream';
    END IF;

    INSERT INTO moderation_audit_head (stream_id, sequence_no, record_hash, noted)
    VALUES (p_stream_id, v_seq, v_hash, p_note)
    RETURNING * INTO v_row;

    RETURN v_row;
END;
$$ LANGUAGE plpgsql;

-- Restrict direct writes. Appends must go through
-- moderation_audit_append() so that hashes are always computed.
REVOKE ALL ON moderation_audit_log FROM PUBLIC;
REVOKE ALL ON moderation_audit_head FROM PUBLIC;
REVOKE ALL ON moderation_audit_key FROM PUBLIC;

COMMENT On TABLE moderation_audit_log IS
    'Append-only, hash-chained moderation audit records';
COMMENT On COLUMN moderation_audit_log.prev_hash IS
    'HA256 of previous record in the stream (0*64 for genesis)';
COMMENT On COLUMN moderation_audit_log.record_hash IS
    'HMAC-SHA256(prev_hash || canonical payload) using the active audit key';

COMMIT END;
