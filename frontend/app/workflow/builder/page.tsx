'use client';

import { useState, useCallback, useRef, useEffect, Suspense } from 'react';
import { useSearchParams, useRouter } from 'next/navigation';
import {
  Play, Sparkles, Plus, Trash2, ArrowRight, X, Search,
  ChevronDown, ChevronUp, Zap, Clock, Layers,
  Save, FileText, ChevronRight, RotateCcw,
  Upload, File, Server, Cloud, CheckCircle2, AlertCircle,
  Brain, Loader2, Eye, Settings, Tag, Shield, BarChart3,
  Activity, ScrollText, FlaskConical, Hash,
  Mic, Image, FolderOpen, MessageSquare, ToggleLeft, ToggleRight,
  Paperclip, Users, Puzzle, Cable, Store, Globe, Database,
  HardDrive, CreditCard, Phone, Mail, Ticket, BookOpen,
  Terminal, Languages, Webhook, Download, Star,
  Github, Search as SearchIcon,
} from 'lucide-react';
import { ROLE_META, INTEGRATION_CATALOG, MARKETPLACE_PLUGINS } from '@/lib/constants';
import type { Expert, ExpertRole, ModelSource, LocalModelConfig, StepIntegration } from '@/lib/types';
import { useWorkflowWS } from '@/lib/hooks/useWorkflowWS';
import { useWorkflowLogger } from '@/lib/hooks/useWorkflowLogger';
import { useExperts, useWorkflows } from '@/lib/hooks/useApi';

/* ── Types ───────────────────────────────────────────── */
interface DraftStep {
  id: string;
  expert: Expert | null;
  taskDescription: string;
  systemInstructions: string;
  maxTokens?: number;
  temperature?: number;
  collapsed: boolean;
  modelSource: ModelSource;
  localModel: LocalModelConfig;
  connectionType: 'sequential' | 'parallel';
  stepFiles: UploadedFile[];
  stepImages: UploadedFile[];
  voiceCommand: string;
  fileLocations: string[];
  integrations: StepIntegration[];
}

interface UploadedFile {
  file: File;
  name: string;
  size: number;
  type: string;
  preview?: string;
}

interface MetricsConfig {
  mlflow: boolean;
  mlflowTrackingUri: string;
  mlflowExperiment: string;
  logging: boolean;
  logLevel: 'debug' | 'info' | 'warn' | 'error';
  logFormat: 'structured' | 'plaintext';
  monitoring: boolean;
  monitoringInterval: number;
  alertOnFailure: boolean;
  alertOnLatency: boolean;
  latencyThresholdMs: number;
}

interface AdvancedConfig {
  maxRetries: number;
  timeoutSec: number;
  failureStrategy: 'stop' | 'skip' | 'retry';
  priority: 'low' | 'normal' | 'high' | 'critical';
  concurrencyLimit: number;
  cacheResults: boolean;
  cacheTtlSec: number;
  notifyOnComplete: boolean;
  notifyChannel: string;
  description: string;
}

interface PermissionsConfig {
  visibility: 'private' | 'team' | 'public';
  allowClone: boolean;
  allowEdit: 'owner' | 'team' | 'anyone';
  requireApproval: boolean;
  maxRunsPerDay: number;
  tokenBudget: number;
}

/* ── Helpers ─────────────────────────────────────────── */
let _id = 0;
function uid() { return `step-${++_id}`; }

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function formatBytes(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

const SUGGESTED_CHAINS: Record<string, string[]> = {
  'research': ['researcher', 'analyst', 'writer', 'reviewer'],
  'code': ['planner', 'coder', 'reviewer'],
  'legal': ['researcher', 'legal', 'reviewer'],
  'data': ['researcher', 'analyst', 'synthesizer'],
  'content': ['researcher', 'writer', 'reviewer'],
};

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* ── Shared Styles ───────────────────────────────────── */
const LABEL: React.CSSProperties = {
  fontSize: 10, fontWeight: 600, color: 'var(--text-3)',
  textTransform: 'uppercase', letterSpacing: '0.08em',
  display: 'block', marginBottom: 4,
};

const SECTION_TITLE: React.CSSProperties = {
  fontSize: 11, fontWeight: 700, color: 'var(--text-3)',
  textTransform: 'uppercase', letterSpacing: '0.1em',
  display: 'block', marginBottom: 8,
};

/* ── Checkbox ────────────────────────────────────────── */
function Checkbox({
  checked,
  onChange,
  label,
  description,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  description?: string;
}) {
  return (
    <label style={{
      display: 'flex', alignItems: 'flex-start', gap: 8,
      cursor: 'pointer', padding: '4px 0',
    }}>
      <input
        type="checkbox"
        checked={checked}
        onChange={e => onChange(e.target.checked)}
        style={{ marginTop: 2, accentColor: 'var(--primary)' }}
      />
      <div>
        <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-1)' }}>{label}</div>
        {description && (
          <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 1 }}>{description}</div>
        )}
      </div>
    </label>
  );
}

/* ── Radio ───────────────────────────────────────────── */
function RadioGroup({
  value,
  onChange,
  options,
  label,
}: {
  value: string;
  onChange: (v: string) => void;
  options: { value: string; label: string }[];
  label: string;
}) {
  return (
    <div>
      <div style={LABEL}>{label}</div>
      <div style={{ display: 'flex', gap: 12 }}>
        {options.map(opt => (
          <label key={opt.value} style={{
            display: 'flex', alignItems: 'center', gap: 5,
            cursor: 'pointer', fontSize: 12, color: 'var(--text-2)',
          }}>
            <input
              type="radio"
              name={label}
              value={opt.value}
              checked={value === opt.value}
              onChange={() => onChange(opt.value)}
              style={{ accentColor: 'var(--primary)' }}
            />
            {opt.label}
          </label>
        ))}
      </div>
    </div>
  );
}

