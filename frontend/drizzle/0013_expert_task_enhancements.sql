-- Add expert-task linkage columns
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS expert_id TEXT;
ALTER TABLE tasks ADD COLUMN IF NOT EXISTS expert_run_id TEXT;

-- Indexes for fast lookup
CREATE INDEX IF NOT EXISTS idx_tasks_expert_id ON tasks(expert_id);
CREATE INDEX IF NOT EXISTS idx_tasks_expert_run_id ON tasks(expert_run_id);
