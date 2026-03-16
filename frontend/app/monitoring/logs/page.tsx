'use client';

import { useEffect, useRef, useState, useCallback, useMemo } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ScrollText, Download, Search, ChevronDown,
  Loader2, X, ChevronRight, Filter,
  Clock,
} from 'lucide-react';
import { useLogs } from '@/lib/hooks/useApi';

/* ── Constants ──────────────────────────────────────────────────────────── */
const SECTION_COLOR = '#ef4444';
const MAX_LOGS      = 500;

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
  { id: '5m',  label: 'Last 5m' },
  { id: '30m', label: 'Last 30m' },
  { id: '1h',  label: 'Last 1h' },
  { id: '6h',  label: 'Last 6h' },
  { id: 'all', label: 'All time' },
];

/* ── Helpers ────────────────────────────────────────────────────────────── */
function formatTs(iso: string) {
  const d = new Date(iso);
  return d.toLocaleTimeString('en-US', {
    hour12: false, hour: '2-digit',
    minute: '2-digit', second: '2-digit',
  }) + '.' + String(d.getMilliseconds()).padStart(3, '0');
}

function isWithinRange(iso: string, range: string): boolean {
  if (range === 'all') return true;
  const ms: Record<string, number> = { '5m': 300_000, '30m': 1_800_000, '1h': 3_600_000, '6h': 21_600_000 };
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
  const [levelFilter, setLevelFilter] = useState('all');
  const [sourceFilter, setSourceFilter] = useState('');
  const [search, setSearch]           = useState('');
  const [timeRange, setTimeRange]     = useState('1h');
  const [autoScroll, setAutoScroll]   = useState(true);
  const bottomRef = useRef<HTMLDivElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const { logs, isLoading, mutate } = useLogs(
    levelFilter === 'all' ? undefined : levelFilter,
    MAX_LOGS,
  );

  /* Filter client-side */
  const filtered = useMemo(() => {
    return (logs as Record<string, unknown>[]).filter(l => {
      const matchLevel  = levelFilter === 'all' || l.level === levelFilter;
      const matchSource = !sourceFilter || ((l.source as string) ?? '').toLowerCase().includes(sourceFilter.toLowerCase());
      const matchSearch = !search ||
        (l.message as string).toLowerCase().includes(search.toLowerCase()) ||
        ((l.source as string) ?? '').toLowerCase().includes(search.toLowerCase());
      const matchTime   = !l.timestamp || isWithinRange(l.timestamp as string, timeRange);
      return matchLevel && matchSource && matchSearch && matchTime;
    });
  }, [logs, levelFilter, sourceFilter, search, timeRange]);

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
    setSearch(''); setSourceFilter(''); setLevelFilter('all'); setTimeRange('1h');
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

      <div style={{ padding: 28, maxWidth: 1280, height: 'calc(100vh - 56px)', display: 'flex', flexDirection: 'column' }}>
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

        {/* Terminal window */}
        <div
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
        </div>

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
