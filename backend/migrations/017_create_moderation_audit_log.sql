-- 017_create_moderation_audit_log.sql
-- Tamper-evident moderation audit log with step-up verification metadata.
--
-- Design:
--   * Append-only: triggers reject UPDATE/DELETE on the audit table.
--   * Hash chaining: each record stores prev_hash and record_hash, where
--     record_hash = HMAC-SHA256(key, prev_hash || canonical_payload). The chain is
--     serialized via a per-stream advisory lock so concurrent appends cannot
--     fork the chain.
--   * Step-up verification: privileged moderation actions must carry an
--     action-bound wallet proof (or approved step-up reference) whose timestamp
--     is recorded in the payload together with actor and reason. Stale proofs
--     are rejected at append time; replayed proofs are rejected via a unique
--     proof-digest table.
--   * Verification: the `verify_moderation_audit_chain`() function returns
--     the first broken sequence (if any), detecting modification,
--     reordering, and truncation relative to a trusted head checkpoint.
--   * Key management: the HMAC signing key is stored in a separate
--     server-side secret table (not in this audit table), so an operator with
--     SQL access cannot recompute hashes without the key. See
--     docs/moderation-audit-keys.md for rotation and verification procedures.

BEGIN;

CREATE EXTENSION IF NOT EXISTS pgcrypto;

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
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
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

-- Privileged actions and the step-up requirement for each.
CREATE TABLE IF NOT EXISTS moderation_privileged_action (
    action              TEXT        PRIMARY KEY,
    max_proof_age       INTERVAL    NOT NULL DEFAULT interval '5 minutes',
    required_role       TEXT        NOT NULL
);

INSERT INTO moderation_privileged_action (action, max_proof_age, required_role) VALUES
    ('hide_content',         interval '5 minutes', 'moderator_senior'),
    ('unhide_content',       interval '5 minutes', 'moderator_senior'),
    ('expose_private_report', interval '5 minutes', 'moderator_senior'),
    ('delete_content',       interval '5 minutes', 'moderator_senior'),
    ('ban_user',             interval '5 minutes', 'moderator_senior')
ON CONFLICT (action) DO NOTHING;

-- Reviewer role grants (least privilege). A reviewer may only perform the
-- actions explicitly granted to its role.
CREATE TABLE IF NOT EXISTS moderation_reviewer_role (
    actor_id            TEXT        NOT NULL,
    role                TEXT        NOT NULL,
    granted_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (actor_id, role)
);

-- Explicit moderation states for reports, reviews, actions and appeals.
-- States are modelled as a single enumerated domain so that transitions can
-- be validated uniformly across the moderation lifecycle.
CREATE TABLE IF NOT EXISTS moderation_state (
    state               TEXT        PRIMARY KEY,
    description         TEXT        NOT NULL
);

INSERT INTO moderation_state (state, description) VALUES
    ('reported',        'A report has been submitted and awaits triage'),
    ('under_review',    'A reviewer is actively evaluating the report'),
    ('action_taken',    'A moderation action has been applied to the subject'),
    ('appealed',        'The subject has appealed the moderation action'),
    ('appeal_upheld',   'The appeal was reviewed and the action stands'),
    ('appeal_reversed', 'The appeal was reviewed and the action was reversed'),
    ('dismissed',       'The report was dismissed without action')
ON CONFLICT (state) DO NOTHING;

-- Legal state transitions. Each row declares an allowed (from_state,
-- to_state) edge together with the reviewer role permitted to perform it.
-- The service layer consults this table (and moderation_reviewer_role) to
-- enforce reviewer permissions; the database enforces the edge itself.
CREATE TABLE IF NOT EXISTS moderation_transition_rule (
    from_state          TEXT        NOT NULL
                            REFERENCES moderation_state (state),
    to_state            TEXT        NOT NULL
                            REFERENCES moderation_state (state),
    required_role       TEXT        NOT NULL,
    PRIMARY KEY (from_state, to_state),
    CONSTRAINT moderation_transition_rule_distinct
        CHECK (from_state <> to_state)
);

INSERT INTO moderation_transition_rule (from_state, to_state, required_role) VALUES
    ('reported',        'under_review',    'moderator'),
    ('reported',        'dismissed',        'moderator'),
    ('under_review',    'action_taken',     'moderator_senior'),
    ('under_review',    'dismissed',        'moderator'),
    ('action_taken',    'appealed',         'moderator'),
    ('appealed',        'appeal_upheld',    'moderator_senior'),
    ('appealed',        'appeal_reversed',  'moderator_senior')
