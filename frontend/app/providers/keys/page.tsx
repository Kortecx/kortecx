'use client';

import { useState } from 'react';
import {
  Key, Eye, EyeOff, RotateCcw, Trash2, Plus, Copy, Check, Shield, Plug,
  Bot, Sparkles, Gem, Route, Zap, Wind, Smile, Compass, Atom,
} from 'lucide-react';
import { PROVIDERS } from '@/lib/constants';

/* ─── Provider Icon Resolver ─────────────────────── */
const PROVIDER_ICON_MAP: Record<string, React.ComponentType<{ size?: number; color?: string }>> = {
  Bot, Sparkles, Gem, Route, Zap, Wind, Smile, Compass, Atom,
};

function ProviderIcon({ name, size = 14, color }: { name: string; size?: number; color?: string }) {
  const Icon = PROVIDER_ICON_MAP[name] || Plug;
  return <Icon size={size} color={color} />;
}

const MOCK_KEYS: Array<{
  providerId: string; keyPrefix: string; keySuffix: string;
  createdAt: string; lastUsed: string;
}> = [];

export default function ApiKeysPage() {
  const [showKey, setShowKey] = useState<Record<string, boolean>>({});
  const [copied, setCopied] = useState<string | null>(null);
  const [addingFor, setAddingFor] = useState<string | null>(null);

  const handleCopy = (providerId: string) => {
    setCopied(providerId);
    setTimeout(() => setCopied(null), 2000);
  };

  const unconnectedProviders = PROVIDERS.filter(p => !p.connected);

  return (
    <div style={{ padding: 24, maxWidth: 1000, margin: '0 auto' }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            API Keys
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Manage API keys for connected providers
          </p>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setAddingFor('new')}>
          <Plus size={13} /> Add API Key
        </button>
      </div>

      {/* Security notice */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 10,
        padding: '12px 16px', marginBottom: 20,
        background: 'rgba(37,99,235,0.05)',
        border: '1px solid rgba(37,99,235,0.15)',
        borderRadius: 6,
      }}>
        <Shield size={16} color="#2563EB" />
        <span style={{ fontSize: 12, color: '#2563EB' }}>
          API keys are encrypted at rest and never exposed in logs. Only key prefix and suffix are displayed.
        </span>
      </div>

      {/* Connected provider keys */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 12, marginBottom: 32 }}>
        {MOCK_KEYS.map(key => {
          const provider = PROVIDERS.find(p => p.id === key.providerId);
          if (!provider) return null;
          const isVisible = showKey[key.providerId];

          return (
            <div key={key.providerId} className="card" style={{ padding: '16px 20px' }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <div style={{
                  width: 32, height: 32, borderRadius: 6,
                  background: `${provider.color}12`, border: `1px solid ${provider.color}25`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  flexShrink: 0,
                }}>
                  <ProviderIcon name={provider.icon} size={16} color={provider.color} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
                    {provider.name}
                  </div>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 4 }}>
                    <code className="mono" style={{
                      fontSize: 12, color: 'var(--text-2)',
                      background: 'var(--bg)', padding: '2px 8px',
                      borderRadius: 3, border: '1px solid var(--border)',
                    }}>
                      {isVisible
                        ? `${key.keyPrefix}-****************************${key.keySuffix}`
                        : `${key.keyPrefix}-••••••••••••••••••••${key.keySuffix}`
                      }
                    </code>
                    <button
                      onClick={() => setShowKey(prev => ({ ...prev, [key.providerId]: !prev[key.providerId] }))}
                      style={{
                        background: 'none', border: 'none', cursor: 'pointer',
                        color: 'var(--text-3)', padding: 2,
                      }}
                    >
                      {isVisible ? <EyeOff size={14} /> : <Eye size={14} />}
                    </button>
                    <button
                      onClick={() => handleCopy(key.providerId)}
                      style={{
                        background: 'none', border: 'none', cursor: 'pointer',
                        color: copied === key.providerId ? 'var(--success)' : 'var(--text-3)', padding: 2,
                      }}
                    >
                      {copied === key.providerId ? <Check size={14} /> : <Copy size={14} />}
                    </button>
                  </div>
                  <div style={{ display: 'flex', gap: 12, marginTop: 6, fontSize: 11, color: 'var(--text-4)' }}>
                    <span>Created: {key.createdAt}</span>
                    <span>Last used: {key.lastUsed}</span>
                  </div>
                </div>
                <div style={{ display: 'flex', gap: 6 }}>
                  <button className="btn btn-secondary btn-sm" title="Rotate key">
                    <RotateCcw size={12} /> Rotate
                  </button>
                  <button className="btn btn-ghost btn-sm" style={{ color: '#DC2626' }} title="Revoke key">
                    <Trash2 size={12} />
                  </button>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Add key for unconnected providers */}
      {unconnectedProviders.length > 0 && (
        <>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 12 }}>
            Available Providers
          </h2>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
            {unconnectedProviders.map(provider => (
              <div key={provider.id} style={{
                display: 'flex', alignItems: 'center', gap: 12,
                padding: '12px 16px',
                background: 'var(--bg-card)',
                border: '1px solid var(--border)',
                borderRadius: 6,
              }}>
                <div style={{
                  width: 28, height: 28, borderRadius: 6,
                  background: `${provider.color}0A`, border: `1px solid ${provider.color}18`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  flexShrink: 0,
                }}>
                  <ProviderIcon name={provider.icon} size={14} color={`${provider.color}80`} />
                </div>
                <div style={{ flex: 1 }}>
                  <span style={{ fontSize: 13, color: 'var(--text-2)' }}>{provider.name}</span>
                  <span style={{ fontSize: 11, color: 'var(--text-4)', marginLeft: 8 }}>
                    {provider.description}
                  </span>
                </div>
                <button
                  className="btn btn-secondary btn-sm"
                  onClick={() => setAddingFor(provider.id)}
                >
                  <Key size={12} /> Add Key
                </button>
              </div>
            ))}
          </div>
        </>
      )}

      {/* Add key modal/form */}
      {addingFor && (
        <div style={{
          position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.4)',
          display: 'flex', alignItems: 'center', justifyContent: 'center', zIndex: 100,
        }}>
          <div style={{
            background: 'var(--bg-surface)', borderRadius: 8,
            padding: 24, width: 440, maxWidth: '90vw',
            boxShadow: '0 20px 60px rgba(0,0,0,0.15)',
          }}>
            <h3 style={{ fontSize: 16, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
              Add API Key
            </h3>
            <div style={{ marginBottom: 12 }}>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                Provider
              </label>
              <select className="input" style={{ width: '100%' }} defaultValue={addingFor}>
                {PROVIDERS.map(p => (
                  <option key={p.id} value={p.id}>{p.name}</option>
                ))}
              </select>
            </div>
            <div style={{ marginBottom: 16 }}>
              <label style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
                API Key
              </label>
              <input className="input" type="password" placeholder="sk-..." style={{ width: '100%' }} />
            </div>
            <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
              <button className="btn btn-secondary btn-sm" onClick={() => setAddingFor(null)}>
                Cancel
              </button>
              <button className="btn btn-primary btn-sm" onClick={() => setAddingFor(null)}>
                Save Key
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
