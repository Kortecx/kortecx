'use client';

import { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ScrollText, Download, Search, ChevronDown,
  Loader2, X, ChevronRight, Filter,
  Clock, Workflow, Users, Brain, Database, Boxes, Cpu, FileText,
} from 'lucide-react';
import { useLogs, useStepExecutions, useLiveMetrics } from '@/lib/hooks/useApi';

/* ── Constants ──────────────────────────────────────────────────────────── */
const SECTION_COLOR = '#ef4444';
const MAX_LOGS      = 1000;

const LEVEL_META: Record<string, { color: string; bg: string; rowBg: string; label: string }> = {
  debug:    { color: '#6b7280', bg: '#6b728018', rowBg: 'transparent',           label: 'DEBUG' },
  info:     { color: '#3b82f6', bg: '#3b82f618', rowBg: 'transparent',           label: 'INFO ' },
  warning:  { color: '#f59e0b', bg: '#f59e0b18', rowBg: 'rgba(245,158,11,0.04)', label: 'WARN ' },
  warn:     { color: '#f59e0b', bg: '#f59e0b18', rowBg: 'rgba(245,158,11,0.04)', label: 'WARN ' },
  error:    { color: '#ef4444', bg: '#ef444418', rowBg: 'rgba(239,68,68,0.06)',  label: 'ERROR' },
  critical: { color: '#ef4444', bg: '#ef444430', rowBg: 'rgba(239,68,68,0.10)', label: 'CRIT ' },
};

const LEVEL_FILTERS = [
  { id: 'all',      label: 'All Levels' },
  { id: 'debug',    label: 'Debug' },
  { id: 'info',     label: 'Info' },
  { id: 'warning',  label: 'Warning' },
  { id: 'error',    label: 'Error' },
];

const TIME_RANGES = [
  { id: '5m',     label: 'Last 5m' },
  { id: '30m',    label: 'Last 30m' },
  { id: '1h',     label: 'Last 1h' },
  { id: '6h',     label: 'Last 6h' },
  { id: '1d',     label: 'Last 24h' },
  { id: '1w',     label: 'Last 7d' },
  { id: 'all',    label: 'All time' },
  { id: 'custom', label: 'Custom...' },
];

const CATEGORY_FILTERS: Array<{ id: string; label: string; icon: React.ComponentType<{ size?: number }>; color: string; sources: string[] }> = [
  { id: 'all',        label: 'All',        icon: Boxes,    color: '#6b7280', sources: [] },
  { id: 'workflows',  label: 'Workflows',  icon: Workflow, color: '#2563EB', sources: ['orchestrator', 'workflow', 'scheduler'] },
  { id: 'experts',    label: 'Experts',    icon: Users,    color: '#7C3AED', sources: ['expert', 'expert_manager'] },
  { id: 'agents',     label: 'Agents',     icon: Brain,    color: '#D97706', sources: ['agent', 'quorum', 'orchestrator'] },
  { id: 'data',       label: 'Data',       icon: Database, color: '#059669', sources: ['dataset', 'synthesis', 'transform', 'schema', 'duckdb', 'spark'] },
  { id: 'artifacts',  label: 'Artifacts',  icon: FileText, color: '#EC4899', sources: ['artifact', 'step_artifacts', 'script'] },
  { id: 'system',     label: 'System',     icon: Cpu,      color: '#ef4444', sources: ['system', 'engine', 'health', 'metrics'] },
];

type ViewTab = 'logs' | 'executions';

/* ── Helpers ────────────────────────────────────────────────────────────── */
function formatTs(iso: string) {
  const d = new Date(iso);
  return d.toLocaleTimeString('en-US', {
    hour12: false, hour: '2-digit',
    minute: '2-digit', second: '2-digit',
  }) + '.' + String(d.getMilliseconds()).padStart(3, '0');
}

