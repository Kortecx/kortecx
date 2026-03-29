CREATE TABLE IF NOT EXISTS quick_checks (
  id              TEXT PRIMARY KEY,
  prompt          TEXT NOT NULL,
  response        TEXT,
  status          VARCHAR(20) NOT NULL DEFAULT 'running',
  model           TEXT DEFAULT 'llama3.1:8b',
  engine          TEXT DEFAULT 'ollama',
  tokens_used     INTEGER DEFAULT 0,
  duration_ms     INTEGER DEFAULT 0,
  context_sources JSONB DEFAULT '[]'::jsonb,
  error_message   TEXT,
  created_at      TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  completed_at    TIMESTAMPTZ
);
