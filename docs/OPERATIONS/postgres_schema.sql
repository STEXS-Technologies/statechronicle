-- StateChronicle durable-ledger baseline schema.
-- Apply with the application migration tool and review the isolation/role
-- policy for the deployment. All mutation tables are tenant-scoped; callers
-- must use one SERIALIZABLE transaction for a settlement.

-- Serialize concurrent startup/migration attempts. PostgreSQL can race on
-- implicit row types created by CREATE TABLE IF NOT EXISTS; an explicit
-- transaction-scoped advisory lock prevents duplicate-type failures when
-- multiple workers apply the baseline concurrently.
BEGIN;
SELECT pg_advisory_xact_lock(735178421906271);

CREATE TABLE IF NOT EXISTS sc_schema_meta (
    schema_name text PRIMARY KEY CHECK (length(schema_name) > 0),
    schema_version integer NOT NULL CHECK (schema_version >= 1)
);

INSERT INTO sc_schema_meta (schema_name, schema_version)
VALUES ('statechronicle', 1)
ON CONFLICT (schema_name) DO NOTHING;

CREATE TABLE IF NOT EXISTS sc_idempotency (
    tenant_id        text        NOT NULL CHECK (length(tenant_id) > 0),
    intent_id        text        NOT NULL CHECK (length(intent_id) > 0),
    payload_digest   bytea       NOT NULL CHECK (octet_length(payload_digest) = 32),
    status           text        NOT NULL CHECK (status IN ('in_progress', 'committed')),
    attempt_id       text        NOT NULL,
    lease_expires_at timestamptz NOT NULL,
    commit_id        text,
    intent_payload   bytea,
    PRIMARY KEY (tenant_id, intent_id),
    CHECK ((status = 'committed' AND commit_id IS NOT NULL)
        OR (status = 'in_progress'))
);

CREATE TABLE IF NOT EXISTS sc_commits (
    tenant_id  text   NOT NULL CHECK (length(tenant_id) > 0),
    commit_id  text   NOT NULL CHECK (length(commit_id) > 0),
    sequence   bigint NOT NULL CHECK (sequence >= 0),
    payload    bytea  NOT NULL,
    PRIMARY KEY (tenant_id, commit_id),
    UNIQUE (tenant_id, sequence)
);

CREATE TABLE IF NOT EXISTS sc_heads (
    tenant_id  text   PRIMARY KEY CHECK (length(tenant_id) > 0),
    commit_id  text   NOT NULL CHECK (length(commit_id) > 0),
    sequence   bigint NOT NULL CHECK (sequence >= 0),
    state_root bytea  NOT NULL CHECK (octet_length(state_root) = 32)
);

DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sc_heads_commit_fk'
          AND conrelid = 'sc_heads'::regclass
    ) THEN
        ALTER TABLE sc_heads
            ADD CONSTRAINT sc_heads_commit_fk
            FOREIGN KEY (tenant_id, commit_id)
            REFERENCES sc_commits (tenant_id, commit_id);
    END IF;
END $$;

CREATE OR REPLACE FUNCTION sc_guard_idempotency_update()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF OLD.status = 'committed' THEN
        RAISE EXCEPTION 'StateChronicle committed idempotency rows are immutable';
    END IF;
    IF NEW.tenant_id <> OLD.tenant_id
       OR NEW.intent_id <> OLD.intent_id
       OR NEW.payload_digest IS DISTINCT FROM OLD.payload_digest
       OR NEW.intent_payload IS DISTINCT FROM OLD.intent_payload
       OR (NEW.status = 'committed' AND NEW.commit_id IS NULL)
       OR (NEW.status = 'in_progress' AND NEW.commit_id IS NOT NULL) THEN
        RAISE EXCEPTION 'StateChronicle idempotency identity is immutable';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS sc_idempotency_state_guard ON sc_idempotency;
CREATE TRIGGER sc_idempotency_state_guard
    BEFORE UPDATE ON sc_idempotency
    FOR EACH ROW EXECUTE FUNCTION sc_guard_idempotency_update();

-- Idempotency rows are created before their commit in the same transaction,
-- so this foreign key is added after both tables exist. It also upgrades
-- compatible legacy schemas and fails migration if orphaned committed rows
-- already exist.
DO $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM pg_constraint
        WHERE conname = 'sc_idempotency_commit_fk'
          AND conrelid = 'sc_idempotency'::regclass
    ) THEN
        ALTER TABLE sc_idempotency
            ADD CONSTRAINT sc_idempotency_commit_fk
            FOREIGN KEY (tenant_id, commit_id)
            REFERENCES sc_commits (tenant_id, commit_id);
    END IF;
END $$;