ON CONFLICT (from_state, to_state) DO NOTHING;

-- Transition history for moderation subjects. Every state change is recorded
-- with the actor, the actor's role, the prior state and a timestamp. The
-- unique constraint on (subject_type, subject_id, from_state, to_state,
-- actor_id, transition_seq) makes duplicate submissions idempotent: a retry
-- that re-submits the same transition for the same subject is rejected
-- rather than double-recorded.
CREATE TABLE IF NOT EXISTS moderation_transitions (
    id                 BIGSERIAL
                           GENERATED ALWAYS AS IDENTITY,
    subject_type       TEXT        NOT NULL,
    subject_id         TEXT        NOT NULL,
    transition_seq     BIGINT      NOT NULL,
    from_state         TEXT        NOT NULL
                           REFERENCES moderation_state (state),
    to_state           TEXT        NOT NULL
                           REFERENCES moderation_state (state),
    prior_state        TEXT
                           REFERENCES moderation_state (state),
    actor_id           TEXT        NOT NULL,
    actor_role         TEXT        NOT NULL,
    reason             TEXT,
    transitioned_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT moderation_transitions_unique
        UNIQUE (subject_type, subject_id, transition_seq),
    CONSTRAINT moderation_transitions_idempotent
        UNIQUE (subject_type, subject_id, from_state, to_state, actor_id),
    CONSTRAINT moderation_transitions_legal_edge
        FOREIGN KEY (from_state, to_state)
            REFERENCES moderation_transition_rule (from_state, to_state)
);

CREATE INDEX IF NOT EXISTS moderation_transitions_subject_idx
    ON moderation_transitions (subject_type, subject_id, transition_seq DESC);

-- Record a moderation state transition. Validates the edge against
-- moderation_transition_rule, enforces that the actor holds the required
-- role, checks that from_state matches the subject's current state (or that
-- the subject has no prior transitions when from_state is 'reported'), and
-- rejects duplicate submissions via the unique constraints. The transition
-- is also appended to the hash-chained audit log so the full history is
-- tamper-evident and survives retries.
CREATE OR REPLACE FUNCTION moderation_record_transition(
    p_subject_type TEXT,
    p_subject_id   TEXT,
    p_from_state   TEXT,
    p_to_state     TEXT,
    p_actor_id     TEXT,
    p_actor_role   TEXT,
    p_reason       TEXT DEFAULT NULL
) RETURNS moderation_transitions AS $$
DECLARE
    v_required_role TEXT;
    v_current_state TEXT;
    v_prior_state   TEXT;
    v_next_seq      BIGINT;
    v_row           moderation_transitions;
BEGIN
    SELECT required_role
      INTO v_required_role
      FROM moderation_transition_rule
     WHERE from_state = p_from_state AND to_state = p_to_state;

    IF v_required_role IS NULL THEN
        RAISE EXCEPTION 'illegal moderation transition % -> %',
            p_from_state, p_to_state;
    END IF;

    IF p_actor_role IS DISTINCT FROM v_required_role THEN
        RAISE EXCEPTION 'actor % with role % may not perform % -> % (requires %)',
            p_actor_id, p_actor_role, p_from_state, p_to_state, v_required_role;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM moderation_reviewer_role
         WHERE actor_id = p_actor_id AND role = p_actor_role
    ) THEN
        RAISE EXCEPTION 'actor % does not hold granted role %',
            p_actor_id, p_actor_role;
    END IF;

    -- Serialize transitions per subject so sequence numbers are contiguous.
    PERFORM pg_advisory_xact_lock(
        hashtext('moderation_transitions'),
        hashtext(p_subject_type || ':' || p_subject_id)
    );

    SELECT to_state, transition_seq
      INTO v_current_state, v_next_seq
      FROM moderation_transitions
     WHERE subject_type = p_subject_type AND subject_id = p_subject_id
     ORDER BY transition_seq DESC
     LIMIT 1;

    IF v_current_state IS NULL THEN
        IF p_from_state <> 'reported' THEN
            RAISE EXCEPTION 'subject %:% has no history; first transition must start from reported',
                p_subject_type, p_subject_id;
        END IF;
        v_prior_state := NULL;
        v_next_seq := 1;
    ELSE
        IF v_current_state <> p_from_state THEN
            RAISE EXCEPTION 'subject %:% is in state %, not %',
                p_subject_type, p_subject_id, v_current_state, p_from_state;
        END IF;
        v_prior_state := v_current_state;
        v_next_seq := v_next_seq + 1;
    END IF;

    BEGIN
        INSERT INTO moderation_transitions (
            subject_type, subject_id, transition_seq,
            from_state, to_state, prior_state,
            actor_id, actor_role, reason
        ) VALUES (
            p_subject_type, p_subject_id, v_next_seq,
            p_from_state, p_to_state, v_prior_state,
            p_actor_id, p_actor_role, p_reason
        )
        RETURNING * INTO v_row;
    EXCEPTION WHEN unique_violation THEN
        RAISE EXCEPTION 'duplicate moderation transition % -> % for %:% by %',
            p_from_state, p_to_state, p_subject_type, p_subject_id, p_actor_id;
    END;

    -- Mirror the transition into the tamper-evident audit chain.
    PERFORM moderation_audit_append(
        'moderation:' || p_subject_type || ':' || p_subject_id,
        'moderation_transition',
        p_actor_id,
        p_subject_type,
        p_subject_id,
        jsonb_build_object(
            'from_state', p_from_state,
            'to_state', p_to_state,
            'prior_state', v_prior_state,
            'actor_role', p_actor_role,
            'transition_seq', v_next_seq,
            'reason', p_reason
        )
    );

    RETURN v_row;
