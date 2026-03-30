import type {
  AIProvider, Expert, Workflow, WorkflowRun,
  Dataset, QueuedTask, Alert, ExpertRole, SocialPlatform,
  IntegrationCapability,
} from './types';

/* ─── AI Providers (configuration — not usage data) ──── */
export const PROVIDERS: AIProvider[] = [
  {
    id: 'anthropic',
    slug: 'anthropic',
    name: 'Anthropic',
    description: 'Claude models — advanced reasoning, coding, and analysis',
    icon: 'Bot',
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
    icon: 'Sparkles',
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
    icon: 'Gem',
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
    icon: 'Route',
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
    icon: 'Zap',
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
    icon: 'Wind',
    color: '#FF7000',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [],
  },
  {
    id: 'huggingface',
    slug: 'huggingface',
    name: 'Hugging Face',
    description: 'Open-source model hub — inference API, datasets, and 500k+ models',
    icon: 'Smile',
    color: '#FFD21E',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [
      {
        id: 'meta-llama/Llama-3.1-70B-Instruct',
        providerId: 'huggingface',
        name: 'Llama 3.1 70B Instruct',
        slug: 'meta-llama/Llama-3.1-70B-Instruct',
        contextWindow: 128000,
        costInputPer1k: 0.0,
        costOutputPer1k: 0.0,
        capabilities: ['reasoning', 'coding', 'writing', 'multilingual'],
        maxOutputTokens: 4096,
        supportsStreaming: true,
        supportsFunctionCalling: false,
      },
      {
        id: 'mistralai/Mixtral-8x7B-Instruct-v0.1',
        providerId: 'huggingface',
        name: 'Mixtral 8x7B Instruct',
        slug: 'mistralai/Mixtral-8x7B-Instruct-v0.1',
        contextWindow: 32768,
        costInputPer1k: 0.0,
        costOutputPer1k: 0.0,
        capabilities: ['reasoning', 'coding', 'fast', 'multilingual'],
        maxOutputTokens: 4096,
        supportsStreaming: true,
        supportsFunctionCalling: false,
      },
      {
        id: 'microsoft/Phi-3-medium-128k-instruct',
        providerId: 'huggingface',
        name: 'Phi-3 Medium 128k',
        slug: 'microsoft/Phi-3-medium-128k-instruct',
        contextWindow: 128000,
        costInputPer1k: 0.0,
        costOutputPer1k: 0.0,
        capabilities: ['reasoning', 'coding', 'fast', 'long-context'],
        maxOutputTokens: 4096,
        supportsStreaming: true,
        supportsFunctionCalling: false,
      },
    ],
  },
  {
    id: 'deepseek',
    slug: 'deepseek',
    name: 'DeepSeek',
    description: 'High-performance reasoning and coding models at low cost',
    icon: 'Compass',
    color: '#4D6BFE',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [
      {
        id: 'deepseek-chat',
        providerId: 'deepseek',
        name: 'DeepSeek V3',
        slug: 'deepseek-chat',
        contextWindow: 128000,
        costInputPer1k: 0.00014,
        costOutputPer1k: 0.00028,
        capabilities: ['reasoning', 'coding', 'analysis', 'writing', 'fast'],
        maxOutputTokens: 8192,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
      {
        id: 'deepseek-reasoner',
        providerId: 'deepseek',
        name: 'DeepSeek R1',
        slug: 'deepseek-reasoner',
        contextWindow: 128000,
        costInputPer1k: 0.00055,
        costOutputPer1k: 0.0022,
        capabilities: ['reasoning', 'math', 'coding', 'analysis'],
        maxOutputTokens: 16384,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
    ],
  },
  {
    id: 'xai',
    slug: 'xai',
    name: 'xAI',
    description: 'Grok models — real-time knowledge and advanced reasoning',
    icon: 'Atom',
    color: '#1DA1F2',
    connected: false,
    apiKeySet: false,
    status: 'unknown',
    models: [
      {
        id: 'grok-3',
        providerId: 'xai',
        name: 'Grok 3',
        slug: 'grok-3',
        contextWindow: 131072,
        costInputPer1k: 0.003,
        costOutputPer1k: 0.015,
        capabilities: ['reasoning', 'coding', 'analysis', 'writing', 'research'],
        maxOutputTokens: 8192,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
      {
        id: 'grok-3-mini',
        providerId: 'xai',
        name: 'Grok 3 Mini',
        slug: 'grok-3-mini',
        contextWindow: 131072,
        costInputPer1k: 0.0003,
        costOutputPer1k: 0.0005,
        capabilities: ['reasoning', 'coding', 'fast'],
        maxOutputTokens: 8192,
        supportsStreaming: true,
        supportsFunctionCalling: true,
      },
    ],
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

/* ─── Role Descriptions (for tooltips) ────────────────── */
export const ROLE_DESCRIPTIONS: Record<string, string> = {
  researcher:      'Investigates topics, gathers evidence, and synthesizes findings',
  analyst:         'Examines data patterns, trends, and provides structured insights',
  writer:          'Creates content — blog posts, documentation, marketing copy',
  coder:           'Generates, reviews, and refactors code across languages',
  reviewer:        'Evaluates quality, identifies issues, and suggests improvements',
  planner:         'Breaks down goals into actionable steps and timelines',
  synthesizer:     'Merges information from multiple sources into unified outputs',
  critic:          'Challenges assumptions and identifies weaknesses in reasoning',
  legal:           'Analyses contracts, compliance requirements, and regulations',
  financial:       'Models financials, forecasts, and evaluates business metrics',
  medical:         'Processes medical literature and health-related information',
  coordinator:     'Orchestrates tasks across teams and manages workflows',
  'data-engineer': 'Designs pipelines, schemas, and data transformations',
  creative:        'Generates creative concepts, designs, and artistic content',
  translator:      'Converts content between languages preserving meaning and tone',
  custom:          'A user-defined role with custom capabilities',
};

/* ─── Empty data arrays (populated from database) ────── */
export const EXPERTS: Expert[] = [];
export const WORKFLOWS: Workflow[] = [];
export const ACTIVE_TASKS: QueuedTask[] = [];
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
};

/* ─── Navigation Structure ───────────────────────────── */
export const NAV_SECTIONS = [
  {
    id: 'orchestration',
    label: 'ORCHESTRATION',
    color: '#D97706',
    items: [
      { id: 'experts',   label: 'Agents',       path: '/experts',  icon: 'Boxes' },
      { id: 'workflows', label: 'Workflows',   path: '/workflow', icon: 'Workflow' },
    ],
  },
  {
    id: 'intelligence',
    label: 'INTELLIGENCE',
    color: '#7C3AED',
    items: [
      { id: 'finetuning', label: 'Fine-tuning',    path: '/intelligence/finetuning', icon: 'Sliders' },
      { id: 'inference',  label: 'Inference',       path: '/intelligence/inference',  icon: 'Sparkles' },
      { id: 'models',     label: 'Models',          path: '/intelligence/models',     icon: 'Boxes' },
    ],
  },
  {
    id: 'artifacts',
    label: 'ARTIFACTS',
    color: '#0EA5E9',
    items: [
      { id: 'data',       label: 'Data Synthesis', path: '/data',            icon: 'Database' },
      { id: 'engineer',   label: 'Data Lab',       path: '/data/engineer',   icon: 'Zap' },
      { id: 'embeddings', label: 'Embeddings',     path: '/embeddings',      icon: 'Boxes' },
    ],
  },
  {
    id: 'monitoring',
    label: 'MONITORING',
    color: '#DC2626',
    items: [
      { id: 'runs',        label: 'Runs',         path: '/monitoring/runs',    icon: 'Zap' },
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
];

/* ─── Social Platforms (configuration) ─────────────────── */
export const PLATFORMS: SocialPlatform[] = [
  { id: 'twitter',   name: 'X (Twitter)',  color: '#1d9bf0', bgColor: 'rgba(29,155,240,0.08)',  connected: false },
  { id: 'linkedin',  name: 'LinkedIn',     color: '#0a66c2', bgColor: 'rgba(10,102,194,0.08)',  connected: false },
  { id: 'facebook',  name: 'Facebook',     color: '#1877F2', bgColor: 'rgba(24,119,242,0.08)',  connected: false },
  { id: 'instagram', name: 'Instagram',    color: '#e1306c', bgColor: 'rgba(225,48,108,0.08)',  connected: false },
  { id: 'youtube',   name: 'YouTube',      color: '#ff0000', bgColor: 'rgba(255,0,0,0.08)',     connected: false },
  { id: 'tiktok',    name: 'TikTok',       color: '#000000', bgColor: 'rgba(0,0,0,0.06)',       connected: false },
  { id: 'pinterest', name: 'Pinterest',    color: '#E60023', bgColor: 'rgba(230,0,35,0.08)',    connected: false },
  { id: 'reddit',    name: 'Reddit',       color: '#ff4500', bgColor: 'rgba(255,69,0,0.08)',    connected: false },
  { id: 'threads',   name: 'Threads',      color: '#000000', bgColor: 'rgba(0,0,0,0.06)',       connected: false },
  { id: 'bluesky',   name: 'Bluesky',      color: '#0085FF', bgColor: 'rgba(0,133,255,0.08)',   connected: false },
  { id: 'discord',   name: 'Discord',      color: '#5865f2', bgColor: 'rgba(88,101,242,0.08)',  connected: false },
  { id: 'telegram',  name: 'Telegram',     color: '#26a5e4', bgColor: 'rgba(38,165,228,0.08)',  connected: false },
  { id: 'whatsapp',  name: 'WhatsApp',     color: '#25d366', bgColor: 'rgba(37,211,102,0.08)',  connected: false },
  { id: 'medium',    name: 'Medium',       color: '#000000', bgColor: 'rgba(0,0,0,0.06)',       connected: false },
  { id: 'substack',  name: 'Substack',     color: '#FF6719', bgColor: 'rgba(255,103,25,0.08)',  connected: false },
  { id: 'devto',     name: 'Dev.to',       color: '#0A0A0A', bgColor: 'rgba(10,10,10,0.06)',    connected: false },
];

/* ─── Integration Catalog ───────────────────────────── */
export const INTEGRATION_CATALOG: Array<{
  id: string; name: string; description: string;
  category: 'api' | 'app' | 'tool' | 'database' | 'storage' | 'messaging' | 'analytics' | 'social' | 'crm' | 'data_analytics';
  icon: string; color: string;
  authType: 'api_key' | 'oauth2' | 'bearer' | 'basic' | 'none';
  capabilities: IntegrationCapability[];
  docsUrl: string;
}> = [
  /* ── Social Media ───────────────────────────────────── */
  { id: 'twitter',     name: 'X (Twitter)',       description: 'Post tweets, threads, polls — consume analytics, schedule, and monitor mentions',                    category: 'social', icon: 'Twitter',       color: '#1d9bf0', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report', 'execute'], docsUrl: 'https://developer.x.com/en/portal/dashboard' },
  { id: 'linkedin',    name: 'LinkedIn',          description: 'Publish articles, posts, and company updates — track engagement and follower analytics',             category: 'social', icon: 'Linkedin',      color: '#0a66c2', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://www.linkedin.com/developers/apps' },
  { id: 'facebook',    name: 'Facebook',          description: 'Manage pages, groups, and ads — publish content, analyze reach, and automate campaigns',             category: 'social', icon: 'Facebook',      color: '#1877F2', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report', 'execute'], docsUrl: 'https://developers.facebook.com/apps/' },
  { id: 'instagram',   name: 'Instagram',         description: 'Publish posts, stories, and reels — analyze engagement, hashtag performance, and audience insights', category: 'social', icon: 'Instagram',     color: '#E1306C', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://developers.facebook.com/docs/instagram-platform' },
  { id: 'youtube',     name: 'YouTube',           description: 'Upload videos, manage playlists, analyze watch time, subscribers, and channel performance',          category: 'social', icon: 'Youtube',       color: '#FF0000', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report', 'execute'], docsUrl: 'https://console.cloud.google.com/apis/credentials' },
  { id: 'tiktok',      name: 'TikTok',            description: 'Publish videos, analyze trends, track viral performance, and manage creator tools',                  category: 'social', icon: 'Video',         color: '#000000', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://developers.tiktok.com/apps/' },
  { id: 'pinterest',   name: 'Pinterest',         description: 'Create pins, manage boards, analyze pin performance, and drive visual discovery traffic',            category: 'social', icon: 'Image',         color: '#E60023', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://developers.pinterest.com/apps/' },
  { id: 'reddit',      name: 'Reddit',            description: 'Post to subreddits, monitor discussions, analyze sentiment, and engage with communities',            category: 'social', icon: 'MessageCircle', color: '#FF4500', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'report', 'execute'],             docsUrl: 'https://www.reddit.com/prefs/apps' },
  { id: 'threads',     name: 'Threads',           description: 'Publish text posts, engage in conversations, and track engagement metrics',                          category: 'social', icon: 'AtSign',        color: '#000000', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'report'],                        docsUrl: 'https://developers.facebook.com/docs/threads' },
  { id: 'bluesky',     name: 'Bluesky',           description: 'Post to decentralized feeds, manage lists, and analyze engagement on the AT Protocol',               category: 'social', icon: 'Cloud',         color: '#0085FF', authType: 'bearer',  capabilities: ['consume', 'generate', 'publish', 'report'],                        docsUrl: 'https://docs.bsky.app/' },
  { id: 'medium',      name: 'Medium',            description: 'Publish articles, manage publications, track reads, claps, and audience retention',                  category: 'social', icon: 'FileText',      color: '#000000', authType: 'bearer',  capabilities: ['consume', 'generate', 'publish', 'report'],                        docsUrl: 'https://medium.com/me/applications' },
  { id: 'substack',    name: 'Substack',          description: 'Publish newsletters, manage subscriber lists, track open rates and growth',                          category: 'social', icon: 'Mail',          color: '#FF6719', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],             docsUrl: 'https://substack.com/settings' },
  { id: 'devto',       name: 'Dev.to',            description: 'Publish tech articles, track reactions, manage series, and engage developer community',              category: 'social', icon: 'Code',          color: '#0A0A0A', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'report'],                        docsUrl: 'https://dev.to/settings/extensions' },
  { id: 'discord',     name: 'Discord',           description: 'Send messages, manage channels, bots, and community engagement analytics',                          category: 'social', icon: 'MessageSquare', color: '#5865F2', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'execute'],                       docsUrl: 'https://discord.com/developers/applications' },
  { id: 'telegram',    name: 'Telegram',          description: 'Send messages, manage channels and groups, automate bots, and broadcast updates',                   category: 'social', icon: 'Send',          color: '#26A5E4', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule', 'execute'],            docsUrl: 'https://core.telegram.org/bots#botfather' },
  { id: 'whatsapp',    name: 'WhatsApp Business', description: 'Send messages, manage templates, automate customer conversations, and broadcast updates',            category: 'social', icon: 'Phone',         color: '#25D366', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],             docsUrl: 'https://developers.facebook.com/docs/whatsapp/cloud-api/get-started' },
  { id: 'snapchat',    name: 'Snapchat',          description: 'Manage stories, ads, and audience insights — track snap performance and engagement',                 category: 'social', icon: 'Camera',        color: '#FFFC00', authType: 'oauth2',  capabilities: ['consume', 'publish', 'report'],                                    docsUrl: 'https://kit.snapchat.com/portal' },
  { id: 'tumblr',      name: 'Tumblr',            description: 'Publish posts, manage blogs, track notes and reblogs, and engage with communities',                 category: 'social', icon: 'BookOpen',      color: '#36465D', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://www.tumblr.com/oauth/apps' },

  /* ── CRM & Marketing ─────────────────────────────────── */
  { id: 'salesforce',     name: 'Salesforce',        description: 'Manage leads, contacts, opportunities, and sales pipelines — generate reports and forecasts',        category: 'crm', icon: 'Cloud',         color: '#00A1E0', authType: 'oauth2',  capabilities: ['consume', 'generate', 'execute', 'report'],                        docsUrl: 'https://developer.salesforce.com/docs/atlas.en-us.api_rest.meta/api_rest/intro_oauth_and_connected_apps.htm' },
  { id: 'hubspot',        name: 'HubSpot',           description: 'CRM, email marketing, deal tracking, and campaign analytics — full inbound automation',             category: 'crm', icon: 'Target',        color: '#FF7A59', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report', 'execute'], docsUrl: 'https://developers.hubspot.com/docs/api/creating-an-app' },
  { id: 'mailchimp',      name: 'Mailchimp',         description: 'Email campaigns, audience segmentation, automation, and engagement analytics',                      category: 'crm', icon: 'Mail',          color: '#FFE01B', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://mailchimp.com/developer/marketing/guides/access-user-data-oauth-2/' },
  { id: 'intercom',       name: 'Intercom',          description: 'Customer messaging, in-app chat, support tickets, and user engagement tracking',                   category: 'crm', icon: 'MessageSquare', color: '#286EFA', authType: 'bearer',  capabilities: ['consume', 'generate', 'publish', 'report', 'execute'],             docsUrl: 'https://developers.intercom.com/docs/build-an-integration/getting-started/' },
  { id: 'zendesk',        name: 'Zendesk',           description: 'Support tickets, knowledge base, customer satisfaction, and agent performance reports',             category: 'crm', icon: 'HelpCircle',    color: '#03363D', authType: 'api_key', capabilities: ['consume', 'generate', 'report', 'execute'],                        docsUrl: 'https://developer.zendesk.com/api-reference/' },
  { id: 'freshdesk',      name: 'Freshdesk',         description: 'Helpdesk ticketing, customer support automation, SLA tracking, and agent analytics',               category: 'crm', icon: 'Headphones',    color: '#2CA01C', authType: 'api_key', capabilities: ['consume', 'generate', 'report', 'execute'],                        docsUrl: 'https://developers.freshdesk.com/api/' },
  { id: 'pipedrive',      name: 'Pipedrive',         description: 'Sales CRM — manage deals, pipelines, activities, and sales performance reporting',                 category: 'crm', icon: 'TrendingUp',    color: '#017737', authType: 'api_key', capabilities: ['consume', 'generate', 'report', 'execute'],                        docsUrl: 'https://developers.pipedrive.com/docs/api/v1' },
  { id: 'activecampaign', name: 'ActiveCampaign',    description: 'Email marketing, automation workflows, CRM contacts, and campaign performance analytics',          category: 'crm', icon: 'Zap',           color: '#356AE6', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://developers.activecampaign.com/reference/overview' },
  { id: 'constantcontact', name: 'Constant Contact',  description: 'Email marketing, event management, social posting, and contact list management',                  category: 'crm', icon: 'Mail',          color: '#0076BE', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://developer.constantcontact.com/api_guide/getting_started.html' },
  { id: 'brevo',          name: 'Brevo (Sendinblue)', description: 'Email, SMS, and chat marketing — automation workflows and transactional messaging',                category: 'crm', icon: 'Send',          color: '#0B996E', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'],            docsUrl: 'https://developers.brevo.com/docs/getting-started' },

  /* ── Data & Analytics ─────────────────────────────────── */
  { id: 'google-analytics', name: 'Google Analytics',  description: 'Website traffic, user behavior, conversion tracking, and audience insights reporting',             category: 'data_analytics', icon: 'BarChart3',   color: '#E37400', authType: 'oauth2',  capabilities: ['consume', 'report'],                        docsUrl: 'https://developers.google.com/analytics/devguides/reporting/data/v1/quickstart-client-libraries' },
  { id: 'mixpanel',         name: 'Mixpanel',          description: 'Product analytics — funnels, retention, A/B testing, and user behavior tracking',                 category: 'data_analytics', icon: 'PieChart',    color: '#7856FF', authType: 'api_key', capabilities: ['consume', 'report', 'execute'],              docsUrl: 'https://developer.mixpanel.com/reference/overview' },
  { id: 'amplitude',        name: 'Amplitude',         description: 'Behavioral analytics — cohorts, journeys, experimentation, and product insights',                 category: 'data_analytics', icon: 'TrendingUp',  color: '#1F2140', authType: 'api_key', capabilities: ['consume', 'report'],                        docsUrl: 'https://www.docs.developers.amplitude.com/analytics/apis/' },
  { id: 'bigquery',         name: 'Google BigQuery',   description: 'Serverless data warehouse — run SQL queries on massive datasets and export results',               category: 'data_analytics', icon: 'Database',    color: '#4285F4', authType: 'oauth2',  capabilities: ['consume', 'execute', 'report'],             docsUrl: 'https://cloud.google.com/bigquery/docs/quickstarts/quickstart-client-libraries' },
  { id: 'snowflake',        name: 'Snowflake',         description: 'Cloud data platform — SQL queries, data sharing, and cross-cloud analytics',                      category: 'data_analytics', icon: 'Snowflake',   color: '#29B5E8', authType: 'basic',   capabilities: ['consume', 'execute', 'report'],             docsUrl: 'https://docs.snowflake.com/en/developer-guide/sql-api/authenticating' },
  { id: 'looker',           name: 'Looker',            description: 'BI platform — data exploration, dashboards, and LookML-powered analytics',                        category: 'data_analytics', icon: 'Eye',         color: '#4285F4', authType: 'oauth2',  capabilities: ['consume', 'report'],                        docsUrl: 'https://cloud.google.com/looker/docs/api-auth' },
  { id: 'tableau',          name: 'Tableau',           description: 'Visual analytics — interactive dashboards, data blending, and enterprise reporting',               category: 'data_analytics', icon: 'LayoutGrid',  color: '#E97627', authType: 'api_key', capabilities: ['consume', 'report'],                        docsUrl: 'https://help.tableau.com/current/api/rest_api/en-us/REST/rest_api_get_started.htm' },
  { id: 'plausible',        name: 'Plausible',         description: 'Privacy-friendly web analytics — lightweight, GDPR-compliant traffic and event tracking',         category: 'data_analytics', icon: 'BarChart3',   color: '#5850EC', authType: 'api_key', capabilities: ['consume', 'report'],                        docsUrl: 'https://plausible.io/docs/stats-api' },
  { id: 'posthog',          name: 'PostHog',           description: 'Product analytics, session recordings, feature flags, and A/B testing in one platform',           category: 'data_analytics', icon: 'Activity',    color: '#F9BD2B', authType: 'api_key', capabilities: ['consume', 'execute', 'report'],              docsUrl: 'https://posthog.com/docs/api' },
  { id: 'hotjar',           name: 'Hotjar',            description: 'Heatmaps, session recordings, feedback polls, and user behavior visualization',                   category: 'data_analytics', icon: 'Flame',       color: '#FF3C00', authType: 'api_key', capabilities: ['consume', 'report'],                        docsUrl: 'https://developer.hotjar.com/docs/getting-started' },

  /* ── Messaging ──────────────────────────────────────── */
  { id: 'slack',       name: 'Slack',             description: 'Send messages, read channels, manage workflows',                                                        category: 'messaging',  icon: 'MessageSquare', color: '#4A154B', authType: 'oauth2',  capabilities: ['consume', 'generate', 'publish', 'execute'],            docsUrl: 'https://api.slack.com/apps' },
  { id: 'twilio',      name: 'Twilio',            description: 'SMS, voice calls, and messaging APIs',                                                                  category: 'messaging',  icon: 'Phone',         color: '#F22F46', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule'],            docsUrl: 'https://www.twilio.com/console' },
  { id: 'sendgrid',    name: 'SendGrid',          description: 'Transactional and marketing email delivery',                                                            category: 'messaging',  icon: 'Mail',          color: '#1A82E2', authType: 'api_key', capabilities: ['consume', 'generate', 'publish', 'schedule', 'report'], docsUrl: 'https://app.sendgrid.com/settings/api_keys' },

  /* ── Tools & Apps ───────────────────────────────────── */
  { id: 'github',      name: 'GitHub',            description: 'Repos, issues, PRs, actions, and code search',                                                          category: 'tool',       icon: 'Github',        color: '#24292F', authType: 'bearer',  capabilities: ['consume', 'generate', 'execute', 'report'],             docsUrl: 'https://github.com/settings/tokens' },
  { id: 'jira',        name: 'Jira',              description: 'Issue tracking, sprint management, project boards',                                                     category: 'app',        icon: 'Ticket',        color: '#0052CC', authType: 'api_key', capabilities: ['consume', 'generate', 'execute', 'report'],             docsUrl: 'https://id.atlassian.com/manage-profile/security/api-tokens' },
  { id: 'notion',      name: 'Notion',            description: 'Pages, databases, and workspace content',                                                               category: 'app',        icon: 'BookOpen',      color: '#000000', authType: 'bearer',  capabilities: ['consume', 'generate', 'publish', 'execute'],            docsUrl: 'https://www.notion.so/my-integrations' },

  /* ── Databases ──────────────────────────────────────── */
  { id: 'postgres',       name: 'PostgreSQL',     description: 'Direct SQL queries against PostgreSQL databases',                                                        category: 'database',   icon: 'Database',      color: '#336791', authType: 'basic',   capabilities: ['consume', 'execute', 'report'],             docsUrl: 'https://www.postgresql.org/docs/current/auth-pg-hba-conf.html' },
  { id: 'redis',          name: 'Redis',          description: 'Key-value store, caching, pub/sub messaging',                                                           category: 'database',   icon: 'HardDrive',     color: '#DC382D', authType: 'basic',   capabilities: ['consume', 'execute'],                       docsUrl: 'https://redis.io/docs/latest/operate/oss_and_stack/management/security/acl/' },
  { id: 'elasticsearch',  name: 'Elasticsearch',  description: 'Full-text search and analytics engine',                                                                 category: 'database',   icon: 'Search',        color: '#FEC514', authType: 'basic',   capabilities: ['consume', 'execute', 'report'],             docsUrl: 'https://www.elastic.co/guide/en/elasticsearch/reference/current/setting-up-authentication.html' },

  /* ── Storage ────────────────────────────────────────── */
  { id: 's3',             name: 'AWS S3',         description: 'Object storage — upload, download, list buckets',                                                       category: 'storage',    icon: 'Cloud',         color: '#FF9900', authType: 'api_key', capabilities: ['consume', 'publish', 'execute'],             docsUrl: 'https://docs.aws.amazon.com/IAM/latest/UserGuide/id_credentials_access-keys.html' },
  { id: 'gcs',            name: 'Google Cloud Storage', description: 'Object storage on Google Cloud Platform',                                                         category: 'storage',    icon: 'Cloud',         color: '#4285F4', authType: 'oauth2',  capabilities: ['consume', 'publish', 'execute'],             docsUrl: 'https://console.cloud.google.com/apis/credentials' },

  /* ── Analytics (Ops) ────────────────────────────────── */
  { id: 'datadog',        name: 'Datadog',        description: 'Monitoring, APM, logging, and alerting',                                                                category: 'analytics',  icon: 'BarChart3',     color: '#632CA6', authType: 'api_key', capabilities: ['consume', 'report', 'execute'],              docsUrl: 'https://app.datadoghq.com/organization-settings/api-keys' },
  { id: 'segment',        name: 'Segment',        description: 'Customer data platform — collect, unify, activate',                                                     category: 'analytics',  icon: 'Activity',      color: '#52BD94', authType: 'api_key', capabilities: ['consume', 'report', 'execute'],              docsUrl: 'https://segment.com/docs/connections/find-writekey/' },

  /* ── API & Payments ─────────────────────────────────── */
  { id: 'stripe',         name: 'Stripe',         description: 'Payments, subscriptions, invoicing, and billing',                                                       category: 'api',        icon: 'CreditCard',    color: '#635BFF', authType: 'api_key', capabilities: ['consume', 'execute', 'report'],              docsUrl: 'https://dashboard.stripe.com/apikeys' },
  { id: 'webhook',        name: 'Custom Webhook', description: 'Send or receive HTTP webhooks to any endpoint',                                                         category: 'api',        icon: 'Webhook',       color: '#6B7280', authType: 'none',    capabilities: ['consume', 'publish', 'execute'],             docsUrl: 'https://en.wikipedia.org/wiki/Webhook' },
  { id: 'rest-api',       name: 'REST API',       description: 'Connect to any REST API with custom authentication',                                                    category: 'api',        icon: 'Globe',         color: '#2563EB', authType: 'api_key', capabilities: ['consume', 'publish', 'execute'],             docsUrl: 'https://restfulapi.net/introduction/' },
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

/* ── Cloud & Data Provider Plugins ──────────────────── */
export const CLOUD_PLUGINS: Array<{
  id: string; name: string; icon: string; color: string;
  description: string; services: string[]; category: string; status: 'available' | 'coming_soon';
}> = [
  { id: 'aws',         name: 'AWS',           icon: '☁️', color: '#FF9900', description: 'S3, SageMaker, Bedrock, Lambda',           services: ['S3', 'SageMaker', 'Bedrock', 'Lambda'],              category: 'cloud_data', status: 'coming_soon' },
  { id: 'gcp',         name: 'Google Cloud',   icon: '🔵', color: '#4285F4', description: 'BigQuery, Vertex AI, Cloud Storage',       services: ['BigQuery', 'Vertex AI', 'Cloud Storage', 'Functions'], category: 'cloud_data', status: 'coming_soon' },
  { id: 'azure',       name: 'Azure',          icon: '🔷', color: '#0078D4', description: 'Azure AI, Blob Storage, Functions',        services: ['Azure AI', 'Blob Storage', 'Azure Functions'],       category: 'cloud_data', status: 'coming_soon' },
  { id: 'snowflake',   name: 'Snowflake',      icon: '❄️', color: '#29B5E8', description: 'Data Warehouse, ML Functions, Cortex',     services: ['Data Warehouse', 'ML Functions', 'Cortex AI'],       category: 'cloud_data', status: 'coming_soon' },
  { id: 'databricks',  name: 'Databricks',     icon: '🧱', color: '#FF3621', description: 'Unity Catalog, MLflow, Spark',             services: ['Unity Catalog', 'MLflow', 'Spark'],                  category: 'cloud_data', status: 'coming_soon' },
  { id: 'confluent',   name: 'Confluent',      icon: '🔄', color: '#172B4D', description: 'Kafka Streaming, Schema Registry',         services: ['Kafka', 'Schema Registry', 'ksqlDB'],               category: 'cloud_data', status: 'coming_soon' },
  { id: 'mongodb',     name: 'MongoDB',        icon: '🍃', color: '#00684A', description: 'Atlas, Vector Search, Aggregation',        services: ['Atlas', 'Vector Search', 'Aggregation'],            category: 'cloud_data', status: 'coming_soon' },
  { id: 'pinecone',    name: 'Pinecone',       icon: '🌲', color: '#000000', description: 'Vector Database, Serverless Index',         services: ['Vector DB', 'Serverless', 'Namespaces'],            category: 'cloud_data', status: 'coming_soon' },
  { id: 'weaviate',    name: 'Weaviate',       icon: '🔮', color: '#35B8BE', description: 'Vector Database, Hybrid Search',            services: ['Vector DB', 'Hybrid Search', 'Generative'],         category: 'cloud_data', status: 'coming_soon' },
  { id: 'langchain',   name: 'LangChain',      icon: '🦜', color: '#1C3C3C', description: 'Agent Framework, Chains, Memory',          services: ['Agents', 'Chains', 'Memory', 'Tools'],              category: 'cloud_data', status: 'coming_soon' },
  { id: 'llamaindex',  name: 'LlamaIndex',     icon: '🦙', color: '#8B5CF6', description: 'RAG Framework, Data Connectors',           services: ['RAG', 'Data Connectors', 'Query Engine'],           category: 'cloud_data', status: 'coming_soon' },
  { id: 'huggingface', name: 'Hugging Face',   icon: '🤗', color: '#FFD21E', description: 'Model Hub, Inference API, Datasets',       services: ['Model Hub', 'Inference API', 'Datasets'],           category: 'cloud_data', status: 'coming_soon' },
];