function isWithinRange(iso: string, range: string, customFrom?: string, customTo?: string): boolean {
  if (range === 'all') return true;
  if (range === 'custom') {
    const t = new Date(iso).getTime();
    if (customFrom && t < new Date(customFrom).getTime()) return false;
    if (customTo && t > new Date(customTo + 'T23:59:59').getTime()) return false;
    return true;
  }
  const ms: Record<string, number> = { '5m': 300_000, '30m': 1_800_000, '1h': 3_600_000, '6h': 21_600_000, '1d': 86_400_000, '1w': 604_800_000 };
  return Date.now() - new Date(iso).getTime() <= (ms[range] ?? Infinity);
}

/* ── Log Row Component ───────────────────────────────────────────────────── */
function LogRow({ log, index }: { log: Record<string, unknown>; index: number }) {
  const [expanded, setExpanded] = useState(false);
  const level  = (log.level as string) ?? 'info';
  const meta   = LEVEL_META[level] ?? LEVEL_META.info;
  const hasExtra = log.metadata != null || log.stack != null || log.context != null;

  return (
    <div
      onClick={() => hasExtra && setExpanded(v => !v)}
      style={{
        background: expanded ? `${meta.bg}` : meta.rowBg,
        borderBottom: '1px solid rgba(255,255,255,0.03)',
        cursor: hasExtra ? 'pointer' : 'default',
        transition: 'background 0.15s',
      }}
    >
      <div style={{
        display: 'grid',
        gridTemplateColumns: '14px 105px 56px 110px 1fr',
        gap: 0,
        padding: '3px 14px 3px 8px',
        alignItems: 'flex-start',
        minHeight: 26,
      }}>
        {/* Expand indicator */}
        <div style={{ paddingTop: 4, color: 'rgba(255,255,255,0.2)', flexShrink: 0 }}>
          {hasExtra ? (
            <ChevronRight
              size={10}
              style={{ transform: expanded ? 'rotate(90deg)' : 'none', transition: 'transform 0.15s' }}
            />
          ) : null}
        </div>

        {/* Timestamp */}
        <span style={{
          fontSize: 10.5, color: 'rgba(255,255,255,0.32)',
          fontFamily: 'monospace', whiteSpace: 'nowrap', paddingTop: 3,
        }}>
          {log.timestamp ? formatTs(log.timestamp as string) : '—'}
        </span>

        {/* Level badge */}
        <span style={{
          fontSize: 9.5, fontWeight: 800, color: meta.color,
          letterSpacing: '0.05em', fontFamily: 'monospace',
          paddingTop: 3,
          ...(level === 'critical' ? { textShadow: `0 0 8px ${meta.color}` } : {}),
        }}>
          {meta.label}
        </span>

        {/* Source */}
        <span style={{
          fontSize: 10.5, color: 'rgba(255,255,255,0.38)',
          fontFamily: 'monospace', overflow: 'hidden',
          textOverflow: 'ellipsis', whiteSpace: 'nowrap', paddingTop: 3,
        }}>
          {(log.source as string) ?? '—'}
        </span>

        {/* Message */}
        <span style={{
          fontSize: 12, fontFamily: 'monospace',
          wordBreak: 'break-word', lineHeight: 1.55, paddingTop: 2,
          color: level === 'error' || level === 'critical'
            ? '#ff6b6b'
            : level === 'warning' || level === 'warn'
              ? '#fbbf24'
              : 'rgba(255,255,255,0.82)',
          fontWeight: level === 'critical' ? 700 : 400,
        }}>
          {log.message as string}
        </span>
      </div>

      {/* Expanded metadata */}
      <AnimatePresence>
        {expanded && hasExtra && (
          <motion.div
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: 'auto', opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18 }}
            style={{ overflow: 'hidden' }}
          >
            <div style={{
              margin: '0 14px 8px 36px',
              padding: '10px 14px', borderRadius: 7,
              background: 'rgba(255,255,255,0.04)',
              border: '1px solid rgba(255,255,255,0.06)',
              fontFamily: 'monospace', fontSize: 11,
              color: 'rgba(255,255,255,0.6)',
              whiteSpace: 'pre-wrap', lineHeight: 1.7,
            }}>
              {JSON.stringify(
                log.metadata ?? log.stack ?? log.context ?? {},
                null, 2,
              )}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ── Page ────────────────────────────────────────────────────────────────── */
export default function LogsPage() {
  const [viewTab, setViewTab] = useState<ViewTab>('logs');
  const [levelFilter, setLevelFilter] = useState('all');
  const [categoryFilter, setCategoryFilter] = useState('all');
  const [sourceFilter, setSourceFilter] = useState('');
  const [search, setSearch]           = useState('');
  const [timeRange, setTimeRange]     = useState('all');
  const [autoScroll, setAutoScroll]   = useState(true);
  const [execRunId, setExecRunId]     = useState('');
  const [customFrom, setCustomFrom]  = useState('');
  const [customTo, setCustomTo]      = useState('');
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const { logs, isLoading, mutate } = useLogs(
    levelFilter === 'all' ? undefined : levelFilter,
    MAX_LOGS,
  );
  const { executions, isLoading: execLoading } = useStepExecutions(execRunId || null);
  const { metrics: liveMetrics } = useLiveMetrics();

  /* Filter client-side */
  const filtered = useMemo(() => {
    const catSources = CATEGORY_FILTERS.find(c => c.id === categoryFilter)?.sources ?? [];
    return (logs as Record<string, unknown>[]).filter(l => {
      const matchLevel  = levelFilter === 'all' || l.level === levelFilter;
      const matchSource = !sourceFilter || ((l.source as string) ?? '').toLowerCase().includes(sourceFilter.toLowerCase());
      const matchCategory = categoryFilter === 'all' || catSources.length === 0 ||
        catSources.some(s => ((l.source as string) ?? '').toLowerCase().includes(s));
      const matchSearch = !search ||
        (l.message as string).toLowerCase().includes(search.toLowerCase()) ||
        ((l.source as string) ?? '').toLowerCase().includes(search.toLowerCase());
      const matchTime   = !l.timestamp || isWithinRange(l.timestamp as string, timeRange, customFrom, customTo);
      return matchLevel && matchSource && matchCategory && matchSearch && matchTime;
    });
  }, [logs, levelFilter, categoryFilter, sourceFilter, search, timeRange, customFrom, customTo]);

  /* Auto-scroll to bottom */
  useEffect(() => {
    if (autoScroll && bottomRef.current) {
      bottomRef.current.scrollIntoView({ behavior: 'smooth' });
    }
  }, [filtered, autoScroll]);

  /* Counts for summary badges */
  const counts = useMemo(() => {
    const all = logs as Record<string, unknown>[];
    return {
      debug:   all.filter(l => l.level === 'debug').length,
      info:    all.filter(l => l.level === 'info').length,
      warning: all.filter(l => l.level === 'warning' || l.level === 'warn').length,
      error:   all.filter(l => l.level === 'error' || l.level === 'critical').length,
    };
  }, [logs]);

  /* Download logs as JSONL */
  const handleDownload = useCallback(() => {
    const content = filtered.map(l => JSON.stringify(l)).join('\n');
    const blob = new Blob([content], { type: 'application/json' });
    const url  = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url; a.download = `logs-${Date.now()}.jsonl`;
    a.click(); URL.revokeObjectURL(url);
  }, [filtered]);

  /* Clear visible logs */
  const handleClear = useCallback(() => {
    setSearch(''); setSourceFilter(''); setLevelFilter('all'); setCategoryFilter('all'); setTimeRange('all');
  }, []);

  return (
    <>
      <style>{`
        @keyframes spin { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
        @keyframes livePulse {
          0%, 100% { opacity: 1; transform: scale(1); }
          50%       { opacity: 0.5; transform: scale(0.85); }
        }
      `}</style>

      <div style={{ padding: 20, maxWidth: '100%', height: 'calc(100vh - 48px)', display: 'flex', flexDirection: 'column' }}>
        {/* Header */}
        <motion.div
          initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
          style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 16, flexShrink: 0 }}
        >
          <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
            <div style={{
              width: 38, height: 38, borderRadius: 9,
              background: `${SECTION_COLOR}18`, border: `1.5px solid ${SECTION_COLOR}30`,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <ScrollText size={18} color={SECTION_COLOR} strokeWidth={2} />
            </div>
            <div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>
                  System Logs
                </h1>
                {/* Live indicator */}
                <div style={{ display: 'flex', alignItems: 'center', gap: 5, padding: '2px 8px', borderRadius: 20, background: '#10b98115', border: '1px solid #10b98130' }}>
                  <div style={{
                    width: 6, height: 6, borderRadius: '50%',
                    background: '#10b981',
                    animation: 'livePulse 1.8s ease-in-out infinite',
                  }} />
                  <span style={{ fontSize: 10, fontWeight: 700, color: '#10b981', letterSpacing: '0.05em' }}>LIVE</span>
                </div>
              </div>
              <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 3 }}>
                {filtered.length} / {(logs as Record<string, unknown>[]).length} entries shown · refreshes every 5s
              </p>
            </div>
          </div>
          <div style={{ display: 'flex', gap: 8 }}>
            <button
              onClick={handleClear}
              style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '7px 14px', borderRadius: 7, border: '1px solid var(--border-md)',
                background: 'var(--bg-surface)', cursor: 'pointer', fontSize: 12, color: 'var(--text-2)',
              }}
            >
              <X size={12} /> Clear Filters
            </button>
            <button
              onClick={handleDownload}
              style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '7px 14px', borderRadius: 7, border: '1px solid var(--border-md)',
                background: 'var(--bg-surface)', cursor: 'pointer', fontSize: 12, color: 'var(--text-2)',
              }}
            >
              <Download size={12} /> Download
            </button>
          </div>
        </motion.div>

        {/* Log count summary */}
        <motion.div
          initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ delay: 0.07 }}
          style={{ display: 'flex', gap: 8, marginBottom: 12, flexShrink: 0 }}
        >
          {[
            { label: 'Info',    count: counts.info,    color: '#3b82f6' },
            { label: 'Warning', count: counts.warning, color: '#f59e0b' },
            { label: 'Error',   count: counts.error,   color: '#ef4444' },
            { label: 'Debug',   count: counts.debug,   color: '#6b7280' },
          ].map(({ label, count, color }) => (
            <div
              key={label}
              style={{
                display: 'flex', alignItems: 'center', gap: 6,
                padding: '5px 12px', borderRadius: 8,
                background: `${color}10`, border: `1px solid ${color}25`,
              }}
            >
              <div style={{ width: 7, height: 7, borderRadius: '50%', background: color }} />
              <span style={{ fontSize: 11, fontWeight: 700, color }}>{count}</span>
              <span style={{ fontSize: 11, color: 'var(--text-4)' }}>{label}</span>
            </div>
          ))}
        </motion.div>

        {/* View tabs + Category filters */}
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.08 }}
          style={{ display: 'flex', gap: 8, marginBottom: 10, alignItems: 'center', flexShrink: 0, flexWrap: 'wrap' }}
        >
          {/* View tabs */}
          <div style={{ display: 'flex', gap: 3, marginRight: 8 }}>
            {([
              { id: 'logs' as const, label: 'System Logs', icon: ScrollText },
              { id: 'executions' as const, label: 'Audit Trail', icon: FileText },
            ]).map(t => (
              <button key={t.id} onClick={() => setViewTab(t.id)} style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: viewTab === t.id ? 700 : 400,
                border: `1.5px solid ${viewTab === t.id ? SECTION_COLOR : 'var(--border-md)'}`,
                background: viewTab === t.id ? `${SECTION_COLOR}12` : 'var(--bg-surface)',
                color: viewTab === t.id ? SECTION_COLOR : 'var(--text-3)',
                cursor: 'pointer', transition: 'all 0.15s',
              }}>
                <t.icon size={12} /> {t.label}
              </button>
            ))}
          </div>

          {/* Separator */}
          <div style={{ width: 1, height: 20, background: 'var(--border-md)' }} />

          {/* Category filters */}
          {CATEGORY_FILTERS.map(cat => {
            const active = categoryFilter === cat.id;
            const Icon = cat.icon;
            return (
              <button key={cat.id} onClick={() => setCategoryFilter(cat.id)} style={{
                display: 'flex', alignItems: 'center', gap: 4,
                padding: '4px 10px', borderRadius: 6, fontSize: 10, fontWeight: active ? 700 : 500,
                border: `1px solid ${active ? cat.color : 'var(--border)'}`,
                background: active ? `${cat.color}12` : 'transparent',
                color: active ? cat.color : 'var(--text-4)',
                cursor: 'pointer', transition: 'all 0.15s',
              }}>
                <Icon size={10} /> {cat.label}
              </button>
            );
          })}

          {/* Live stats */}
          {liveMetrics && (
            <div style={{ marginLeft: 'auto', display: 'flex', gap: 10, fontSize: 10, color: 'var(--text-4)' }}>
              <span>Agents: <strong style={{ color: '#2563EB' }}>{liveMetrics.activeAgents ?? 0}</strong></span>
              <span>Runs: <strong style={{ color: '#059669' }}>{liveMetrics.tasksCompleted ?? 0}</strong></span>
              <span>Errors: <strong style={{ color: '#ef4444' }}>{liveMetrics.tasksFailed ?? 0}</strong></span>
            </div>
          )}
        </motion.div>

        {/* Filter bar */}
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.1 }}
          style={{ display: 'flex', gap: 8, marginBottom: 12, alignItems: 'center', flexWrap: 'wrap', flexShrink: 0 }}
        >
          {/* Level filter buttons */}
          <div style={{ display: 'flex', gap: 4, flexShrink: 0 }}>
            {LEVEL_FILTERS.map(f => {
              const active = levelFilter === f.id;
              const meta = LEVEL_META[f.id];
              const col = meta?.color ?? SECTION_COLOR;
              return (
                <button
                  key={f.id}
                  onClick={() => setLevelFilter(f.id)}
                  style={{
                    padding: '4px 10px', borderRadius: 6, fontSize: 11, cursor: 'pointer',
                    border: active ? `1.5px solid ${col}` : '1px solid var(--border-md)',
                    background: active ? `${col}15` : 'var(--bg-surface)',
                    color: active ? col : 'var(--text-3)',
                    fontWeight: active ? 700 : 400,
                    fontFamily: 'monospace', transition: 'all 0.15s',
                  }}
                >
                  {f.id === 'all' ? 'ALL' : (meta?.label.trim() ?? f.label)}
                </button>
              );
            })}
          </div>

          {/* Source filter */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 7,
            padding: '5px 11px', borderRadius: 7, border: '1px solid var(--border-md)',
            background: 'var(--bg-surface)',
          }}>
            <Filter size={11} color="var(--text-4)" />
            <input
              value={sourceFilter}
              onChange={e => setSourceFilter(e.target.value)}
              placeholder="Source…"
              style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 12, color: 'var(--text-1)', width: 100, fontFamily: 'monospace' }}
            />
          </div>

          {/* Search */}
          <div style={{
            display: 'flex', alignItems: 'center', gap: 7,
            padding: '5px 11px', borderRadius: 7, border: '1px solid var(--border-md)',
            background: 'var(--bg-surface)',
          }}>
            <Search size={11} color="var(--text-4)" />
            <input
              value={search}
              onChange={e => setSearch(e.target.value)}
              placeholder="Search messages…"
              style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 12, color: 'var(--text-1)', width: 180, fontFamily: 'monospace' }}
            />
            {search && (
              <button onClick={() => setSearch('')} style={{ border: 'none', background: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-4)', display: 'flex', alignItems: 'center' }}>
                <X size={10} />
              </button>
            )}
          </div>

          {/* Time range */}
          <div style={{ position: 'relative', flexShrink: 0 }}>
            <Clock size={11} color="var(--text-4)" style={{ position: 'absolute', left: 10, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }} />
            <select
              value={timeRange}
              onChange={e => setTimeRange(e.target.value)}
              style={{
                padding: '5px 28px 5px 26px', borderRadius: 7, fontSize: 12,
                border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                color: 'var(--text-2)', cursor: 'pointer', outline: 'none', appearance: 'none',
              }}
            >
              {TIME_RANGES.map(r => (
                <option key={r.id} value={r.id}>{r.label}</option>
              ))}
            </select>
            <ChevronDown size={11} color="var(--text-4)" style={{ position: 'absolute', right: 8, top: '50%', transform: 'translateY(-50%)', pointerEvents: 'none' }} />
          </div>

          {/* Custom date range — shown when "Custom..." is selected */}
          {timeRange === 'custom' && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
              <input
                type="date"
                value={customFrom}
                onChange={e => setCustomFrom(e.target.value)}
                style={{
                  padding: '4px 8px', borderRadius: 6, fontSize: 11,
                  border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                  color: 'var(--text-2)', outline: 'none',
                  fontFamily: 'monospace',
                }}
              />
              <span style={{ fontSize: 10, color: 'var(--text-4)' }}>to</span>
              <input
                type="date"
                value={customTo}
                onChange={e => setCustomTo(e.target.value)}
                style={{
                  padding: '4px 8px', borderRadius: 6, fontSize: 11,
                  border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                  color: 'var(--text-2)', outline: 'none',
                  fontFamily: 'monospace',
                }}
              />
              {(customFrom || customTo) && (
                <button onClick={() => { setCustomFrom(''); setCustomTo(''); }}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-4)', display: 'flex' }}>
                  <X size={10} />
                </button>
              )}
            </div>
          )}

          {/* Auto-scroll toggle */}
          <div style={{ marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 7 }}>
            <span style={{ fontSize: 11, color: 'var(--text-4)' }}>Auto-scroll</span>
            <button
              onClick={() => setAutoScroll(v => !v)}
              style={{
                width: 36, height: 20, borderRadius: 10,
                background: autoScroll ? SECTION_COLOR : 'var(--border-md)',
                border: 'none', cursor: 'pointer',
                position: 'relative', transition: 'background 0.2s', flexShrink: 0,
              }}
              aria-label="Toggle auto-scroll"
            >
              <motion.div
                animate={{ x: autoScroll ? 17 : 2 }}
                transition={{ type: 'spring', damping: 20, stiffness: 300 }}
                style={{
                  position: 'absolute', top: 2,
                  width: 16, height: 16, borderRadius: '50%',
                  background: '#fff', boxShadow: '0 1px 3px rgba(0,0,0,0.3)',
                }}
              />
            </button>
          </div>
        </motion.div>

        {/* Audit Trail — execution history (shown when viewTab === 'executions') */}
        {viewTab === 'executions' && (
          <div style={{
            flex: 1, background: '#0c0c0c', borderRadius: 11,
            border: '1px solid rgba(255,255,255,0.07)',
            overflow: 'auto', fontFamily: 'monospace', minHeight: 0,
          }}>
            {/* Search bar for run ID */}
            <div style={{
              padding: '10px 14px', borderBottom: '1px solid rgba(255,255,255,0.05)',
              display: 'flex', alignItems: 'center', gap: 8,
              position: 'sticky', top: 0, background: '#0c0c0c', zIndex: 1,
            }}>
              {['#ef4444', '#f59e0b', '#10b981'].map(c => (
                <div key={c} style={{ width: 10, height: 10, borderRadius: '50%', background: c, opacity: 0.7 }} />
              ))}
              <span style={{ marginLeft: 4, fontSize: 11, color: 'rgba(255,255,255,0.25)' }}>audit trail</span>
              <div style={{
                marginLeft: 12, flex: 1, display: 'flex', alignItems: 'center', gap: 6,
                padding: '4px 10px', borderRadius: 5, border: '1px solid rgba(255,255,255,0.1)',
                background: 'rgba(255,255,255,0.03)',
              }}>
                <Search size={11} color="rgba(255,255,255,0.3)" />
                <input
                  value={execRunId}
                  onChange={e => setExecRunId(e.target.value)}
                  placeholder="Enter run ID to view execution history..."
                  style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 11, color: 'rgba(255,255,255,0.8)', flex: 1, fontFamily: 'monospace' }}
                />
                {execRunId && (
                  <button onClick={() => setExecRunId('')} style={{ background: 'none', border: 'none', cursor: 'pointer', padding: 0, color: 'rgba(255,255,255,0.3)', display: 'flex' }}>
                    <X size={10} />
                  </button>
                )}
              </div>
            </div>

            {!execRunId ? (
              <div style={{ padding: '48px 20px', textAlign: 'center', color: 'rgba(255,255,255,0.25)', fontSize: 12 }}>
                Enter a run ID above to view step execution audit trail, or select a run from{' '}
                <a href="/workflow/history" style={{ color: '#3b82f6', textDecoration: 'underline' }}>Workflow History</a>
              </div>
            ) : execLoading ? (
              <div style={{ padding: '48px 0', textAlign: 'center', color: 'rgba(255,255,255,0.3)', fontSize: 12 }}>
                <Loader2 size={18} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite', display: 'block' }} />
                Loading audit trail...
              </div>
            ) : (executions as Record<string, unknown>[]).length === 0 ? (
              <div style={{ padding: '48px 20px', color: 'rgba(255,255,255,0.25)', fontSize: 12 }}>
                No execution records found for run <code style={{ color: '#3b82f6' }}>{execRunId}</code>
              </div>
            ) : (
              <div>
                {/* Column headers */}
                <div style={{
                  display: 'grid',
                  gridTemplateColumns: '80px 100px 80px 90px 80px 70px 70px 1fr',
                  gap: 0, padding: '6px 14px',
                  borderBottom: '1px solid rgba(255,255,255,0.05)',
                  position: 'sticky', top: 44, background: '#0c0c0c', zIndex: 1,
                }}>
                  {['STATUS', 'STEP', 'AGENT', 'MODEL', 'TOKENS', 'CPU', 'DURATION', 'RESPONSE'].map(h => (
                    <span key={h} style={{ fontSize: 9, fontWeight: 700, color: 'rgba(255,255,255,0.18)', letterSpacing: '0.07em' }}>{h}</span>
                  ))}
                </div>

                {(executions as Record<string, unknown>[]).map((exec, i) => {
                  const status = (exec.status as string) || 'pending';
                  const statusColor = status === 'completed' ? '#10b981' : status === 'failed' ? '#ef4444' : status === 'thinking' ? '#f59e0b' : '#3b82f6';
                  return (
                    <div key={(exec.id as string) || i} style={{
                      display: 'grid',
                      gridTemplateColumns: '80px 100px 80px 90px 80px 70px 70px 1fr',
                      gap: 0, padding: '5px 14px',
                      borderBottom: '1px solid rgba(255,255,255,0.03)',
                      background: status === 'failed' ? 'rgba(239,68,68,0.04)' : 'transparent',
                      fontSize: 11, color: 'rgba(255,255,255,0.7)',
                    }}>
                      <span style={{ fontWeight: 700, color: statusColor, textTransform: 'uppercase', fontSize: 9, letterSpacing: '0.05em', paddingTop: 2 }}>
                        {status}
                      </span>
                      <span style={{ overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', color: 'rgba(255,255,255,0.5)' }}>
                        {(exec.stepName as string) || (exec.step_name as string) || (exec.stepId as string) || (exec.step_id as string) || '—'}
                      </span>
                      <span style={{ color: 'rgba(255,255,255,0.35)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {((exec.agentId as string) || (exec.agent_id as string) || '—').slice(-8)}
                      </span>
                      <span style={{ color: '#7C3AED', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {(exec.model as string) || '—'}
                      </span>
                      <span style={{ color: '#f59e0b' }}>
                        {((exec.tokensUsed as number) || (exec.tokens_used as number) || 0).toLocaleString()}
                      </span>
                      <span style={{ color: 'rgba(255,255,255,0.4)' }}>
                        {((exec.cpuPercent as number) || (exec.cpu_percent as number) || 0).toFixed(0)}%
                      </span>
                      <span style={{ color: 'rgba(255,255,255,0.5)' }}>
                        {(((exec.durationMs as number) || (exec.duration_ms as number) || 0) / 1000).toFixed(1)}s
                      </span>
                      <span style={{ color: 'rgba(255,255,255,0.4)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                        {((exec.responsePreview as string) || (exec.response_preview as string) || (exec.errorMessage as string) || (exec.error_message as string) || '').slice(0, 80) || '—'}
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        )}

        {/* Terminal window — system logs (shown when viewTab === 'logs') */}
        {viewTab === 'logs' && <div
          ref={containerRef}
          style={{
            flex: 1, background: '#0c0c0c', borderRadius: 11,
            border: '1px solid rgba(255,255,255,0.07)',
            overflow: 'auto', fontFamily: 'monospace',
            minHeight: 0,
          }}
        >
          {/* Terminal title bar */}
          <div style={{
            padding: '8px 14px', display: 'flex', alignItems: 'center', gap: 6,
            borderBottom: '1px solid rgba(255,255,255,0.05)',
            position: 'sticky', top: 0, background: '#0c0c0c', zIndex: 1,
          }}>
            {['#ef4444', '#f59e0b', '#10b981'].map(c => (
              <div key={c} style={{ width: 10, height: 10, borderRadius: '50%', background: c, opacity: 0.7 }} />
            ))}
            <span style={{ marginLeft: 8, fontSize: 11, color: 'rgba(255,255,255,0.25)', fontFamily: 'monospace' }}>
              kortecx-platform — system logs — {filtered.length} entries
            </span>
          </div>

          {isLoading ? (
            <div style={{ padding: '48px 0', textAlign: 'center', color: 'rgba(255,255,255,0.3)', fontSize: 12 }}>
              <Loader2 size={18} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite', display: 'block' }} />
              Loading logs…
            </div>
          ) : filtered.length === 0 ? (
            <div style={{ padding: '48px 20px', color: 'rgba(255,255,255,0.25)', fontSize: 12, fontFamily: 'monospace' }}>
              $ tail -f /var/log/kortecx.log<br />
              <span style={{ color: 'rgba(255,255,255,0.15)' }}>No log entries match your filters.</span>
            </div>
          ) : (
            <div>
              {/* Column headers */}
              <div style={{
                display: 'grid',
                gridTemplateColumns: '14px 105px 56px 110px 1fr',
                gap: 0, padding: '5px 14px 5px 8px',
                borderBottom: '1px solid rgba(255,255,255,0.05)',
                position: 'sticky', top: 42, background: '#0c0c0c', zIndex: 1,
              }}>
                {['', 'TIMESTAMP', 'LEVEL', 'SOURCE', 'MESSAGE'].map(h => (
                  <span key={h} style={{ fontSize: 9, fontWeight: 700, color: 'rgba(255,255,255,0.18)', letterSpacing: '0.07em', fontFamily: 'monospace' }}>
                    {h}
                  </span>
                ))}
              </div>

              {filtered.slice(0, MAX_LOGS).map((log, i) => (
                <LogRow key={(log.id ?? i) as string} log={log} index={i} />
              ))}

              {filtered.length > MAX_LOGS && (
                <div style={{ padding: '8px 22px', fontSize: 11, color: 'rgba(255,255,255,0.25)', fontFamily: 'monospace' }}>
                  … {filtered.length - MAX_LOGS} more entries not shown (refine your filters)
                </div>
              )}

              <div ref={bottomRef} style={{ height: 8 }} />
            </div>
          )}
        </div>}

        {/* Footer */}
        <div style={{
          marginTop: 8, display: 'flex', alignItems: 'center',
          justifyContent: 'space-between', flexShrink: 0,
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, color: 'var(--text-4)', fontSize: 11 }}>
            <ChevronDown size={11} />
            Showing {Math.min(filtered.length, MAX_LOGS)} of {filtered.length} matching entries · Max {MAX_LOGS} displayed
          </div>
          <button
            onClick={() => mutate()}
            style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '4px 10px', borderRadius: 6, fontSize: 11,
              border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
              cursor: 'pointer', color: 'var(--text-3)',
            }}
          >
            Refresh now
          </button>
        </div>
      </div>
    </>
  );
}
