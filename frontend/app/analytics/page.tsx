'use client';

import useSWR from 'swr';
import { motion } from 'framer-motion';
import { BarChart3, TrendingUp, Zap, DollarSign, Activity, ChevronRight, Loader2, Workflow, Database } from 'lucide-react';
import { useExperts, useMetrics, useLogs, useLiveMetrics } from '@/lib/hooks/useApi';
import type { AIProvider, Expert } from '@/lib/types';

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

function fmt(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

export default function AnalyticsPage() {
  const { experts, isLoading: expertsLoading } = useExperts() as { experts: Expert[]; total: number; isLoading: boolean; error: unknown; mutate: () => void };
  const { metrics, totals, hourly, isLoading: metricsLoading } = useMetrics() as {
    metrics: { successRate: number } | null;
    totals: { tasksThisWeek: number; tokensThisWeek: number; costThisWeek: number } | null;
    hourly: { day: string; tasks: number; tokens: number }[];
    isLoading: boolean;
    error: unknown;
    mutate: () => void;
  };
  const { data: providersData, isLoading: providersLoading } = useSWR<{ providers: AIProvider[] }>('/api/providers', fetcher);
  const providers = providersData?.providers ?? [];
  const { logs: recentLogs } = useLogs(undefined, 20);
  const { metrics: liveMetrics } = useLiveMetrics();
  const { data: analyticsData } = useSWR('/api/analytics', fetcher);
  const analytics = analyticsData ?? null;

  const isLoading = expertsLoading || metricsLoading || providersLoading;

  const weeklyStats = {
    tasks: totals?.tasksThisWeek ?? 0,
    tokens: totals?.tokensThisWeek ?? 0,
    cost: totals?.costThisWeek ?? 0,
    successRate: metrics?.successRate ?? 0,
  };

  const daily: Array<{ day: string; tasks: number; tokens: number }> = hourly ?? [];
  const maxTasks = Math.max(1, ...daily.map(d => d.tasks));

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        style={{ marginBottom: 28 }}
      >
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
          Analytics
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          Platform-wide performance and usage analytics
        </p>
      </motion.div>

      {/* Weekly metrics */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 20 }}>
        {[
          { icon: Activity, label: 'Tasks this week', value: isLoading ? '...' : weeklyStats.tasks.toLocaleString(), color: 'var(--text-1)' },
          { icon: Zap, label: 'Tokens used', value: isLoading ? '...' : fmt(weeklyStats.tokens), color: 'var(--amber)' },
          { icon: DollarSign, label: 'Total cost', value: isLoading ? '...' : `$${weeklyStats.cost.toFixed(2)}`, color: 'var(--text-1)' },
          { icon: TrendingUp, label: 'Success rate', value: isLoading ? '...' : `${(weeklyStats.successRate * 100).toFixed(1)}%`, color: 'var(--success)' },
        ].map((m, i) => (
          <motion.div
            key={m.label}
            className="card"
            initial={{ opacity: 0, scale: 0.95 }}
            animate={{ opacity: 1, scale: 1 }}
            transition={{ delay: i * 0.06, type: 'spring', damping: 20 }}
            style={{ padding: 16 }}
          >
            <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between' }}>
              <div>
                <div style={{ fontSize: 22, fontWeight: 800, color: m.color, letterSpacing: '-0.03em' }}>
                  {m.value}
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4 }}>{m.label}</div>
              </div>
              <div style={{
                width: 32, height: 32, borderRadius: 6,
                background: 'var(--primary-dim)', border: '1px solid var(--border-md)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <m.icon size={14} color="var(--primary)" />
              </div>
            </div>
          </motion.div>
        ))}
      </div>

      <motion.div
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.28, duration: 0.35 }}
        style={{ display: 'grid', gridTemplateColumns: '1fr 360px', gap: 12, marginBottom: 20 }}
      >
        {/* Daily task chart */}
        <div className="card" style={{ padding: 20 }}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
            Daily Task Volume
          </h2>
          {metricsLoading ? (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: 160, color: 'var(--text-3)', gap: 8, fontSize: 13 }}>
              <Loader2 size={16} className="animate-spin" /> Loading chart data...
            </div>
          ) : daily.length === 0 ? (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: 160, color: 'var(--text-3)', fontSize: 13 }}>
              No task data available yet.
            </div>
          ) : (
            <div style={{ display: 'flex', alignItems: 'flex-end', gap: 8, height: 160 }}>
              {daily.map(d => (
                <div key={d.day} style={{ flex: 1, display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 4 }}>
                  <span className="mono" style={{ fontSize: 10, color: 'var(--text-3)' }}>
                    {d.tasks}
                  </span>
                  <div style={{
                    width: '100%', borderRadius: '3px 3px 0 0',
                    background: 'var(--primary)',
                    opacity: 0.7 + (d.tasks / maxTasks) * 0.3,
                    height: `${(d.tasks / maxTasks) * 120}px`,
                    transition: 'height 0.3s ease',
                  }} />
                  <span style={{ fontSize: 11, color: 'var(--text-3)' }}>{d.day}</span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* AIProvider usage */}
        <div className="card" style={{ padding: 20 }}>
          <h2 style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)', margin: '0 0 16px' }}>
            Provider Usage
          </h2>
          {providersLoading ? (
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', height: 100, color: 'var(--text-3)', gap: 8, fontSize: 13 }}>
              <Loader2 size={14} className="animate-spin" /> Loading providers...
            </div>
          ) : providers.filter(p => p.connected && p.monthlyTokensUsed).length === 0 ? (
            <div style={{ textAlign: 'center', padding: 20, color: 'var(--text-3)', fontSize: 13 }}>
              No provider usage data available.
            </div>
          ) : (
            providers.filter(p => p.connected && p.monthlyTokensUsed).map(provider => {
              const pct = Math.round(((provider.monthlyTokensUsed ?? 0) / (provider.monthlyTokenLimit ?? 1)) * 100);
              return (
                <div key={provider.id} style={{ marginBottom: 14 }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', marginBottom: 4 }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{ width: 8, height: 8, borderRadius: '50%', background: provider.color }} />
                      <span style={{ fontSize: 13, color: 'var(--text-1)' }}>{provider.name}</span>
                    </div>
                    <span className="mono" style={{ fontSize: 11, color: 'var(--text-2)' }}>
                      {fmt(provider.monthlyTokensUsed ?? 0)} / {fmt(provider.monthlyTokenLimit ?? 0)}
                    </span>
                  </div>
                  <div style={{
                    height: 4, background: 'var(--bg-elevated)',
                    borderRadius: 2, overflow: 'hidden',
                  }}>
                    <div style={{
                      height: '100%', width: `${pct}%`,
                      background: provider.color, borderRadius: 2,
                    }} />
                  </div>
                </div>
              );
            })
          )}
        </div>
      </motion.div>

      {/* Expert performance table */}
      <motion.div
        className="card"
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.36, duration: 0.35 }}
        style={{ overflow: 'hidden' }}
      >
        <div style={{
          padding: '13px 20px', borderBottom: '1px solid var(--border)',
          fontSize: 14, fontWeight: 600, color: 'var(--text-1)',
          display: 'flex', alignItems: 'center', gap: 8,
        }}>
          <BarChart3 size={14} color="var(--text-2)" /> Expert Performance
        </div>
        {expertsLoading ? (
          <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
            <Loader2 size={16} className="animate-spin" /> Loading experts...
          </div>
        ) : experts.length === 0 ? (
          <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>
            No experts configured yet.
          </div>
        ) : (
          <div style={{ overflowX: 'auto' }}>
            <table style={{ width: '100%', borderCollapse: 'collapse' }}>
              <thead>
                <tr style={{ borderBottom: '1px solid var(--border)' }}>
                  {['Expert', 'Role', 'Runs', 'Success', 'Avg Tokens', 'Avg Latency', 'Cost/Run'].map(h => (
                    <th key={h} style={{
                      padding: '8px 16px', textAlign: 'left',
                      fontSize: 10, fontWeight: 600, color: 'var(--text-3)',
                      textTransform: 'uppercase', letterSpacing: '0.08em',
                    }}>
                      {h}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {experts.map(expert => (
                  <tr key={expert.id} style={{ borderBottom: '1px solid var(--border)' }}>
                    <td style={{ padding: '10px 16px', fontSize: 13, fontWeight: 500, color: 'var(--text-1)' }}>
                      {expert.name}
                    </td>
                    <td style={{ padding: '10px 16px' }}>
                      <span style={{
                        fontSize: 10, fontWeight: 600, textTransform: 'uppercase',
                        letterSpacing: '0.06em', color: 'var(--text-3)',
                      }}>
                        {expert.role}
                      </span>
                    </td>
                    <td className="mono" style={{ padding: '10px 16px', fontSize: 12, color: 'var(--text-2)' }}>
                      {expert.stats.totalRuns.toLocaleString()}
                    </td>
                    <td className="mono" style={{
                      padding: '10px 16px', fontSize: 12,
                      color: expert.stats.successRate > 0.95 ? 'var(--success)' : 'var(--text-2)',
                    }}>
                      {(expert.stats.successRate * 100).toFixed(1)}%
                    </td>
                    <td className="mono" style={{ padding: '10px 16px', fontSize: 12, color: 'var(--text-2)' }}>
                      {fmt(expert.stats.avgTokensPerRun)}
                    </td>
                    <td className="mono" style={{ padding: '10px 16px', fontSize: 12, color: 'var(--text-2)' }}>
                      {(expert.stats.avgLatencyMs / 1000).toFixed(1)}s
                    </td>
                    <td className="mono" style={{ padding: '10px 16px', fontSize: 12, color: 'var(--text-2)' }}>
                      ${expert.stats.avgCostPerRun.toFixed(3)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </motion.div>

      {/* Workflow Run Stats */}
      <motion.div
        className="card"
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.44, duration: 0.35 }}
        style={{ padding: 20, marginTop: 16 }}
      >
        <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)', marginBottom: 16, display: 'flex', alignItems: 'center', gap: 8 }}>
          <Workflow size={15} color="var(--text-3)" />
          Workflow Performance
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(5, 1fr)', gap: 12 }}>
          {[
            { label: 'Total Runs', value: analytics?.overview?.tasksThisWeek ?? 0, color: '#2563EB' },
            { label: 'Success Rate', value: `${((analytics?.overview?.avgSuccessRate ?? 0) * 100).toFixed(1)}%`, color: '#10b981' },
            { label: 'Avg Duration', value: `${((analytics?.overview?.avgDurationMs ?? 0) / 1000).toFixed(1)}s`, color: '#f59e0b' },
            { label: 'Total Tokens', value: fmt(analytics?.overview?.tokensThisWeek ?? 0), color: '#8b5cf6' },
            { label: 'Total Cost', value: `$${(analytics?.overview?.costThisWeek ?? 0).toFixed(2)}`, color: '#ef4444' },
          ].map((stat, i) => (
            <motion.div
              key={stat.label}
              initial={{ opacity: 0, scale: 0.95 }}
              animate={{ opacity: 1, scale: 1 }}
              transition={{ delay: 0.48 + i * 0.05, type: 'spring', damping: 20 }}
              style={{ padding: 14, background: 'var(--bg)', borderRadius: 6, border: '1px solid var(--border)', textAlign: 'center' }}
            >
              <div style={{ fontSize: 20, fontWeight: 800, color: stat.color, fontFamily: 'var(--font-mono, monospace)' }}>{stat.value}</div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 4, textTransform: 'uppercase', letterSpacing: '0.06em', fontWeight: 600 }}>{stat.label}</div>
            </motion.div>
          ))}
        </div>
      </motion.div>

      {/* System Health */}
      <motion.div
        className="card"
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.52, duration: 0.35 }}
        style={{ padding: 20, marginTop: 16 }}
      >
        <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)', marginBottom: 16, display: 'flex', alignItems: 'center', gap: 8 }}>
          <Activity size={15} color="var(--text-3)" />
          System Health
        </div>
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12 }}>
          {[
            { label: 'Active Agents', value: liveMetrics?.activeAgents ?? 0, max: 20, color: '#2563EB' },
            { label: 'Error Rate', value: liveMetrics?.errorCount ?? 0, max: 100, color: '#ef4444' },
            { label: 'Avg Latency', value: `${liveMetrics?.avgLatencyMs ?? 0}ms`, max: null, color: '#f59e0b' },
            { label: 'Uptime', value: '99.9%', max: null, color: '#10b981' },
          ].map(item => (
            <div key={item.label} style={{ padding: 14, background: 'var(--bg)', borderRadius: 6, border: '1px solid var(--border)' }}>
              <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center', marginBottom: 8 }}>
                <span style={{ fontSize: 10, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', fontWeight: 600 }}>{item.label}</span>
                <span style={{ fontSize: 16, fontWeight: 800, color: item.color, fontFamily: 'var(--font-mono, monospace)' }}>{item.value}</span>
              </div>
              {item.max && (
                <div style={{ height: 3, background: 'var(--border)', borderRadius: 2, overflow: 'hidden' }}>
                  <div style={{ height: '100%', width: `${Math.min((Number(item.value) / item.max) * 100, 100)}%`, background: item.color, borderRadius: 2, transition: 'width 0.3s' }} />
                </div>
              )}
            </div>
          ))}
        </div>
      </motion.div>

      {/* Activity Feed */}
      <motion.div
        className="card"
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.6, duration: 0.35 }}
        style={{ padding: 20, marginTop: 16 }}
      >
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16 }}>
          <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 8 }}>
            <Database size={15} color="var(--text-3)" />
            Recent Activity
          </div>
          <a href="/monitoring/logs" style={{ fontSize: 11, color: 'var(--text-3)', textDecoration: 'none' }}>View all logs →</a>
        </div>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6, maxHeight: 240, overflowY: 'auto' }}>
          {recentLogs.length === 0 && <div style={{ fontSize: 12, color: 'var(--text-4)', padding: 20, textAlign: 'center' }}>No recent activity</div>}
          {recentLogs.map((log: Record<string, unknown>, i: number) => (
            <div key={i} style={{ display: 'flex', alignItems: 'flex-start', gap: 8, padding: '6px 8px', borderRadius: 4, background: i % 2 === 0 ? 'var(--bg)' : 'transparent' }}>
              <span style={{
                width: 6, height: 6, borderRadius: '50%', marginTop: 5, flexShrink: 0,
                background: (log.level as string) === 'error' ? '#ef4444' : (log.level as string) === 'warning' ? '#f59e0b' : '#10b981',
              }} />
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 11, color: 'var(--text-2)', lineHeight: 1.4 }}>{log.message as string}</div>
                <div style={{ display: 'flex', gap: 8, fontSize: 9, color: 'var(--text-4)', marginTop: 2 }}>
                  <span>{log.source as string}</span>
                  <span>{new Date(log.timestamp as string).toLocaleTimeString()}</span>
                </div>
              </div>
            </div>
          ))}
        </div>
      </motion.div>
    </div>
  );
}
