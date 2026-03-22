/* ─────────────────────────────────────────────────────
   SUNDAY — AI Expert Orchestration Platform
   Core Type Definitions
───────────────────────────────────────────────────── */

/* ── Providers ──────────────────────────────────────── */
export type ProviderSlug =
  | 'anthropic' | 'openai' | 'google' | 'openrouter'
  | 'mistral' | 'cohere' | 'together' | 'groq'
  | 'huggingface' | 'deepseek' | 'xai' | 'custom';

export interface AIProvider {
  id: string;
  slug: ProviderSlug;
  name: string;
  description: string;
  icon: string;               /* lucide icon name */
  logoUrl?: string;
  color: string;
  connected: boolean;
  apiKeySet: boolean;
  status: 'operational' | 'degraded' | 'outage' | 'unknown';
  latencyMs?: number;
  models: AIModel[];
  monthlyTokensUsed?: number;
  monthlyTokenLimit?: number;
}

export interface AIModel {
  id: string;
  providerId: string;
  name: string;
  slug: string;
  contextWindow: number;
  costInputPer1k: number;    /* USD */
  costOutputPer1k: number;   /* USD */
  capabilities: ModelCapability[];
  maxOutputTokens: number;
  supportsStreaming: boolean;
  supportsFunctionCalling: boolean;
}

export type ModelCapability =
  | 'reasoning' | 'coding' | 'analysis' | 'writing'
  | 'research' | 'math' | 'vision' | 'long-context'
  | 'fast' | 'multilingual' | 'structured-output';

/* ── Experts ─────────────────────────────────────────── */
export type ExpertRole =
  | 'researcher' | 'analyst' | 'writer' | 'coder'
  | 'reviewer' | 'planner' | 'synthesizer' | 'critic'
  | 'legal' | 'financial' | 'medical' | 'coordinator'
  | 'data-engineer' | 'creative' | 'translator' | 'custom';

export type ExpertStatus =
  | 'active' | 'idle' | 'training' | 'fine-tuning'
  | 'deploying' | 'offline' | 'error';

export interface Expert {
  id: string;
  name: string;
  description: string;
  role: ExpertRole;
  status: ExpertStatus;
  version: string;
  createdAt: string;
  updatedAt: string;

  /* Backing model */
  modelId: string;
  modelName: string;
  providerId: string;
  providerName: string;

  /* Model source — local inference or cloud provider */
  modelSource: ModelSource;
  localModelConfig?: LocalModelConfig;

  /* Configuration */
  systemPrompt: string;
  temperature: number;      /* 0–2 */
  maxTokens: number;
  topP?: number;

  /* Capabilities this expert is good at */
  capabilities: ModelCapability[];
  specializations: string[];  /* domain tags */

  /* Fine-tuning */
  isFinetuned: boolean;
  baseModelId?: string;
  trainingJobId?: string;
  fineTuneDatasetId?: string;

  /* Performance */
  stats: ExpertStats;

  /* Ownership */
  isPublic: boolean;
  ownerId?: string;
  tags: string[];

  /* Hosting */
  deploymentId?: string;
  endpointUrl?: string;
  replicaCount?: number;
}

export interface ExpertStats {
  totalRuns: number;
  successRate: number;      /* 0–1 */
  avgTokensPerRun: number;
  avgLatencyMs: number;
  avgCostPerRun: number;    /* USD */
  rating: number;           /* 1–5 */
  lastRunAt?: string;
}

/* ── Model Source ────────────────────────────────────── */
export type ModelSource = 'local' | 'provider';
export type LocalInferenceEngine = 'ollama' | 'llamacpp' | 'vllm';

export interface LocalModelConfig {
  engine: LocalInferenceEngine;
  modelName: string;         /* e.g. "llama3.1:8b", "mistral:7b" */
  baseUrl?: string;          /* override default engine URL */
}

/* ── Agent Memory & Orchestration ───────────────────── */
export interface AgentMemory {
  plan: string;              /* agent's execution plan */
  context: string;           /* injected context from goal + files */
  findings: string[];        /* accumulated findings during execution */
  sharedMemoryRef: string;   /* run ID for shared memory access */
}

export interface AgentContext {
  stepId: string;
  expertId?: string;
  taskDescription: string;
  inputFiles: string[];      /* URLs of attached input files */
  systemPrompt: string;
  memory: AgentMemory;
  modelSource: ModelSource;
  localModel?: LocalModelConfig;
}

export interface SharedMemoryEntry {
  agentId: string;
  stepId: string;
  key: string;
  value: string;
  timestamp: string;
}

export interface SharedMemory {
  runId: string;
  entries: Record<string, string>;  /* agentId → serialized memory */
  globals: Record<string, string>;  /* shared key-value store */
}

