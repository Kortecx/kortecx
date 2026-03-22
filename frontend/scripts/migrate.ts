/**
 * Kortecx Schema Migration Runner
 *
 * Reads kortecx.config.json for the target schema version,
 * compares with the current DB version, and applies pending migrations.
 *
 * All migrations are idempotent (IF NOT EXISTS / ADD COLUMN IF NOT EXISTS).
 * User data is NEVER dropped or lost.
 *
 * Usage: npx tsx scripts/migrate.ts
 */

import { Pool } from 'pg';
import * as fs from 'fs';
import * as path from 'path';

const ROOT = path.resolve(__dirname, '..', '..');
const CONFIG_PATH = path.join(ROOT, 'kortecx.config.json');

// ── Load config ────────────────────────────────────────

function loadConfig() {
  if (!fs.existsSync(CONFIG_PATH)) {
    console.error('kortecx.config.json not found at', CONFIG_PATH);
    process.exit(1);
  }
  return JSON.parse(fs.readFileSync(CONFIG_PATH, 'utf-8'));
}

// ── Migrations ─────────────────────────────────────────

interface Migration {
  id: string;
  version: number;
  description: string;
  up: string; // SQL to apply
}

const MIGRATIONS: Migration[] = [
  {
    id: '001_initial',
    version: 1,
    description:
      'Core tables: metrics, tasks, workflows, experts, training, datasets, integrations',
    up: `
      -- Core tables (all IF NOT EXISTS for idempotency)
      CREATE TABLE IF NOT EXISTS metrics (
        id SERIAL PRIMARY KEY,
        captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        active_agents INTEGER DEFAULT 0,
        tasks_completed INTEGER DEFAULT 0,
        tokens_used BIGINT DEFAULT 0,
        avg_latency_ms INTEGER DEFAULT 0,
        success_rate DECIMAL(5,4) DEFAULT 0,
        cost_usd DECIMAL(10,4) DEFAULT 0,
        error_count INTEGER DEFAULT 0
      );

      CREATE TABLE IF NOT EXISTS tasks (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        workflow_id TEXT,
        workflow_name TEXT,
        status VARCHAR(20) NOT NULL DEFAULT 'queued',
        priority VARCHAR(20) DEFAULT 'normal',
        current_step INTEGER DEFAULT 0,
        total_steps INTEGER DEFAULT 1,
        current_expert TEXT,
        tokens_used INTEGER DEFAULT 0,
        estimated_tokens INTEGER,
        progress INTEGER DEFAULT 0,
        input TEXT,
        output TEXT,
        error_message TEXT,
        started_at TIMESTAMPTZ,
        completed_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS workflows (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        goal_statement TEXT,
        goal_file_url TEXT,
        input_file_urls TEXT[],
        status VARCHAR(20) DEFAULT 'draft',
        estimated_tokens INTEGER,
        estimated_cost_usd DECIMAL(8,4),
        estimated_duration_sec INTEGER,
        total_runs INTEGER DEFAULT 0,
        successful_runs INTEGER DEFAULT 0,
        tags TEXT[],
        is_template BOOLEAN DEFAULT false,
        template_category TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        last_run_at TIMESTAMPTZ
      );

      CREATE TABLE IF NOT EXISTS workflow_steps (
        id TEXT PRIMARY KEY,
        workflow_id TEXT NOT NULL,
        step_order INTEGER NOT NULL,
        expert_id TEXT,
        task_description TEXT NOT NULL,
        system_instructions TEXT,
        voice_command TEXT,
        file_locations TEXT[],
        step_file_urls TEXT[],
        step_image_urls TEXT[],
        integrations JSONB,
        model_source VARCHAR(20) NOT NULL DEFAULT 'provider',
        local_model_config JSONB,
        connection_type VARCHAR(20) DEFAULT 'sequential',
        temperature DECIMAL(3,2) DEFAULT 0.7,
        max_tokens INTEGER DEFAULT 4096,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS workflow_runs (
        id TEXT PRIMARY KEY,
        workflow_id TEXT NOT NULL,
        workflow_name TEXT NOT NULL,
        status VARCHAR(20) NOT NULL,
        started_at TIMESTAMPTZ,
        completed_at TIMESTAMPTZ,
        total_tokens_used INTEGER DEFAULT 0,
        total_cost_usd DECIMAL(10,4) DEFAULT 0,
        duration_sec INTEGER,
        input TEXT,
        expert_chain TEXT[],
        error_message TEXT,
        metadata JSONB,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS experts (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        role VARCHAR(30) NOT NULL,
        status VARCHAR(20) NOT NULL DEFAULT 'idle',
        version VARCHAR(20) DEFAULT '1.0.0',
        model_id TEXT NOT NULL,
        model_name TEXT,
        provider_id TEXT NOT NULL,
        provider_name TEXT,
        model_source VARCHAR(20) DEFAULT 'provider',
        local_model_config JSONB,
        system_prompt TEXT,
        temperature DECIMAL(3,2) DEFAULT 0.7,
        max_tokens INTEGER DEFAULT 4096,
        total_runs INTEGER DEFAULT 0,
        success_rate DECIMAL(5,4) DEFAULT 0,
        avg_latency_ms INTEGER DEFAULT 0,
        avg_cost_per_run DECIMAL(8,4) DEFAULT 0,
        rating DECIMAL(3,2) DEFAULT 0,
        tags TEXT[],
        is_public BOOLEAN DEFAULT false,
        is_finetuned BOOLEAN DEFAULT false,
        replica_count INTEGER DEFAULT 1,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS alerts (
        id TEXT PRIMARY KEY,
        severity VARCHAR(20) NOT NULL,
        title TEXT NOT NULL,
        message TEXT NOT NULL,
        provider_id TEXT,
        expert_id TEXT,
        acknowledged BOOLEAN DEFAULT false,
        acknowledged_at TIMESTAMPTZ,
        resolved_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS logs (
        id SERIAL PRIMARY KEY,
        timestamp TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        level VARCHAR(10) NOT NULL,
        message TEXT NOT NULL,
        source TEXT,
        metadata JSONB,
        task_id TEXT,
        run_id TEXT
      );

      -- Legacy: training_jobs kept for data preservation
      CREATE TABLE IF NOT EXISTS training_jobs (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        expert_id TEXT,
        base_model_id TEXT NOT NULL,
        dataset_id TEXT,
        status VARCHAR(20) NOT NULL DEFAULT 'queued',
        progress INTEGER DEFAULT 0,
        epochs INTEGER DEFAULT 3,
        current_epoch INTEGER DEFAULT 0,
        learning_rate DECIMAL(10,8),
        batch_size INTEGER DEFAULT 16,
        training_samples INTEGER,
        validation_samples INTEGER,
        eval_loss DECIMAL(8,6),
        eval_accuracy DECIMAL(5,4),
        gpu_hours DECIMAL(8,2),
        cost_usd DECIMAL(8,2),
        started_at TIMESTAMPTZ,
        completed_at TIMESTAMPTZ,
        estimated_completion_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS datasets (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        status VARCHAR(20) NOT NULL DEFAULT 'draft',
        format VARCHAR(20) DEFAULT 'jsonl',
        sample_count INTEGER DEFAULT 0,
        size_bytes BIGINT DEFAULT 0,
        quality_score INTEGER,
        output_path TEXT,
        source_job_id TEXT,
        schema_id TEXT,
        tags TEXT[],
        categories TEXT[],
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS integrations (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        category VARCHAR(30) NOT NULL,
        icon TEXT,
        color TEXT,
        auth_type VARCHAR(20) DEFAULT 'api_key',
        config_fields JSONB,
        base_url TEXT,
        docs_url TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS integration_connections (
        id TEXT PRIMARY KEY,
        integration_id TEXT NOT NULL,
        name TEXT NOT NULL,
        config JSONB,
        status VARCHAR(20) DEFAULT 'active',
        last_tested_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS plugins (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        version VARCHAR(20) DEFAULT '1.0.0',
        author TEXT,
        source VARCHAR(20) NOT NULL DEFAULT 'personal',
        status VARCHAR(20) DEFAULT 'active',
        icon TEXT,
        color TEXT,
        category TEXT,
        capabilities TEXT[],
        config_schema JSONB,
        config JSONB,
        installed BOOLEAN DEFAULT false,
        downloads INTEGER DEFAULT 0,
        rating DECIMAL(3,2),
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS projects (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        status VARCHAR(20) DEFAULT 'active',
        platforms TEXT[],
        posts_count INTEGER DEFAULT 0,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS project_assets (
        id TEXT PRIMARY KEY,
        project_id TEXT NOT NULL,
        asset_type VARCHAR(30) NOT NULL,
        asset_id TEXT NOT NULL,
        asset_name TEXT NOT NULL,
        asset_path TEXT,
        mlflow_run_id TEXT,
        metadata JSONB,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS api_keys (
        id TEXT PRIMARY KEY,
        provider_id TEXT NOT NULL,
        key_hash TEXT NOT NULL,
        key_prefix TEXT,
        key_suffix TEXT,
        encrypted_key TEXT NOT NULL,
        status VARCHAR(20) DEFAULT 'active',
        last_used_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS synthesis_jobs (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        source VARCHAR(20) NOT NULL,
        model TEXT NOT NULL,
        status VARCHAR(20) DEFAULT 'queued',
        target_samples INTEGER DEFAULT 100,
        current_samples INTEGER DEFAULT 0,
        output_format VARCHAR(20) DEFAULT 'jsonl',
        temperature DECIMAL(3,2) DEFAULT 0.8,
        max_tokens INTEGER DEFAULT 1024,
        batch_size INTEGER DEFAULT 5,
        output_path TEXT,
        tokens_used INTEGER DEFAULT 0,
        progress INTEGER DEFAULT 0,
        error TEXT,
        tags TEXT[],
        started_at TIMESTAMPTZ,
        completed_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS assets (
        id TEXT PRIMARY KEY,
        name TEXT NOT NULL,
        description TEXT,
        folder TEXT DEFAULT '/',
        mime_type TEXT,
        file_type VARCHAR(20),
        file_path TEXT NOT NULL,
        file_name TEXT NOT NULL,
        size_bytes BIGINT DEFAULT 0,
        tags TEXT[],
        metadata JSONB,
        dataset_id TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS hf_datasets (
        id TEXT PRIMARY KEY,
        hf_id TEXT NOT NULL,
        author TEXT,
        name TEXT NOT NULL,
        description TEXT,
        tags TEXT[],
        downloads INTEGER DEFAULT 0,
        likes INTEGER DEFAULT 0,
        config TEXT,
        splits JSONB,
        num_rows INTEGER DEFAULT 0,
        columns TEXT[],
        features JSONB,
        cache_path TEXT,
        size_bytes BIGINT DEFAULT 0,
        status VARCHAR(20) DEFAULT 'available',
        error_message TEXT,
        downloaded_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS dataset_schemas (
        id TEXT PRIMARY KEY,
        dataset_id TEXT,
        name TEXT NOT NULL,
        columns JSONB NOT NULL,
        version INTEGER DEFAULT 1,
        is_template BOOLEAN DEFAULT false,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS data_versions (
        id TEXT PRIMARY KEY,
        dataset_id TEXT NOT NULL,
        version_num INTEGER DEFAULT 1,
        file_path TEXT NOT NULL,
        size_bytes BIGINT DEFAULT 0,
        rows_affected INTEGER DEFAULT 0,
        change_type VARCHAR(20),
        change_summary TEXT,
        created_by TEXT,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS lineage (
        id TEXT PRIMARY KEY,
        source_type VARCHAR(30) NOT NULL,
        source_id TEXT NOT NULL,
        target_type VARCHAR(30) NOT NULL,
        target_id TEXT NOT NULL,
        relationship VARCHAR(30) NOT NULL,
        metadata JSONB,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );
    `,
  },
  {
    id: '002_social_mcp_lineage',
    version: 2,
    description: 'Social connections, OAuth credentials',
    up: `
      CREATE TABLE IF NOT EXISTS oauth_credentials (
        id TEXT PRIMARY KEY,
        platform VARCHAR(30) NOT NULL UNIQUE,
        client_id TEXT NOT NULL,
        client_secret TEXT NOT NULL,
        key_hash TEXT NOT NULL,
        status VARCHAR(20) DEFAULT 'active',
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS social_connections (
        id TEXT PRIMARY KEY,
        platform VARCHAR(30) NOT NULL,
        access_token TEXT NOT NULL,
        refresh_token TEXT,
        token_expires_at TIMESTAMPTZ,
        scopes TEXT[],
        platform_user_id TEXT,
        platform_username TEXT,
        platform_avatar TEXT,
        platform_meta JSONB,
        permissions JSONB,
        status VARCHAR(20) DEFAULT 'active',
        last_used_at TIMESTAMPTZ,
        last_refreshed_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );
    `,
  },
  {
    id: '003_quorum_experts_execution',
    version: 3,
    description:
      'Quorum engine, execution audit, step executions, model comparisons, workflow enhancements',
    up: `
      -- Workflow enhancements
      ALTER TABLE workflows ADD COLUMN IF NOT EXISTS metadata JSONB;
      ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS name TEXT;
      ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS step_description TEXT;
      ALTER TABLE workflow_steps ADD COLUMN IF NOT EXISTS share_memory BOOLEAN DEFAULT true;

      -- Quorum engine tables
      CREATE TABLE IF NOT EXISTS quorum_runs (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        project TEXT NOT NULL,
        task TEXT NOT NULL,
        system_prompt TEXT DEFAULT '',
        backend TEXT NOT NULL,
        model TEXT,
        workers INTEGER NOT NULL,
        status TEXT NOT NULL DEFAULT 'queued',
        config JSONB,
        started_at TIMESTAMPTZ,
        finished_at TIMESTAMPTZ,
        total_tokens BIGINT DEFAULT 0,
        total_duration_ms BIGINT DEFAULT 0,
        decompose_ms BIGINT DEFAULT 0,
        execute_ms BIGINT DEFAULT 0,
        synthesize_ms BIGINT DEFAULT 0,
        final_output TEXT,
        error TEXT,
        workers_succeeded INTEGER DEFAULT 0,
        workers_failed INTEGER DEFAULT 0,
        workers_recovered INTEGER DEFAULT 0,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS quorum_operations (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        run_id UUID NOT NULL REFERENCES quorum_runs(id) ON DELETE CASCADE,
        agent_id TEXT NOT NULL,
        phase TEXT NOT NULL,
        operation TEXT NOT NULL,
        prompt TEXT,
        response TEXT,
        tokens_used BIGINT DEFAULT 0,
        duration_ms BIGINT DEFAULT 0,
        status TEXT NOT NULL DEFAULT 'ok',
        error TEXT,
        metadata JSONB,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS quorum_metrics (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        cpu_usage REAL NOT NULL,
        memory_usage_mb REAL DEFAULT 0,
        active_runs INTEGER NOT NULL,
        queued_runs INTEGER NOT NULL,
        tokens_per_sec DOUBLE PRECISION DEFAULT 0,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS quorum_shared_memory (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        run_id UUID NOT NULL REFERENCES quorum_runs(id) ON DELETE CASCADE,
        phase TEXT NOT NULL,
        memory JSONB NOT NULL,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      CREATE TABLE IF NOT EXISTS quorum_projects (
        id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
        name TEXT NOT NULL UNIQUE,
        config JSONB NOT NULL,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
        updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      -- Execution audit
      CREATE TABLE IF NOT EXISTS execution_audit (
        id TEXT PRIMARY KEY,
        run_id TEXT NOT NULL,
        workflow_id TEXT,
        workflow_name TEXT,
        agent_id TEXT,
        step_id TEXT,
        expert_id TEXT,
        phase VARCHAR(30),
        operation VARCHAR(30) NOT NULL,
        prompt TEXT,
        response TEXT,
        tokens_used INTEGER DEFAULT 0,
        duration_ms INTEGER DEFAULT 0,
        model TEXT,
        engine VARCHAR(20),
        temperature DECIMAL(3,2),
        status VARCHAR(20) DEFAULT 'ok',
        error TEXT,
        metadata JSONB,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      -- Step executions
      CREATE TABLE IF NOT EXISTS step_executions (
        id TEXT PRIMARY KEY,
        run_id TEXT NOT NULL,
        workflow_id TEXT,
        step_id TEXT NOT NULL,
        agent_id TEXT,
        expert_id TEXT,
        step_name TEXT,
        status VARCHAR(20) NOT NULL DEFAULT 'pending',
        model TEXT,
        engine VARCHAR(20),
        tokens_used INTEGER DEFAULT 0,
        prompt_tokens INTEGER DEFAULT 0,
        completion_tokens INTEGER DEFAULT 0,
        duration_ms INTEGER DEFAULT 0,
        queue_wait_ms INTEGER DEFAULT 0,
        inference_ms INTEGER DEFAULT 0,
        cpu_percent DECIMAL(5,2) DEFAULT 0,
        gpu_percent DECIMAL(5,2) DEFAULT 0,
        memory_mb DECIMAL(8,2) DEFAULT 0,
        prompt_preview TEXT,
        response_preview TEXT,
        error_message TEXT,
        metadata JSONB,
        started_at TIMESTAMPTZ,
        completed_at TIMESTAMPTZ,
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      -- Model comparisons
      CREATE TABLE IF NOT EXISTS model_comparisons (
        id TEXT PRIMARY KEY,
        run_id TEXT NOT NULL,
        step_id TEXT NOT NULL,
        original_model TEXT NOT NULL,
        original_engine VARCHAR(20),
        original_tokens INTEGER DEFAULT 0,
        original_duration_ms INTEGER DEFAULT 0,
        original_response TEXT,
        comparison_model TEXT NOT NULL,
        comparison_engine VARCHAR(20),
        comparison_tokens INTEGER DEFAULT 0,
        comparison_duration_ms INTEGER DEFAULT 0,
        comparison_response TEXT,
        temperature DECIMAL(3,2),
        created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );

      -- Quorum indexes
      CREATE INDEX IF NOT EXISTS idx_quorum_ops_run ON quorum_operations(run_id);
      CREATE INDEX IF NOT EXISTS idx_quorum_ops_agent ON quorum_operations(agent_id);
      CREATE INDEX IF NOT EXISTS idx_quorum_ops_phase ON quorum_operations(run_id, phase);
      CREATE INDEX IF NOT EXISTS idx_quorum_runs_project ON quorum_runs(project);
      CREATE INDEX IF NOT EXISTS idx_quorum_runs_status ON quorum_runs(status);
      CREATE INDEX IF NOT EXISTS idx_quorum_metrics_time ON quorum_metrics(created_at);
      CREATE INDEX IF NOT EXISTS idx_quorum_memory_run ON quorum_shared_memory(run_id);

      -- Step execution indexes
      CREATE INDEX IF NOT EXISTS idx_step_exec_run ON step_executions(run_id);
      CREATE INDEX IF NOT EXISTS idx_step_exec_step ON step_executions(step_id);
      CREATE INDEX IF NOT EXISTS idx_exec_audit_run ON execution_audit(run_id);

      -- Schema version tracking table
      CREATE TABLE IF NOT EXISTS _kortecx_schema (
        version INTEGER NOT NULL,
        migration_id TEXT NOT NULL,
        description TEXT,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );
    `,
  },
];