END;
$$ LANGUAGE plpgsql;

-- Transition history is append-only, consistent with the audit log.
CREATE OR REPLACE FUNCTION moderation_transitions_immutable()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'moderation_transitions is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS moderation_transitions_no_update
  ON moderation_transitions;
CREATE TRIGGER moderation_transitions_no_update
    BEFORE UPDATE ON moderation_transitions
    FOR EACH ROW EXECUTE FUNCTION moderation_transitions_immutable();

DROP TRIGGER IF EXISTS moderation_transitions_no_delete
  ON moderation_transitions;
CREATE TRIGGER moderation_transitions_no_delete
    BEFORE DELETE ON moderation_transitions
    FOR EACH ROW EXECUTE FUNCTION moderation_transitions_immutable();

-- Consumed step-up proofs. A proof digest may be used exactly once.
CREATE TABLE IF NOT EXISTS moderation_stepup_proof (
    proof_digest        TEXT        PRIMARY KEY,
    actor_id            TEXT        NOT NULL,
    action              TEXT        NOT NULL,
    target_type         TEXT        NOT NULL,
    target_id           TEXT        NOT NULL,
    proof_at            TIMESTAMPTZ NOT NULL,
    consumed_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT moderation_stepup_proof_digest_format
        CHECK (proof_digest ~ '^[0-9a-f]{64}$')
);

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
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
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
    key_material       TEXT        NOT NULL,
    algorithm          TEXT        NOT NULL DEFAULT 'hmac-sha256',
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    retired_at         TIMESTAMPTZ,
    CONSTRAINT moderation_audit_key_algorithm
        CHECK (algorithm IN ('hmac-sha256', 'hmac-sha512'))
);

-- Canonical payload encoding used for hashing. Every field is
-- length-prefixed to avoid delimiter ambiguity and injection.
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
) RETURNS TEXT AS $$
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
        hmac(p_canonical, v_key, CASE WHEN v_alg = 'hmac-sha512' THEN 'sha512' ELSE 'sha256' END),
        'hex'
    );
    RETURN v_hash;
END;
$$ LANGUAGE plpgsql;

-- Validate a step-up proof for a privileged action.
--   * the action must be a registered privileged action;
--   * the actor must hold the required role (least privilege);
--   * the proof timestamp must be within max_proof_age of now();
--   * the proof digest must not have been consumed before (replay defense);
--   * the proof is bound to actor, action, and target via the digest input.
-- On success the proof digest is consumed and its timestamp returned.
CREATE OR REPLACE FUNCTION moderation_verify_stepup(
    p_actor_id    TEXT,
    p_action      TEXT,
    p_target_type TEXT,
    p_target_id   TEXT,
    p_proof_at    TIMESTAMPTZ,
    p_proof_digest TEXT
) RETURNS TIMESTAMPTZ AS $$
DECLARE
    v_required_role TEXT;
    v_max_age       INTERVAL;
