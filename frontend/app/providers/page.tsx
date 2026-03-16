'use client';

import { useState } from 'react';
import { Plug, Activity, Zap, Clock, ChevronRight, ExternalLink, Check, X } from 'lucide-react';
import { PROVIDERS } from '@/lib/constants';

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function statusBadge(status: string) {
  switch (status) {
    case 'operational':
      return <span className="badge badge-success">Operational</span>;
    case 'degraded':
      return <span className="badge badge-amber">Degraded</span>;
    case 'outage':
      return <span className="badge badge-error">Outage</span>;
    default:
      return <span className="badge badge-neutral">Unknown</span>;
  }
}

export default function ProvidersPage() {
  const [filter, setFilter] = useState<'all' | 'connected' | 'available'>('all');

  const filtered = PROVIDERS.filter(p => {
    if (filter === 'connected') return p.connected;
    if (filter === 'available') return !p.connected;
    return true;
  });

  const connectedCount = PROVIDERS.filter(p => p.connected).length;

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
            {connectedCount} of {PROVIDERS.length} providers connected
          </p>
        </div>
        <button className="btn btn-primary btn-sm">
          <Plug size={13} /> Add Provider
        </button>
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
      }}>
        {filtered.map(provider => (
          <div key={provider.id} className="card" style={{ padding: 20 }}>
            {/* Provider Header */}
            <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 14 }}>
              <div style={{
                width: 40, height: 40, borderRadius: 8,
                background: `${provider.color}12`,
                border: `1px solid ${provider.color}30`,
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                flexShrink: 0,
              }}>
                <div style={{
                  width: 12, height: 12, borderRadius: '50%',
                  background: provider.color,
                }} />
              </div>
              <div style={{ flex: 1 }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span style={{ fontSize: 15, fontWeight: 600, color: 'var(--text-1)' }}>
                    {provider.name}
                  </span>
                  {provider.connected && statusBadge(provider.status)}
                </div>
                <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0', lineHeight: 1.5 }}>
                  {provider.description}
                </p>
              </div>
            </div>

            {/* Connection Status */}
            <div style={{
              padding: '10px 14px',
              background: provider.connected ? 'rgba(5,150,105,0.05)' : 'var(--bg)',
              border: `1px solid ${provider.connected ? 'rgba(5,150,105,0.15)' : 'var(--border)'}`,
              borderRadius: 5,
              marginBottom: 12,
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                {provider.connected ? (
                  <Check size={14} color="#059669" />
                ) : (
                  <X size={14} color="var(--text-4)" />
                )}
                <span style={{
                  fontSize: 12, fontWeight: 500,
                  color: provider.connected ? '#059669' : 'var(--text-3)',
                }}>
                  {provider.connected ? 'Connected' : 'Not connected'}
                </span>
                {provider.connected && provider.apiKeySet && (
                  <>
                    <span style={{ color: 'var(--text-4)' }}>·</span>
                    <span style={{ fontSize: 11, color: 'var(--text-3)' }}>API key set</span>
                  </>
                )}
              </div>
            </div>

            {/* Stats (connected only) */}
            {provider.connected && (
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 8, marginBottom: 14 }}>
                <div style={{
                  padding: '8px', background: 'var(--bg)',
                  border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
                }}>
                  <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
                    {provider.latencyMs}ms
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
                <div style={{
                  padding: '8px', background: 'var(--bg)',
                  border: '1px solid var(--border)', borderRadius: 4, textAlign: 'center',
                }}>
                  <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
                    {provider.models.length}
                  </div>
                  <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>Models</div>
                </div>
              </div>
            )}

            {/* Models list (connected only) */}
            {provider.connected && provider.models.length > 0 && (
              <div style={{ marginBottom: 14 }}>
                <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, textTransform: 'uppercase', letterSpacing: '0.06em' }}>
                  Models
                </div>
                {provider.models.map(model => (
                  <div key={model.id} style={{
                    display: 'flex', alignItems: 'center', gap: 8,
                    padding: '5px 0', fontSize: 12, color: 'var(--text-2)',
                  }}>
                    <span style={{ width: 4, height: 4, borderRadius: '50%', background: provider.color, flexShrink: 0 }} />
                    <span style={{ flex: 1 }}>{model.name}</span>
                    <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>
                      {(model.contextWindow / 1000).toFixed(0)}k ctx
                    </span>
                  </div>
                ))}
              </div>
            )}

            {/* Action */}
            <div style={{ display: 'flex', gap: 6 }}>
              {provider.connected ? (
                <>
                  <button className="btn btn-secondary btn-sm" style={{ flex: 1, justifyContent: 'center' }}>
                    Configure <ChevronRight size={12} />
                  </button>
                  <button className="btn btn-ghost btn-sm" style={{ color: '#DC2626' }}>
                    Disconnect
                  </button>
                </>
              ) : (
                <button className="btn btn-primary btn-sm" style={{ flex: 1, justifyContent: 'center' }}>
                  <Plug size={12} /> Connect
                </button>
              )}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
