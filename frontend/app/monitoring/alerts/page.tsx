'use client';

import { useState } from 'react';
import {
  Bell, AlertTriangle, AlertCircle, Info, ShieldAlert,
  Check, Clock, Filter, X,
} from 'lucide-react';
import { ALERTS } from '@/lib/constants';
import type { Alert, AlertSeverity } from '@/lib/types';

const SEVERITY_CONFIG: Record<AlertSeverity, { icon: React.ElementType; color: string; bg: string }> = {
  info:     { icon: Info,         color: '#2563EB', bg: 'rgba(37,99,235,0.06)' },
  warning:  { icon: AlertTriangle, color: '#D97706', bg: 'rgba(217,119,6,0.06)' },
  error:    { icon: AlertCircle,  color: '#DC2626', bg: 'rgba(220,38,38,0.06)' },
  critical: { icon: ShieldAlert,  color: '#7C3AED', bg: 'rgba(124,58,237,0.06)' },
};

function elapsed(iso: string): string {
  const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

export default function AlertsPage() {
  const [filter, setFilter] = useState<AlertSeverity | 'all'>('all');

  const filtered = ALERTS.filter(a => filter === 'all' || a.severity === filter);
  const unacknowledged = ALERTS.filter(a => !a.acknowledgedAt).length;

  return (
    <div style={{ padding: 24, maxWidth: 1000, margin: '0 auto' }}>
      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Alerts
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            {ALERTS.length} total · {unacknowledged} unacknowledged
          </p>
        </div>
        {unacknowledged > 0 && (
          <button className="btn btn-secondary btn-sm">
            <Check size={13} /> Acknowledge All
          </button>
        )}
      </div>

      {/* Severity filters */}
      <div style={{ display: 'flex', gap: 6, marginBottom: 20 }}>
        <button
          onClick={() => setFilter('all')}
          style={{
            padding: '5px 14px', borderRadius: 20, fontSize: 12, fontWeight: 500,
            border: '1px solid', cursor: 'pointer',
            background: filter === 'all' ? 'var(--primary-dim)' : 'var(--bg-card)',
            borderColor: filter === 'all' ? 'var(--primary)' : 'var(--border)',
            color: filter === 'all' ? 'var(--primary-text)' : 'var(--text-2)',
          }}
        >
          All ({ALERTS.length})
        </button>
        {(Object.keys(SEVERITY_CONFIG) as AlertSeverity[]).map(severity => {
          const cfg = SEVERITY_CONFIG[severity];
          const count = ALERTS.filter(a => a.severity === severity).length;
          const active = filter === severity;
          return (
            <button
              key={severity}
              onClick={() => setFilter(active ? 'all' : severity)}
              style={{
                padding: '5px 14px', borderRadius: 20, fontSize: 12, fontWeight: 500,
                border: '1px solid', cursor: 'pointer', textTransform: 'capitalize',
                background: active ? cfg.bg : 'var(--bg-card)',
                borderColor: active ? cfg.color : 'var(--border)',
                color: active ? cfg.color : 'var(--text-2)',
              }}
            >
              {severity} ({count})
            </button>
          );
        })}
      </div>

      {/* Alert list */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
        {filtered.map(alert => {
          const cfg = SEVERITY_CONFIG[alert.severity];
          const Icon = cfg.icon;
          const isAcknowledged = !!alert.acknowledgedAt;
          const isResolved = !!alert.resolvedAt;

          return (
            <div
              key={alert.id}
              className="card"
              style={{
                padding: '14px 18px',
                borderLeft: `3px solid ${cfg.color}`,
                opacity: isResolved ? 0.6 : 1,
              }}
            >
              <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
                <div style={{
                  width: 32, height: 32, borderRadius: 6,
                  background: cfg.bg, display: 'flex',
                  alignItems: 'center', justifyContent: 'center', flexShrink: 0,
                  marginTop: 2,
                }}>
                  <Icon size={16} color={cfg.color} />
                </div>
                <div style={{ flex: 1 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
                    <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
                      {alert.title}
                    </span>
                    <span style={{
                      fontSize: 10, fontWeight: 600, textTransform: 'uppercase',
                      letterSpacing: '0.06em', color: cfg.color,
                    }}>
                      {alert.severity}
                    </span>
                  </div>
                  <p style={{ fontSize: 12, color: 'var(--text-2)', margin: '0 0 8px', lineHeight: 1.5 }}>
                    {alert.message}
                  </p>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 10, fontSize: 11, color: 'var(--text-4)' }}>
                    <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                      <Clock size={10} /> {elapsed(alert.createdAt)}
                    </span>
                    {isAcknowledged && (
                      <>
                        <span style={{ color: 'var(--text-4)' }}>·</span>
                        <span style={{ color: 'var(--success)', display: 'flex', alignItems: 'center', gap: 3 }}>
                          <Check size={10} /> Acknowledged
                        </span>
                      </>
                    )}
                    {isResolved && (
                      <>
                        <span style={{ color: 'var(--text-4)' }}>·</span>
                        <span style={{ color: 'var(--success)' }}>Resolved</span>
                      </>
                    )}
                  </div>
                </div>
                {!isResolved && (
                  <div style={{ display: 'flex', gap: 6, flexShrink: 0 }}>
                    {!isAcknowledged && (
                      <button className="btn btn-secondary btn-sm">
                        <Check size={12} /> Ack
                      </button>
                    )}
                    <button className="btn btn-ghost btn-sm">
                      Resolve
                    </button>
                  </div>
                )}
              </div>
            </div>
          );
        })}
      </div>

      {filtered.length === 0 && (
        <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 14 }}>
          No alerts match your filter.
        </div>
      )}
    </div>
  );
}
