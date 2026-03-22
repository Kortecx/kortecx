-- Quorum Engine — Database Schema
-- Extends existing kortecx_dev PostgreSQL database

CREATE TABLE IF NOT EXISTS quorum_runs (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    project           TEXT NOT NULL,
    task              TEXT NOT NULL,
    system_prompt     TEXT DEFAULT '',
    backend           TEXT NOT NULL,
    model             TEXT,
    workers           INTEGER NOT NULL,
    status            TEXT NOT NULL DEFAULT 'queued',
    config            JSONB,
    started_at        TIMESTAMPTZ,
    finished_at       TIMESTAMPTZ,
    total_tokens      BIGINT DEFAULT 0,
    total_duration_ms BIGINT DEFAULT 0,
    decompose_ms      BIGINT DEFAULT 0,
    execute_ms        BIGINT DEFAULT 0,
    synthesize_ms     BIGINT DEFAULT 0,
    final_output      TEXT,
    error             TEXT,
    workers_succeeded INTEGER DEFAULT 0,
    workers_failed    INTEGER DEFAULT 0,
    workers_recovered INTEGER DEFAULT 0,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at        TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_operations (
    id            UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id        UUID NOT NULL REFERENCES quorum_runs(id) ON DELETE CASCADE,
    agent_id      TEXT NOT NULL,
    phase         TEXT NOT NULL,
    operation     TEXT NOT NULL,
    prompt        TEXT,
    response      TEXT,
    tokens_used   BIGINT DEFAULT 0,
    duration_ms   BIGINT DEFAULT 0,
    status        TEXT NOT NULL DEFAULT 'ok',
    error         TEXT,
    metadata      JSONB,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_metrics (
    id              UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    cpu_usage       REAL NOT NULL,
    memory_usage_mb REAL DEFAULT 0,
    active_runs     INTEGER NOT NULL,
    queued_runs     INTEGER NOT NULL,
    tokens_per_sec  DOUBLE PRECISION DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_shared_memory (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id     UUID NOT NULL REFERENCES quorum_runs(id) ON DELETE CASCADE,
    phase      TEXT NOT NULL,
    memory     JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE TABLE IF NOT EXISTS quorum_projects (
    id         UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name       TEXT NOT NULL UNIQUE,
    config     JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_quorum_ops_run ON quorum_operations(run_id);
CREATE INDEX IF NOT EXISTS idx_quorum_ops_agent ON quorum_operations(agent_id);
CREATE INDEX IF NOT EXISTS idx_quorum_ops_phase ON quorum_operations(run_id, phase);
CREATE INDEX IF NOT EXISTS idx_quorum_runs_project ON quorum_runs(project);
CREATE INDEX IF NOT EXISTS idx_quorum_runs_status ON quorum_runs(status);
CREATE INDEX IF NOT EXISTS idx_quorum_metrics_time ON quorum_metrics(created_at);
CREATE INDEX IF NOT EXISTS idx_quorum_memory_run ON quorum_shared_memory(run_id);