// ── Runner ─────────────────────────────────────────────

async function main() {
  const config = loadConfig();
  const targetVersion = config.schema?.version ?? MIGRATIONS.length;
  const dbUrl = process.env.DATABASE_URL;

  if (!dbUrl) {
    console.error('DATABASE_URL is not set');
    process.exit(1);
  }

  console.log(`Kortecx Migration Runner v${config.version}`);
  console.log(`Target schema version: ${targetVersion}`);
  console.log('\u2500'.repeat(50));

  const pool = new Pool({ connectionString: dbUrl });

  try {
    // Ensure schema tracking table exists
    await pool.query(`
      CREATE TABLE IF NOT EXISTS _kortecx_schema (
        version INTEGER NOT NULL,
        migration_id TEXT NOT NULL,
        description TEXT,
        applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
      );
    `);

    // Check current version
    let currentVersion = 0;
    const result = await pool.query(
      'SELECT MAX(version) as v FROM _kortecx_schema'
    );
    currentVersion = result.rows[0]?.v ?? 0;

    console.log(`Current schema version: ${currentVersion}`);

    if (currentVersion >= targetVersion) {
      console.log('Schema is up to date. No migrations needed.');
      return;
    }

    // Apply pending migrations
    const pending = MIGRATIONS.filter(
      (m) => m.version > currentVersion && m.version <= targetVersion
    );
    console.log(`Applying ${pending.length} migration(s)...\n`);

    for (const migration of pending) {
      console.log(`[${migration.id}] ${migration.description}`);
      const start = Date.now();

      await pool.query(migration.up);

      // Record migration
      await pool.query(
        'INSERT INTO _kortecx_schema (version, migration_id, description) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING',
        [migration.version, migration.id, migration.description]
      );

      const elapsed = Date.now() - start;
      console.log(`  Applied in ${elapsed}ms\n`);
    }

    console.log(`Schema upgraded to version ${targetVersion}`);
  } catch (err) {
    console.error('Migration failed:', err);
    process.exit(1);
  } finally {
    await pool.end();
  }
}

main();
