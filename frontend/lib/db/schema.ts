import {
  pgTable, serial, text, integer, bigint,
  timestamp, decimal, boolean, jsonb, varchar,
} from 'drizzle-orm/pg-core';

/* ─── System Metrics (time-series snapshots) ─────────── */
export const metrics = pgTable('metrics', {
  id:            serial('id').primaryKey(),
  capturedAt:    timestamp('captured_at', { withTimezone: true }).defaultNow().notNull(),
  activeAgents:  integer('active_agents').default(0),
  tasksCompleted:integer('tasks_completed').default(0),
  tokensUsed:    bigint('tokens_used', { mode: 'number' }).default(0),
  avgLatencyMs:  integer('avg_latency_ms').default(0),
  successRate:   decimal('success_rate', { precision: 5, scale: 4 }).default('0'),
  costUsd:       decimal('cost_usd', { precision: 10, scale: 4 }).default('0'),
  errorCount:    integer('error_count').default(0),
});

/* ─── Task Queue ─────────────────────────────────────── */
export const tasks = pgTable('tasks', {
  id:             text('id').primaryKey(),
  name:           text('name').notNull(),
  workflowId:     text('workflow_id'),
  workflowName:   text('workflow_name'),
  status:         varchar('status', { length: 20 }).notNull().default('queued'),
  // queued | running | completed | failed | cancelled
  priority:       varchar('priority', { length: 20 }).default('normal'),
  // critical | high | normal | low
  currentStep:    integer('current_step').default(0),
  totalSteps:     integer('total_steps').default(1),
  currentExpert:  text('current_expert'),
  tokensUsed:     integer('tokens_used').default(0),
  estimatedTokens:integer('estimated_tokens'),
  progress:       integer('progress').default(0),
  input:          text('input'),
  output:         text('output'),
  errorMessage:   text('error_message'),
  startedAt:      timestamp('started_at', { withTimezone: true }),
  completedAt:    timestamp('completed_at', { withTimezone: true }),
  createdAt:      timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:      timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Workflow Run History ───────────────────────────── */
export const workflowRuns = pgTable('workflow_runs', {
  id:             text('id').primaryKey(),
  workflowId:     text('workflow_id').notNull(),
  workflowName:   text('workflow_name').notNull(),
  status:         varchar('status', { length: 20 }).notNull(),
  // completed | failed | running | cancelled
  startedAt:      timestamp('started_at', { withTimezone: true }),
  completedAt:    timestamp('completed_at', { withTimezone: true }),
  totalTokensUsed:integer('total_tokens_used').default(0),
  totalCostUsd:   decimal('total_cost_usd', { precision: 10, scale: 4 }).default('0'),
  durationSec:    integer('duration_sec'),
  input:          text('input'),
  expertChain:    text('expert_chain').array(),
  errorMessage:   text('error_message'),
  metadata:       jsonb('metadata'),
  createdAt:      timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── System Alerts ──────────────────────────────────── */
export const alerts = pgTable('alerts', {
  id:             text('id').primaryKey(),
  severity:       varchar('severity', { length: 20 }).notNull(),
  // info | warning | error | critical
  title:          text('title').notNull(),
  message:        text('message').notNull(),
  providerId:     text('provider_id'),
  expertId:       text('expert_id'),
  acknowledged:   boolean('acknowledged').default(false),
  acknowledgedAt: timestamp('acknowledged_at', { withTimezone: true }),
  resolvedAt:     timestamp('resolved_at', { withTimezone: true }),
  createdAt:      timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── System Logs ────────────────────────────────────── */
export const logs = pgTable('logs', {
  id:        serial('id').primaryKey(),
  timestamp: timestamp('timestamp', { withTimezone: true }).defaultNow().notNull(),
  level:     varchar('level', { length: 10 }).notNull(),
  // debug | info | warning | error
  message:   text('message').notNull(),
  source:    text('source'),
  // expert name, provider, system, workflow, etc.
  metadata:  jsonb('metadata'),
  taskId:    text('task_id'),
  runId:     text('run_id'),
});

/* ─── Experts (hosted) ───────────────────────────────── */
export const experts = pgTable('experts', {
  id:            text('id').primaryKey(),
  name:          text('name').notNull(),
  description:   text('description'),
  role:          varchar('role', { length: 30 }).notNull(),
  status:        varchar('status', { length: 20 }).notNull().default('idle'),
  // active | idle | training | offline | error
  version:       varchar('version', { length: 20 }).default('1.0.0'),
  modelId:       text('model_id').notNull(),
  modelName:     text('model_name'),
  providerId:    text('provider_id').notNull(),
  providerName:  text('provider_name'),
  modelSource:   varchar('model_source', { length: 20 }).default('provider'),
  // local | provider
  localModelConfig: jsonb('local_model_config'),
  // { engine: 'ollama' | 'llamacpp', model: string, baseUrl?: string }
  systemPrompt:  text('system_prompt'),
  temperature:   decimal('temperature', { precision: 3, scale: 2 }).default('0.7'),
  maxTokens:     integer('max_tokens').default(4096),
  totalRuns:     integer('total_runs').default(0),
  successRate:   decimal('success_rate', { precision: 5, scale: 4 }).default('0'),
  avgLatencyMs:  integer('avg_latency_ms').default(0),
  avgCostPerRun: decimal('avg_cost_per_run', { precision: 8, scale: 4 }).default('0'),
  rating:        decimal('rating', { precision: 3, scale: 2 }).default('0'),
  tags:          text('tags').array(),
  isPublic:      boolean('is_public').default(false),
  isFinetuned:   boolean('is_finetuned').default(false),
  replicaCount:  integer('replica_count').default(1),
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:     timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Workflows ──────────────────────────────────────── */
export const workflows = pgTable('workflows', {
  id:                text('id').primaryKey(),
  name:              text('name').notNull(),
  description:       text('description'),
  goalStatement:     text('goal_statement'),
  goalFileUrl:       text('goal_file_url'),
  inputFileUrls:     text('input_file_urls').array(),
  status:            varchar('status', { length: 20 }).default('draft'),
  // draft | ready | archived
  estimatedTokens:   integer('estimated_tokens'),
  estimatedCostUsd:  decimal('estimated_cost_usd', { precision: 8, scale: 4 }),
  estimatedDurationSec: integer('estimated_duration_sec'),
  totalRuns:         integer('total_runs').default(0),
  successfulRuns:    integer('successful_runs').default(0),
  tags:              text('tags').array(),
  isTemplate:        boolean('is_template').default(false),
  templateCategory:  text('template_category'),
  createdAt:         timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:         timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
  lastRunAt:         timestamp('last_run_at', { withTimezone: true }),
});

/* ─── Workflow Steps ─────────────────────────────────── */
export const workflowSteps = pgTable('workflow_steps', {
  id:                text('id').primaryKey(),
  workflowId:        text('workflow_id').notNull(),
  order:             integer('step_order').notNull(),
  expertId:          text('expert_id'),
  taskDescription:   text('task_description').notNull(),
  systemInstructions:text('system_instructions'),
  voiceCommand:      text('voice_command'),
  fileLocations:     text('file_locations').array(),
  stepFileUrls:      text('step_file_urls').array(),
  stepImageUrls:     text('step_image_urls').array(),
  integrations:      jsonb('integrations'),
  // [{ type: 'web' | 'database' | 'api' | 'sdk', name, config }]
  modelSource:       varchar('model_source', { length: 20 }).notNull().default('provider'),
  // local | provider
  localModelConfig:  jsonb('local_model_config'),
  connectionType:    varchar('connection_type', { length: 20 }).default('sequential'),
  // sequential | parallel | conditional
  temperature:       decimal('temperature', { precision: 3, scale: 2 }).default('0.7'),
  maxTokens:         integer('max_tokens').default(4096),
  createdAt:         timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Training Jobs ──────────────────────────────────── */
export const trainingJobs = pgTable('training_jobs', {
  id:                text('id').primaryKey(),
  name:              text('name').notNull(),
  expertId:          text('expert_id'),
  baseModelId:       text('base_model_id').notNull(),
  datasetId:         text('dataset_id'),
  status:            varchar('status', { length: 20 }).notNull().default('queued'),
  // queued | preparing | training | evaluating | completed | failed | cancelled
  progress:          integer('progress').default(0),
  epochs:            integer('epochs').default(3),
  currentEpoch:      integer('current_epoch').default(0),
  learningRate:      decimal('learning_rate', { precision: 10, scale: 8 }),
  batchSize:         integer('batch_size').default(16),
  trainingSamples:   integer('training_samples'),
  validationSamples: integer('validation_samples'),
  evalLoss:          decimal('eval_loss', { precision: 8, scale: 6 }),
  evalAccuracy:      decimal('eval_accuracy', { precision: 5, scale: 4 }),
  gpuHours:          decimal('gpu_hours', { precision: 8, scale: 2 }),
  costUsd:           decimal('cost_usd', { precision: 8, scale: 2 }),
  startedAt:         timestamp('started_at', { withTimezone: true }),
  completedAt:       timestamp('completed_at', { withTimezone: true }),
  estimatedCompletionAt: timestamp('estimated_completion_at', { withTimezone: true }),
  createdAt:         timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Datasets ──────────────────────────────────────── */
export const datasets = pgTable('datasets', {
  id:              text('id').primaryKey(),
  name:            text('name').notNull(),
  description:     text('description'),
  status:          varchar('status', { length: 20 }).notNull().default('draft'),
  // draft | generating | ready | failed | archived
  format:          varchar('format', { length: 20 }).default('jsonl'),
  sampleCount:     integer('sample_count').default(0),
  sizeBytes:       bigint('size_bytes', { mode: 'number' }).default(0),
  qualityScore:    integer('quality_score'),
  outputPath:      text('output_path'),                  // file path for generated/imported data
  sourceJobId:     text('source_job_id'),                // synthesis job ID if generated
  schemaId:        text('schema_id'),                    // linked schema definition
  tags:            text('tags').array(),
  categories:      text('categories').array(),
  createdAt:       timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:       timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── HuggingFace Datasets (downloaded/tracked) ───── */
export const hfDatasets = pgTable('hf_datasets', {
  id:              text('id').primaryKey(),
  hfId:            text('hf_id').notNull(),          // e.g. "squad", "imdb", "tatsu-lab/alpaca"
  author:          text('author'),
  name:            text('name').notNull(),            // display name
  description:     text('description'),
  tags:            text('tags').array(),
  downloads:       integer('downloads').default(0),
  likes:           integer('likes').default(0),
  config:          text('config'),                    // selected config name
  splits:          jsonb('splits'),                   // { "train": 87599, "validation": 10570 }
  numRows:         integer('num_rows').default(0),
  columns:         text('columns').array(),
  features:        jsonb('features'),                 // { "text": "Value(dtype='string')", ... }
  cachePath:       text('cache_path'),                // HF cache directory path
  sizeBytes:       bigint('size_bytes', { mode: 'number' }).default(0),
  status:          varchar('status', { length: 20 }).default('available'),
  // available | downloading | downloaded | error
  errorMessage:    text('error_message'),
  downloadedAt:    timestamp('downloaded_at', { withTimezone: true }),
  createdAt:       timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:       timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Integrations ──────────────────────────────────── */
export const integrations = pgTable('integrations', {
  id:            text('id').primaryKey(),
  name:          text('name').notNull(),
  description:   text('description'),
  category:      varchar('category', { length: 30 }).notNull(),
  // api | app | tool | database | storage | messaging | analytics | social | crm | data_analytics
  icon:          text('icon'),
  color:         text('color'),
  authType:      varchar('auth_type', { length: 20 }).default('api_key'),
  // api_key | oauth2 | bearer | basic | none
  configFields:  jsonb('config_fields'),
  baseUrl:       text('base_url'),
  docsUrl:       text('docs_url'),
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:     timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Integration Connections (user instances) ──────── */
export const integrationConnections = pgTable('integration_connections', {
  id:              text('id').primaryKey(),
  integrationId:   text('integration_id').notNull(),
  name:            text('name').notNull(),
  config:          jsonb('config'),
  // encrypted credentials / settings
  status:          varchar('status', { length: 20 }).default('active'),
  // active | error | expired
  lastTestedAt:    timestamp('last_tested_at', { withTimezone: true }),
  createdAt:       timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Plugins ───────────────────────────────────────── */
export const plugins = pgTable('plugins', {
  id:            text('id').primaryKey(),
  name:          text('name').notNull(),
  description:   text('description'),
  version:       varchar('version', { length: 20 }).default('1.0.0'),
  author:        text('author'),
  source:        varchar('source', { length: 20 }).notNull().default('personal'),
  // personal | marketplace
  status:        varchar('status', { length: 20 }).default('active'),
  // active | disabled | error | installing
  icon:          text('icon'),
  color:         text('color'),
  category:      text('category'),
  capabilities:  text('capabilities').array(),
  configSchema:  jsonb('config_schema'),
  config:        jsonb('config'),
  installed:     boolean('installed').default(false),
  downloads:     integer('downloads').default(0),
  rating:        decimal('rating', { precision: 3, scale: 2 }),
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:     timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Projects ──────────────────────────────────────── */
export const projects = pgTable('projects', {
  id:            text('id').primaryKey(),
  name:          text('name').notNull(),
  description:   text('description'),
  status:        varchar('status', { length: 20 }).default('active'),
  // active | draft | completed | archived
  platforms:     text('platforms').array(),
  postsCount:    integer('posts_count').default(0),
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:     timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Project Assets (links items to projects) ──── */
export const projectAssets = pgTable('project_assets', {
  id:          text('id').primaryKey(),
  projectId:   text('project_id').notNull(),
  assetType:   varchar('asset_type', { length: 30 }).notNull(),
  // dataset | chart | model | document | script | workflow | expert
  assetId:     text('asset_id').notNull(),            // ID reference to the actual entity
  assetName:   text('asset_name').notNull(),
  assetPath:   text('asset_path'),                     // local file path if applicable
  mlflowRunId: text('mlflow_run_id'),                  // MLflow run ID for tracking
  metadata:    jsonb('metadata'),                      // extra context
  createdAt:   timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Provider API Keys ────────────────────────────── */
export const apiKeys = pgTable('api_keys', {
  id:           text('id').primaryKey(),
  providerId:   text('provider_id').notNull(),
  keyHash:      text('key_hash').notNull(),           // SHA-256 hash for identification
  keyPrefix:    text('key_prefix'),                    // first 8 chars for display
  keySuffix:    text('key_suffix'),                    // last 4 chars for display
  encryptedKey: text('encrypted_key').notNull(),       // AES-256-GCM encrypted via OAUTH_ENCRYPTION_KEY
  status:       varchar('status', { length: 20 }).default('active'),
  // active | revoked | expired
  lastUsedAt:   timestamp('last_used_at', { withTimezone: true }),
  createdAt:    timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Synthesis Jobs ───────────────────────────────── */
export const synthesisJobs = pgTable('synthesis_jobs', {
  id:              text('id').primaryKey(),
  name:            text('name').notNull(),
  description:     text('description'),
  source:          varchar('source', { length: 20 }).notNull(),
  // ollama | llamacpp | huggingface
  model:           text('model').notNull(),
  status:          varchar('status', { length: 20 }).default('queued'),
  // queued | running | completed | failed | cancelled
  targetSamples:   integer('target_samples').default(100),
  currentSamples:  integer('current_samples').default(0),
  outputFormat:    varchar('output_format', { length: 20 }).default('jsonl'),
  temperature:     decimal('temperature', { precision: 3, scale: 2 }).default('0.8'),
  maxTokens:       integer('max_tokens').default(1024),
  batchSize:       integer('batch_size').default(5),
  outputPath:      text('output_path'),
  tokensUsed:      integer('tokens_used').default(0),
  progress:        integer('progress').default(0),
  error:           text('error'),
  tags:            text('tags').array(),
  startedAt:       timestamp('started_at', { withTimezone: true }),
  completedAt:     timestamp('completed_at', { withTimezone: true }),
  createdAt:       timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Assets (files, documents, images, etc.) ──────── */
export const assets = pgTable('assets', {
  id:           text('id').primaryKey(),
  name:         text('name').notNull(),
  description:  text('description'),
  folder:       text('folder').default('/'),             // virtual folder path
  mimeType:     text('mime_type'),
  fileType:     varchar('file_type', { length: 20 }),    // file | image | video | audio | document | dataset | other
  filePath:     text('file_path').notNull(),             // absolute path on disk
  fileName:     text('file_name').notNull(),             // original file name
  sizeBytes:    bigint('size_bytes', { mode: 'number' }).default(0),
  tags:         text('tags').array(),
  metadata:     jsonb('metadata'),                       // extra info: dimensions, duration, etc.
  datasetId:    text('dataset_id'),                      // optional link to datasets table
  createdAt:    timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:    timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Dataset Schema Definition ────────────────────── */
export const datasetSchemas = pgTable('dataset_schemas', {
  id:            text('id').primaryKey(),
  datasetId:     text('dataset_id'),                     // linked dataset (null if template)
  name:          text('name').notNull(),
  columns:       jsonb('columns').notNull(),              // [{name, type, description, required, defaultValue, constraints}]
  version:       integer('version').default(1),
  isTemplate:    boolean('is_template').default(false),
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:     timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Data Version History ─────────────────────────── */
export const dataVersions = pgTable('data_versions', {
  id:            text('id').primaryKey(),
  datasetId:     text('dataset_id').notNull(),
  versionNum:    integer('version_num').default(1),
  filePath:      text('file_path').notNull(),             // path to version snapshot
  sizeBytes:     bigint('size_bytes', { mode: 'number' }).default(0),
  rowsAffected:  integer('rows_affected').default(0),
  changeType:    varchar('change_type', { length: 20 }),  // edit | schema_update | restore
  changeSummary: text('change_summary'),
  createdBy:     text('created_by'),                      // user or system
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Data Lineage Catalog ─────────────────────────── */
export const lineage = pgTable('lineage', {
  id:              text('id').primaryKey(),
  sourceType:      varchar('source_type', { length: 30 }).notNull(),
  // dataset | expert | workflow | training_job | synthesis_job
  sourceId:        text('source_id').notNull(),
  targetType:      varchar('target_type', { length: 30 }).notNull(),
  // dataset | expert | workflow | training_job | synthesis_job | workflow_step
  targetId:        text('target_id').notNull(),
  relationship:    varchar('relationship', { length: 30 }).notNull(),
  // created_by | trained_on | used_in | produces | depends_on
  metadata:        jsonb('metadata'),                    // extra context
  createdAt:       timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── OAuth App Credentials (client ID/secret per platform) ── */
export const oauthCredentials = pgTable('oauth_credentials', {
  id:            text('id').primaryKey(),
  platform:      varchar('platform', { length: 30 }).notNull().unique(),
  // twitter | linkedin | facebook | youtube | tiktok | pinterest | reddit | medium | discord | tumblr | etc.
  clientId:      text('client_id').notNull(),
  clientSecret:  text('client_secret').notNull(),      // AES-256-GCM encrypted
  keyHash:       text('key_hash').notNull(),            // SHA-256 hash of client_id for dedup
  status:        varchar('status', { length: 20 }).default('active'),
  // active | revoked
  createdAt:     timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:     timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Social Platform Connections (OAuth tokens) ────── */
export const socialConnections = pgTable('social_connections', {
  id:              text('id').primaryKey(),
  platform:        varchar('platform', { length: 30 }).notNull(),
  // twitter | linkedin | facebook | instagram | youtube | tiktok | pinterest | reddit | threads | bluesky | medium | substack | devto | discord | telegram | whatsapp | snapchat | tumblr
  accessToken:     text('access_token').notNull(),        // AES-256-GCM encrypted
  refreshToken:    text('refresh_token'),                  // AES-256-GCM encrypted (null for non-refreshable)
  tokenExpiresAt:  timestamp('token_expires_at', { withTimezone: true }),
  scopes:          text('scopes').array(),
  platformUserId:  text('platform_user_id'),               // user's ID on the platform
  platformUsername:text('platform_username'),               // display name / handle
  platformAvatar:  text('platform_avatar'),                 // profile image URL
  platformMeta:    jsonb('platform_meta'),                  // extra profile data (followers, etc.)
  permissions:     jsonb('permissions'),                  // { consume: true, generate: true, publish: true, schedule: true, report: true, execute: true }
  status:          varchar('status', { length: 20 }).default('active'),
  // active | expired | revoked | error
  lastUsedAt:      timestamp('last_used_at', { withTimezone: true }),
  lastRefreshedAt: timestamp('last_refreshed_at', { withTimezone: true }),
  createdAt:       timestamp('created_at', { withTimezone: true }).defaultNow().notNull(),
  updatedAt:       timestamp('updated_at', { withTimezone: true }).defaultNow().notNull(),
});

/* ─── Type exports ───────────────────────────────────── */
export type Metric        = typeof metrics.$inferSelect;
export type Task          = typeof tasks.$inferSelect;
export type WorkflowRun   = typeof workflowRuns.$inferSelect;
export type Alert         = typeof alerts.$inferSelect;
export type Log           = typeof logs.$inferSelect;
export type Expert        = typeof experts.$inferSelect;
export type Workflow      = typeof workflows.$inferSelect;
export type TrainingJob   = typeof trainingJobs.$inferSelect;

export type Dataset       = typeof datasets.$inferSelect;
export type IntegrationRow = typeof integrations.$inferSelect;
export type IntegrationConnectionRow = typeof integrationConnections.$inferSelect;
export type PluginRow     = typeof plugins.$inferSelect;
export type Project       = typeof projects.$inferSelect;

export type NewTask       = typeof tasks.$inferInsert;
export type NewWorkflowRun = typeof workflowRuns.$inferInsert;
export type NewAlert      = typeof alerts.$inferInsert;
export type NewLog        = typeof logs.$inferInsert;
export type NewMetric     = typeof metrics.$inferInsert;
export type NewTrainingJob = typeof trainingJobs.$inferInsert;
export type NewDataset    = typeof datasets.$inferInsert;
export type HfDataset     = typeof hfDatasets.$inferSelect;
export type NewHfDataset  = typeof hfDatasets.$inferInsert;
export type NewProject    = typeof projects.$inferInsert;

export type WorkflowStep  = typeof workflowSteps.$inferSelect;
export type NewWorkflowStep = typeof workflowSteps.$inferInsert;

export type ApiKey        = typeof apiKeys.$inferSelect;
export type NewApiKey     = typeof apiKeys.$inferInsert;

export type SynthesisJob  = typeof synthesisJobs.$inferSelect;
export type NewSynthesisJob = typeof synthesisJobs.$inferInsert;

export type Asset         = typeof assets.$inferSelect;
export type NewAsset      = typeof assets.$inferInsert;

export type DatasetSchema = typeof datasetSchemas.$inferSelect;
export type NewDatasetSchema = typeof datasetSchemas.$inferInsert;
export type Lineage       = typeof lineage.$inferSelect;
export type NewLineage    = typeof lineage.$inferInsert;

export type DataVersion   = typeof dataVersions.$inferSelect;
export type NewDataVersion = typeof dataVersions.$inferInsert;

export type SocialConnection    = typeof socialConnections.$inferSelect;
export type NewSocialConnection = typeof socialConnections.$inferInsert;

export type OAuthCredential     = typeof oauthCredentials.$inferSelect;
export type NewOAuthCredential  = typeof oauthCredentials.$inferInsert;

export type ProjectAsset        = typeof projectAssets.$inferSelect;
export type NewProjectAsset     = typeof projectAssets.$inferInsert;
