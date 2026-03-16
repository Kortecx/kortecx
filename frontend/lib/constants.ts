import type {
  AIProvider, Expert, Workflow, WorkflowRun,
  TrainingJob, Dataset, QueuedTask, Alert, ExpertRole, SocialPlatform
} from './types';

/* ─── AI Providers (configuration — not usage data) ──── */
export const PROVIDERS: AIProvider[] = [
  {
    id: 'anthropic',
    slug: 'anthropic',
    name: 'Anthropic',
    description: 'Claude models — advanced reasoning, coding, and analysis',
    color: '#D97757',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [
      {
        id: 'claude-opus-4-6',
        providerId: 'anthropic',
        name: 'Claude Opus 4.6',
        slug: 'claude-opus-4-6',
        contextWindow: 200000,
        costInputPer1k: 0.015,
        costOutputPer1k: 0.075,
        capabilities: ['reasoning', 'coding', 'analysis', 'writing', 'long-context'],
        maxOutputTokens: 8192,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
      {
        id: 'claude-sonnet-4-6',
        providerId: 'anthropic',
        name: 'Claude Sonnet 4.6',
        slug: 'claude-sonnet-4-6',
        contextWindow: 200000,
        costInputPer1k: 0.003,
        costOutputPer1k: 0.015,
        capabilities: ['reasoning', 'coding', 'analysis', 'writing', 'fast'],
        maxOutputTokens: 8192,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
      {
        id: 'claude-haiku-4-5',
        providerId: 'anthropic',
        name: 'Claude Haiku 4.5',
        slug: 'claude-haiku-4-5-20251001',
        contextWindow: 200000,
        costInputPer1k: 0.0008,
        costOutputPer1k: 0.004,
        capabilities: ['fast', 'writing', 'analysis'],
        maxOutputTokens: 4096,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
    ],
  },
  {
    id: 'openai',
    slug: 'openai',
    name: 'OpenAI',
    description: 'GPT-4 and o-series models with broad capability coverage',
    color: '#74AA9C',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [
      {
        id: 'gpt-4o',
        providerId: 'openai',
        name: 'GPT-4o',
        slug: 'gpt-4o',
        contextWindow: 128000,
        costInputPer1k: 0.005,
        costOutputPer1k: 0.015,
        capabilities: ['reasoning', 'coding', 'vision', 'fast', 'structured-output'],
        maxOutputTokens: 16384,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
      {
        id: 'o3-mini',
        providerId: 'openai',
        name: 'o3-mini',
        slug: 'o3-mini',
        contextWindow: 200000,
        costInputPer1k: 0.0011,
        costOutputPer1k: 0.0044,
        capabilities: ['reasoning', 'math', 'coding', 'fast'],
        maxOutputTokens: 65536,
        supportsStreaming: false,
        supportsFunctionCalling: true,
      },
    ],
  },
  {
    id: 'google',
    slug: 'google',
    name: 'Google',
    description: 'Gemini models — multimodal with long context support',
    color: '#4285F4',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [
      {
        id: 'gemini-2-pro',
        providerId: 'google',
        name: 'Gemini 2.0 Pro',
        slug: 'gemini-2.0-pro',
        contextWindow: 2000000,
        costInputPer1k: 0.00125,
        costOutputPer1k: 0.005,
        capabilities: ['reasoning', 'long-context', 'vision', 'multilingual'],
        maxOutputTokens: 8192,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
    ],
  },
  {
    id: 'openrouter',
    slug: 'openrouter',
    name: 'OpenRouter',
    description: 'Unified gateway to 200+ models from all major providers',
    color: '#6C6FD1',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [],
  },
  {
    id: 'groq',
    slug: 'groq',
    name: 'Groq',
    description: 'Ultra-fast inference with LPU hardware acceleration',
    color: '#F55036',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [],
  },
  {
    id: 'mistral',
    slug: 'mistral',
    name: 'Mistral AI',
    description: 'European frontier models — efficient and privacy-focused',
    color: '#FF7000',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [],
  },
];

/* ─── Expert Role Meta (UI metadata) ───────────────────── */
export const ROLE_META: Record<ExpertRole, { label: string; emoji: string; color: string; dimColor: string }> = {
  researcher:      { label: 'Researcher',    emoji: '🔍', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  analyst:         { label: 'Analyst',       emoji: '📊', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  writer:          { label: 'Writer',        emoji: '✍️', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  coder:           { label: 'Coder',         emoji: '💻', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  reviewer:        { label: 'Reviewer',      emoji: '✅', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  planner:         { label: 'Planner',       emoji: '🗺️', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  synthesizer:     { label: 'Synthesizer',   emoji: '⚗️', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  critic:          { label: 'Critic',        emoji: '🎯', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  legal:           { label: 'Legal',         emoji: '⚖️', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  financial:       { label: 'Financial',     emoji: '💰', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  medical:         { label: 'Medical',       emoji: '🏥', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  coordinator:     { label: 'Coordinator',   emoji: '🎯', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  'data-engineer': { label: 'Data Engineer', emoji: '🔧', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  creative:        { label: 'Creative',      emoji: '🎨', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
  translator:      { label: 'Translator',    emoji: '🌐', color: '#0d0d0d', dimColor: 'rgba(13,13,13,0.07)' },
  custom:          { label: 'Custom',        emoji: '⚙️', color: '#1a1a1a', dimColor: 'rgba(13,13,13,0.07)' },
};

/* ─── Empty data arrays (populated from database) ────── */
export const EXPERTS: Expert[] = [];
export const WORKFLOWS: Workflow[] = [];
export const ACTIVE_TASKS: QueuedTask[] = [];
export const TRAINING_JOBS: TrainingJob[] = [];
export const DATASETS: Dataset[] = [];
export const ALERTS: Alert[] = [];
export const RECENT_RUNS: WorkflowRun[] = [];

/* ─── System Metrics (zeroed — populated from database) ── */
export const SYSTEM_METRICS = {
  activeAgents: 0,
  tasksToday: 0,
  tokensUsedToday: 0,
  tokenBudgetDaily: 5_000_000,
  successRate: 0,
  avgLatencyMs: 0,
  costToday: 0,
  errorCount: 0,
  activeExperts: 0,
  idleExperts: 0,
  trainingExperts: 0,
};

/* ─── Navigation Structure ───────────────────────────── */
export const NAV_SECTIONS = [
  {
    id: 'ops',
    label: 'OPS',
    color: '#F04500',
    items: [
      { id: 'dashboard', label: 'Dashboard',    path: '/dashboard',       icon: 'LayoutDashboard' },
      { id: 'tasks',     label: 'Task Queue',   path: '/tasks',           icon: 'ListOrdered' },
      { id: 'agents',    label: 'Active Agents',path: '/agents',          icon: 'Cpu' },
    ],
  },
  {
    id: 'experts',
    label: 'EXPERTS',
    color: '#D97706',
    items: [
      { id: 'experts',    label: 'Experts',         path: '/experts',                    icon: 'Users' },
      { id: 'marketplace',label: 'Marketplace',     path: '/experts?tab=marketplace',    icon: 'Store' },
      { id: 'deploy',     label: 'Deploy New',      path: '/experts/deploy',             icon: 'Rocket' },
    ],
  },
  {
    id: 'workflow',
    label: 'WORKFLOW',
    color: '#2563EB',
    items: [
      { id: 'builder',   label: 'Builder',     path: '/workflow/builder',   icon: 'LayoutTemplate' },
      { id: 'workflows', label: 'Workflows',   path: '/workflow',           icon: 'Workflow' },
      { id: 'history',   label: 'Run History', path: '/workflow/history',   icon: 'History' },
    ],
  },
  {
    id: 'intelligence',
    label: 'INTELLIGENCE',
    color: '#7C3AED',
    items: [
      { id: 'training', label: 'Training Lab',  path: '/training',          icon: 'Brain' },
      { id: 'data',     label: 'Data Synthesis',path: '/data',              icon: 'Database' },
      { id: 'finetune', label: 'Fine-tuning',   path: '/training/finetune', icon: 'Sliders' },
    ],
  },
  {
    id: 'monitoring',
    label: 'MONITORING',
    color: '#DC2626',
    items: [
      { id: 'performance', label: 'Performance', path: '/monitoring',         icon: 'Activity' },
      { id: 'logs',        label: 'Logs',         path: '/monitoring/logs',   icon: 'ScrollText' },
      { id: 'alerts',      label: 'Alerts',        path: '/monitoring/alerts', icon: 'Bell' },
    ],
  },
  {
    id: 'providers',
    label: 'PROVIDERS',
    color: '#059669',
    items: [
      { id: 'providers',   label: 'Providers',   path: '/providers',             icon: 'Plug' },
      { id: 'connections', label: 'Connections',  path: '/providers/connections', icon: 'Cable' },
    ],
  },
  {
    id: 'platform',
    label: 'PLATFORM',
    color: '#6B7280',
    items: [
      { id: 'settings', label: 'Settings', path: '/settings', icon: 'Settings' },
    ],
  },
];

/* ─── Social Platforms (configuration) ─────────────────── */
export const PLATFORMS: SocialPlatform[] = [
  { id: 'twitter',  name: 'X (Twitter)', color: '#1d9bf0', bgColor: 'rgba(29,155,240,0.08)',  connected: false },
  { id: 'linkedin', name: 'LinkedIn',    color: '#0a66c2', bgColor: 'rgba(10,102,194,0.08)',  connected: false },
  { id: 'reddit',   name: 'Reddit',      color: '#ff4500', bgColor: 'rgba(255,69,0,0.08)',    connected: false },
  { id: 'discord',  name: 'Discord',     color: '#5865f2', bgColor: 'rgba(88,101,242,0.08)', connected: false },
  { id: 'telegram', name: 'Telegram',    color: '#26a5e4', bgColor: 'rgba(38,165,228,0.08)', connected: false },
  { id: 'whatsapp', name: 'WhatsApp',    color: '#25d366', bgColor: 'rgba(37,211,102,0.08)', connected: false },
  { id: 'youtube',  name: 'YouTube',     color: '#ff0000', bgColor: 'rgba(255,0,0,0.08)',    connected: false },
  { id: 'instagram',name: 'Instagram',   color: '#e1306c', bgColor: 'rgba(225,48,108,0.08)', connected: false },
];

/* ─── Integration Catalog ───────────────────────────── */
export const INTEGRATION_CATALOG: Array<{
  id: string; name: string; description: string;
  category: 'api' | 'app' | 'tool' | 'database' | 'storage' | 'messaging' | 'analytics';
  icon: string; color: string;
  authType: 'api_key' | 'oauth2' | 'bearer' | 'basic' | 'none';
}> = [
  { id: 'slack',       name: 'Slack',         description: 'Send messages, read channels, manage workflows',          category: 'messaging',  icon: 'MessageSquare', color: '#4A154B', authType: 'oauth2' },
  { id: 'github',      name: 'GitHub',        description: 'Repos, issues, PRs, actions, and code search',           category: 'tool',       icon: 'Github',        color: '#24292F', authType: 'bearer' },
  { id: 'jira',        name: 'Jira',          description: 'Issue tracking, sprint management, project boards',      category: 'app',        icon: 'Ticket',        color: '#0052CC', authType: 'api_key' },
  { id: 'notion',      name: 'Notion',        description: 'Pages, databases, and workspace content',                category: 'app',        icon: 'BookOpen',      color: '#000000', authType: 'bearer' },
  { id: 'postgres',    name: 'PostgreSQL',    description: 'Direct SQL queries against PostgreSQL databases',         category: 'database',   icon: 'Database',      color: '#336791', authType: 'basic' },
  { id: 'redis',       name: 'Redis',         description: 'Key-value store, caching, pub/sub messaging',            category: 'database',   icon: 'HardDrive',     color: '#DC382D', authType: 'basic' },
  { id: 's3',          name: 'AWS S3',        description: 'Object storage — upload, download, list buckets',        category: 'storage',    icon: 'Cloud',         color: '#FF9900', authType: 'api_key' },
  { id: 'gcs',         name: 'Google Cloud Storage', description: 'Object storage on Google Cloud Platform',         category: 'storage',    icon: 'Cloud',         color: '#4285F4', authType: 'oauth2' },
  { id: 'stripe',      name: 'Stripe',        description: 'Payments, subscriptions, invoicing, and billing',        category: 'api',        icon: 'CreditCard',    color: '#635BFF', authType: 'api_key' },
  { id: 'twilio',      name: 'Twilio',        description: 'SMS, voice calls, and messaging APIs',                   category: 'messaging',  icon: 'Phone',         color: '#F22F46', authType: 'api_key' },
  { id: 'sendgrid',    name: 'SendGrid',      description: 'Transactional and marketing email delivery',             category: 'messaging',  icon: 'Mail',          color: '#1A82E2', authType: 'api_key' },
  { id: 'elasticsearch', name: 'Elasticsearch', description: 'Full-text search and analytics engine',                category: 'database',   icon: 'Search',        color: '#FEC514', authType: 'basic' },
  { id: 'datadog',     name: 'Datadog',       description: 'Monitoring, APM, logging, and alerting',                 category: 'analytics',  icon: 'BarChart3',     color: '#632CA6', authType: 'api_key' },
  { id: 'segment',     name: 'Segment',       description: 'Customer data platform — collect, unify, activate',      category: 'analytics',  icon: 'Activity',      color: '#52BD94', authType: 'api_key' },
  { id: 'webhook',     name: 'Custom Webhook', description: 'Send or receive HTTP webhooks to any endpoint',         category: 'api',        icon: 'Webhook',       color: '#6B7280', authType: 'none' },
  { id: 'rest-api',    name: 'REST API',      description: 'Connect to any REST API with custom authentication',     category: 'api',        icon: 'Globe',         color: '#2563EB', authType: 'api_key' },
];

/* ─── Marketplace Plugins ───────────────────────────── */
export const MARKETPLACE_PLUGINS: Array<{
  id: string; name: string; description: string; version: string;
  author: string; category: string; icon: string; color: string;
  capabilities: string[]; downloads: number; rating: number;
}> = [
  { id: 'mp-web-scraper',    name: 'Web Scraper',         description: 'Extract structured data from any webpage with CSS selectors or AI parsing', version: '2.1.0', author: 'Kortecx Labs',  category: 'data',         icon: 'Globe',        color: '#059669', capabilities: ['scrape', 'extract', 'parse'],        downloads: 12400, rating: 4.7 },
  { id: 'mp-pdf-parser',     name: 'PDF Parser',          description: 'Extract text, tables, and images from PDF documents',                       version: '1.4.2', author: 'Kortecx Labs',  category: 'data',         icon: 'FileText',     color: '#DC2626', capabilities: ['parse', 'extract', 'ocr'],           downloads: 8900,  rating: 4.5 },
  { id: 'mp-code-executor',  name: 'Code Executor',       description: 'Safely execute Python, JavaScript, or shell scripts in sandboxed containers', version: '1.2.0', author: 'Kortecx Labs',  category: 'tool',         icon: 'Terminal',     color: '#7C3AED', capabilities: ['execute', 'sandbox', 'multi-lang'],  downloads: 15200, rating: 4.8 },
  { id: 'mp-image-gen',      name: 'Image Generator',     description: 'Generate images using DALL-E, Stable Diffusion, or Midjourney APIs',        version: '1.0.5', author: 'Community',     category: 'creative',     icon: 'Image',        color: '#EC4899', capabilities: ['generate', 'edit', 'upscale'],       downloads: 6700,  rating: 4.3 },
  { id: 'mp-translator',     name: 'Multi-Translator',    description: 'Translate text between 100+ languages with context-aware quality',           version: '1.1.0', author: 'Community',     category: 'language',     icon: 'Languages',    color: '#0EA5E9', capabilities: ['translate', 'detect', 'glossary'],   downloads: 4300,  rating: 4.6 },
  { id: 'mp-chart-builder',  name: 'Chart Builder',       description: 'Generate interactive charts and visualizations from data',                   version: '1.3.1', author: 'Kortecx Labs',  category: 'analytics',    icon: 'BarChart3',    color: '#F59E0B', capabilities: ['chart', 'visualize', 'export'],     downloads: 5100,  rating: 4.4 },
  { id: 'mp-email-composer', name: 'Email Composer',      description: 'Draft, format, and schedule professional emails with templates',             version: '1.0.2', author: 'Community',     category: 'communication', icon: 'Mail',      color: '#2563EB', capabilities: ['compose', 'template', 'schedule'],  downloads: 3200,  rating: 4.2 },
  { id: 'mp-vector-search',  name: 'Vector Search',       description: 'Semantic search across documents, embeddings, and knowledge bases',          version: '2.0.0', author: 'Kortecx Labs',  category: 'data',         icon: 'Search',       color: '#8B5CF6', capabilities: ['search', 'embed', 'rag'],           downloads: 9800,  rating: 4.9 },
];

/* ─── Voice Command Suggestions ──────────────────────── */
export const COMMAND_SUGGESTIONS: string[] = [
  'Write a LinkedIn post about our latest AI research',
  'Create a Twitter thread summarizing our workflow',
  'Draft an Instagram caption for our product launch',
  'Generate a Reddit post for the r/MachineLearning community',
  'Write a Discord announcement for our community',
  'Create a Telegram message about today\'s updates',
];