CREATE TABLE IF NOT EXISTS sc_events (
    tenant_id   text   NOT NULL CHECK (length(tenant_id) > 0),
    event_id    text   NOT NULL CHECK (length(event_id) > 0),
    commit_id   text   NOT NULL CHECK (length(commit_id) > 0),
    event_index integer NOT NULL CHECK (event_index >= 0),
    payload     bytea  NOT NULL,
    PRIMARY KEY (tenant_id, event_id),
    UNIQUE (tenant_id, commit_id, event_index),
    FOREIGN KEY (tenant_id, commit_id)
        REFERENCES sc_commits (tenant_id, commit_id)
);

-- Accepted history is append-only. Corrections are new commits; no role used
-- by the application may mutate or delete historical rows.
CREATE OR REPLACE FUNCTION sc_reject_history_mutation()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    RAISE EXCEPTION 'StateChronicle history rows are immutable';
END;
$$;

DROP TRIGGER IF EXISTS sc_commits_immutable ON sc_commits;
CREATE TRIGGER sc_commits_immutable
    BEFORE UPDATE OR DELETE ON sc_commits
    FOR EACH ROW EXECUTE FUNCTION sc_reject_history_mutation();
DROP TRIGGER IF EXISTS sc_events_immutable ON sc_events;
CREATE TRIGGER sc_events_immutable
    BEFORE UPDATE OR DELETE ON sc_events
    FOR EACH ROW EXECUTE FUNCTION sc_reject_history_mutation();

CREATE TABLE IF NOT EXISTS sc_projections (
    tenant_id   text   NOT NULL CHECK (length(tenant_id) > 0),
    resource_id text   NOT NULL CHECK (length(resource_id) > 0),
    version     bigint NOT NULL CHECK (version >= 0),
    payload     bytea  NOT NULL,
    PRIMARY KEY (tenant_id, resource_id)
);

CREATE OR REPLACE FUNCTION sc_guard_projection_update()
RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    IF NEW.version < OLD.version THEN
        RAISE EXCEPTION 'StateChronicle projection version cannot regress';
    END IF;
    IF NEW.version = OLD.version AND NEW.payload <> OLD.payload THEN
        RAISE EXCEPTION 'StateChronicle equal-version projection conflict';
    END IF;
    RETURN NEW;
END;
$$;

DROP TRIGGER IF EXISTS sc_projections_monotonic ON sc_projections;
CREATE TRIGGER sc_projections_monotonic
    BEFORE UPDATE ON sc_projections
    FOR EACH ROW EXECUTE FUNCTION sc_guard_projection_update();

CREATE TABLE IF NOT EXISTS sc_outbox (
    delivery_key  text        PRIMARY KEY CHECK (length(delivery_key) > 0),
    tenant_id     text        NOT NULL CHECK (length(tenant_id) > 0),
    commit_id     text        NOT NULL CHECK (length(commit_id) > 0),
    payload_digest bytea       NOT NULL CHECK (octet_length(payload_digest) = 32),
    payload       bytea       NOT NULL,
    lease_owner   text,
    lease_until   timestamptz,
    delivered_at  timestamptz,
    quarantined_at timestamptz,
    quarantine_error text CHECK (quarantine_error IS NULL OR octet_length(quarantine_error) <= 4096),
    last_error    text CHECK (last_error IS NULL OR octet_length(last_error) <= 4096),
    FOREIGN KEY (tenant_id, commit_id)
        REFERENCES sc_commits (tenant_id, commit_id),
    CHECK (delivered_at IS NULL OR lease_owner IS NULL),
    CHECK (quarantined_at IS NULL OR delivered_at IS NULL)
);

ALTER TABLE sc_outbox ADD COLUMN IF NOT EXISTS last_error text;

CREATE TABLE IF NOT EXISTS sc_consumer_deliveries (
    delivery_key text PRIMARY KEY CHECK (length(delivery_key) > 0),
    status text NOT NULL CHECK (status IN ('in_progress', 'applied')),
    attempt_id text NOT NULL CHECK (length(attempt_id) > 0),
    lease_until timestamptz NOT NULL,
    applied_at timestamptz,
    last_error text
);

CREATE TABLE IF NOT EXISTS sc_projection_rebuild_checkpoints (
    checkpoint_key text   PRIMARY KEY CHECK (length(checkpoint_key) > 0),
    next_event     bigint NOT NULL CHECK (next_event >= 0)
);

CREATE INDEX IF NOT EXISTS sc_outbox_pending_idx
    ON sc_outbox (tenant_id, lease_until, delivery_key)
    WHERE delivered_at IS NULL AND quarantined_at IS NULL;

-- Worker claim pattern (run inside a short transaction):
--   SELECT delivery_key FROM sc_outbox
--   WHERE delivered_at IS NULL AND quarantined_at IS NULL
--     AND (lease_until IS NULL OR lease_until <= now())
--   ORDER BY delivery_key FOR UPDATE SKIP LOCKED LIMIT $1;
-- Then update only the selected keys with lease_owner/lease_until, commit the
-- claim, publish outside the transaction, and mark delivered idempotently.

COMMIT;
