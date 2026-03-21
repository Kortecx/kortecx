'use client';

import { useState, useCallback, useEffect } from 'react';
import Link from 'next/link';
import {
  Plug, ChevronRight, Check, X,
  Key, Loader2, Eye, EyeOff, ExternalLink, Shield, Terminal,
  Bot, Sparkles, Gem, Route, Zap, Wind, Smile, Compass, Atom,
  Search, Download, Trash2, HardDrive, Server, Cpu,
} from 'lucide-react';
import { PROVIDERS } from '@/lib/constants';
import type { AIProvider } from '@/lib/types';

/* ─── Provider Icon Resolver ─────────────────────── */
const PROVIDER_ICON_MAP: Record<string, React.ComponentType<{ size?: number; color?: string }>> = {
  Bot, Sparkles, Gem, Route, Zap, Wind, Smile, Compass, Atom,
};

function ProviderIcon({ name, size = 18, color }: { name: string; size?: number; color?: string }) {
  const Icon = PROVIDER_ICON_MAP[name] || Plug;
  return <Icon size={size} color={color} />;
}

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

/* ─── Provider-specific hints ──────────────────────── */
const PROVIDER_HINTS: Record<string, { keyPrefix: string; docsUrl: string; envVar: string; note?: string }> = {
  anthropic:    { keyPrefix: 'sk-ant-',      docsUrl: 'https://console.anthropic.com/settings/keys', envVar: 'ANTHROPIC_API_KEY' },
  openai:       { keyPrefix: 'sk-',          docsUrl: 'https://platform.openai.com/api-keys',       envVar: 'OPENAI_API_KEY' },
  google:       { keyPrefix: 'AIza',         docsUrl: 'https://aistudio.google.com/apikey',         envVar: 'GOOGLE_API_KEY' },
  openrouter:   { keyPrefix: 'sk-or-',       docsUrl: 'https://openrouter.ai/keys',                 envVar: 'OPENROUTER_API_KEY' },
  groq:         { keyPrefix: 'gsk_',         docsUrl: 'https://console.groq.com/keys',              envVar: 'GROQ_API_KEY' },
  mistral:      { keyPrefix: '',             docsUrl: 'https://console.mistral.ai/api-keys',        envVar: 'MISTRAL_API_KEY' },
  huggingface:  { keyPrefix: 'hf_',          docsUrl: 'https://huggingface.co/settings/tokens',     envVar: 'HF_TOKEN', note: 'Also enables dataset downloads and model hub access' },
  deepseek:     { keyPrefix: 'sk-',          docsUrl: 'https://platform.deepseek.com/api_keys',     envVar: 'DEEPSEEK_API_KEY' },
  xai:          { keyPrefix: 'xai-',         docsUrl: 'https://console.x.ai/',                      envVar: 'XAI_API_KEY' },
};

