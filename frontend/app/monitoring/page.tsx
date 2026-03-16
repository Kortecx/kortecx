'use client';

import { useState } from 'react';
import {
  Activity, Bell, AlertTriangle, Info, CheckCircle2,
  Zap, Clock, TrendingUp, Cpu, BarChart3, RefreshCcw,
  ChevronDown, X, ScrollText,
} from 'lucide-react';
import { ALERTS, EXPERTS, PROVIDERS, SYSTEM_METRICS } from '@/lib/constants';
import type { Alert } from '@/lib/types';

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function timeAgo(iso: string) {
  const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  if (diff < 86400) return `${Math.floor(diff / 3600)}h ago`;
  return `${Math.floor(diff / 86400)}d ago`;
}

function AlertRow({ alert }: { alert: Alert }) {
  const [dismissed, setDismissed] = useState(false);
  if (dismissed) return null;

  const colors: Record<string, string> = {
    critical: 'var(--error)',
    error:    'var(--error)',
    warning:  'var(--warning)',
    info:     'var(--primary)',
  };
  const Icon = {
    critical: AlertTriangle,
    error:    AlertTriangle,
    warning:  AlertTriangle,
    info:     Info,
  }[alert.severity] ?? Info;

  const color = colors[alert.severity];
  const isResolved = !!alert.resolvedAt;

  return (
    <div style={{
      display: 'flex', alignItems: 'flex-start', gap: 12,
      padding: '12px 16px',
      borderBottom: '1px solid var(--border)',
      opacity: isResolved ? 0.5 : 1,
    }}>
      <div style={{
        width: 32, height: 32, borderRadius: 6, flexShrink: 0,
        background: `${color}14`, border: `1px solid ${color}28`,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        marginTop: 1,
      }}>
        <Icon size={14} color={color} />
      </div>

      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
          <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
            {alert.title}
          </span>
          <span className={`badge ${
            alert.severity === 'critical' || alert.severity === 'error' ? 'badge-error' :
            alert.severity === 'warning' ? 'badge-warning' : 'badge-info'
          }`}>
            {alert.severity}
          </span>
          {isResolved && <span className="badge badge-success">Resolved</span>}
          {alert.acknowledgedAt && !isResolved && (
            <span className="badge badge-neutral">Acknowledged</span>
          )}
        </div>
        <p style={{ fontSize: 12, color: 'var(--text-3)', margin: 0, lineHeight: 1.5 }}>
          {alert.message}
        </p>
        <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 5 }}>
          {timeAgo(alert.createdAt)}
          {alert.expertId && <span> · Expert alert</span>}
          {alert.providerId && <span> · Provider alert</span>}
        </div>
      </div>

      {!isResolved && (
        <button
          className="btn btn-ghost btn-icon btn-sm"
          onClick={() => setDismissed(true)}
          style={{ color: 'var(--text-3)', flexShrink: 0 }}
        >
          <X size={14} />
        </button>
      )}
    </div>
  );
}

/* Simple sparkline using CSS bars */
function Sparkline({ data, color }: { data: number[]; color: string }) {
  const max = Math.max(...data);
  return (
    <div style={{ display: 'flex', alignItems: 'flex-end', gap: 3, height: 36 }}>
      {data.map((v, i) => (
        <div key={i} style={{
          flex: 1,
          height: `${(v / max) * 100}%`,
          minHeight: 2,
          background: color,
          borderRadius: 1,
          opacity: 0.4 + (i / data.length) * 0.6,
        }} />
      ))}
    </div>
  );
}

const MOCK_HOURLY_TOKENS = [210, 340, 280, 420, 380, 510, 440, 620, 580, 750, 680, 820];
const MOCK_HOURLY_SUCCESS = [94, 98, 96, 99, 97, 100, 98, 97, 99, 98, 99, 98];