/* ── Workflow Execution Events (WebSocket) ──────────── */
export type WorkflowExecutionEventType =
  | 'agent.spawned'
  | 'agent.thinking'
  | 'agent.memory.update'
  | 'agent.step.complete'
  | 'agent.step.failed'
  | 'workflow.complete'
  | 'workflow.failed'
  | 'shared.memory.sync';

export interface WorkflowExecutionEvent {
  runId: string;
  event: WorkflowExecutionEventType;
  agentId?: string;
  stepId?: string;
  data: Record<string, unknown>;
  timestamp: string;
}

/* ── Workflows ───────────────────────────────────────── */
export type WorkflowStatus =
  | 'draft' | 'ready' | 'running' | 'paused'
  | 'completed' | 'failed' | 'cancelled';

export type StepStatus =
  | 'pending' | 'running' | 'completed' | 'failed' | 'skipped';

export type StepConnectionType = 'sequential' | 'parallel' | 'conditional';

export interface WorkflowStep {
  id: string;
  order: number;
  expertId?: string;
  expertName?: string;
  expertRole?: ExpertRole;
  label?: string;           /* custom step label */
  taskDescription: string;
  inputFrom?: string[];     /* step IDs to receive input from */
  outputTo?: string[];      /* step IDs to pass output to */
  connectionType: StepConnectionType;

  /* Model source */
  modelSource: ModelSource;
  localModel?: LocalModelConfig;

  /* Runtime */
  status: StepStatus;
  startedAt?: string;
  completedAt?: string;
  tokensUsed?: number;
  costUsd?: number;
  output?: string;
  error?: string;

  /* Config overrides for this step */
  temperature?: number;
  maxTokens?: number;
  systemPromptOverride?: string;
}

export interface Workflow {
  id: string;
  name: string;
  description: string;
  goalStatement: string;
  goalFileUrl?: string;          /* uploaded .md file */
  inputFileUrls?: string[];      /* attached context files */
  steps: WorkflowStep[];
  status: WorkflowStatus;
  createdAt: string;
  updatedAt: string;
  lastRunAt?: string;

  /* Estimation */
  estimatedTokens: number;
  estimatedCostUsd: number;
  estimatedDurationSec: number;

  /* Run history */
  totalRuns: number;
  successfulRuns: number;

  /* Tags */
  tags: string[];
  isTemplate: boolean;
  templateCategory?: string;
}

export interface WorkflowRun {
  id: string;
  workflowId: string;
  workflowName: string;
  status: WorkflowStatus;
  startedAt: string;
  completedAt?: string;
  steps: WorkflowStep[];
  totalTokensUsed: number;
  totalCostUsd: number;
  durationSec?: number;
  input: string;
  output?: string;
  error?: string;
  expertChain: string[];  /* expert names in order */
  sharedMemory?: SharedMemory;
}

/* ── Workflow Execution Request ─────────────────────── */
export interface WorkflowExecuteRequest {
  workflowId?: string;
  name: string;
  goalFileUrl: string;
  inputFileUrls: string[];
  steps: WorkflowStepConfig[];
}

export interface WorkflowStepConfig {
  stepId: string;
  expertId?: string;
  taskDescription: string;
  modelSource: ModelSource;
  localModel?: LocalModelConfig;
  temperature: number;
  maxTokens: number;
  connectionType: StepConnectionType;
}

/* ── Data Synthesis ──────────────────────────────────── */
export type DatasetStatus = 'draft' | 'generating' | 'ready' | 'failed' | 'archived';
export type DataFormat = 'jsonl' | 'csv' | 'parquet' | 'delta' | 'alpaca' | 'chatml' | 'sharegpt';
export type TrainingMethod = 'sft' | 'dpo' | 'rlhf' | 'orpo';

export interface Dataset {
  id: string;
  name: string;
  description: string;
  status: DatasetStatus;
  format: DataFormat;
  sampleCount: number;
  sizeBytes: number;
  createdAt: string;
  updatedAt: string;
  generatorExpertId?: string;
  sourceDocuments?: string[];
  qualityScore?: number;   /* 0–100 */
  tags: string[];
  categories: string[];
}

export interface SynthesisJob {
  id: string;
  datasetId: string;
  name: string;
  prompt: string;
  targetSamples: number;
  currentSamples: number;
  status: 'queued' | 'running' | 'completed' | 'failed';
  createdAt: string;
  completedAt?: string;
  tokensUsed?: number;
  costUsd?: number;
}

/* ── Monitoring ──────────────────────────────────────── */
export interface SystemMetrics {
  timestamp: string;
  activeAgents: number;
  tasksToday: number;
  tokensUsedToday: number;
  tokenBudget: number;
  successRate: number;
  avgLatencyMs: number;
  costToday: number;
  errorCount: number;
}