/* ─── Connect Modal ────────────────────────────────── */
function ConnectModal({
  provider,
  onClose,
  onConnect,
}: {
  provider: AIProvider;
  onClose: () => void;
  onConnect: (providerId: string, apiKey: string) => Promise<void>;
}) {
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const hints = PROVIDER_HINTS[provider.id] ?? { keyPrefix: '', docsUrl: '', envVar: '' };

  const handleSave = async () => {
    if (!apiKey.trim()) { setError('API key is required'); return; }
    setSaving(true);
    setError('');
    try {
      await onConnect(provider.id, apiKey.trim());
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Connection failed');
    } finally {
      setSaving(false);
    }
  };

  return (
    <div style={{
      position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
      backdropFilter: 'blur(4px)',
      zIndex: 200, display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
      paddingTop: 80,
    }} onClick={onClose}>
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 480, maxWidth: '92vw',
          background: 'var(--bg-surface)', border: '1px solid var(--border)',
          borderRadius: 12, overflow: 'hidden',
          boxShadow: '0 24px 64px rgba(0,0,0,0.3)',
        }}
      >
        {/* Header */}
        <div style={{
          padding: '18px 22px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', gap: 12,
        }}>
          <div style={{
            width: 36, height: 36, borderRadius: 8,
            background: `${provider.color}14`, border: `1px solid ${provider.color}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <ProviderIcon name={provider.icon} size={18} color={provider.color} />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
              Connect {provider.name}
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-3)' }}>{provider.description}</div>
          </div>
          <button onClick={onClose} style={{
            background: 'none', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', display: 'flex', padding: 4,
          }}>
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div style={{ padding: '20px 22px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          {/* API Key input */}
          <div>
            <label style={{
              fontSize: 11, fontWeight: 600, color: 'var(--text-3)',
              display: 'flex', alignItems: 'center', gap: 6, marginBottom: 6,
            }}>
              <Key size={11} /> API Key
            </label>
            <div style={{ display: 'flex', gap: 6 }}>
              <div style={{
                flex: 1, display: 'flex', alignItems: 'center',
                border: '1px solid var(--border-md)', borderRadius: 6,
                background: 'var(--bg)', overflow: 'hidden',
              }}>
                <input
                  type={showKey ? 'text' : 'password'}
                  className="input"
                  style={{
                    flex: 1, border: 'none', background: 'none', fontSize: 13,
                    fontFamily: 'var(--font-mono, monospace)',
                  }}
                  placeholder={hints.keyPrefix ? `${hints.keyPrefix}...` : 'Enter API key'}
                  value={apiKey}
                  onChange={e => setApiKey(e.target.value)}
                  onKeyDown={e => e.key === 'Enter' && handleSave()}
                />
                <button
                  onClick={() => setShowKey(prev => !prev)}
                  style={{
                    background: 'none', border: 'none', cursor: 'pointer',
                    color: 'var(--text-4)', padding: '0 10px', display: 'flex',
                  }}
                >
                  {showKey ? <EyeOff size={14} /> : <Eye size={14} />}
                </button>
              </div>
            </div>
          </div>

          {/* Env var hint */}
          {hints.envVar && (
            <div style={{
              padding: '8px 12px', borderRadius: 6,
              background: 'var(--bg-elevated)', border: '1px solid var(--border)',
              fontSize: 11, color: 'var(--text-3)',
              display: 'flex', alignItems: 'center', gap: 8,
            }}>
              <Terminal size={12} color="var(--text-4)" />
              <span>
                Also set as <code style={{
                  padding: '1px 5px', borderRadius: 3,
                  background: 'var(--bg)', border: '1px solid var(--border)',
                  fontFamily: 'var(--font-mono, monospace)', fontSize: 10,
                }}>{hints.envVar}</code> in your environment
              </span>
            </div>
          )}

          {/* Special note */}
          {hints.note && (
            <div style={{
              padding: '8px 12px', borderRadius: 6,
              background: `${provider.color}08`, border: `1px solid ${provider.color}20`,
              fontSize: 11, color: 'var(--text-2)',
            }}>
              {hints.note}
            </div>
          )}

          {/* Security note */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 6,
            fontSize: 11, color: 'var(--text-4)',
          }}>
            <Shield size={11} />
            Keys are encrypted at rest and never exposed in logs
          </div>

          {error && (
            <div style={{ fontSize: 12, color: '#ef4444', display: 'flex', alignItems: 'center', gap: 6 }}>
              <X size={12} /> {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          padding: '14px 22px', borderTop: '1px solid var(--border)',
          display: 'flex', gap: 8, justifyContent: 'space-between', alignItems: 'center',
        }}>
          {hints.docsUrl && (
            <a
              href={hints.docsUrl}
              target="_blank"
              rel="noopener noreferrer"
              style={{
                fontSize: 12, color: provider.color,
                textDecoration: 'none', display: 'flex', alignItems: 'center', gap: 4,
              }}
            >
              Get API key <ExternalLink size={10} />
            </a>
          )}
          <div style={{ display: 'flex', gap: 8, marginLeft: 'auto' }}>
            <button onClick={onClose} style={{
              padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 500,
              border: '1px solid var(--border-md)', background: 'transparent',
              color: 'var(--text-3)', cursor: 'pointer',
            }}>Cancel</button>
            <button onClick={handleSave} disabled={saving || !apiKey.trim()} style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 18px', borderRadius: 7, fontSize: 12, fontWeight: 700,
              border: `1.5px solid ${provider.color}`,
              background: `${provider.color}14`, color: provider.color,
              cursor: saving ? 'wait' : 'pointer', opacity: saving || !apiKey.trim() ? 0.5 : 1,
            }}>
              {saving ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Plug size={12} />}
              Connect
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

/* ─── Page ─────────────────────────────────────────── */
export default function ProvidersPage() {
  const [filter, setFilter] = useState<'all' | 'connected' | 'available'>('all');
  const [connectProvider, setConnectProvider] = useState<AIProvider | null>(null);
  const [connectedProviders, setConnectedProviders] = useState<Set<string>>(new Set());

  /* Load persisted connection status from the API on mount */
  useEffect(() => {
    fetch('/api/providers')
      .then(r => r.json())
      .then(data => {
        const ids = (data.providers ?? [])
          .filter((p: { connected?: boolean }) => p.connected)
          .map((p: { id: string }) => p.id);
        if (ids.length) setConnectedProviders(new Set(ids));
      })
      .catch(() => {/* ignore — falls back to empty set */});
  }, []);

  const handleConnect = useCallback(async (providerId: string, apiKey: string) => {
    const res = await fetch('/api/providers', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ providerId, apiKey }),
    });
    if (!res.ok) throw new Error('Connection failed');
    setConnectedProviders(prev => new Set(prev).add(providerId));
  }, []);

  const handleDisconnect = useCallback(async (providerId: string) => {
    const res = await fetch(`/api/providers?providerId=${providerId}`, { method: 'DELETE' });
    if (!res.ok) return;
    setConnectedProviders(prev => {
      const next = new Set(prev);
      next.delete(providerId);
      return next;
    });
  }, []);

  const isConnected = (id: string) => connectedProviders.has(id);

  const filtered = PROVIDERS.filter(p => {
    const connected = p.connected || isConnected(p.id);
    if (filter === 'connected') return connected;
    if (filter === 'available') return !connected;
    return true;
  });

  const connectedCount = PROVIDERS.filter(p => p.connected || isConnected(p.id)).length;

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Providers
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Manage local models, cloud providers, and API keys for inference
          </p>
        </div>
        <Link href="/providers/keys">
          <button className="btn btn-secondary btn-sm">
            <Key size={13} /> Manage Keys
          </button>
        </Link>
      </div>

      {/* ── Local Models Section (above providers) ── */}
      <ModelsSection />

      {/* ── Provider Connections ── */}
      <div style={{ marginTop: 32, marginBottom: 16 }}>
        <h2 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: '0 0 4px', display: 'flex', alignItems: 'center', gap: 8 }}>
          <Plug size={18} /> Provider Connections
        </h2>
        <p style={{ fontSize: 12, color: 'var(--text-3)', margin: 0 }}>
          {connectedCount} of {PROVIDERS.length} providers connected — connect via API key to enable inference
        </p>
      </div>

      {/* Filter tabs */}
      <div style={{ display: 'flex', gap: 6, marginBottom: 20 }}>
        {(['all', 'connected', 'available'] as const).map(f => (
          <button
            key={f}
            onClick={() => setFilter(f)}
            style={{
              padding: '5px 14px',
              borderRadius: 20,
              fontSize: 12,
              fontWeight: 500,
              border: '1px solid',
              cursor: 'pointer',
              background: filter === f ? 'var(--primary-dim)' : 'var(--bg-card)',
              borderColor: filter === f ? 'var(--primary)' : 'var(--border)',
              color: filter === f ? 'var(--primary-text)' : 'var(--text-2)',
              textTransform: 'capitalize',
            }}
          >
            {f}
          </button>
        ))}
      </div>

      {/* Provider Grid */}
      <div style={{
        display: 'grid',
        gridTemplateColumns: 'repeat(auto-fill, minmax(340px, 1fr))',
        gap: 12,
        marginBottom: 32,
      }}>
        {filtered.map(provider => {
          const connected = provider.connected || isConnected(provider.id);
          return (
            <div key={provider.id} className="card" style={{ padding: 20, position: 'relative' }}>
              {/* Status dot — top right */}
              <div style={{
                position: 'absolute', top: 12, right: 12,
                width: 9, height: 9, borderRadius: '50%',
                background: connected ? '#059669' : '#DC2626',
                boxShadow: connected ? '0 0 0 3px rgba(5,150,105,0.15)' : '0 0 0 3px rgba(220,38,38,0.12)',
              }} title={connected ? 'Connected' : 'Not connected'} />

              {/* Provider Header */}
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 14 }}>
                <div style={{
                  width: 40, height: 40, borderRadius: 8,
                  background: `${provider.color}12`,
                  border: `1px solid ${provider.color}30`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  flexShrink: 0,
                }}>
                  <ProviderIcon name={provider.icon} size={20} color={provider.color} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span style={{ fontSize: 15, fontWeight: 600, color: 'var(--text-1)' }}>
                      {provider.name}
                    </span>
                  </div>
                  <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0', lineHeight: 1.5 }}>
                    {provider.description}
                  </p>
                </div>
              </div>

              {/* Stats (connected only) — Latency & Tokens/mo */}
              {connected && (
                <div style={{ display: 'grid', gridTemplateColumns: 'repeat(2,1fr)', gap: 8, marginBottom: 14 }}>
                  <div style={{
                    padding: '8px', background: 'var(--bg)',
                    border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
                  }}>
                    <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
                      {provider.latencyMs ?? '\u2014'}ms
                    </div>
                    <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>Latency</div>
                  </div>
                  <div style={{
                    padding: '8px', background: 'var(--bg)',
                    border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
                  }}>
                    <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
                      {fmt(provider.monthlyTokensUsed ?? 0)}
                    </div>
                    <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>Tokens/mo</div>
                  </div>
                </div>
              )}

              {/* Action */}
              <div style={{ display: 'flex', gap: 6 }}>
                {connected ? (
                  <>
                    <button className="btn btn-secondary btn-sm" style={{ flex: 1, justifyContent: 'center' }}>
                      Configure <ChevronRight size={12} />
                    </button>
                    <button
                      className="btn btn-ghost btn-sm"
                      style={{ color: '#DC2626' }}
                      onClick={() => handleDisconnect(provider.id)}
                    >
                      Disconnect
                    </button>
                  </>
                ) : (
                  <button
                    className="btn btn-primary btn-sm"
                    style={{ flex: 1, justifyContent: 'center' }}
                    onClick={() => setConnectProvider(provider)}
                  >
                    <Key size={12} /> Connect with API Key
                  </button>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {/* Connect Modal */}
      {connectProvider && (
        <ConnectModal
          provider={connectProvider}
          onClose={() => setConnectProvider(null)}
          onConnect={handleConnect}
        />
      )}
    </div>
  );
}

/* ─── Models Section ──────────────────────────────── */
interface LocalModel {
  name: string;
  size?: number;
  modified_at?: string;
  source: string;
  local?: boolean;
  digest?: string;
  // HF search result fields
  downloads?: number;
  likes?: number;
  pipeline_tag?: string;
}

function ModelsSection() {
  const [source, setSource] = useState<'ollama' | 'llamacpp'>('ollama');
  const [localModels, setLocalModels] = useState<{ ollama: LocalModel[]; llamacpp: LocalModel[] }>({ ollama: [], llamacpp: [] });
  const [searchQuery, setSearchQuery] = useState('');
  const [searchResults, setSearchResults] = useState<LocalModel[]>([]);
  const [searching, setSearching] = useState(false);
  const [pulling, setPulling] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [notice, setNotice] = useState<{ type: 'success' | 'error'; msg: string } | null>(null);

  // Fetch local models on mount
  const fetchLocal = useCallback(async () => {
    try {
      const res = await fetch('/api/models');
      if (res.ok) {
        const d = await res.json();
        setLocalModels({
          ollama: (d.ollama ?? []).map((m: { name: string; size?: number; modified_at?: string; digest?: string }) => ({ ...m, source: 'ollama', local: true })),
          llamacpp: (d.llamacpp ?? []).map((m: { name: string; size?: number }) => ({ ...m, source: 'llamacpp', local: true })),
        });
      }
    } catch { /* ignore */ }
  }, []);

  // Fetch on mount + poll every 15s for download completions
  useEffect(() => {
    requestAnimationFrame(() => fetchLocal());
    const interval = setInterval(fetchLocal, 15000);
    return () => clearInterval(interval);
  }, [fetchLocal]);

  // Auto-dismiss notice
  useEffect(() => {
    if (notice) { const t = setTimeout(() => setNotice(null), 5000); return () => clearTimeout(t); }
  }, [notice]);

  const handleSearch = async () => {
    if (!searchQuery.trim()) return;
    setSearching(true);
    setSearchResults([]);
    try {
      const res = await fetch('/api/models', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'search', query: searchQuery, source, limit: 20 }),
      });
      const d = await res.json();
      setSearchResults((d.models ?? []).map((m: { name?: string; id?: string; size?: number; downloads?: number; likes?: number; pipeline_tag?: string; local?: boolean }) => ({
        name: m.name || m.id || '',
        size: m.size,
        downloads: m.downloads,
        likes: m.likes,
        pipeline_tag: m.pipeline_tag,
        source,
        local: m.local ?? false,
      })));
    } catch {
      setNotice({ type: 'error', msg: 'Search failed' });
    }
    setSearching(false);
  };

  const handlePull = async (modelName: string) => {
    setPulling(modelName);
    try {
      const res = await fetch('/api/models', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'pull', engine: source, model: modelName }),
      });
      const d = await res.json();
      if (d.error) {
        setNotice({ type: 'error', msg: d.error });
      } else {
        setNotice({ type: 'success', msg: `Pulling ${modelName} — this may take a few minutes` });
        // Refresh local models after a delay
        setTimeout(fetchLocal, 5000);
      }
    } catch {
      setNotice({ type: 'error', msg: 'Pull failed' });
    }
    setPulling(null);
  };

  const handleDelete = async (modelName: string, engine: string) => {
    setDeleting(modelName);
    try {
      const res = await fetch('/api/models', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'delete', engine, model: modelName }),
      });
      const d = await res.json();
      if (d.deleted) {
        setNotice({ type: 'success', msg: `Deleted ${modelName}` });
        fetchLocal();
      } else {
        setNotice({ type: 'error', msg: d.error || 'Delete failed' });
      }
    } catch {
      setNotice({ type: 'error', msg: 'Delete failed' });
    }
    setDeleting(null);
  };

  const fmtSize = (bytes?: number) => {
    if (!bytes) return '';
    if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
    if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
    return `${bytes} B`;
  };

  const displayModels = source === 'llamacpp' ? localModels.llamacpp : source === 'ollama' ? localModels.ollama : [];
  const isLocalSource = source === 'ollama' || source === 'llamacpp';

  return (
    <div style={{ marginTop: 32 }}>
      {/* Section header */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
        <div>
          <h2 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
            <HardDrive size={18} /> Local Models
          </h2>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
            Download and manage models for Ollama, llama.cpp, and HuggingFace
          </p>
        </div>
      </div>

      {/* Source tabs */}
      <div style={{ display: 'flex', gap: 6, marginBottom: 16 }}>
        {([
          { id: 'ollama' as const, label: 'Ollama', icon: Server, color: '#10B981' },
          { id: 'llamacpp' as const, label: 'llama.cpp', icon: Cpu, color: '#3B82F6' },
        ]).map(s => (
          <button key={s.id} onClick={() => { setSource(s.id); setSearchResults([]); setSearchQuery(''); }} style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: source === s.id ? 600 : 400,
            border: `1px solid ${source === s.id ? s.color : 'var(--border)'}`,
            background: source === s.id ? `${s.color}12` : 'transparent',
            color: source === s.id ? s.color : 'var(--text-3)',
            cursor: 'pointer',
          }}>
            <s.icon size={13} />
            {s.label}
            {(
              <span style={{ fontSize: 10, fontWeight: 700, opacity: 0.6 }}>
                ({s.id === 'ollama' ? localModels.ollama.length : localModels.llamacpp.length})
              </span>
            )}
          </button>
        ))}
      </div>

      {/* Notice */}
      {notice && (
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8, padding: '8px 14px', marginBottom: 12,
          borderRadius: 6, fontSize: 12, fontWeight: 500,
          background: notice.type === 'success' ? 'rgba(5,150,105,0.06)' : 'rgba(220,38,38,0.06)',
          border: `1px solid ${notice.type === 'success' ? 'rgba(5,150,105,0.15)' : 'rgba(220,38,38,0.15)'}`,
          color: notice.type === 'success' ? '#059669' : '#DC2626',
        }}>
          {notice.type === 'success' ? <Check size={13} /> : <X size={13} />}
          {notice.msg}
          <button onClick={() => setNotice(null)} style={{ marginLeft: 'auto', background: 'none', border: 'none', cursor: 'pointer', color: 'inherit', display: 'flex', padding: 0 }}>
            <X size={11} />
          </button>
        </div>
      )}

      {/* Search bar */}
      <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
        <div style={{
          flex: 1, display: 'flex', alignItems: 'center', gap: 8,
          padding: '0 12px', borderRadius: 6,
          border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
        }}>
          <Search size={14} color="var(--text-4)" />
          <input
            className="input"
            style={{ border: 'none', padding: '8px 0', fontSize: 13, flex: 1, background: 'none' }}
            placeholder={source === 'ollama' ? 'Search Ollama models (e.g. llama3.1, mistral, codellama)...' : 'Search loaded models...'}
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            onKeyDown={e => e.key === 'Enter' && handleSearch()}
          />
        </div>
        <button className="btn btn-primary btn-sm" onClick={handleSearch} disabled={searching || !searchQuery.trim()}>
          {searching ? <Loader2 size={13} className="spin" /> : <Search size={13} />}
          {searching ? 'Searching...' : 'Search'}
        </button>
      </div>

      {/* Search results */}
      {searchResults.length > 0 && (
        <div style={{ marginBottom: 20 }}>
          <div style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>
            Search Results ({searchResults.length})
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {searchResults.map((m, i) => (
              <div key={`${m.name}-${i}`} className="card" style={{ padding: '10px 14px' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: '#0d0d0d' }} className="mono">{m.name}</div>
                    <div style={{ display: 'flex', gap: 8, fontSize: 11, color: 'var(--text-3)', marginTop: 2 }}>
                      {m.pipeline_tag && <span style={{ padding: '1px 6px', borderRadius: 4, background: 'var(--bg-elevated)', fontSize: 10 }}>{m.pipeline_tag}</span>}
                      {m.downloads !== undefined && <span>{m.downloads.toLocaleString()} downloads</span>}
                      {m.likes !== undefined && <span>{m.likes} likes</span>}
                      {m.size ? <span>{fmtSize(m.size)}</span> : null}
                    </div>
                  </div>
                  {m.local ? (
                    <span style={{ fontSize: 10, fontWeight: 600, color: '#059669', padding: '2px 8px', borderRadius: 10, background: 'rgba(5,150,105,0.08)' }}>
                      Downloaded
                    </span>
                  ) : (
                    <button
                      className="btn btn-primary btn-sm"
                      onClick={() => handlePull(m.name)}
                      disabled={pulling === m.name}
                      style={{ display: 'flex', alignItems: 'center', gap: 4 }}
                    >
                      {pulling === m.name ? <Loader2 size={12} className="spin" /> : <Download size={12} />}
                      {pulling === m.name ? 'Pulling...' : 'Download'}
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}


      {/* llama.cpp instructions */}
      {source === 'llamacpp' && localModels.llamacpp.length === 0 && (
        <div className="card" style={{ padding: 20, marginBottom: 16 }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: '#0d0d0d', marginBottom: 8 }}>Loading Models into llama.cpp</div>
          <div style={{ fontSize: 12, color: 'var(--text-3)', lineHeight: 1.7 }}>
            <p style={{ margin: '0 0 8px' }}>llama.cpp loads GGUF model files directly. To add a model:</p>
            <ol style={{ margin: 0, paddingLeft: 20 }}>
              <li>Download a GGUF file from HuggingFace (search above with HuggingFace tab)</li>
              <li>Start llama.cpp server with the model: <code style={{ padding: '1px 5px', borderRadius: 3, background: 'var(--bg-elevated)', fontSize: 11 }}>./llama-server -m model.gguf --port 8080</code></li>
              <li>The loaded model will appear here automatically</li>
            </ol>
            <p style={{ margin: '8px 0 0', fontSize: 11, color: 'var(--text-4)' }}>
              Ensure your llama.cpp server is running on port 8080 (configurable in engine settings).
            </p>
          </div>
        </div>
      )}

      {/* Local model list */}
      {isLocalSource && displayModels.length > 0 && (
        <div>
          <div style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 8 }}>
            Installed Models ({displayModels.length})
          </div>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {displayModels.map((m, i) => (
              <div key={`${m.name}-${i}`} className="card" style={{ padding: '10px 14px' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                  <div style={{
                    width: 32, height: 32, borderRadius: 6,
                    background: source === 'ollama' ? 'rgba(16,185,129,0.08)' : 'rgba(59,130,246,0.08)',
                    border: `1px solid ${source === 'ollama' ? 'rgba(16,185,129,0.15)' : 'rgba(59,130,246,0.15)'}`,
                    display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                  }}>
                    {source === 'ollama' ? <Server size={14} color="#10B981" /> : <Cpu size={14} color="#3B82F6" />}
                  </div>
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: '#0d0d0d' }} className="mono">{m.name}</div>
                    <div style={{ display: 'flex', gap: 10, fontSize: 11, color: 'var(--text-3)', marginTop: 2 }}>
                      {m.size ? <span>{fmtSize(m.size)}</span> : null}
                      {m.modified_at && <span>{new Date(m.modified_at).toLocaleDateString()}</span>}
                      {m.digest && <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>{m.digest.slice(0, 12)}</span>}
                    </div>
                  </div>
                  {source === 'ollama' && (
                    <button
                      onClick={() => handleDelete(m.name, 'ollama')}
                      disabled={deleting === m.name}
                      style={{
                        background: 'none', border: 'none', cursor: 'pointer',
                        color: deleting === m.name ? 'var(--text-4)' : 'var(--text-3)',
                        padding: 4, display: 'flex',
                      }}
                      title="Delete model"
                    >
                      {deleting === m.name ? <Loader2 size={13} className="spin" /> : <Trash2 size={13} />}
                    </button>
                  )}
                </div>
              </div>
            ))}
          </div>
        </div>
      )}

      {/* Empty state for ollama */}
      {source === 'ollama' && localModels.ollama.length === 0 && (
        <div style={{ padding: '24px 0', textAlign: 'center', color: 'var(--text-4)', fontSize: 13 }}>
          No Ollama models installed. Search and download models above, or run <code style={{ padding: '1px 5px', borderRadius: 3, background: 'var(--bg-elevated)', fontSize: 11 }}>ollama pull llama3.1</code> in terminal.
        </div>
      )}
    </div>
  );
}