export default function MonitoringPage() {
  const [tab, setTab] = useState<'overview' | 'alerts' | 'logs'>('overview');

  const unackedAlerts = ALERTS.filter(a => !a.acknowledgedAt);
  const topExperts = EXPERTS
    .filter(e => e.status === 'active')
    .sort((a, b) => b.stats.totalRuns - a.stats.totalRuns)
    .slice(0, 5);

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Monitoring
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            System health, performance metrics, and alerts
          </p>
        </div>
        <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
          <div style={{
            display: 'flex', alignItems: 'center', gap: 6,
            fontSize: 12, color: 'var(--success)',
          }}>
            <span className="status-dot dot-online dot-pulse" />
            All systems nominal
          </div>
          <button className="btn btn-ghost btn-sm">
            <RefreshCcw size={13} /> Refresh
          </button>
        </div>
      </div>

      {/* System health cards */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 20 }}>
        {[
          {
            label: 'SUCCESS RATE',
            value: `${(SYSTEM_METRICS.successRate * 100).toFixed(1)}%`,
            sub: `${SYSTEM_METRICS.errorCount} errors today`,
            color: 'var(--success)', icon: CheckCircle2,
          },
          {
            label: 'AVG LATENCY',
            value: `${(SYSTEM_METRICS.avgLatencyMs / 1000).toFixed(2)}s`,
            sub: 'across all experts',
            color: 'var(--primary)', icon: Clock,
          },
          {
            label: 'TOKENS TODAY',
            value: fmt(SYSTEM_METRICS.tokensUsedToday),
            sub: `$${SYSTEM_METRICS.costToday.toFixed(2)} spent`,
            color: 'var(--amber)', icon: Zap,
          },
          {
            label: 'ACTIVE AGENTS',
            value: String(SYSTEM_METRICS.activeAgents),
            sub: `${SYSTEM_METRICS.tasksToday} tasks today`,
            color: 'var(--indigo)', icon: Cpu,
          },
        ].map(card => (
          <div key={card.label} className="metric-card" style={{ position: 'relative', overflow: 'hidden' }}>
            <div style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 2, background: card.color, opacity: 0.7 }} />
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
              <div>
                <div className="metric-value">{card.value}</div>
                <div className="metric-label" style={{ marginTop: 6 }}>{card.label}</div>
                <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 4 }}>{card.sub}</div>
              </div>
              <card.icon size={16} color={card.color} />
            </div>
          </div>
        ))}
      </div>

      {/* Charts row */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 20 }}>
        <div className="card" style={{ padding: 18 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <Zap size={14} color="var(--amber)" /> Token Usage (12h)
            </div>
            <span style={{ fontSize: 11, color: 'var(--text-3)' }}>Hourly (thousands)</span>
          </div>
          <Sparkline data={MOCK_HOURLY_TOKENS} color="var(--amber)" />
          <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 8, fontSize: 10, color: 'var(--text-4)' }}>
            <span>12h ago</span>
            <span>Now</span>
          </div>
        </div>

        <div className="card" style={{ padding: 18 }}>
          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <TrendingUp size={14} color="var(--success)" /> Success Rate (12h)
            </div>
            <span style={{ fontSize: 11, color: 'var(--text-3)' }}>Percentage</span>
          </div>
          <Sparkline data={MOCK_HOURLY_SUCCESS} color="var(--success)" />
          <div style={{ display: 'flex', justifyContent: 'space-between', marginTop: 8, fontSize: 10, color: 'var(--text-4)' }}>
            <span>12h ago</span>
            <span>Now</span>
          </div>
        </div>
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 20 }}>
        {([
          { key: 'overview', label: 'Expert Performance' },
          { key: 'alerts',   label: `Alerts ${unackedAlerts.length > 0 ? `(${unackedAlerts.length})` : ''}` },
          { key: 'logs',     label: 'System Logs' },
        ] as const).map(t => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            style={{
              padding: '10px 18px',
              background: 'none', border: 'none',
              borderBottom: `2px solid ${tab === t.key ? 'var(--primary)' : 'transparent'}`,
              cursor: 'pointer', fontSize: 13,
              fontWeight: tab === t.key ? 600 : 400,
              color: tab === t.key ? 'var(--text-1)' : 'var(--text-3)',
              marginBottom: -1,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* Expert performance */}
      {tab === 'overview' && (
        <div className="card">
          <div style={{ padding: '14px 16px', borderBottom: '1px solid var(--border)', fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
            Top Experts — Last 24h
          </div>
          <table className="table-base">
            <thead>
              <tr>
                <th>Expert</th>
                <th>Role</th>
                <th>Provider</th>
                <th>Success Rate</th>
                <th>Avg Latency</th>
                <th>Avg Tokens</th>
                <th>Cost/Run</th>
                <th>Total Runs</th>
              </tr>
            </thead>
            <tbody>
              {topExperts.map(expert => (
                <tr key={expert.id}>
                  <td>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <span className="status-dot dot-online" />
                      <span style={{ color: 'var(--text-1)', fontWeight: 500 }}>{expert.name}</span>
                    </div>
                  </td>
                  <td style={{ textTransform: 'capitalize' }}>{expert.role}</td>
                  <td>{expert.providerName}</td>
                  <td>
                    <span style={{ color: expert.stats.successRate > 0.95 ? 'var(--success)' : 'var(--warning)' }}>
                      {(expert.stats.successRate * 100).toFixed(1)}%
                    </span>
                  </td>
                  <td className="mono">{(expert.stats.avgLatencyMs / 1000).toFixed(2)}s</td>
                  <td className="mono">{fmt(expert.stats.avgTokensPerRun)}</td>
                  <td className="mono">${expert.stats.avgCostPerRun.toFixed(3)}</td>
                  <td className="mono">{expert.stats.totalRuns.toLocaleString()}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      {/* Alerts */}
      {tab === 'alerts' && (
        <div className="card">
          <div style={{
            padding: '14px 16px', borderBottom: '1px solid var(--border)',
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <Bell size={14} color="var(--warning)" />
              Alerts
              {unackedAlerts.length > 0 && (
                <span className="badge badge-warning">{unackedAlerts.length} unresolved</span>
              )}
            </div>
          </div>
          {ALERTS.length === 0 ? (
            <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>
              No alerts. System is healthy.
            </div>
          ) : (
            ALERTS.map(alert => <AlertRow key={alert.id} alert={alert} />)
          )}
        </div>
      )}

      {/* Logs */}
      {tab === 'logs' && (
        <div className="card">
          <div style={{
            padding: '14px 16px', borderBottom: '1px solid var(--border)',
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          }}>
            <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 8 }}>
              <ScrollText size={14} color="var(--text-3)" /> System Logs
            </div>
            <div style={{ display: 'flex', gap: 8 }}>
              <select className="input" style={{ width: 'auto', fontSize: 12 }}>
                <option>All levels</option>
                <option>Error only</option>
                <option>Warn+</option>
                <option>Info+</option>
              </select>
            </div>
          </div>
          <div style={{
            padding: '12px 16px',
            background: 'var(--bg)',
            fontFamily: 'var(--font-geist-mono), monospace',
            fontSize: 12,
          }}>
            {[
              { level: 'INFO', msg: 'Task task-001 started — expert: DataAnalyst', time: '10:42:03' },
              { level: 'INFO', msg: 'ResearchPro completed step 1 — 2840 tokens used', time: '10:42:01' },
              { level: 'WARN', msg: 'Google API latency elevated: 620ms (baseline: 270ms)', time: '10:41:55' },
              { level: 'INFO', msg: 'Training checkpoint saved: epoch 3/5 — job-001', time: '10:30:12' },
              { level: 'ERROR', msg: 'Task task-XXX failed: token limit exceeded (15k)', time: '08:01:45' },
              { level: 'INFO', msg: 'Workflow run-001 completed — 9840 tokens, $0.47', time: '08:03:12' },
              { level: 'INFO', msg: 'Expert QAShield auto-scaled: 8 → 10 replicas', time: '07:58:30' },
            ].map((log, i) => (
              <div key={i} style={{
                display: 'flex', gap: 12, marginBottom: 6,
                color: log.level === 'ERROR' ? 'var(--error)' : log.level === 'WARN' ? 'var(--warning)' : 'var(--text-3)',
              }}>
                <span style={{ color: 'var(--text-4)', flexShrink: 0 }}>{log.time}</span>
                <span style={{
                  width: 36, flexShrink: 0,
                  color: log.level === 'ERROR' ? 'var(--error)' : log.level === 'WARN' ? 'var(--warning)' : 'var(--success)',
                  fontWeight: 600,
                }}>
                  {log.level}
                </span>
                <span style={{ color: 'var(--text-2)' }}>{log.msg}</span>
              </div>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