export interface ExpertMetric {
  expertId: string;
  expertName: string;
  runsLast24h: number;
  successRate: number;
  avgTokens: number;
  avgLatencyMs: number;
  costToday: number;
  errorCount: number;
}

export type AlertSeverity = 'info' | 'warning' | 'error' | 'critical';

export interface Alert {
  id: string;
  severity: AlertSeverity;
  title: string;
  message: string;
  expertId?: string;
  workflowId?: string;
  providerId?: string;
  createdAt: string;
  acknowledgedAt?: string;
  resolvedAt?: string;
}

export interface LogEntry {
  id: string;
  level: 'debug' | 'info' | 'warn' | 'error';
  message: string;
  source: string;          /* expert name / workflow / system */
  metadata?: Record<string, unknown>;
  timestamp: string;
}

/* ── Task Queue ──────────────────────────────────────── */
export type TaskStatus = 'queued' | 'running' | 'completed' | 'failed' | 'cancelled';
export type TaskPriority = 'low' | 'normal' | 'high' | 'critical';

export interface QueuedTask {
  id: string;
  name: string;
  workflowId?: string;
  workflowName?: string;
  status: TaskStatus;
  priority: TaskPriority;
  currentStep: number;
  totalSteps: number;
  currentExpert?: string;
  tokensUsed: number;
  estimatedTokens: number;
  startedAt?: string;
  estimatedCompletionAt?: string;
  createdAt: string;
  progress: number;  /* 0–100 */
}

/* ── User & Platform ─────────────────────────────────── */
export interface User {
  id: string;
  name: string;
  email: string;
  avatar?: string;
  role: 'admin' | 'developer' | 'viewer';
  createdAt: string;
  timezone: string;
  tokenBudgetMonthly: number;
  tokensUsedThisMonth: number;
}

export interface PlatformSettings {
  tokenBudgetMonthly: number;
  defaultProvider: ProviderSlug;
  defaultModel: string;
  maxConcurrentAgents: number;
  loggingLevel: 'debug' | 'info' | 'warn' | 'error';
  dataRetentionDays: number;
  webhookUrl?: string;
}

/* ── Integrations & Plugins ──────────────────────────── */
export type IntegrationCategory = 'api' | 'app' | 'tool' | 'database' | 'storage' | 'messaging' | 'analytics' | 'social' | 'crm' | 'data_analytics';

/* ── Integration Capabilities (agentic lifecycle) ──── */
export type IntegrationCapability = 'consume' | 'generate' | 'publish' | 'schedule' | 'report' | 'execute';

export interface Integration {
  id: string;
  name: string;
  description: string;
  category: IntegrationCategory;
  icon: string;              /* lucide icon name or URL */
  color: string;
  connected: boolean;
  authType: 'api_key' | 'oauth2' | 'bearer' | 'basic' | 'none';
  configFields: IntegrationConfigField[];
  baseUrl?: string;
  docsUrl?: string;
  createdAt: string;
  updatedAt: string;
}

export interface IntegrationConfigField {
  key: string;
  label: string;
  type: 'text' | 'password' | 'url' | 'number' | 'select';
  required: boolean;
  placeholder?: string;
  options?: string[];         /* for select type */
}

export interface IntegrationConnection {
  id: string;
  integrationId: string;
  name: string;               /* user-given label */
  config: Record<string, string>;
  status: 'active' | 'error' | 'expired';
  lastTestedAt?: string;
  createdAt: string;
}

export type PluginSource = 'personal' | 'marketplace';
export type PluginStatus = 'active' | 'disabled' | 'error' | 'installing';

export interface Plugin {
  id: string;
  name: string;
  description: string;
  version: string;
  author: string;
  source: PluginSource;
  status: PluginStatus;
  icon: string;
  color: string;
  category: string;
  capabilities: string[];
  configSchema?: IntegrationConfigField[];
  config?: Record<string, string>;
  installed: boolean;
  downloads?: number;
  rating?: number;
  createdAt: string;
  updatedAt: string;
}

export interface StepIntegration {
  id: string;
  type: 'integration' | 'plugin';
  referenceId: string;          /* integration or plugin ID */
  name: string;
  icon: string;
  color: string;
  config?: Record<string, string>;
}

/* ── MCP Servers ─────────────────────────────────────── */
export type McpServerStatus = 'idle' | 'running' | 'tested' | 'error';
export type McpServerSource = 'prebuilt' | 'generated' | 'persisted';
export type McpLanguage = 'python' | 'typescript' | 'javascript';

