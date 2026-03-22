'use client';

import { useState, useCallback } from 'react';
import {
  Settings, Save, RotateCcw, Shield, Cpu,
  Bell, Database, Zap, ChevronRight,
  CheckCircle2,
  ToggleRight, HardDrive, Activity,
} from 'lucide-react';
import { PROVIDERS } from '@/lib/constants';
import { TIMEZONES, formatTzLabel } from '@/lib/timezones';
import type { ProviderSlug } from '@/lib/types';

/* ── Settings Shape ──────────────────────────────────── */
interface WorkspaceSettings {
  // General
  workspaceName: string;
  timezone: string;
  dateFormat: '12h' | '24h';
  theme: 'system' | 'light' | 'dark';
  language: string;

  // Inference
  defaultProvider: ProviderSlug;
  defaultModel: string;
  localInference: {
    ollamaEnabled: boolean;
    ollamaUrl: string;
    llamacppEnabled: boolean;
    llamacppUrl: string;
    defaultEngine: 'ollama' | 'llamacpp';
    defaultModel: string;
  };

  // Agents & Execution
  maxConcurrentAgents: number;
  maxConcurrentRuns: number;
  defaultRetries: number;
  defaultTimeoutSec: number;
  failureStrategy: 'stop' | 'skip' | 'retry';
  autoSaveEnabled: boolean;
  autoSaveIntervalSec: number;

  // Tokens & Budget
  tokenBudgetMonthly: number;
  tokenBudgetDaily: number;
  costAlertThreshold: number;
  dataRetentionDays: number;

  // Logging
  loggingLevel: 'debug' | 'info' | 'warn' | 'error';
  logRetentionDays: number;
  auditTrailEnabled: boolean;
  metricsCollectionEnabled: boolean;
  metricsIntervalSec: number;

  // Notifications
  emailNotifications: boolean;
  emailRecipients: string;
  slackWebhookUrl: string;
  webhookUrl: string;
  notifyOnRunComplete: boolean;
  notifyOnRunFail: boolean;
  notifyOnBudgetAlert: boolean;

  // Security
  apiAccessEnabled: boolean;
  corsOrigins: string;
  rateLimitPerMin: number;
  sessionTimeoutMin: number;

  // Features
  features: {
    quorumEngine: boolean;
    expertMarketplace: boolean;
    mcpServers: boolean;
    executionArtifacts: boolean;
    scriptExecution: boolean;
    modelComparison: boolean;
    draftAutoSave: boolean;
    workflowScheduling: boolean;
  };
}

const STORAGE_KEY = 'kortecx_workspace_settings';

const DEFAULT_SETTINGS: WorkspaceSettings = {
  workspaceName: 'Kortecx Workspace',
  timezone: Intl.DateTimeFormat().resolvedOptions().timeZone,
  dateFormat: '24h',
  theme: 'system',
  language: 'en',

  defaultProvider: 'anthropic',
  defaultModel: 'claude-sonnet-4-6',
  localInference: {
    ollamaEnabled: true,
    ollamaUrl: 'http://localhost:11434',
    llamacppEnabled: false,
    llamacppUrl: 'http://localhost:8080',
    defaultEngine: 'ollama',
    defaultModel: 'llama3.2:3b',
  },

  maxConcurrentAgents: 10,
  maxConcurrentRuns: 4,
  defaultRetries: 3,
  defaultTimeoutSec: 300,
  failureStrategy: 'stop',
  autoSaveEnabled: true,
  autoSaveIntervalSec: 10,

  tokenBudgetMonthly: 50_000_000,
  tokenBudgetDaily: 5_000_000,
  costAlertThreshold: 50,
  dataRetentionDays: 90,

  loggingLevel: 'info',
  logRetentionDays: 30,
  auditTrailEnabled: true,
  metricsCollectionEnabled: true,
  metricsIntervalSec: 5,

  emailNotifications: false,
  emailRecipients: '',
  slackWebhookUrl: '',
  webhookUrl: '',
  notifyOnRunComplete: false,
  notifyOnRunFail: true,
  notifyOnBudgetAlert: true,

  apiAccessEnabled: true,
  corsOrigins: '*',
  rateLimitPerMin: 60,
  sessionTimeoutMin: 480,

  features: {
    quorumEngine: true,
    expertMarketplace: true,
    mcpServers: true,
    executionArtifacts: true,
    scriptExecution: true,
    modelComparison: true,
    draftAutoSave: true,
    workflowScheduling: true,
  },
};