/* ── Expert Selector Modal ───────────────────────────── */
function ExpertSelectorModal({
  onSelect,
  onClose,
  allExperts,
}: {
  onSelect: (expert: Expert) => void;
  onClose: () => void;
  allExperts: Expert[];
}) {
  const [search, setSearch] = useState('');
  const [roleFilter, setRoleFilter] = useState<ExpertRole | 'all'>('all');
  const [sourceFilter, setSourceFilter] = useState<'all' | 'local' | 'provider'>('all');

  const filtered = allExperts.filter(e => {
    if (e.status !== 'active' && e.status !== 'idle' && e.status !== 'deploying') return false;
    if (search && !e.name.toLowerCase().includes(search.toLowerCase()) &&
      !(ROLE_META[e.role as ExpertRole]?.label || '').toLowerCase().includes(search.toLowerCase())) return false;
    if (roleFilter !== 'all' && e.role !== roleFilter) return false;
    if (sourceFilter !== 'all' && (e.modelSource || 'provider') !== sourceFilter) return false;
    return true;
  });

  const roles: ExpertRole[] = [...new Set(allExperts.map(e => e.role as ExpertRole))];

  return (
    <div style={{
      position: 'fixed', inset: 0,
      background: 'rgba(7,7,26,0.85)',
      zIndex: 200,
      display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
      paddingTop: 80,
    }}>
      <div className="animate-in" style={{
        width: 560, maxHeight: 'calc(100vh - 140px)',
        background: 'var(--bg-card)', border: '1px solid var(--border-md)',
        borderRadius: 8, display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        <div style={{
          padding: '16px 18px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>Select Expert</div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>Choose a specialist for this workflow step</div>
          </div>
          <button className="btn btn-ghost btn-icon" onClick={onClose} style={{ color: 'var(--text-3)' }}>
            <X size={16} />
          </button>
        </div>
        <div style={{ padding: '12px 18px', borderBottom: '1px solid var(--border)' }}>
          <div style={{
            display: 'flex', alignItems: 'center', gap: 8,
            background: 'var(--bg)', border: '1px solid var(--border-md)',
            borderRadius: 4, padding: '7px 10px', marginBottom: 10,
          }}>
            <Search size={13} color="var(--text-3)" />
            <input autoFocus className="input" style={{ background: 'none', border: 'none', padding: 0 }}
              placeholder="Search by name or role..." value={search} onChange={e => setSearch(e.target.value)} />
          </div>
          <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap' }}>
            <button onClick={() => setRoleFilter('all')} style={{
              padding: '3px 10px', borderRadius: 20, fontSize: 11, fontWeight: 500,
              border: '1px solid', cursor: 'pointer',
              background: roleFilter === 'all' ? 'var(--primary-dim)' : 'transparent',
              borderColor: roleFilter === 'all' ? 'var(--primary)' : 'var(--border)',
              color: roleFilter === 'all' ? 'var(--primary-text)' : 'var(--text-3)',
            }}>All</button>
            {roles.map(r => {
              const m = ROLE_META[r];
              const active = roleFilter === r;
              return (
                <button key={r} onClick={() => setRoleFilter(active ? 'all' : r)} style={{
                  padding: '3px 10px', borderRadius: 20, fontSize: 11, fontWeight: 500,
                  border: '1px solid', cursor: 'pointer',
                  background: active ? m.dimColor : 'transparent',
                  borderColor: active ? m.color : 'var(--border)',
                  color: active ? m.color : 'var(--text-3)', transition: 'all 0.1s',
                }}>{m.emoji} {m.label}</button>
              );
            })}
          </div>
          {/* Source filter */}
          <div style={{ display: 'flex', gap: 5, marginTop: 8 }}>
            {([
              { key: 'all' as const, label: 'All Sources' },
              { key: 'local' as const, label: 'Local' },
              { key: 'provider' as const, label: 'Provider' },
            ]).map(s => (
              <button key={s.key} onClick={() => setSourceFilter(s.key)} style={{
                padding: '3px 10px', borderRadius: 20, fontSize: 11, fontWeight: 500,
                border: '1px solid', cursor: 'pointer',
                background: sourceFilter === s.key ? 'var(--primary-dim)' : 'transparent',
                borderColor: sourceFilter === s.key ? 'var(--primary)' : 'var(--border)',
                color: sourceFilter === s.key ? 'var(--primary-text)' : 'var(--text-3)',
              }}>{s.label}</button>
            ))}
          </div>
        </div>
        <div style={{ flex: 1, overflowY: 'auto' }}>
          {filtered.length === 0 && (
            <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>No experts match your search.</div>
          )}
          {filtered.map(expert => {
            const m = ROLE_META[expert.role as ExpertRole] || { emoji: '⚙️', label: expert.role, color: '#6b7280', dimColor: 'rgba(107,114,128,0.07)' };
            const isLocal = (expert.modelSource || 'provider') === 'local';
            return (
              <div key={expert.id} onClick={() => { onSelect(expert); onClose(); }}
                style={{ display: 'flex', alignItems: 'center', gap: 12, padding: '12px 18px',
                  borderBottom: '1px solid var(--border)', cursor: 'pointer', transition: 'background 0.1s' }}
                onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-elevated)')}
                onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}>
                <div style={{ width: 40, height: 40, borderRadius: 6, flexShrink: 0, background: m.dimColor, color: m.color,
                  display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 18 }}>{m.emoji}</div>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{expert.name}</span>
                    <span style={{ fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
                      background: isLocal ? 'rgba(5,150,105,0.1)' : 'rgba(37,99,235,0.1)',
                      color: isLocal ? '#059669' : '#2563EB' }}>
                      {isLocal ? 'LOCAL' : 'PROVIDER'}
                    </span>
                    {expert.isFinetuned && <span className="badge badge-violet" style={{ fontSize: 9 }}>Fine-tuned</span>}
                  </div>
                  <div style={{ fontSize: 11, color: m.color, marginTop: 1, fontWeight: 500 }}>{m.label}</div>
                  <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 1 }}>{expert.modelName} · {expert.providerName}</div>
                </div>
                <ChevronRight size={14} color="var(--text-4)" />
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}


/* ── Icon Resolver (for integrations/plugins) ─────── */
const STEP_ICON_MAP: Record<string, React.ComponentType<{ size?: number; color?: string }>> = {
  MessageSquare, Github, Ticket, BookOpen, Database, HardDrive,
  Cloud, CreditCard, Phone, Mail, Search: SearchIcon, BarChart3,
  Activity, Webhook, Globe, Terminal, FileText, Image, Languages,
  Puzzle, Cable, Store,
};

function StepResolveIcon({ name, size = 12, color }: { name: string; size?: number; color?: string }) {
  const Icon = STEP_ICON_MAP[name] || Puzzle;
  return <Icon size={size} color={color} />;
}

/* ── Integration/Plugin Selector Modal ──────────────── */
function IntegrationSelectorModal({
  onSelect,
  onClose,
  existingIds,
}: {
  onSelect: (item: StepIntegration) => void;
  onClose: () => void;
  existingIds: string[];
}) {
  const [tab, setTab] = useState<'integrations' | 'plugins'>('integrations');
  const [search, setSearch] = useState('');

  const filteredIntegrations = INTEGRATION_CATALOG.filter(i => {
    if (existingIds.includes(i.id)) return false;
    if (search && !i.name.toLowerCase().includes(search.toLowerCase())) return false;
    return true;
  });

  const filteredPlugins = MARKETPLACE_PLUGINS.filter(p => {
    if (existingIds.includes(p.id)) return false;
    if (search && !p.name.toLowerCase().includes(search.toLowerCase())) return false;
    return true;
  });

  return (
    <div style={{
      position: 'fixed', inset: 0,
      background: 'rgba(7,7,26,0.85)',
      zIndex: 200,
      display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
      paddingTop: 80,
    }}>
      <div className="animate-in" style={{
        width: 500, maxHeight: 'calc(100vh - 140px)',
        background: 'var(--bg-card)', border: '1px solid var(--border-md)',
        borderRadius: 8, display: 'flex', flexDirection: 'column', overflow: 'hidden',
      }}>
        {/* Header */}
        <div style={{
          padding: '16px 18px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>Add Integration or Plugin</div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>Extend this step with external tools and services</div>
          </div>
          <button className="btn btn-ghost btn-icon" onClick={onClose} style={{ color: 'var(--text-3)' }}>
            <X size={16} />
          </button>
        </div>

        {/* Tabs + Search */}
        <div style={{ padding: '10px 18px', borderBottom: '1px solid var(--border)' }}>
          <div style={{ display: 'flex', gap: 4, marginBottom: 10 }}>
            {([
              { id: 'integrations' as const, label: 'Integrations', icon: Cable },
              { id: 'plugins' as const, label: 'Plugins', icon: Puzzle },
            ]).map(t => (
              <button key={t.id} onClick={() => setTab(t.id)} style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '4px 12px', borderRadius: 4, fontSize: 11, fontWeight: tab === t.id ? 600 : 400,
                border: `1px solid ${tab === t.id ? 'var(--primary)' : 'var(--border)'}`,
                background: tab === t.id ? 'var(--primary-dim)' : 'transparent',
                color: tab === t.id ? 'var(--primary-text)' : 'var(--text-3)',
                cursor: 'pointer', transition: 'all 0.1s',
              }}>
                <t.icon size={11} /> {t.label}
              </button>
            ))}
          </div>
          <div style={{
            display: 'flex', alignItems: 'center', gap: 8,
            background: 'var(--bg)', border: '1px solid var(--border-md)',
            borderRadius: 4, padding: '7px 10px',
          }}>
            <Search size={13} color="var(--text-3)" />
            <input autoFocus className="input" style={{ background: 'none', border: 'none', padding: 0, fontSize: 12 }}
              placeholder={tab === 'integrations' ? 'Search integrations...' : 'Search plugins...'}
              value={search} onChange={e => setSearch(e.target.value)} />
          </div>
        </div>

        {/* List */}
        <div style={{ flex: 1, overflowY: 'auto' }}>
          {tab === 'integrations' && (
            <>
              {filteredIntegrations.length === 0 && (
                <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>No integrations available.</div>
              )}
              {filteredIntegrations.map(item => (
                <div key={item.id}
                  onClick={() => {
                    onSelect({
                      id: `si-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                      type: 'integration',
                      referenceId: item.id,
                      name: item.name,
                      icon: item.icon,
                      color: item.color,
                    });
                    onClose();
                  }}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 12, padding: '12px 18px',
                    borderBottom: '1px solid var(--border)', cursor: 'pointer', transition: 'background 0.1s',
                  }}
                  onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-elevated)')}
                  onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                >
                  <div style={{
                    width: 34, height: 34, borderRadius: 6, flexShrink: 0,
                    background: `${item.color}12`, border: `1px solid ${item.color}25`,
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                  }}>
                    <StepResolveIcon name={item.icon} size={15} color={item.color} />
                  </div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{item.name}</span>
                      <span style={{
                        fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
                        background: `${item.color}12`, color: item.color,
                        textTransform: 'uppercase',
                      }}>{item.category}</span>
                    </div>
                    <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 2 }}>{item.description}</div>
                  </div>
                  <ChevronRight size={14} color="var(--text-4)" />
                </div>
              ))}
            </>
          )}
          {tab === 'plugins' && (
            <>
              {filteredPlugins.length === 0 && (
                <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>No plugins available.</div>
              )}
              {filteredPlugins.map(plugin => (
                <div key={plugin.id}
                  onClick={() => {
                    onSelect({
                      id: `sp-${Date.now()}-${Math.random().toString(36).slice(2, 6)}`,
                      type: 'plugin',
                      referenceId: plugin.id,
                      name: plugin.name,
                      icon: plugin.icon,
                      color: plugin.color,
                    });
                    onClose();
                  }}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 12, padding: '12px 18px',
                    borderBottom: '1px solid var(--border)', cursor: 'pointer', transition: 'background 0.1s',
                  }}
                  onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-elevated)')}
                  onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                >
                  <div style={{
                    width: 34, height: 34, borderRadius: 6, flexShrink: 0,
                    background: `${plugin.color}12`, border: `1px solid ${plugin.color}25`,
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                  }}>
                    <StepResolveIcon name={plugin.icon} size={15} color={plugin.color} />
                  </div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{plugin.name}</span>
                      <span className="mono" style={{ fontSize: 9, color: 'var(--text-4)' }}>v{plugin.version}</span>
                      <span style={{
                        fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
                        background: `${plugin.color}12`, color: plugin.color, textTransform: 'uppercase',
                      }}>{plugin.category}</span>
                    </div>
                    <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 2 }}>{plugin.description}</div>
                    <div style={{ display: 'flex', gap: 8, marginTop: 3, fontSize: 10, color: 'var(--text-4)' }}>
                      <span style={{ display: 'flex', alignItems: 'center', gap: 2 }}><Download size={8} /> {plugin.downloads.toLocaleString()}</span>
                      <span style={{ display: 'flex', alignItems: 'center', gap: 2, color: '#F59E0B' }}><Star size={8} /> {plugin.rating}</span>
                    </div>
                  </div>
                  <ChevronRight size={14} color="var(--text-4)" />
                </div>
              ))}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/* ── Step Card ───────────────────────────────────────── */
function StepCard({ step, index, onRemove, onUpdate, onSwap }: {
  step: DraftStep; index: number; onRemove: () => void;
  onUpdate: (updates: Partial<DraftStep>) => void; onSwap: () => void;
}) {
  const hasExpert = step.expert !== null;
  const m = hasExpert ? (ROLE_META[step.expert!.role as ExpertRole] || { emoji: '⚙️', label: step.expert!.role, color: '#6b7280', dimColor: 'rgba(107,114,128,0.07)' }) : null;
  const isLocal = (step.expert?.modelSource || step.modelSource) === 'local';
  const stepFileInputRef = useRef<HTMLInputElement>(null);
  const stepImageInputRef = useRef<HTMLInputElement>(null);
  const [locInput, setLocInput] = useState('');
  const [showIntegrations, setShowIntegrations] = useState(false);

  return (
    <div className="workflow-step" style={{ minWidth: 280, maxWidth: 340 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <span className="mono" style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-3)', letterSpacing: '0.08em' }}>
            STEP {String(index + 1).padStart(2, '0')}
          </span>
          <span style={{ fontSize: 9, fontWeight: 700, padding: '1px 5px', borderRadius: 3,
            background: isLocal ? 'rgba(5,150,105,0.1)' : 'rgba(37,99,235,0.1)',
            color: isLocal ? '#059669' : '#2563EB' }}>
            {isLocal ? 'LOCAL' : 'PROVIDER'}
          </span>
          {step.connectionType === 'parallel' && (
            <span style={{ fontSize: 9, fontWeight: 700, padding: '1px 6px', borderRadius: 3,
              background: 'rgba(124,58,237,0.1)', color: '#7C3AED', textTransform: 'uppercase' }}>Parallel</span>
          )}
        </div>
        <div style={{ display: 'flex', gap: 3 }}>
          <button className="btn btn-ghost btn-icon btn-sm" onClick={onSwap} title="Swap expert" style={{ padding: 4 }}>
            <RotateCcw size={11} />
          </button>
          <button className="btn btn-ghost btn-icon btn-sm" onClick={onRemove} title="Remove step"
            style={{ padding: 4, color: 'var(--error)' }}><Trash2 size={11} /></button>
        </div>
      </div>

      {/* Expert badge — required */}
      {hasExpert && m ? (
        <div style={{ display: 'flex', alignItems: 'center', gap: 10, padding: '8px 10px',
          background: m.dimColor, border: `1px solid ${m.color}28`, borderRadius: 4, marginBottom: 10 }}>
          <span style={{ fontSize: 16 }}>{m.emoji}</span>
          <div style={{ flex: 1, minWidth: 0 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{step.expert!.name}</div>
            <div style={{ fontSize: 10, color: m.color, fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.06em' }}>{m.label}</div>
          </div>
          <div style={{ fontSize: 10, color: 'var(--text-4)' }}>{step.expert!.modelName}</div>
        </div>
      ) : (
        <div onClick={onSwap} style={{
          padding: '12px 10px', border: '2px dashed var(--border-md)', borderRadius: 4,
          marginBottom: 10, textAlign: 'center', cursor: 'pointer',
          color: 'var(--text-3)', fontSize: 12,
        }}>
          <Users size={16} style={{ margin: '0 auto 4px', display: 'block' }} />
          Select an Expert
        </div>
      )}

      {/* Task description */}
      <div style={{ marginBottom: 8 }}>
        <div style={{ ...LABEL, display: 'flex', alignItems: 'center', gap: 4 }}>
          Task Description <span style={{ color: 'var(--error)' }}>*</span>
        </div>
        <textarea className="textarea" style={{
          minHeight: 50, fontSize: 12,
          borderColor: !step.taskDescription.trim() && step.expert ? 'var(--error)' : undefined,
        }}
          placeholder="Describe what this expert should do..." value={step.taskDescription}
          onChange={e => onUpdate({ taskDescription: e.target.value })} />
      </div>

      {/* System instructions */}
      <div style={{ marginBottom: 8 }}>
        <div style={{ ...LABEL, display: 'flex', alignItems: 'center', gap: 4 }}>
          <MessageSquare size={10} /> System Instructions
        </div>
        <textarea className="textarea" style={{ minHeight: 40, fontSize: 11, fontFamily: 'var(--font-mono, monospace)' }}
          placeholder="Override or extend the expert's system prompt for this step..."
          value={step.systemInstructions}
          onChange={e => onUpdate({ systemInstructions: e.target.value })} />
      </div>

      {/* Attachments row — compact icons */}
      <div style={{ display: 'flex', gap: 4, marginBottom: 8 }}>
        {/* Files */}
        <button className="btn btn-ghost btn-sm" onClick={() => stepFileInputRef.current?.click()}
          title="Attach files" style={{ flex: 1, fontSize: 10, gap: 4, padding: '5px 6px' }}>
          <Paperclip size={11} /> Files {step.stepFiles.length > 0 && `(${step.stepFiles.length})`}
        </button>
        {/* Images */}
        <button className="btn btn-ghost btn-sm" onClick={() => stepImageInputRef.current?.click()}
          title="Attach images" style={{ flex: 1, fontSize: 10, gap: 4, padding: '5px 6px' }}>
          <Image size={11} /> Images {step.stepImages.length > 0 && `(${step.stepImages.length})`}
        </button>
        {/* Voice */}
        <button className="btn btn-ghost btn-sm"
          onClick={() => {
            const cmd = prompt('Enter voice command text:');
            if (cmd) onUpdate({ voiceCommand: cmd });
          }}
          title={step.voiceCommand ? `Voice: "${step.voiceCommand.slice(0, 30)}..."` : 'Add voice command'}
          style={{ flex: 1, fontSize: 10, gap: 4, padding: '5px 6px',
            color: step.voiceCommand ? '#7C3AED' : undefined }}>
          <Mic size={11} /> Voice {step.voiceCommand ? '1' : ''}
        </button>
      </div>

      {/* Hidden file inputs */}
      <input ref={stepFileInputRef} type="file" multiple accept=".pdf,.txt,.csv,.json,.md,.jsonl,.py,.go,.ts,.js"
        style={{ display: 'none' }}
        onChange={e => {
          if (!e.target.files?.length) return;
          const newFiles: UploadedFile[] = Array.from(e.target.files).map(f => ({ file: f, name: f.name, size: f.size, type: f.type }));
          onUpdate({ stepFiles: [...step.stepFiles, ...newFiles] });
          e.target.value = '';
        }} />
      <input ref={stepImageInputRef} type="file" multiple accept="image/*"
        style={{ display: 'none' }}
        onChange={e => {
          if (!e.target.files?.length) return;
          const newImages: UploadedFile[] = Array.from(e.target.files).map(f => ({ file: f, name: f.name, size: f.size, type: f.type }));
          onUpdate({ stepImages: [...step.stepImages, ...newImages] });
          e.target.value = '';
        }} />

      {/* Attached files list */}
      {(step.stepFiles.length > 0 || step.stepImages.length > 0) && (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 3, marginBottom: 8 }}>
          {[...step.stepFiles, ...step.stepImages].map((f, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '3px 6px',
              background: 'var(--bg)', borderRadius: 3, fontSize: 10, color: 'var(--text-3)' }}>
              {f.type.startsWith('image/') ? <Image size={9} /> : <File size={9} />}
              <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.name}</span>
              <button onClick={() => {
                onUpdate({
                  stepFiles: step.stepFiles.filter(sf => sf.name !== f.name),
                  stepImages: step.stepImages.filter(si => si.name !== f.name),
                });
              }} style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', padding: 0, display: 'flex' }}>
                <X size={9} />
              </button>
            </div>
          ))}
        </div>
      )}

      {/* File locations */}
      <div style={{ marginBottom: 8 }}>
        <div style={{ ...LABEL, display: 'flex', alignItems: 'center', gap: 4 }}>
          <FolderOpen size={10} /> File Locations
        </div>
        <div style={{ display: 'flex', gap: 4 }}>
          <input className="input" style={{ fontSize: 10, padding: '4px 6px', flex: 1 }}
            placeholder="/path/to/file or URL..."
            value={locInput} onChange={e => setLocInput(e.target.value)}
            onKeyDown={e => {
              if (e.key === 'Enter' && locInput.trim()) {
                onUpdate({ fileLocations: [...step.fileLocations, locInput.trim()] });
                setLocInput('');
              }
            }} />
          <button className="btn btn-ghost btn-sm" style={{ padding: '4px 6px', fontSize: 10 }}
            onClick={() => { if (locInput.trim()) { onUpdate({ fileLocations: [...step.fileLocations, locInput.trim()] }); setLocInput(''); } }}>
            <Plus size={10} />
          </button>
        </div>
        {step.fileLocations.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2, marginTop: 4 }}>
            {step.fileLocations.map((loc, i) => (
              <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 10, color: 'var(--text-3)' }}>
                <FolderOpen size={9} />
                <span className="mono" style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{loc}</span>
                <button onClick={() => onUpdate({ fileLocations: step.fileLocations.filter((_, j) => j !== i) })}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', padding: 0, display: 'flex' }}>
                  <X size={9} />
                </button>
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Voice command preview */}
      {step.voiceCommand && (
        <div style={{ marginBottom: 8, padding: '5px 8px', background: 'rgba(124,58,237,0.05)',
          border: '1px solid rgba(124,58,237,0.15)', borderRadius: 4 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 10, color: '#7C3AED', fontWeight: 600 }}>
              <Mic size={10} /> Voice Command
            </div>
            <button onClick={() => onUpdate({ voiceCommand: '' })}
              style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', padding: 0, display: 'flex' }}>
              <X size={9} />
            </button>
          </div>
          <div style={{ fontSize: 11, color: 'var(--text-2)', marginTop: 3, fontStyle: 'italic' }}>
            "{step.voiceCommand.slice(0, 100)}{step.voiceCommand.length > 100 ? '...' : ''}"
          </div>
        </div>
      )}

      {/* Integrations & Plugins */}
      <div style={{ marginBottom: 8 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
          <div style={{ ...LABEL, marginBottom: 0, display: 'flex', alignItems: 'center', gap: 4 }}>
            <Puzzle size={10} /> Integrations
          </div>
          <button className="btn btn-ghost btn-sm"
            onClick={() => setShowIntegrations(true)}
            title="Add integration or plugin"
            style={{ padding: '2px 6px', fontSize: 10, gap: 3, color: 'var(--primary-text)' }}>
            <Plus size={10} /> Add
          </button>
        </div>
        {step.integrations.length > 0 && (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
            {step.integrations.map(si => (
              <div key={si.id} style={{
                display: 'flex', alignItems: 'center', gap: 6, padding: '4px 8px',
                background: `${si.color}08`, border: `1px solid ${si.color}20`,
                borderRadius: 4,
              }}>
                <StepResolveIcon name={si.icon} size={10} color={si.color} />
                <span style={{ flex: 1, fontSize: 10, fontWeight: 500, color: 'var(--text-2)' }}>{si.name}</span>
                <span style={{
                  fontSize: 8, fontWeight: 700, padding: '0px 4px', borderRadius: 2,
                  background: si.type === 'plugin' ? 'rgba(124,58,237,0.1)' : 'rgba(37,99,235,0.1)',
                  color: si.type === 'plugin' ? '#7C3AED' : '#2563EB',
                  textTransform: 'uppercase',
                }}>{si.type === 'plugin' ? 'PLG' : 'INT'}</span>
                <button onClick={() => onUpdate({ integrations: step.integrations.filter(x => x.id !== si.id) })}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', padding: 0, display: 'flex' }}>
                  <X size={9} />
                </button>
              </div>
            ))}
          </div>
        )}
        {step.integrations.length === 0 && (
          <div onClick={() => setShowIntegrations(true)} style={{
            padding: '6px 8px', border: '1px dashed var(--border)', borderRadius: 4,
            textAlign: 'center', cursor: 'pointer', fontSize: 10, color: 'var(--text-4)',
            transition: 'border-color 0.15s',
          }}
            onMouseEnter={e => (e.currentTarget.style.borderColor = 'var(--primary)')}
            onMouseLeave={e => (e.currentTarget.style.borderColor = 'var(--border)')}>
            + APIs, tools, plugins
          </div>
        )}
      </div>
      {showIntegrations && (
        <IntegrationSelectorModal
          existingIds={step.integrations.map(i => i.referenceId)}
          onSelect={item => onUpdate({ integrations: [...step.integrations, item] })}
          onClose={() => setShowIntegrations(false)}
        />
      )}

      {/* Advanced toggle */}
      <button className="btn btn-ghost btn-sm"
        style={{ width: '100%', justifyContent: 'space-between', fontSize: 11 }}
        onClick={() => onUpdate({ collapsed: !step.collapsed })}>
        <span>Advanced config</span>
        {step.collapsed ? <ChevronDown size={12} /> : <ChevronUp size={12} />}
      </button>
      {!step.collapsed && (
        <div style={{ marginTop: 8, display: 'flex', flexDirection: 'column', gap: 8 }}>
          <div>
            <label style={LABEL}>Max Tokens</label>
            <input type="number" className="input" style={{ marginTop: 4, fontSize: 12 }}
              placeholder="4096" value={step.maxTokens ?? ''}
              onChange={e => onUpdate({ maxTokens: Number(e.target.value) || undefined })} />
          </div>
          <div>
            <label style={LABEL}>Temperature ({(step.temperature ?? 0.7).toFixed(1)})</label>
            <input type="range" min={0} max={2} step={0.1} style={{ marginTop: 4, width: '100%' }}
              value={step.temperature ?? 0.7} onChange={e => onUpdate({ temperature: Number(e.target.value) })} />
          </div>
        </div>
      )}

      {/* Source footer */}
      <div style={{ marginTop: 8, padding: '5px 8px', background: 'var(--bg)', border: '1px solid var(--border)',
        borderRadius: 4, display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--text-4)' }}>
        <span>{isLocal ? 'Local' : 'Provider'}</span>
        <span className="mono" style={{ color: isLocal ? '#059669' : '#2563EB' }}>
          {step.expert?.modelName || 'no expert'}
        </span>
      </div>
    </div>
  );
}

/* ── File Drop Zone ──────────────────────────────────── */
function FileDropZone({ label, accept, files, onFilesChange, multiple }: {
  label: string; accept: string; files: UploadedFile[];
  onFilesChange: (files: UploadedFile[]) => void; multiple: boolean;
}) {
  const inputRef = useRef<HTMLInputElement>(null);
  const [dragOver, setDragOver] = useState(false);

  const processFiles = (fileList: FileList) => {
    const newFiles: UploadedFile[] = Array.from(fileList).map(f => ({
      file: f, name: f.name, size: f.size, type: f.type,
    }));
    if (multiple) {
      onFilesChange([...files, ...newFiles]);
    } else {
      const file = newFiles[0];
      if (file && (file.name.endsWith('.md') || file.name.endsWith('.txt'))) {
        const reader = new FileReader();
        reader.onload = () => { onFilesChange([{ ...file, preview: reader.result as string }]); };
        reader.readAsText(file.file);
      } else {
        onFilesChange(newFiles);
      }
    }
  };

  return (
    <div>
      {files.length === 0 ? (
        <div onClick={() => inputRef.current?.click()}
          onDragOver={e => { e.preventDefault(); setDragOver(true); }}
          onDragLeave={() => setDragOver(false)}
          onDrop={e => { e.preventDefault(); setDragOver(false); if (e.dataTransfer.files.length) processFiles(e.dataTransfer.files); }}
          style={{ border: `2px dashed ${dragOver ? 'var(--primary)' : 'var(--border-md)'}`, borderRadius: 6,
            padding: '20px 16px', textAlign: 'center', cursor: 'pointer',
            background: dragOver ? 'var(--primary-dim)' : 'transparent', transition: 'all 0.15s' }}>
          <Upload size={18} color="var(--text-3)" style={{ margin: '0 auto 6px' }} />
          <div style={{ fontSize: 12, color: 'var(--text-2)', fontWeight: 500 }}>Drop {multiple ? 'files' : 'file'} or click to browse</div>
          <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 3 }}>{accept}</div>
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {files.map((f, i) => (
            <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '6px 10px',
              background: 'var(--bg-elevated)', border: '1px solid var(--border)', borderRadius: 4 }}>
              <File size={13} color="var(--text-3)" />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 11, fontWeight: 500, color: 'var(--text-1)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.name}</div>
                <div style={{ fontSize: 10, color: 'var(--text-4)' }}>{formatBytes(f.size)}</div>
              </div>
              <button className="btn btn-ghost btn-icon btn-sm" onClick={() => onFilesChange(files.filter((_, idx) => idx !== i))}
                style={{ padding: 3, color: 'var(--text-4)' }}><X size={11} /></button>
            </div>
          ))}
          {multiple && (
            <button className="btn btn-ghost btn-sm" onClick={() => inputRef.current?.click()} style={{ alignSelf: 'flex-start', fontSize: 11 }}>
              <Plus size={11} /> Add more
            </button>
          )}
        </div>
      )}
      <input ref={inputRef} type="file" accept={accept} multiple={multiple} style={{ display: 'none' }}
        onChange={e => { if (e.target.files?.length) processFiles(e.target.files); e.target.value = ''; }} />
    </div>
  );
}

/* ── Live Execution Panel ────────────────────────────── */
function LiveExecutionPanel({ status, agents, events, output, error }: {
  status: string;
  agents: Record<string, { agentId: string; stepId: string; status: string; output?: string; tokensUsed?: number; durationMs?: number; error?: string; taskDescription?: string }>;
  events: Array<{ event: string; agentId?: string; stepId?: string; timestamp: string; data: Record<string, unknown> }>;
  output: string | null; error: string | null;
}) {
  const [showEvents, setShowEvents] = useState(false);
  return (
    <div className="card" style={{ padding: 20, marginBottom: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <Brain size={16} color={status === 'running' ? 'var(--amber)' : status === 'completed' ? '#059669' : status === 'failed' ? 'var(--error)' : 'var(--text-3)'} />
          <span style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>Live Execution</span>
          {status === 'running' && <Loader2 size={14} color="var(--amber)" style={{ animation: 'spin 1s linear infinite' }} />}
          {status === 'completed' && <CheckCircle2 size={14} color="#059669" />}
          {status === 'failed' && <AlertCircle size={14} color="var(--error)" />}
        </div>
        <button className="btn btn-ghost btn-sm" onClick={() => setShowEvents(!showEvents)} style={{ fontSize: 11 }}>
          <Eye size={12} /> {showEvents ? 'Hide' : 'Show'} Events ({events.length})
        </button>
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(260px, 1fr))', gap: 10, marginBottom: 16 }}>
        {Object.values(agents).map(agent => (
          <div key={agent.agentId} style={{ padding: '12px 14px', background: 'var(--bg)',
            border: `1px solid ${agent.status === 'completed' ? '#05966940' : agent.status === 'failed' ? 'var(--error-dim)' : agent.status === 'thinking' ? '#D9770640' : 'var(--border)'}`,
            borderRadius: 6 }}>
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
              <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>{agent.stepId}</span>
              <span style={{ fontSize: 10, fontWeight: 700, textTransform: 'uppercase',
                color: agent.status === 'completed' ? '#059669' : agent.status === 'failed' ? 'var(--error)' : agent.status === 'thinking' ? '#D97706' : 'var(--text-3)' }}>
                {agent.status}
              </span>
            </div>
            {agent.taskDescription && <div style={{ fontSize: 12, color: 'var(--text-2)', marginBottom: 6, lineHeight: 1.4 }}>{agent.taskDescription}</div>}
            {agent.output && (
              <div style={{ fontSize: 11, color: 'var(--text-3)', lineHeight: 1.4, maxHeight: 80, overflow: 'hidden',
                background: 'var(--bg-elevated)', padding: '6px 8px', borderRadius: 3, marginTop: 4 }}>
                {agent.output.slice(0, 200)}{agent.output.length > 200 ? '...' : ''}
              </div>
            )}
            {agent.tokensUsed !== undefined && agent.tokensUsed > 0 && (
              <div style={{ display: 'flex', gap: 12, marginTop: 6, fontSize: 10, color: 'var(--text-4)' }}>
                <span className="mono">{fmt(agent.tokensUsed)} tokens</span>
                {agent.durationMs !== undefined && <span className="mono">{(agent.durationMs / 1000).toFixed(1)}s</span>}
              </div>
            )}
            {agent.error && <div style={{ fontSize: 11, color: 'var(--error)', marginTop: 6 }}>{agent.error}</div>}
          </div>
        ))}
      </div>
      {output && (
        <div style={{ padding: '14px 16px', background: 'rgba(5,150,105,0.05)', border: '1px solid #05966930', borderRadius: 6, marginBottom: 12 }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: '#059669', marginBottom: 8, textTransform: 'uppercase', letterSpacing: '0.08em' }}>Final Output</div>
          <div style={{ fontSize: 13, color: 'var(--text-1)', lineHeight: 1.6, whiteSpace: 'pre-wrap' }}>{output}</div>
        </div>
      )}
      {error && (
        <div style={{ padding: '12px 16px', background: 'rgba(220,38,38,0.05)', border: '1px solid rgba(220,38,38,0.2)', borderRadius: 6, fontSize: 13, color: 'var(--error)' }}>
          {error}
        </div>
      )}
      {showEvents && events.length > 0 && (
        <div style={{ marginTop: 12, maxHeight: 200, overflowY: 'auto', background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 4, padding: 8 }}>
          {events.map((ev, i) => (
            <div key={i} style={{ fontSize: 10, fontFamily: 'var(--font-mono, monospace)', color: 'var(--text-3)', padding: '2px 0',
              borderBottom: i < events.length - 1 ? '1px solid var(--border)' : 'none' }}>
              <span style={{ color: 'var(--text-4)' }}>{new Date(ev.timestamp).toLocaleTimeString()}</span>{' '}
              <span style={{ color: ev.event.includes('complete') ? '#059669' : ev.event.includes('failed') ? 'var(--error)' : ev.event.includes('thinking') ? '#D97706' : 'var(--primary-text)' }}>{ev.event}</span>
              {ev.agentId && <span style={{ color: 'var(--text-4)' }}> [{ev.agentId}]</span>}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

/* ── Advanced Options Panel ──────────────────────────── */
function AdvancedOptionsPanel({
  metrics, onMetricsChange,
  config, onConfigChange,
  tags, onTagsChange,
  permissions, onPermissionsChange,
  logger,
  workflowId,
}: {
  metrics: MetricsConfig; onMetricsChange: (m: MetricsConfig) => void;
  config: AdvancedConfig; onConfigChange: (c: AdvancedConfig) => void;
  tags: string[]; onTagsChange: (t: string[]) => void;
  permissions: PermissionsConfig; onPermissionsChange: (p: PermissionsConfig) => void;
  logger: ReturnType<typeof useWorkflowLogger>;
  workflowId: string;
}) {
  const [expanded, setExpanded] = useState(false);
  const [activeTab, setActiveTab] = useState<'metrics' | 'config' | 'tags' | 'permissions'>('metrics');
  const [tagInput, setTagInput] = useState('');

  const updateMetrics = (patch: Partial<MetricsConfig>) => {
    const next = { ...metrics, ...patch };
    onMetricsChange(next);
    logger.logMetricsConfig(next);
  };

  const updateConfig = (patch: Partial<AdvancedConfig>) => {
    const next = { ...config, ...patch };
    onConfigChange(next);
    logger.saveConfig({ advanced: next });
  };

  const updatePermissions = (patch: Partial<PermissionsConfig>) => {
    const next = { ...permissions, ...patch };
    onPermissionsChange(next);
    logger.savePermissions(next);
  };

  const addTag = () => {
    const t = tagInput.trim();
    if (t && !tags.includes(t)) {
      const next = [...tags, t];
      onTagsChange(next);
      logger.saveTags(next);
      logger.logInteraction('tag.added', { tag: t });
    }
    setTagInput('');
  };

  const removeTag = (tag: string) => {
    const next = tags.filter(t => t !== tag);
    onTagsChange(next);
    logger.saveTags(next);
    logger.logInteraction('tag.removed', { tag });
  };

  const tabs = [
    { id: 'metrics' as const, label: 'Metrics', icon: BarChart3 },
    { id: 'config' as const, label: 'Configuration', icon: Settings },
    { id: 'tags' as const, label: 'Tags', icon: Tag },
    { id: 'permissions' as const, label: 'Permissions', icon: Shield },
  ];

  return (
    <div className="card" style={{ marginBottom: 16, overflow: 'hidden' }}>
      <button
        onClick={() => { setExpanded(!expanded); logger.logInteraction(expanded ? 'advanced.collapsed' : 'advanced.expanded'); }}
        style={{
          width: '100%', display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '14px 20px', background: 'none', border: 'none', cursor: 'pointer',
          color: 'var(--text-1)',
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <Settings size={15} color="var(--text-3)" />
          <span style={{ fontSize: 13, fontWeight: 600 }}>Advanced Options</span>
          <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
            Metrics, Configuration, Tags, Permissions
          </span>
        </div>
        {expanded ? <ChevronUp size={14} color="var(--text-3)" /> : <ChevronDown size={14} color="var(--text-3)" />}
      </button>

      {expanded && (
        <div style={{ borderTop: '1px solid var(--border)' }}>
          {/* Tab bar */}
          <div style={{ display: 'flex', borderBottom: '1px solid var(--border)', padding: '0 20px' }}>
            {tabs.map(tab => (
              <button key={tab.id} onClick={() => { setActiveTab(tab.id); logger.logInteraction('advanced.tab.switch', { tab: tab.id }); }}
                style={{
                  display: 'flex', alignItems: 'center', gap: 6, padding: '10px 16px',
                  background: 'none', border: 'none', borderBottom: `2px solid ${activeTab === tab.id ? 'var(--primary)' : 'transparent'}`,
                  cursor: 'pointer', fontSize: 12, fontWeight: activeTab === tab.id ? 600 : 400,
                  color: activeTab === tab.id ? 'var(--primary-text)' : 'var(--text-3)',
                  transition: 'all 0.12s',
                }}>
                <tab.icon size={13} />
                {tab.label}
              </button>
            ))}
          </div>

          {/* Tab content */}
          <div style={{ padding: 20 }}>
            {/* ── Metrics Tab ── */}
            {activeTab === 'metrics' && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 20 }}>
                {/* MLflow */}
                <div style={{ padding: 16, background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
                    <FlaskConical size={14} color="#7C3AED" />
                    <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>MLflow</span>
                  </div>
                  <Checkbox checked={metrics.mlflow} onChange={v => updateMetrics({ mlflow: v })}
                    label="Enable MLflow Tracking" description="Track experiments, parameters, and metrics" />
                  {metrics.mlflow && (
                    <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                      <div>
                        <div style={LABEL}>Tracking URI</div>
                        <input className="input" style={{ fontSize: 11 }} placeholder="http://localhost:5050"
                          value={metrics.mlflowTrackingUri} onChange={e => updateMetrics({ mlflowTrackingUri: e.target.value })} />
                      </div>
                      <div>
                        <div style={LABEL}>Experiment Name</div>
                        <input className="input" style={{ fontSize: 11 }} placeholder="kortecx-workflows"
                          value={metrics.mlflowExperiment} onChange={e => updateMetrics({ mlflowExperiment: e.target.value })} />
                      </div>
                    </div>
                  )}
                </div>

                {/* Logging */}
                <div style={{ padding: 16, background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
                    <ScrollText size={14} color="#D97706" />
                    <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>Logging</span>
                  </div>
                  <Checkbox checked={metrics.logging} onChange={v => updateMetrics({ logging: v })}
                    label="Enable Detailed Logging" description="Log all agent interactions and decisions" />
                  {metrics.logging && (
                    <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 8 }}>
                      <RadioGroup value={metrics.logLevel} onChange={v => updateMetrics({ logLevel: v as MetricsConfig['logLevel'] })}
                        label="Log Level" options={[
                          { value: 'debug', label: 'Debug' }, { value: 'info', label: 'Info' },
                          { value: 'warn', label: 'Warn' }, { value: 'error', label: 'Error' },
                        ]} />
                      <RadioGroup value={metrics.logFormat} onChange={v => updateMetrics({ logFormat: v as MetricsConfig['logFormat'] })}
                        label="Format" options={[
                          { value: 'structured', label: 'Structured JSON' }, { value: 'plaintext', label: 'Plaintext' },
                        ]} />
                    </div>
                  )}
                </div>

                {/* Monitoring */}
                <div style={{ padding: 16, background: 'var(--bg)', border: '1px solid var(--border)', borderRadius: 6 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
                    <Activity size={14} color="#DC2626" />
                    <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>Monitoring</span>
                  </div>
                  <Checkbox checked={metrics.monitoring} onChange={v => updateMetrics({ monitoring: v })}
                    label="Enable Real-time Monitoring" description="Track performance and health metrics" />
                  {metrics.monitoring && (
                    <div style={{ marginTop: 10, display: 'flex', flexDirection: 'column', gap: 6 }}>
                      <div>
                        <div style={LABEL}>Poll Interval (seconds)</div>
                        <input type="number" className="input" style={{ fontSize: 11 }}
                          value={metrics.monitoringInterval} onChange={e => updateMetrics({ monitoringInterval: Number(e.target.value) || 30 })} />
                      </div>
                      <Checkbox checked={metrics.alertOnFailure} onChange={v => updateMetrics({ alertOnFailure: v })}
                        label="Alert on Failure" />
                      <Checkbox checked={metrics.alertOnLatency} onChange={v => updateMetrics({ alertOnLatency: v })}
                        label="Alert on High Latency" />
                      {metrics.alertOnLatency && (
                        <div>
                          <div style={LABEL}>Latency Threshold (ms)</div>
                          <input type="number" className="input" style={{ fontSize: 11 }}
                            value={metrics.latencyThresholdMs} onChange={e => updateMetrics({ latencyThresholdMs: Number(e.target.value) || 5000 })} />
                        </div>
                      )}
                    </div>
                  )}
                </div>
              </div>
            )}

            {/* ── Config Tab ── */}
            {activeTab === 'config' && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div style={SECTION_TITLE}>Execution</div>
                  <RadioGroup value={config.failureStrategy} onChange={v => updateConfig({ failureStrategy: v as AdvancedConfig['failureStrategy'] })}
                    label="Failure Strategy" options={[
                      { value: 'stop', label: 'Stop' }, { value: 'skip', label: 'Skip Failed' }, { value: 'retry', label: 'Retry' },
                    ]} />
                  <div>
                    <div style={LABEL}>Max Retries</div>
                    <input type="number" className="input" style={{ fontSize: 12, maxWidth: 120 }}
                      value={config.maxRetries} min={0} max={10}
                      onChange={e => updateConfig({ maxRetries: Number(e.target.value) || 0 })} />
                  </div>
                  <div>
                    <div style={LABEL}>Timeout (seconds)</div>
                    <input type="number" className="input" style={{ fontSize: 12, maxWidth: 120 }}
                      value={config.timeoutSec} min={30}
                      onChange={e => updateConfig({ timeoutSec: Number(e.target.value) || 300 })} />
                  </div>
                  <RadioGroup value={config.priority} onChange={v => updateConfig({ priority: v as AdvancedConfig['priority'] })}
                    label="Priority" options={[
                      { value: 'low', label: 'Low' }, { value: 'normal', label: 'Normal' },
                      { value: 'high', label: 'High' }, { value: 'critical', label: 'Critical' },
                    ]} />
                  <div>
                    <div style={LABEL}>Concurrency Limit</div>
                    <input type="number" className="input" style={{ fontSize: 12, maxWidth: 120 }}
                      value={config.concurrencyLimit} min={1} max={20}
                      onChange={e => updateConfig({ concurrencyLimit: Number(e.target.value) || 5 })} />
                  </div>
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div style={SECTION_TITLE}>Caching & Notifications</div>
                  <Checkbox checked={config.cacheResults} onChange={v => updateConfig({ cacheResults: v })}
                    label="Cache Step Results" description="Reuse outputs from identical steps across runs" />
                  {config.cacheResults && (
                    <div>
                      <div style={LABEL}>Cache TTL (seconds)</div>
                      <input type="number" className="input" style={{ fontSize: 12, maxWidth: 120 }}
                        value={config.cacheTtlSec} min={60}
                        onChange={e => updateConfig({ cacheTtlSec: Number(e.target.value) || 3600 })} />
                    </div>
                  )}
                  <Checkbox checked={config.notifyOnComplete} onChange={v => updateConfig({ notifyOnComplete: v })}
                    label="Notify on Completion" description="Send notification when workflow finishes" />
                  {config.notifyOnComplete && (
                    <div>
                      <div style={LABEL}>Notification Channel</div>
                      <input className="input" style={{ fontSize: 12 }} placeholder="e.g. #workflows, email"
                        value={config.notifyChannel} onChange={e => updateConfig({ notifyChannel: e.target.value })} />
                    </div>
                  )}
                  <div>
                    <div style={SECTION_TITLE}>Description</div>
                    <textarea className="textarea" style={{ minHeight: 80, fontSize: 12 }}
                      placeholder="Describe the purpose and scope of this workflow..."
                      value={config.description}
                      onChange={e => updateConfig({ description: e.target.value })} />
                  </div>
                </div>
              </div>
            )}

            {/* ── Tags Tab ── */}
            {activeTab === 'tags' && (
              <div>
                <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 6, flex: 1,
                    border: '1px solid var(--border-md)', borderRadius: 4, padding: '0 10px' }}>
                    <Hash size={13} color="var(--text-3)" />
                    <input className="input" style={{ border: 'none', padding: '8px 0', fontSize: 12, background: 'none' }}
                      placeholder="Add a tag..." value={tagInput}
                      onChange={e => setTagInput(e.target.value)}
                      onKeyDown={e => { if (e.key === 'Enter') addTag(); }} />
                  </div>
                  <button className="btn btn-secondary btn-sm" onClick={addTag} disabled={!tagInput.trim()}>
                    <Plus size={12} /> Add
                  </button>
                </div>
                <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
                  {tags.length === 0 && (
                    <span style={{ fontSize: 12, color: 'var(--text-4)' }}>No tags added yet</span>
                  )}
                  {tags.map(tag => (
                    <span key={tag} style={{
                      display: 'inline-flex', alignItems: 'center', gap: 5,
                      padding: '4px 10px', borderRadius: 20, fontSize: 12,
                      background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                      color: 'var(--text-2)',
                    }}>
                      <Hash size={10} color="var(--text-4)" />
                      {tag}
                      <button onClick={() => removeTag(tag)} style={{
                        background: 'none', border: 'none', cursor: 'pointer',
                        padding: 0, color: 'var(--text-4)', display: 'flex',
                      }}><X size={10} /></button>
                    </span>
                  ))}
                </div>
              </div>
            )}

            {/* ── Permissions Tab ── */}
            {activeTab === 'permissions' && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16 }}>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div style={SECTION_TITLE}>Access Control</div>
                  <RadioGroup value={permissions.visibility} onChange={v => updatePermissions({ visibility: v as PermissionsConfig['visibility'] })}
                    label="Visibility" options={[
                      { value: 'private', label: 'Private' }, { value: 'team', label: 'Team' }, { value: 'public', label: 'Public' },
                    ]} />
                  <RadioGroup value={permissions.allowEdit} onChange={v => updatePermissions({ allowEdit: v as PermissionsConfig['allowEdit'] })}
                    label="Edit Access" options={[
                      { value: 'owner', label: 'Owner Only' }, { value: 'team', label: 'Team' }, { value: 'anyone', label: 'Anyone' },
                    ]} />
                  <Checkbox checked={permissions.allowClone} onChange={v => updatePermissions({ allowClone: v })}
                    label="Allow Cloning" description="Others can create copies of this workflow" />
                  <Checkbox checked={permissions.requireApproval} onChange={v => updatePermissions({ requireApproval: v })}
                    label="Require Run Approval" description="Runs must be approved before execution starts" />
                </div>
                <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                  <div style={SECTION_TITLE}>Resource Limits</div>
                  <div>
                    <div style={LABEL}>Max Runs Per Day</div>
                    <input type="number" className="input" style={{ fontSize: 12, maxWidth: 120 }}
                      value={permissions.maxRunsPerDay} min={0}
                      onChange={e => updatePermissions({ maxRunsPerDay: Number(e.target.value) || 0 })} />
                    <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>0 = unlimited</div>
                  </div>
                  <div>
                    <div style={LABEL}>Token Budget</div>
                    <input type="number" className="input" style={{ fontSize: 12, maxWidth: 160 }}
                      value={permissions.tokenBudget} min={0}
                      onChange={e => updatePermissions({ tokenBudget: Number(e.target.value) || 0 })} />
                    <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>Max tokens per run (0 = unlimited)</div>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/* ── Main Page ───────────────────────────────────────── */
function WorkflowBuilderInner() {
  const searchParams = useSearchParams();
  const router = useRouter();
  const editId = searchParams.get('id');

  const [steps, setSteps] = useState<DraftStep[]>([]);
  const [showSelector, setShowSelector] = useState(false);
  const [swapIndex, setSwapIndex] = useState<number | null>(null);
  const [workflowName, setWorkflowName] = useState('');
  const [nameError, setNameError] = useState(false);
  const [loadingExisting, setLoadingExisting] = useState(!!editId);

  // Goal: dual input — text or file
  const [goalMode, setGoalMode] = useState<'text' | 'file'>('text');
  const [goalText, setGoalText] = useState('');
  const [goalFiles, setGoalFiles] = useState<UploadedFile[]>([]);
  const [inputFiles, setInputFiles] = useState<UploadedFile[]>([]);

  // Advanced options
  const [metrics, setMetrics] = useState<MetricsConfig>({
    mlflow: false, mlflowTrackingUri: 'http://localhost:5050', mlflowExperiment: 'kortecx-workflows',
    logging: true, logLevel: 'info', logFormat: 'structured',
    monitoring: false, monitoringInterval: 30, alertOnFailure: true, alertOnLatency: false, latencyThresholdMs: 5000,
  });
  const [advancedConfig, setAdvancedConfig] = useState<AdvancedConfig>({
    maxRetries: 2, timeoutSec: 300, failureStrategy: 'stop', priority: 'normal',
    concurrencyLimit: 5, cacheResults: false, cacheTtlSec: 3600,
    notifyOnComplete: false, notifyChannel: '', description: '',
  });
  const [tags, setTags] = useState<string[]>([]);
  const [permissions, setPermissions] = useState<PermissionsConfig>({
    visibility: 'private', allowClone: true, allowEdit: 'owner',
    requireApproval: false, maxRunsPerDay: 0, tokenBudget: 0,
  });

  // Workflow ID (generated once for logging, or from URL)
  const workflowIdRef = useRef(editId || `wf-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`);
  const wfLogger = useWorkflowLogger(workflowIdRef.current);

  const [saving, setSaving] = useState(false);
  const [saveErrors, setSaveErrors] = useState<{ name?: string; goal?: string; steps?: string; stepDetails?: Record<string, string>; general?: string }>({});
  const [saveSuccess, setSaveSuccess] = useState(false);

  const ws = useWorkflowWS();
  const { experts: dbExperts } = useExperts();
  const { workflows: dbWorkflows, mutate: mutateWorkflows } = useWorkflows();

  // Log page load
  useEffect(() => {
    wfLogger.logSessionEvent('page.loaded', { page: 'workflow-builder' });
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Load existing workflow when ?id= is present
  useEffect(() => {
    if (!editId) return;
    let cancelled = false;

    (async () => {
      try {
        const res = await fetch(`/api/workflows?id=${editId}`);
        if (!res.ok) { setLoadingExisting(false); return; }
        const data = await res.json();
        if (cancelled) return;

        const wf = data.workflow;
        const dbSteps = (data.steps ?? []) as Record<string, unknown>[];

        if (wf) {
          workflowIdRef.current = wf.id;
          setWorkflowName(wf.name ?? '');
          setGoalText(wf.goalStatement ?? '');
          setTags(wf.tags ?? []);
          if (wf.description) setAdvancedConfig(prev => ({ ...prev, description: wf.description }));
        }

        if (dbSteps.length > 0) {
          // Reconstruct DraftSteps from DB rows
          const reconstructed: DraftStep[] = dbSteps.map((s: Record<string, unknown>) => {
            const expertId = s.expertId as string | null;
            const matchedExpert = expertId ? dbExperts.find((e: Expert) => e.id === expertId) ?? null : null;
            const modelSrc = (s.modelSource as ModelSource) || 'provider';
            const localCfg = s.localModelConfig as LocalModelConfig | null;

            return {
              id: (s.id as string) || uid(),
              expert: matchedExpert,
              taskDescription: (s.taskDescription as string) || '',
              systemInstructions: (s.systemInstructions as string) || '',
              maxTokens: (s.maxTokens as number) || 4096,
              temperature: s.temperature != null ? Number(s.temperature) : 0.7,
              collapsed: true,
              modelSource: modelSrc,
              localModel: localCfg
                ? { engine: localCfg.engine || 'ollama', modelName: localCfg.modelName || (localCfg as unknown as Record<string, unknown>).model as string || '' }
                : { engine: 'ollama' as const, modelName: '' },
              connectionType: ((s.connectionType as string) || 'sequential') as 'sequential' | 'parallel',
              stepFiles: [],
              stepImages: [],
              voiceCommand: (s.voiceCommand as string) || '',
              fileLocations: (s.fileLocations as string[]) || [],
              integrations: ((s.integrations as StepIntegration[]) || []),
            };
          });
          setSteps(reconstructed);
        }
      } catch (err) {
        console.error('Failed to load workflow:', err);
      } finally {
        if (!cancelled) setLoadingExisting(false);
      }
    })();

    return () => { cancelled = true; };
  }, [editId, dbExperts]); // eslint-disable-line react-hooks/exhaustive-deps

  /* Validation */
  const goalContent = goalMode === 'text' ? goalText : (goalFiles[0]?.preview || '');
  const hasGoal = goalMode === 'text' ? goalText.trim().length > 0 : goalFiles.length > 0;
  const isValid = workflowName.trim().length > 0 && hasGoal && steps.length > 0;
  const isRunning = ws.status === 'running' || ws.status === 'connecting';

  /* Estimation */
  const totalTokens = steps.reduce((sum, s) => sum + (s.maxTokens ?? 4096), 0);
  const totalSec = steps.length * 30;

  // Global parallel toggle
  const [globalParallel, setGlobalParallel] = useState(false);

  const makeStep = (expert: Expert | null): DraftStep => {
    const expertSource = (expert?.modelSource || 'provider') as ModelSource;
    const expertLocalModel: LocalModelConfig = expert?.localModelConfig
      ? { engine: expert.localModelConfig.engine, modelName: expert.localModelConfig.modelName || '' }
      : { engine: 'ollama', modelName: '' };
    return {
      id: uid(), expert, taskDescription: '', systemInstructions: '', collapsed: true,
      modelSource: expertSource, localModel: expertLocalModel,
      connectionType: globalParallel ? 'parallel' : 'sequential',
      stepFiles: [], stepImages: [], voiceCommand: '', fileLocations: [],
      integrations: [],
    };
  };

  const addExpert = useCallback((expert: Expert) => {
    if (swapIndex !== null) {
      setSteps(prev => prev.map((s, i) =>
        i === swapIndex ? { ...s, expert, modelSource: (expert.modelSource || 'provider') as ModelSource,
          localModel: expert.localModelConfig ? { engine: expert.localModelConfig.engine, modelName: expert.localModelConfig.modelName || '' } : s.localModel,
        } : s,
      ));
      setSwapIndex(null);
      wfLogger.logStepChange('updated', { stepIndex: swapIndex, expertId: expert.id, action: 'swap' });
    } else {
      const step = makeStep(expert);
      setSteps(prev => [...prev, step]);
      wfLogger.logStepChange('added', { stepId: step.id, expertId: expert.id, modelSource: expert.modelSource || 'provider' });
    }
  }, [swapIndex, wfLogger, globalParallel]); // eslint-disable-line react-hooks/exhaustive-deps

  const removeStep = useCallback((id: string) => {
    setSteps(prev => prev.filter(s => s.id !== id));
    wfLogger.logStepChange('removed', { stepId: id });
  }, [wfLogger]);

  const updateStep = useCallback((id: string, updates: Partial<DraftStep>) => {
    setSteps(prev => prev.map(s => s.id === id ? { ...s, ...updates } : s));
  }, []);

  const openSwap = useCallback((idx: number) => {
    setSwapIndex(idx);
    setShowSelector(true);
  }, []);

  /* Suggest chain */
  const suggestChain = () => {
    const content = goalContent.toLowerCase();
    let roleKey = 'research';
    if (content.includes('code') || content.includes('refactor') || content.includes('bug')) roleKey = 'code';
    else if (content.includes('legal') || content.includes('contract')) roleKey = 'legal';
    else if (content.includes('data') || content.includes('dataset')) roleKey = 'data';
    else if (content.includes('write') || content.includes('article') || content.includes('blog')) roleKey = 'content';

    const roles = SUGGESTED_CHAINS[roleKey] ?? SUGGESTED_CHAINS.research;
    const newSteps: DraftStep[] = [];
    for (const role of roles) {
      const expert = dbExperts.find((e: Expert) => e.role === role && (e.status === 'active' || e.status === 'idle'));
      if (expert) {
        newSteps.push(makeStep(expert));
      }
    }
    if (newSteps.length === 0) {
      // No experts found — add placeholder steps without experts
      for (const role of roles) {
        const s = makeStep(null);
        s.taskDescription = `${role} task — deploy an expert with this role first`;
        newSteps.push(s);
      }
    }
    setSteps(newSteps);
    wfLogger.logInteraction('chain.suggested', { roleKey, stepsCount: newSteps.length });
  };

  /* Save workflow */
  const handleSave = async () => {
    setSaveErrors({});
    setSaveSuccess(false);

    const errors: typeof saveErrors = {};
    if (!workflowName.trim()) {
      setNameError(true);
      errors.name = 'Workflow name is required';
    }
    if (!hasGoal) {
      errors.goal = 'Task goal is required — write or upload a markdown goal';
    }
    if (steps.length === 0) {
      errors.steps = 'Add at least one expert step to the workflow';
    } else {
      const stepIssues: Record<string, string> = {};
      steps.forEach((s, i) => {
        const msgs: string[] = [];
        if (!s.expert) msgs.push('no expert assigned');
        if (!s.taskDescription.trim()) msgs.push('task description is empty');
        if (msgs.length > 0) stepIssues[s.id] = `Step ${i + 1}: ${msgs.join(', ')}`;
      });
      if (Object.keys(stepIssues).length > 0) {
        errors.steps = `${Object.keys(stepIssues).length} step(s) have issues`;
        errors.stepDetails = stepIssues;
      }
    }

    if (Object.keys(errors).length > 0) {
      setSaveErrors(errors);
      return;
    }

    setSaving(true);
    try {
      const stepPayload = steps.map((s) => {
        const expert = s.expert;
        const source = expert?.modelSource || s.modelSource;
        return {
          expertId: expert?.id || null,
          taskDescription: s.taskDescription,
          systemInstructions: s.systemInstructions || '',
          voiceCommand: s.voiceCommand || '',
          fileLocations: s.fileLocations,
          stepFileNames: s.stepFiles.map(f => f.name),
          stepImageNames: s.stepImages.map(f => f.name),
          modelSource: source,
          localModel: source === 'local'
            ? (expert?.localModelConfig
                ? { engine: expert.localModelConfig.engine, model: expert.localModelConfig.modelName }
                : { engine: s.localModel.engine, model: s.localModel.modelName })
            : null,
          temperature: s.temperature ?? (expert ? Number(expert.temperature) : 0.7),
          maxTokens: s.maxTokens ?? (expert?.maxTokens || 4096),
          connectionType: s.connectionType,
          integrations: s.integrations.map(si => ({
            id: si.id, type: si.type, referenceId: si.referenceId,
            name: si.name, icon: si.icon, color: si.color,
            config: si.config || {},
          })),
        };
      });

      const isUpdate = !!editId;
      const res = await fetch('/api/workflows', {
        method: isUpdate ? 'PATCH' : 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          ...(isUpdate ? { id: editId } : {}),
          name: workflowName.trim(),
          description: advancedConfig.description || null,
          goalStatement: goalContent,
          tags,
          steps: stepPayload,
        }),
      });

      if (!res.ok) {
        const data = await res.json();
        throw new Error(data.error || 'Save failed');
      }

      mutateWorkflows();
      wfLogger.logInteraction('workflow.saved', { name: workflowName, mode: isUpdate ? 'update' : 'create' });
      router.push('/workflow');
    } catch (err) {
      setSaveErrors({ general: err instanceof Error ? err.message : 'Save failed' });
    } finally {
      setSaving(false);
    }
  };

  /* Run workflow */
  const handleRun = async () => {
    if (!workflowName.trim()) { setNameError(true); return; }
    if (!isValid) return;
    setNameError(false);

    // Log all config before run
    wfLogger.saveGoal(goalContent, goalMode);
    wfLogger.saveConfig({
      workflow: { name: workflowName, stepsCount: steps.length },
      metrics,
      advanced: advancedConfig,
      permissions,
      tags,
    });
    wfLogger.logInteraction('workflow.run.started', {
      name: workflowName, goalMode, stepsCount: steps.length,
      localSteps: steps.filter(s => s.modelSource === 'local').length,
      providerSteps: steps.filter(s => s.modelSource === 'provider').length,
    });

    try {
      // Upload files (goal + input + per-step files/images)
      const formData = new FormData();
      if (goalMode === 'file' && goalFiles[0]) formData.append('files', goalFiles[0].file);
      if (goalMode === 'text') {
        const blob = new Blob([goalText], { type: 'text/markdown' });
        formData.append('files', blob, 'goal.md');
      }
      for (const f of inputFiles) formData.append('files', f.file);
      for (const s of steps) {
        for (const f of s.stepFiles) formData.append('files', f.file);
        for (const f of s.stepImages) formData.append('files', f.file);
      }

      const uploadResp = await fetch(`${ENGINE_URL}/api/orchestrator/upload`, { method: 'POST', body: formData });

      let goalFileUrl = '';
      const inputFileUrls: string[] = [];
      if (uploadResp.ok) {
        const uploadData = await uploadResp.json();
        const uploaded = uploadData.files || [];
        if (uploaded.length > 0) goalFileUrl = uploaded[0].url;
        for (let i = 1; i < uploaded.length; i++) inputFileUrls.push(uploaded[i].url);
      }

      const runId = `run-${Date.now()}`;
      ws.connect(runId);

      await fetch('/api/workflows/run', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: workflowName.trim(),
          goalFileUrl,
          inputFileUrls,
          steps: steps.map(s => {
            const expert = s.expert;
            const source = expert?.modelSource || s.modelSource;
            return {
              stepId: s.id,
              expertId: expert?.id || null,
              taskDescription: s.taskDescription,
              systemInstructions: s.systemInstructions || '',
              voiceCommand: s.voiceCommand || '',
              fileLocations: s.fileLocations,
              stepFileNames: s.stepFiles.map(f => f.name),
              stepImageNames: s.stepImages.map(f => f.name),
              modelSource: source,
              localModel: source === 'local'
                ? (expert?.localModelConfig
                    ? { engine: expert.localModelConfig.engine, model: expert.localModelConfig.modelName }
                    : { engine: s.localModel.engine, model: s.localModel.modelName })
                : null,
              temperature: s.temperature ?? (expert ? Number(expert.temperature) : 0.7),
              maxTokens: s.maxTokens ?? (expert?.maxTokens || 4096),
              connectionType: s.connectionType,
              integrations: s.integrations.map(si => ({
                id: si.id,
                type: si.type,
                referenceId: si.referenceId,
                name: si.name,
                icon: si.icon,
                color: si.color,
                config: si.config || {},
              })),
            };
          }),
          metrics,
          config: advancedConfig,
          tags,
          permissions,
        }),
      });
    } catch (err) {
      console.error('Failed to start workflow:', err);
      wfLogger.logInteraction('workflow.run.error', { error: String(err) });
    }
  };

  if (loadingExisting) {
    return (
      <div style={{ paddingLeft: 24, paddingRight: 24, paddingBottom: 24, paddingTop: 120, maxWidth: 1400, margin: '0 auto', textAlign: 'center' }}>
        <Loader2 size={20} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite' }} color="var(--text-3)" />
        <div style={{ fontSize: 13, color: 'var(--text-3)' }}>Loading workflow...</div>
      </div>
    );
  }

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {showSelector && (
        <ExpertSelectorModal onSelect={addExpert}
          onClose={() => { setShowSelector(false); setSwapIndex(null); }}
          allExperts={dbExperts} />
      )}

      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>Workflow Builder</h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Chain specialized agents to solve complex tasks — local or cloud models
          </p>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <button className="btn btn-secondary btn-sm" onClick={() => window.location.href = '/workflow/templates'}><FileText size={13} /> Templates</button>
          <button className="btn btn-secondary btn-sm" disabled={saving} onClick={handleSave}>
            {saving
              ? <><Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} /> Saving...</>
              : saveSuccess
              ? <><CheckCircle2 size={13} color="#059669" /> Saved</>
              : <><Save size={13} /> Save</>}
          </button>
          <button className="btn btn-primary btn-sm" onClick={handleRun} disabled={!isValid || isRunning}>
            {isRunning
              ? <><Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} /> Running...</>
              : <><Play size={13} /> Run Workflow</>}
          </button>
        </div>
      </div>

      {/* Workflow Name (MANDATORY) */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <label style={{ ...SECTION_TITLE, color: nameError ? 'var(--error)' : 'var(--text-3)' }}>
          Workflow Name <span style={{ color: 'var(--error)' }}>*</span>
        </label>
        <input className="input" style={{ maxWidth: 480, borderColor: nameError ? 'var(--error)' : undefined }}
          placeholder="Give your workflow a name (required)" value={workflowName}
          onChange={e => { setWorkflowName(e.target.value);
            if (e.target.value.trim()) { setNameError(false); setSaveErrors(prev => { const { name, ...rest } = prev; return rest; }); }
            wfLogger.logInteraction('name.changed', { name: e.target.value }); }}
          onBlur={() => { if (!workflowName.trim()) setNameError(true); }} />
        {(nameError || saveErrors.name) && <div style={{ fontSize: 11, color: 'var(--error)', marginTop: 4, display: 'flex', alignItems: 'center', gap: 4 }}>
          <AlertCircle size={11} /> {saveErrors.name || 'Workflow name is required'}
        </div>}
        {saveErrors.general && <div style={{ fontSize: 11, color: 'var(--error)', marginTop: 4, display: 'flex', alignItems: 'center', gap: 4 }}>
          <AlertCircle size={11} /> {saveErrors.general}
        </div>}
      </div>

      {/* Goal + Input Files */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16, marginBottom: 16 }}>
        {/* Goal — dual input: text or file */}
        <div className="card" style={{ padding: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 10 }}>
            <span style={SECTION_TITLE}>Task Goal (Markdown) <span style={{ color: 'var(--error)' }}>*</span></span>
            <div style={{ display: 'flex', gap: 4 }}>
              {(['text', 'file'] as const).map(mode => (
                <button key={mode} onClick={() => { setGoalMode(mode); wfLogger.logInteraction('goal.mode.changed', { mode }); }}
                  style={{
                    padding: '3px 10px', borderRadius: 4, fontSize: 10, fontWeight: 600,
                    border: '1px solid', cursor: 'pointer', textTransform: 'uppercase',
                    background: goalMode === mode ? 'var(--primary-dim)' : 'transparent',
                    borderColor: goalMode === mode ? 'var(--primary)' : 'var(--border)',
                    color: goalMode === mode ? 'var(--primary-text)' : 'var(--text-4)',
                    transition: 'all 0.12s',
                  }}>
                  {mode === 'text' ? 'Write' : 'Upload'}
                </button>
              ))}
            </div>
          </div>

          {goalMode === 'text' ? (
            <textarea className="textarea" style={{ minHeight: 140, fontSize: 13, fontFamily: 'var(--font-mono, monospace)', lineHeight: 1.6 }}
              placeholder="Write your task goal in markdown format...&#10;&#10;## Objective&#10;Describe what you want to accomplish...&#10;&#10;## Requirements&#10;- Requirement 1&#10;- Requirement 2"
              value={goalText}
              onChange={e => { setGoalText(e.target.value); }} />
          ) : (
            <FileDropZone label="" accept=".md,.markdown,.txt" files={goalFiles}
              onFilesChange={f => { setGoalFiles(f); if (f[0]?.preview) wfLogger.saveGoal(f[0].preview, 'file'); }}
              multiple={false} />
          )}

          {goalMode === 'file' && goalFiles[0]?.preview && (
            <div style={{ marginTop: 10, padding: '10px 12px', background: 'var(--bg)', border: '1px solid var(--border)',
              borderRadius: 4, maxHeight: 150, overflowY: 'auto' }}>
              <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-3)', marginBottom: 4, textTransform: 'uppercase', letterSpacing: '0.08em' }}>Preview</div>
              <pre style={{ fontSize: 11, color: 'var(--text-2)', lineHeight: 1.5, whiteSpace: 'pre-wrap', wordBreak: 'break-word', margin: 0, fontFamily: 'inherit' }}>
                {goalFiles[0].preview}
              </pre>
            </div>
          )}

          {hasGoal && (
            <div style={{ marginTop: 10 }}>
              <button className="btn btn-secondary btn-sm" onClick={suggestChain} title="Auto-suggest chain">
                <Sparkles size={13} /> Suggest Chain
              </button>
            </div>
          )}
          {saveErrors.goal && (
            <div style={{ fontSize: 11, color: 'var(--error)', marginTop: 8, display: 'flex', alignItems: 'center', gap: 4 }}>
              <AlertCircle size={11} /> {saveErrors.goal}
            </div>
          )}
        </div>

        {/* Input Files */}
        <div className="card" style={{ padding: 20 }}>
          <div style={{ ...SECTION_TITLE, marginBottom: 10 }}>Input Files (Context)</div>
          <FileDropZone label="" accept=".pdf,.txt,.csv,.json,.md,.jsonl,.py,.go,.ts,.tsx,.js"
            files={inputFiles} onFilesChange={f => {
              setInputFiles(f);
              wfLogger.logInteraction('input.files.changed', { count: f.length, totalBytes: f.reduce((s, x) => s + x.size, 0) });
            }} multiple={true} />
          {inputFiles.length > 0 && (
            <div style={{ marginTop: 8, fontSize: 11, color: 'var(--text-4)' }}>
              {inputFiles.length} file{inputFiles.length > 1 ? 's' : ''} · {formatBytes(inputFiles.reduce((s, f) => s + f.size, 0))} total
            </div>
          )}
        </div>
      </div>

      {/* Estimation bar */}
      {steps.length > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 24, padding: '12px 20px',
          background: 'var(--bg-card)', border: '1px solid var(--border)', borderRadius: 6, marginBottom: 16 }}>
          <span style={SECTION_TITLE}>Estimation</span>
          <div style={{ display: 'flex', gap: 24, flex: 1 }}>
            {[
              { icon: Layers, label: 'Steps', value: steps.length.toString(), color: 'var(--primary-text)' },
              { icon: Zap, label: 'Max Tokens', value: fmt(totalTokens), color: 'var(--amber)' },
              { icon: Clock, label: 'Est. Time', value: `~${totalSec}s`, color: 'var(--text-2)' },
              { icon: Server, label: 'Local', value: String(steps.filter(s => s.modelSource === 'local').length), color: '#059669' },
              { icon: Cloud, label: 'Provider', value: String(steps.filter(s => s.modelSource === 'provider').length), color: '#2563EB' },
            ].map(item => (
              <div key={item.label} style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                <item.icon size={13} color={item.color} />
                <span className="mono" style={{ fontSize: 14, fontWeight: 700, color: item.color }}>{item.value}</span>
                <span style={{ fontSize: 11, color: 'var(--text-3)' }}>{item.label}</span>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Workflow canvas */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>Workflow Steps</div>
            {/* Global parallel toggle */}
            <button
              onClick={() => {
                const next = !globalParallel;
                setGlobalParallel(next);
                // Apply to all existing steps
                setSteps(prev => prev.map(s => ({ ...s, connectionType: next ? 'parallel' : 'sequential' })));
                wfLogger.logInteraction('parallel.toggled', { enabled: next });
              }}
              style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '4px 10px', borderRadius: 20, fontSize: 10, fontWeight: 600,
                border: `1px solid ${globalParallel ? '#7C3AED' : 'var(--border)'}`,
                background: globalParallel ? 'rgba(124,58,237,0.1)' : 'transparent',
                color: globalParallel ? '#7C3AED' : 'var(--text-4)',
                cursor: 'pointer', transition: 'all 0.15s',
              }}
              title={globalParallel ? 'All steps run in parallel' : 'Steps run sequentially — click to enable parallel'}
            >
              {globalParallel ? <ToggleRight size={13} /> : <ToggleLeft size={13} />}
              {globalParallel ? 'Parallel On' : 'Sequential'}
            </button>
          </div>
          <div style={{ display: 'flex', gap: 6 }}>
            {steps.length > 0 && (
              <button className="btn btn-ghost btn-sm" onClick={() => { setSteps([]); wfLogger.logInteraction('steps.reset'); }}>
                <RotateCcw size={12} /> Reset
              </button>
            )}
            <button className="btn btn-secondary btn-sm" onClick={() => setShowSelector(true)}>
              <Plus size={12} /> Add Expert
            </button>
          </div>
        </div>
        {saveErrors.steps && (
          <div style={{ marginBottom: 12 }}>
            <div style={{ fontSize: 11, color: 'var(--error)', display: 'flex', alignItems: 'center', gap: 4, marginBottom: 4 }}>
              <AlertCircle size={11} /> {saveErrors.steps}
            </div>
            {saveErrors.stepDetails && Object.entries(saveErrors.stepDetails).map(([stepId, msg]) => (
              <div key={stepId} style={{ fontSize: 10, color: 'var(--error)', paddingLeft: 16, marginTop: 2 }}>
                {msg}
              </div>
            ))}
          </div>
        )}
        {steps.length === 0 ? (
          <div style={{ display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center',
            padding: '60px 20px', border: '1px dashed var(--border-md)', borderRadius: 6, textAlign: 'center', gap: 16 }}>
            <div style={{ width: 56, height: 56, borderRadius: 12, background: 'var(--bg-elevated)',
              border: '1px solid var(--border-md)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Users size={24} color="var(--text-3)" />
            </div>
            <div>
              <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>No experts chained yet</div>
              <div style={{ fontSize: 13, color: 'var(--text-3)', lineHeight: 1.5, maxWidth: 420 }}>
                Deploy experts from the <strong style={{ color: 'var(--text-2)' }}>Expert Catalog</strong> (local or provider),
                then chain them here. Each expert gets system instructions, files, images, and voice commands.
              </div>
            </div>
            <button className="btn btn-primary" onClick={() => setShowSelector(true)}>
              <Users size={14} /> Add Expert Step
            </button>
          </div>
        ) : (
          <div style={{ overflowX: 'auto', paddingBottom: 8 }}>
            <div style={{ display: 'flex', alignItems: 'flex-start', gap: 0, minWidth: 'max-content' }}>
              {steps.map((step, idx) => (
                <div key={step.id} style={{ display: 'flex', alignItems: 'flex-start' }}>
                  <StepCard step={step} index={idx} onRemove={() => removeStep(step.id)}
                    onUpdate={updates => updateStep(step.id, updates)} onSwap={() => openSwap(idx)} />
                  {idx < steps.length - 1 && (
                    <div className="step-connector" style={{ alignSelf: 'center', paddingTop: 0 }}><ArrowRight size={16} /></div>
                  )}
                </div>
              ))}
              <div style={{ display: 'flex', alignItems: 'center' }}>
                {steps.length > 0 && <div className="step-connector"><ArrowRight size={16} /></div>}
                <button
                  onClick={() => setShowSelector(true)}
                  style={{
                    width: 48, height: 48, borderRadius: 8,
                    border: '2px dashed var(--border-md)', background: 'transparent',
                    cursor: 'pointer', display: 'flex', flexDirection: 'column',
                    alignItems: 'center', justifyContent: 'center', gap: 2,
                    color: 'var(--text-3)', transition: 'all 0.15s',
                    alignSelf: 'flex-start', marginTop: 40,
                  }}
                  onMouseEnter={e => { e.currentTarget.style.borderColor = 'var(--primary)'; e.currentTarget.style.color = 'var(--primary-text)'; e.currentTarget.style.background = 'var(--primary-dim)'; }}
                  onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--border-md)'; e.currentTarget.style.color = 'var(--text-3)'; e.currentTarget.style.background = 'transparent'; }}
                  title="Add expert step"
                >
                  <Plus size={16} />
                </button>
              </div>
            </div>
          </div>
        )}
      </div>

      {/* Advanced Options */}
      <AdvancedOptionsPanel
        metrics={metrics} onMetricsChange={setMetrics}
        config={advancedConfig} onConfigChange={setAdvancedConfig}
        tags={tags} onTagsChange={setTags}
        permissions={permissions} onPermissionsChange={setPermissions}
        logger={wfLogger} workflowId={workflowIdRef.current}
      />

      {/* Live Execution Panel */}
      {ws.status !== 'idle' && (
        <LiveExecutionPanel status={ws.status} agents={ws.agents} events={ws.events} output={ws.output} error={ws.error} />
      )}

      {/* Templates */}
      {ws.status === 'idle' && dbWorkflows.length > 0 && (
        <div className="card" style={{ padding: 20 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', marginBottom: 14,
            display: 'flex', alignItems: 'center', gap: 8 }}>
            <FileText size={14} color="var(--text-3)" /> Workflow Templates
          </div>
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(240px, 1fr))', gap: 10 }}>
            {dbWorkflows.map((wf: Record<string, any>) => (
              <div key={wf.id} className="card-hover" style={{ padding: 14, cursor: 'pointer' }}
                onClick={() => { setWorkflowName(wf.name); wfLogger.logInteraction('template.selected', { templateId: wf.id }); }}>
                <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', marginBottom: 4 }}>{wf.name}</div>
                <div style={{ fontSize: 12, color: 'var(--text-3)', lineHeight: 1.4, marginBottom: 10 }}>{wf.description}</div>
                <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                  <div style={{ display: 'flex', gap: 10, fontSize: 11, color: 'var(--text-3)' }}>
                    <span className="mono">{fmt(wf.estimatedTokens)} tok</span>
                    <span>{wf.totalRuns} runs</span>
                  </div>
                  {wf.totalRuns > 0 && (
                    <span className="badge badge-success" style={{ fontSize: 10 }}>
                      <CheckCircle2 size={9} /> {((wf.successfulRuns / wf.totalRuns) * 100).toFixed(0)}%
                    </span>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}

export default function WorkflowBuilderPage() {
  return (
    <Suspense fallback={
      <div style={{ paddingLeft: 24, paddingRight: 24, paddingBottom: 24, paddingTop: 120, maxWidth: 1400, margin: '0 auto', textAlign: 'center' }}>
        <Loader2 size={20} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite' }} color="var(--text-3)" />
        <div style={{ fontSize: 13, color: 'var(--text-3)' }}>Loading...</div>
      </div>
    }>
      <WorkflowBuilderInner />
    </Suspense>
  );
}