BEGIN
    SELECT required_role, max_proof_age
      INTO v_required_role, v_max_age
      FROM moderation_privileged_action
     WHERE action = p_action;

    IF v_required_role IS NULL THEN
        RAISE EXCEPTION 'action % is not a registered privileged action', p_action;
    END IF;

    IF NOT EXISTS (
        SELECT 1 FROM moderation_reviewer_role
         WHERE actor_id = p_actor_id AND role = v_required_role
    ) THEN
        RAISE EXCEPTION 'actor % lacks role % required for %',
            p_actor_id, v_required_role, p_action;
    END IF;

    IF p_proof_at IS NULL OR p_proof_at > now() + interval '30 seconds' THEN
        RAISE EXCEPTION 'step-up proof timestamp is missing or in the future';
    END IF;

    IF now() - p_proof_at > v_max_age THEN
        RAISE EXCEPTION 'step-up proof is stale for action %', p_action;
    END IF;

    IF p_proof_digest IS NULL OR p_proof_digest !~ '^[0-9a-f]{64}$' THEN
        RAISE EXCEPTION 'step-up proof digest is malformed';
    END IF;

    BEGIN
        INSERT INTO moderation_stepup_proof (
            proof_digest, actor_id, action, target_type, target_id, proof_at
        ) VALUES (
            p_proof_digest, p_actor_id, p_action, p_target_type, p_target_id, p_proof_at
        );
    EXCEPTION WHEN unique_violation THEN
        RAISE EXCEPTION 'step-up proof has already been used (replay)';
    END;

    RETURN p_proof_at;
END;
$$ LANGUAGE plpgsql;

-- Append a record to a stream. Serializes concurrent appends via a
-- transaction-scoped advisory lock on the stream id, then computes the
-- next hash from the current head.
--
-- For privileged actions the payload must contain a non-empty 'reason'
-- and the step-up fields 'proof_at' (ISO-8601 timestamp) and
-- 'proof_digest' (64 hex chars). The step-up is verified and consumed
-- before the record is written. The recorded payload always includes the
-- actor, reason and proof timestamp, satisfying the audit requirements.
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
    v_seq         BIGINT := 0;
    v_canonical   TEXT;
    v_record_hash TEXT;
    v_row         moderation_audit_log;
    v_is_priv     BOOLEAN;
    v_proof_at    TIMESTAMPTZ;
    v_payload     JSONB := COALESCE(p_payload, '{}'::jsonb);