/* ── Helpers ──────────────────────────────────────────── */
const LABEL: React.CSSProperties = {
  fontSize: 12, fontWeight: 500, color: 'var(--text-2)',
  display: 'block', marginBottom: 4,
};
const HINT: React.CSSProperties = {
  fontSize: 10, color: 'var(--text-4)', marginTop: 3, display: 'block',
};

function Toggle({ checked, onChange, label, description }: {
  checked: boolean; onChange: (v: boolean) => void; label: string; description?: string;
}) {
  return (
    <label style={{ display: 'flex', alignItems: 'flex-start', gap: 10, cursor: 'pointer', padding: '6px 0' }}>
      <button onClick={() => onChange(!checked)} style={{
        width: 34, height: 18, borderRadius: 9, border: 'none', cursor: 'pointer', flexShrink: 0,
        background: checked ? '#059669' : 'var(--border-md)', position: 'relative', transition: 'background 0.2s', marginTop: 1,
      }}>
        <div style={{
          position: 'absolute', top: 2, left: checked ? 17 : 2,
          width: 14, height: 14, borderRadius: '50%', background: '#fff',
          boxShadow: '0 1px 2px rgba(0,0,0,0.2)', transition: 'left 0.15s',
        }} />
      </button>
      <div>
        <div style={{ fontSize: 12, fontWeight: 550, color: 'var(--text-1)' }}>{label}</div>
        {description && <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1, lineHeight: 1.4 }}>{description}</div>}
      </div>
    </label>
  );
}

/* ── Category Panels ─────────────────────────────────── */
const CATEGORIES = [
  { id: 'general',       label: 'General',        icon: Settings,  color: '#6b7280' },
  { id: 'inference',     label: 'Inference',       icon: Cpu,       color: '#7C3AED' },
  { id: 'execution',     label: 'Agents & Runs',   icon: Zap,       color: '#D97706' },
  { id: 'budget',        label: 'Tokens & Budget',  icon: Database,  color: '#2563EB' },
  { id: 'logging',       label: 'Logging & Metrics',icon: Activity,  color: '#059669' },
  { id: 'notifications', label: 'Notifications',   icon: Bell,      color: '#EC4899' },
  { id: 'security',      label: 'Security & API',   icon: Shield,    color: '#ef4444' },
  { id: 'features',      label: 'Feature Flags',   icon: ToggleRight,color: '#0EA5E9' },
];