export interface McpServer {
  id: string;
  name: string;
  description: string;
  language: McpLanguage;
  filename: string;
  source: McpServerSource;
  code: string;
  status: McpServerStatus;
  test_output: string;
  created_at: string;
  prompt: string;
  is_public: boolean;
  generation_time_ms: number;
  cpu_percent: number;
}

/* ── Navigation ──────────────────────────────────────── */
export interface NavSection {
  id: string;
  label: string;
  color: string;
  items: NavItem[];
}

export interface NavItem {
  id: string;
  label: string;
  path: string;
  icon: string;
  badge?: number;
  badgeType?: 'count' | 'dot';
}

/* ── Social Platforms ───────────────────────────────── */
export interface SocialPlatform {
  id: string;
  name: string;
  color: string;
  bgColor: string;
  connected: boolean;
  username?: string;
  followers?: number;
  lastPost?: Date;
}

/* ── Voice ──────────────────────────────────────────── */
export type VoiceState = 'idle' | 'listening' | 'processing' | 'success' | 'error';

export interface VoiceCommand {
  id: string;
  transcript: string;
  intent: string;
  timestamp: Date;
  status: 'pending' | 'processed' | 'failed';
}

export interface ContentItem {
  id: string;
  content: string;
  platforms: string[];
  status: 'draft' | 'published' | 'scheduled';
  createdAt: Date;
}

/* ── Quorum Multi-Agent Orchestration ──────────────── */

export type QuorumRunStatus = 'queued' | 'running' | 'complete' | 'failed' | 'cancelled';
export type QuorumPhase = 'decompose' | 'execute' | 'recovery' | 'synthesize';
export type QuorumAgentRole = 'master' | 'worker';
export type QuorumAgentStatus = 'created' | 'thinking' | 'success' | 'failed' | 'recovered';
export type QuorumBackend = 'ollama' | 'llamacpp';

export interface QuorumSubmitRequest {
  project: string;
  task: string;
  model: string;
  backend: QuorumBackend;
  workers: number;
  prompt?: string;
  temperature?: number;
  max_tokens?: number;
  retries?: number;
}

export interface QuorumRunSummary {
  id: string;
  project: string;
  task: string;
  status: QuorumRunStatus;
  backend: string;
  model: string;
  workers: number;
  phase?: string;
  totalTokens?: number;
  totalDurationMs?: number;
  createdAt: string;
  startedAt?: string;
  finishedAt?: string;
  error?: string;
}

export interface QuorumRunResult {
  runId: string;
  totalTokens: number;
  totalDurationMs: number;
  decomposeMs: number;
  executeMs: number;
  synthesizeMs: number;
  finalOutput: string;
  workersSucceeded: number;
  workersFailed: number;
  workersRecovered: number;
}

export interface QuorumPhaseUpdate {
  runId: string;
  phase: QuorumPhase;
  status: 'started' | 'complete';
  detail?: string;
  wallClockMs?: number;
  sumIndividualMs?: number;
  speedup?: number;
  parallel?: boolean;
  subtasks?: string[];
}

export interface QuorumAgentEvent {
  runId: string;
  agentId: string;
  role?: QuorumAgentRole;
  phase?: QuorumPhase;
  subtask?: string;
  model?: string;
  reasoning?: string;
  tokensUsed?: number;
  durationMs?: number;
  contentPreview?: string;
  attempt?: number;
  status?: QuorumAgentStatus;
  error?: string;
  attempts?: number;
  originalAgent?: string;
}

export interface QuorumMetricsSnapshot {
  activeRuns: number;
  queuedRuns: number;
  maxConcurrent: number;
  cpuUsage: number;
  memoryUsageMb: number;
  tokensPerSec: number;
  totalRunsCompleted: number;
  totalTokensUsed: number;
  avgRunDurationMs: number;
}

export interface QuorumOperation {
  id: string;
  runId: string;
  agentId: string;
  phase: QuorumPhase;
  operation: string;
  promptPreview?: string;
  responsePreview?: string;
  tokensUsed: number;
  durationMs: number;
  status: string;
  error?: string;
  createdAt: string;
}

export interface QuorumProjectConfig {
  name: string;
  config: Record<string, unknown>;
  createdAt: string;
  updatedAt: string;
}

export type QuorumEventType =
  | 'quorum.run.queued'
  | 'quorum.run.started'
  | 'quorum.run.complete'
  | 'quorum.run.failed'
  | 'quorum.phase.update'
  | 'quorum.agent.created'
  | 'quorum.agent.thinking'
  | 'quorum.agent.output'
  | 'quorum.agent.failed'
  | 'quorum.agent.recovered'
  | 'quorum.metrics.snapshot'
  | 'quorum.error';