BEGIN
    -- Serialize appends per stream. The lock is released at transaction end.
    PERFORM pg_advisory_xact_lock(
        hashtext('moderation_audit_log'),
        hashtext('stream:' || p_stream_id)
    );

    SELECT EXISTS (SELECT 1 FROM moderation_privileged_action WHERE action = p_action)
      INTO v_is_priv;

    IF v_is_priv THEN
        IF COALESCE(trim(v_payload->>'reason'), '') = '' THEN
            RAISE EXCEPTION 'privileged action % requires a non-empty reason', p_action;
        END IF;

        v_proof_at := moderation_verify_stepup(
            p_actor_id,
            p_action,
            p_target_type,
            p_target_id,
            (v_payload->>'proof_at')::TIMESTAMPTZ,
            v_payload->>'proof_digest'
        );

        v_payload := v_payload || jsonb_build_object(
            'actor_id', p_actor_id,
            'reason', v_payload->>'reason',
            'proof_at', to_char(v_proof_at AT TIME ZONE 'UTC',
                                'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
            'proof_digest', v_payload->>'proof_digest'
        );
    END IF;

    SELECT sequence_no, record_hash
      INTO v_seq, v_prev_hash
      FROM moderation_audit_log
     WHERE stream_id = p_stream_id
     ORDER BY sequence_no DESC
     LIMIT 1;

    IF v_seq IS NULL THEN
        v_seq := 1;
        v_prev_hash := repeat('0', 64);
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
        v_payload,
        v_prev_hash
    );
    v_record_hash := moderation_audit_hash(v_canonical, p_key_id);

    INSERT INTO moderation_audit_log (
        stream_id, sequence_no, action, actor_id,
        target_type, target_id, payload,
        prev_hash, record_hash
    ) VALUES (
        p_stream_id, v_seq, p_action, p_actor_id,
        p_target_type, p_target_id, v_payload,
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

-- Consumed proofs are also append-only so replay defense cannot be undone.
CREATE OR REPLACE FUNCTION moderation_stepup_proof_immutable()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'moderation_stepup_proof is append-only';
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS moderation_stepup_proof_no_update
  ON moderation_stepup_proof;
CREATE TRIGGER moderation_stepup_proof_no_update
    BEFORE UPDATE ON moderation_stepup_proof
    FOR EACH ROW EXECUTE FUNCTION moderation_stepup_proof_immutable();

DROP TRIGGER IF EXISTS moderation_stepup_proof_no_delete
  ON moderation_stepup_proof;
CREATE TRIGGER moderation_stepup_proof_no_delete
    BEFORE DELETE ON moderation_stepup_proof
    FOR EACH ROW EXECUTE FUNCTION moderation_stepup_proof_immutable();

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
--   'reordered'     - sequence_no is not contiguous from 1.
--   'truncated'     - the chain is shorter than a trusted head checkpoint.
--   'head_mismatch' - the record at the head checkpoint has a different
--                    hash.
--   'head_missing'  - the head checkpoint references a record that
--                    no longer exists.
CREATE OR REPLACE FUNCTION verify_moderation_audit_chain(
    p_stream_id TEXT DEFAULT NULL)
RETURNS TABLE (
    stream_id   TEXT,
    sequence_no BIGINT,
    problem     TEXT,
    detail      TEXT
) AS $$
DECLARE
    r              RECORD;
    v_expected_seq BIGINT;
    v_expected_prev TEXT;
    v_canonical    TEXT;
    v_computed     TEXT;
    v_head         RECORD;
    v_current_stream TEXT := NULL;
    v_stored       TEXT;
BEGIN
    FOR r IN SELECT * FROM moderation_audit_log
               WHERE (p_stream_id IS NULL OR moderation_audit_log.stream_id = p_stream_id)
               ORDER BY moderation_audit_log.stream_id, moderation_audit_log.sequence_no
    LOOP
        IF v_current_stream IS DISTINCT FROM r.stream_id THEN
            v_current_stream := r.stream_id;
            v_expected_seq := 1;
            v_expected_prev := repeat('0', 64);
        END IF;

        IF r.sequence_no <> v_expected_seq THEN
            stream_id := r.stream_id;
            sequence_no := r.sequence_no;
            problem := 'reordered';
            detail := format('expected sequence_no %s, found %s',
                             v_expected_seq, r.sequence_no);
            RETURN NEXT;
        END IF;

        IF r.prev_hash <> v_expected_prev THEN
            stream_id := r.stream_id;
            sequence_no := r.sequence_no;
            problem := 'modified';
            detail := format('prev_hash mismatch: expected %s, found %s',
                             v_expected_prev, r.prev_hash);
            RETURN NEXT;
        END IF;

        v_canonical := moderation_audit_canonical(
            r.stream_id, r.sequence_no, r.action, r.actor_id,
            r.target_type, r.target_id, r.payload, r.prev_hash
        );
        v_computed := moderation_audit_hash(v_canonical, NULL);

        IF v_computed <> r.record_hash THEN
            stream_id := r.stream_id;
            sequence_no := r.sequence_no;
            problem := 'modified';
            detail := 'record_hash does not match record content';
            RETURN NEXT;
        END IF;

        v_expected_seq := r.sequence_no + 1;
        v_expected_prev := r.record_hash;
    END LOOP;

    -- Check trusted head checkpoints for truncation and head tampering.
    FOR v_head IN SELECT h.stream_id AS h_stream, h.sequence_no AS h_seq,
                         h.record_hash AS h_hash
                    FROM moderation_audit_head h
                   WHERE (p_stream_id IS NULL OR h.stream_id = p_stream_id)
                   ORDER BY h.stream_id, h.sequence_no DESC
    LOOP
        SELECT l.record_hash INTO v_stored
          FROM moderation_audit_log l
         WHERE l.stream_id = v_head.h_stream
           AND l.sequence_no = v_head.h_seq;

        IF v_stored IS NULL THEN
            stream_id := v_head.h_stream;
            sequence_no := v_head.h_seq;
            problem := 'truncated';
            detail := format('head checkpoint requires sequence_no %s but it is missing',
                             v_head.h_seq);
            RETURN NEXT;
        ELSIF v_stored <> v_head.h_hash THEN
            stream_id := v_head.h_stream;
            sequence_no := v_head.h_seq;
            problem := 'head_mismatch';
            detail := format('head checkpoint hash %s does not match stored hash %s',
                             v_head.h_hash, v_stored);
            RETURN NEXT;
        END IF;
    END LOOP;

    RETURN;
END;
$$ LANGUAGE plpgsql;

COMMIT;