/* ── Main Page ───────────────────────────────────────── */
export default function SettingsPage() {
  const [category, setCategory] = useState('general');
  const [settings, setSettings] = useState<WorkspaceSettings>(() => {
    if (typeof window === 'undefined') return DEFAULT_SETTINGS;
    try {
      const raw = localStorage.getItem(STORAGE_KEY);
      if (raw) {
        const stored = JSON.parse(raw);
        return {
          ...DEFAULT_SETTINGS,
          ...stored,
          localInference: { ...DEFAULT_SETTINGS.localInference, ...stored.localInference },
          features: { ...DEFAULT_SETTINGS.features, ...stored.features },
        };
      }
    } catch { /* ignore */ }
    return DEFAULT_SETTINGS;
  });
  const [saved, setSaved] = useState(false);
  const [dirty, setDirty] = useState(false);

  const update = useCallback(<K extends keyof WorkspaceSettings>(key: K, value: WorkspaceSettings[K]) => {
    setSettings(prev => ({ ...prev, [key]: value }));
    setDirty(true);
  }, []);

  const updateLocal = useCallback(<K extends keyof WorkspaceSettings['localInference']>(key: K, value: WorkspaceSettings['localInference'][K]) => {
    setSettings(prev => ({ ...prev, localInference: { ...prev.localInference, [key]: value } }));
    setDirty(true);
  }, []);

  const updateFeature = useCallback(<K extends keyof WorkspaceSettings['features']>(key: K, value: boolean) => {
    setSettings(prev => ({ ...prev, features: { ...prev.features, [key]: value } }));
    setDirty(true);
  }, []);

  const handleSave = () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(settings));
    setSaved(true);
    setDirty(false);
    setTimeout(() => setSaved(false), 2000);
  };

  const handleReset = () => {
    setSettings(DEFAULT_SETTINGS);
    setDirty(true);
  };

  const selectedProvider = PROVIDERS.find(p => p.slug === settings.defaultProvider);
  const cat = CATEGORIES.find(c => c.id === category)!;

  return (
    <div style={{ padding: 20, maxWidth: '100%', display: 'flex', flexDirection: 'column', height: 'calc(100vh - 48px)' }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}>
        <div>
          <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
            <Settings size={18} color="var(--text-3)" /> Workspace Settings
          </h1>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
            Configure platform, inference, security, and feature settings
          </p>
        </div>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          {dirty && <span style={{ fontSize: 10, color: 'var(--warning)', fontWeight: 600 }}>Unsaved changes</span>}
          <button className="btn btn-secondary btn-sm" onClick={handleReset}>
            <RotateCcw size={12} /> Reset
          </button>
          <button className="btn btn-primary btn-sm" onClick={handleSave}>
            {saved ? <><CheckCircle2 size={12} /> Saved!</> : <><Save size={12} /> Save</>}
          </button>
        </div>
      </div>

      {/* Content: Left panel + Right content */}
      <div style={{ display: 'flex', gap: 16, flex: 1, minHeight: 0 }}>
        {/* Left panel — category navigation */}
        <div style={{ width: 200, flexShrink: 0, display: 'flex', flexDirection: 'column', gap: 2 }}>
          {CATEGORIES.map(c => {
            const active = category === c.id;
            const Icon = c.icon;
            return (
              <button key={c.id} onClick={() => setCategory(c.id)} style={{
                display: 'flex', alignItems: 'center', gap: 8,
                padding: '8px 12px', borderRadius: 6, border: 'none',
                background: active ? `${c.color}10` : 'transparent',
                color: active ? c.color : 'var(--text-2)',
                fontSize: 12, fontWeight: active ? 600 : 450,
                cursor: 'pointer', textAlign: 'left', width: '100%',
                transition: 'all 0.12s',
                borderLeft: active ? `2px solid ${c.color}` : '2px solid transparent',
              }}>
                <Icon size={14} />
                <span style={{ flex: 1 }}>{c.label}</span>
                {active && <ChevronRight size={12} />}
              </button>
            );
          })}
        </div>

        {/* Right content — scrollable */}
        <div style={{ flex: 1, overflowY: 'auto', minHeight: 0 }}>
          <div className="card" style={{ padding: 24, minHeight: 400 }}>
            <h2 style={{ fontSize: 15, fontWeight: 700, color: cat.color, margin: '0 0 20px', display: 'flex', alignItems: 'center', gap: 8 }}>
              <cat.icon size={16} /> {cat.label}
            </h2>

            {/* ── General ───────────────────────────── */}
            {category === 'general' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <div>
                  <label style={LABEL}>Workspace Name</label>
                  <input className="input" value={settings.workspaceName} onChange={e => update('workspaceName', e.target.value)} />
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
                  <div>
                    <label style={LABEL}>Timezone</label>
                    <select className="input" value={settings.timezone} onChange={e => update('timezone', e.target.value)}>
                      {TIMEZONES.map(tz => (
                        <option key={tz} value={tz}>{formatTzLabel(tz)}</option>
                      ))}
                    </select>
                    <span style={HINT}>Used for log timestamps and scheduling</span>
                  </div>
                  <div>
                    <label style={LABEL}>Time Format</label>
                    <select className="input" value={settings.dateFormat} onChange={e => update('dateFormat', e.target.value as '12h' | '24h')}>
                      <option value="24h">24-hour (14:30)</option>
                      <option value="12h">12-hour (2:30 PM)</option>
                    </select>
                  </div>
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
                  <div>
                    <label style={LABEL}>Theme</label>
                    <select className="input" value={settings.theme} onChange={e => update('theme', e.target.value as 'system' | 'light' | 'dark')}>
                      <option value="system">System</option>
                      <option value="light">Light</option>
                      <option value="dark">Dark</option>
                    </select>
                  </div>
                  <div>
                    <label style={LABEL}>Language</label>
                    <select className="input" value={settings.language} onChange={e => update('language', e.target.value)}>
                      <option value="en">English</option>
                      <option value="es">Espa&#241;ol</option>
                      <option value="fr">Fran&#231;ais</option>
                      <option value="de">Deutsch</option>
                      <option value="ja">&#26085;&#26412;&#35486;</option>
                      <option value="zh">&#20013;&#25991;</option>
                    </select>
                  </div>
                </div>
              </div>
            )}

            {/* ── Inference ──────────────────────────── */}
            {category === 'inference' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
                  <div>
                    <label style={LABEL}>Default Cloud Provider</label>
                    <select className="input" value={settings.defaultProvider} onChange={e => update('defaultProvider', e.target.value as ProviderSlug)}>
                      {PROVIDERS.map(p => <option key={p.id} value={p.slug}>{p.name}</option>)}
                    </select>
                  </div>
                  <div>
                    <label style={LABEL}>Default Cloud Model</label>
                    <select className="input" value={settings.defaultModel} onChange={e => update('defaultModel', e.target.value)}>
                      {(selectedProvider?.models ?? []).map(m => <option key={m.id} value={m.id}>{m.name}</option>)}
                    </select>
                  </div>
                </div>

                <div style={{ borderTop: '1px solid var(--border)', paddingTop: 16 }}>
                  <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', marginBottom: 12, display: 'flex', alignItems: 'center', gap: 6 }}>
                    <HardDrive size={14} /> Local Inference Backends
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
                    <div style={{ padding: 14, background: 'var(--bg)', border: `1px solid ${settings.localInference.ollamaEnabled ? '#05966940' : 'var(--border)'}`, borderRadius: 6 }}>
                      <Toggle checked={settings.localInference.ollamaEnabled} onChange={v => updateLocal('ollamaEnabled', v)}
                        label="Ollama" description="Local LLM server on port 11434" />
                      {settings.localInference.ollamaEnabled && (
                        <div style={{ marginTop: 8 }}>
                          <label style={LABEL}>URL</label>
                          <input className="input" style={{ fontSize: 11 }} value={settings.localInference.ollamaUrl} onChange={e => updateLocal('ollamaUrl', e.target.value)} />
                        </div>
                      )}
                    </div>
                    <div style={{ padding: 14, background: 'var(--bg)', border: `1px solid ${settings.localInference.llamacppEnabled ? '#05966940' : 'var(--border)'}`, borderRadius: 6 }}>
                      <Toggle checked={settings.localInference.llamacppEnabled} onChange={v => updateLocal('llamacppEnabled', v)}
                        label="llama.cpp" description="llama.cpp server on port 8080" />
                      {settings.localInference.llamacppEnabled && (
                        <div style={{ marginTop: 8 }}>
                          <label style={LABEL}>URL</label>
                          <input className="input" style={{ fontSize: 11 }} value={settings.localInference.llamacppUrl} onChange={e => updateLocal('llamacppUrl', e.target.value)} />
                        </div>
                      )}
                    </div>
                  </div>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14, marginTop: 12 }}>
                    <div>
                      <label style={LABEL}>Default Local Engine</label>
                      <select className="input" value={settings.localInference.defaultEngine} onChange={e => updateLocal('defaultEngine', e.target.value as 'ollama' | 'llamacpp')}>
                        <option value="ollama">Ollama</option>
                        <option value="llamacpp">llama.cpp</option>
                      </select>
                    </div>
                    <div>
                      <label style={LABEL}>Default Local Model</label>
                      <input className="input" placeholder="llama3.2:3b" value={settings.localInference.defaultModel} onChange={e => updateLocal('defaultModel', e.target.value)} />
                    </div>
                  </div>
                </div>
              </div>
            )}

            {/* ── Agents & Execution ─────────────────── */}
            {category === 'execution' && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
                <div>
                  <label style={LABEL}>Max Concurrent Agents</label>
                  <input className="input" type="number" min={1} max={50} value={settings.maxConcurrentAgents} onChange={e => update('maxConcurrentAgents', parseInt(e.target.value) || 1)} />
                  <span style={HINT}>Maximum agents running simultaneously</span>
                </div>
                <div>
                  <label style={LABEL}>Max Concurrent Runs</label>
                  <input className="input" type="number" min={1} max={20} value={settings.maxConcurrentRuns} onChange={e => update('maxConcurrentRuns', parseInt(e.target.value) || 1)} />
                  <span style={HINT}>Quorum scheduler capacity</span>
                </div>
                <div>
                  <label style={LABEL}>Default Retries</label>
                  <input className="input" type="number" min={0} max={10} value={settings.defaultRetries} onChange={e => update('defaultRetries', parseInt(e.target.value) || 0)} />
                </div>
                <div>
                  <label style={LABEL}>Default Timeout (seconds)</label>
                  <input className="input" type="number" min={30} value={settings.defaultTimeoutSec} onChange={e => update('defaultTimeoutSec', parseInt(e.target.value) || 300)} />
                </div>
                <div>
                  <label style={LABEL}>Failure Strategy</label>
                  <select className="input" value={settings.failureStrategy} onChange={e => update('failureStrategy', e.target.value as WorkspaceSettings['failureStrategy'])}>
                    <option value="stop">Stop on failure</option>
                    <option value="skip">Skip failed steps</option>
                    <option value="retry">Retry failed steps</option>
                  </select>
                </div>
                <div>
                  <label style={LABEL}>Auto-Save Interval (seconds)</label>
                  <input className="input" type="number" min={5} max={120} value={settings.autoSaveIntervalSec}
                    onChange={e => update('autoSaveIntervalSec', parseInt(e.target.value) || 10)}
                    disabled={!settings.autoSaveEnabled} />
                </div>
                <div style={{ gridColumn: '1 / -1' }}>
                  <Toggle checked={settings.autoSaveEnabled} onChange={v => update('autoSaveEnabled', v)}
                    label="Auto-Save Drafts" description="Automatically save workflow drafts to localStorage every N seconds" />
                </div>
              </div>
            )}

            {/* ── Tokens & Budget ────────────────────── */}
            {category === 'budget' && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
                <div>
                  <label style={LABEL}>Monthly Token Budget</label>
                  <input className="input" type="number" value={settings.tokenBudgetMonthly} onChange={e => update('tokenBudgetMonthly', parseInt(e.target.value) || 0)} />
                  <span style={HINT}>{(settings.tokenBudgetMonthly / 1_000_000).toFixed(0)}M tokens</span>
                </div>
                <div>
                  <label style={LABEL}>Daily Token Budget</label>
                  <input className="input" type="number" value={settings.tokenBudgetDaily} onChange={e => update('tokenBudgetDaily', parseInt(e.target.value) || 0)} />
                  <span style={HINT}>{(settings.tokenBudgetDaily / 1_000_000).toFixed(1)}M tokens</span>
                </div>
                <div>
                  <label style={LABEL}>Cost Alert Threshold ($)</label>
                  <input className="input" type="number" min={0} value={settings.costAlertThreshold} onChange={e => update('costAlertThreshold', parseFloat(e.target.value) || 0)} />
                  <span style={HINT}>Alert when daily spend exceeds this amount</span>
                </div>
                <div>
                  <label style={LABEL}>Data Retention (days)</label>
                  <input className="input" type="number" min={7} value={settings.dataRetentionDays} onChange={e => update('dataRetentionDays', parseInt(e.target.value) || 30)} />
                  <span style={HINT}>How long to keep logs, metrics, and audit data</span>
                </div>
              </div>
            )}

            {/* ── Logging & Metrics ──────────────────── */}
            {category === 'logging' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
                  <div>
                    <label style={LABEL}>Log Level</label>
                    <select className="input" value={settings.loggingLevel} onChange={e => update('loggingLevel', e.target.value as WorkspaceSettings['loggingLevel'])}>
                      <option value="debug">Debug — everything</option>
                      <option value="info">Info — standard</option>
                      <option value="warn">Warning — issues only</option>
                      <option value="error">Error — failures only</option>
                    </select>
                  </div>
                  <div>
                    <label style={LABEL}>Log Retention (days)</label>
                    <input className="input" type="number" min={1} value={settings.logRetentionDays} onChange={e => update('logRetentionDays', parseInt(e.target.value) || 7)} />
                  </div>
                </div>
                <Toggle checked={settings.auditTrailEnabled} onChange={v => update('auditTrailEnabled', v)}
                  label="Audit Trail" description="Record every agent operation (prompts, responses, tokens, timing) for compliance and debugging" />
                <Toggle checked={settings.metricsCollectionEnabled} onChange={v => update('metricsCollectionEnabled', v)}
                  label="Metrics Collection" description="Periodically capture CPU, memory, active agents, and token throughput" />
                {settings.metricsCollectionEnabled && (
                  <div style={{ maxWidth: 200 }}>
                    <label style={LABEL}>Metrics Interval (seconds)</label>
                    <input className="input" type="number" min={1} max={60} value={settings.metricsIntervalSec} onChange={e => update('metricsIntervalSec', parseInt(e.target.value) || 5)} />
                  </div>
                )}
              </div>
            )}

            {/* ── Notifications ──────────────────────── */}
            {category === 'notifications' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <Toggle checked={settings.notifyOnRunComplete} onChange={v => update('notifyOnRunComplete', v)} label="Notify on Run Completion" />
                <Toggle checked={settings.notifyOnRunFail} onChange={v => update('notifyOnRunFail', v)} label="Notify on Run Failure" />
                <Toggle checked={settings.notifyOnBudgetAlert} onChange={v => update('notifyOnBudgetAlert', v)} label="Notify on Budget Alert" />
                <div style={{ borderTop: '1px solid var(--border)', paddingTop: 14 }}>
                  <Toggle checked={settings.emailNotifications} onChange={v => update('emailNotifications', v)} label="Email Notifications" description="Send email alerts for enabled events" />
                  {settings.emailNotifications && (
                    <div style={{ marginTop: 8 }}>
                      <label style={LABEL}>Recipients (comma-separated)</label>
                      <input className="input" placeholder="team@company.com" value={settings.emailRecipients} onChange={e => update('emailRecipients', e.target.value)} />
                    </div>
                  )}
                </div>
                <div>
                  <label style={LABEL}>Slack Webhook URL</label>
                  <input className="input" type="url" placeholder="https://hooks.slack.com/services/..." value={settings.slackWebhookUrl} onChange={e => update('slackWebhookUrl', e.target.value)} />
                </div>
                <div>
                  <label style={LABEL}>Generic Webhook URL</label>
                  <input className="input" type="url" placeholder="https://your-domain.com/webhook" value={settings.webhookUrl} onChange={e => update('webhookUrl', e.target.value)} />
                  <span style={HINT}>POST requests sent for all enabled notification events</span>
                </div>
              </div>
            )}

            {/* ── Security & API ──────────────────────── */}
            {category === 'security' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <Toggle checked={settings.apiAccessEnabled} onChange={v => update('apiAccessEnabled', v)}
                  label="API Access" description="Enable REST API access for external integrations and triggers" />
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 14 }}>
                  <div>
                    <label style={LABEL}>CORS Origins</label>
                    <input className="input" placeholder="* or https://your-app.com" value={settings.corsOrigins} onChange={e => update('corsOrigins', e.target.value)} />
                    <span style={HINT}>Comma-separated allowed origins (* = all)</span>
                  </div>
                  <div>
                    <label style={LABEL}>Rate Limit (req/min)</label>
                    <input className="input" type="number" min={1} value={settings.rateLimitPerMin} onChange={e => update('rateLimitPerMin', parseInt(e.target.value) || 60)} />
                  </div>
                </div>
                <div style={{ maxWidth: 300 }}>
                  <label style={LABEL}>Session Timeout (minutes)</label>
                  <input className="input" type="number" min={5} value={settings.sessionTimeoutMin} onChange={e => update('sessionTimeoutMin', parseInt(e.target.value) || 480)} />
                  <span style={HINT}>{Math.round(settings.sessionTimeoutMin / 60)} hours</span>
                </div>
              </div>
            )}

            {/* ── Feature Flags ───────────────────────── */}
            {category === 'features' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                <p style={{ fontSize: 11, color: 'var(--text-3)', margin: '0 0 8px', lineHeight: 1.5 }}>
                  Enable or disable platform features. Disabled features are hidden from the UI and their background services are paused.
                </p>
                {([
                  { key: 'quorumEngine' as const, label: 'Quorum Engine', desc: 'Multi-agent orchestration with parallel execution and backpressure' },
                  { key: 'expertMarketplace' as const, label: 'Expert Marketplace', desc: 'Prebuilt expert templates with per-file versioning' },
                  { key: 'mcpServers' as const, label: 'MCP Servers', desc: 'Model Context Protocol server generation and management' },
                  { key: 'executionArtifacts' as const, label: 'Execution Artifacts', desc: 'Persist prompts, responses, and scripts to disk per step' },
                  { key: 'scriptExecution' as const, label: 'Script Execution', desc: 'Auto-extract and run code blocks from LLM responses' },
                  { key: 'modelComparison' as const, label: 'Model Comparison', desc: 'Re-run steps with different models to compare performance' },
                  { key: 'draftAutoSave' as const, label: 'Draft Auto-Save', desc: 'Automatically save workflow drafts to browser storage' },
                  { key: 'workflowScheduling' as const, label: 'Workflow Scheduling', desc: 'Cron-based and one-time workflow scheduling with triggers' },
                ]).map(f => (
                  <Toggle key={f.key} checked={settings.features[f.key]} onChange={v => updateFeature(f.key, v)} label={f.label} description={f.desc} />
                ))}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
