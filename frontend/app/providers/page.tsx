'use client';

import { useState, useCallback, useEffect } from 'react';
import Link from 'next/link';
import {
  Plug, Clock, ChevronRight, Check, X,
  Key, Loader2, Eye, EyeOff, ExternalLink, Shield, Terminal,
  Bot, Sparkles, Gem, Route, Zap, Wind, Smile, Compass, Atom,
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
            Provider Connections
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            {connectedCount} of {PROVIDERS.length} providers connected — connect via API key to enable inference and expert deployment
          </p>
        </div>
        <Link href="/providers/keys">
          <button className="btn btn-secondary btn-sm">
            <Key size={13} /> Manage Keys
          </button>
        </Link>
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
