'use client';

import { useState, useMemo, useCallback, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  History, Search, RefreshCw, CheckCircle2, XCircle,
  Clock, Loader2, TrendingUp, ArrowRight, ChevronDown,
  ChevronLeft, ChevronRight, Download, Filter,
  AlertCircle, Play, Ban, BarChart2, Zap,
  Timer, ChevronUp, FileText, Workflow, ScrollText, Trash2,
  Cpu, FolderOpen,
} from 'lucide-react';
import useSWR from 'swr';
import { useWorkflowRuns, useWorkflows } from '@/lib/hooks/useApi';
import type { WorkflowRun, WorkflowStep } from '@/lib/types';
import RunDetailDialog from './_components/RunDetailDialog';

const statsFetcher = (url: string) => fetch(url).then(r => r.ok ? r.json() : null);

/* ── Section color ─────────────────────────────────── */
const SECTION_COLOR = '#06b6d4';

/* ── Constants ──────────────────────────────────────── */
const PAGE_SIZE = 25;
const DATE_RANGES = ['Today', 'Last 7 days', 'Last 30 days', 'Last 90 days', 'All time'] as const;
type DateRange = typeof DATE_RANGES[number];

/* ── Status meta ────────────────────────────────────── */
type RunStatus = 'completed' | 'failed' | 'running' | 'cancelled';

const STATUS_META: Record<RunStatus, { color: string; bg: string; icon: React.ElementType; label: string; animation?: string }> = {
  completed: { color: '#10b981', bg: '#10b98115', icon: CheckCircle2,  label: 'Completed', animation: 'fadeIn 0.3s ease-out' },
  failed:    { color: '#ef4444', bg: '#ef444415', icon: XCircle,       label: 'Failed',    animation: 'shake 0.4s ease-out' },
  running:   { color: '#3b82f6', bg: '#3b82f615', icon: Play,          label: 'Running',   animation: 'pulse 1.5s ease-in-out infinite' },
  cancelled: { color: '#6b7280', bg: '#6b728015', icon: Ban,           label: 'Cancelled' },
};

const STATUS_FILTERS = ['All', 'Completed', 'Failed', 'Running', 'Cancelled'] as const;

/* ── Grid template (shared between header, row, skeleton) ── */
const GRID_COLS = '22px minmax(0,1.4fr) minmax(0,2fr) minmax(0,1.6fr) 90px 64px 80px 24px 28px 28px 28px';

/* ── Helpers ────────────────────────────────────────── */
function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000)     return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function fmtDuration(secs: number): string {
  if (!secs) return '0s';
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

function fmtElapsed(iso: string): string {
  const d = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (d < 60)    return `${d}s ago`;
  if (d < 3600)  return `${Math.floor(d / 60)}m ago`;
  if (d < 86400) return `${Math.floor(d / 3600)}h ago`;
  return `${Math.floor(d / 86400)}d ago`;
}

function fmtDateTime(iso: string): string {
  return new Date(iso).toLocaleString(undefined, {
    month: 'short', day: 'numeric',
    hour: '2-digit', minute: '2-digit',
  });
}

function filterByDateRange(runs: WorkflowRun[], range: DateRange): WorkflowRun[] {
  if (range === 'All time') return runs;
  const now  = Date.now();
  const days: Record<DateRange, number> = {
    'Today': 1, 'Last 7 days': 7, 'Last 30 days': 30, 'Last 90 days': 90, 'All time': Infinity,
  };
  const ms = days[range] * 86_400_000;
  return runs.filter(r => now - new Date(r.startedAt).getTime() < ms);
}

function cpuColor(pct: number): string {
  if (pct > 80) return '#DC2626';
  if (pct > 50) return '#D97706';
  return '#10b981';
}

/* ── Skeleton row ───────────────────────────────────── */
function SkeletonRow() {
  return (
    <div style={{
      display: 'grid', gridTemplateColumns: GRID_COLS,
      gap: 12, alignItems: 'center', padding: '13px 16px',
      borderRadius: 9, border: '1px solid var(--border-sm)',
      background: 'var(--bg-surface)', marginBottom: 4,
    }}>
      {[22, 110, 160, 130, 70, 48, 60, 18].map((w, i) => (
        <div key={i} style={{
          height: 12, borderRadius: 6, width: w,
          background: 'var(--border-md)',
          animation: 'pulse 1.5s ease-in-out infinite',
          animationDelay: `${i * 0.06}s`,
        }} />
      ))}
    </div>
  );
}

/* ── Neural loading animation ─────────────────────── */
function NeuralLoader() {
  return (
    <div style={{ position: 'relative', width: 18, height: 18 }}>
      {/* Outer orbit ring */}
      <span style={{
        position: 'absolute', inset: -2, borderRadius: '50%',
        border: '1.5px solid transparent',
        borderTopColor: '#3b82f6', borderBottomColor: '#3b82f640',
        animation: 'orbit 1.2s linear infinite',
      }} />
      {/* Middle pulse ring */}
      <span style={{
        position: 'absolute', inset: 1, borderRadius: '50%',
        border: '1px solid transparent',
        borderLeftColor: '#60a5fa', borderRightColor: '#60a5fa40',
        animation: 'orbit 0.8s linear infinite reverse',
      }} />
      {/* Core dot */}
      <span style={{
        position: 'absolute', top: '50%', left: '50%',
        transform: 'translate(-50%, -50%)',
        width: 4, height: 4, borderRadius: '50%',
        background: '#3b82f6',
        animation: 'neuralPulse 0.6s ease-in-out infinite alternate',
        boxShadow: '0 0 6px #3b82f680',
      }} />
    </div>
  );
}

/* ── Step badge ─────────────────────────────────────── */
const STEP_STATUS_META: Record<string, { color: string; icon: React.ElementType }> = {
  completed: { color: '#10b981', icon: CheckCircle2 },
  failed:    { color: '#ef4444', icon: XCircle },
  running:   { color: '#3b82f6', icon: Loader2 },
  pending:   { color: '#6b7280', icon: Clock },
  skipped:   { color: '#9ca3af', icon: Ban },
};

function StepBreakdown({ steps }: { steps: WorkflowStep[] }) {
  if (!steps || steps.length === 0) return (
    <div style={{ fontSize: 12, color: 'var(--text-4)', fontStyle: 'italic' }}>No step data available</div>
  );
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
      {steps.map((step, i) => {
        const sm = STEP_STATUS_META[step.status ?? 'pending'] ?? STEP_STATUS_META.pending;
        const StepIcon = sm.icon;
        return (
          <div key={step.id || i} style={{
            display: 'flex', alignItems: 'flex-start', gap: 8,
            padding: '7px 10px', borderRadius: 7,
            background: `${sm.color}08`, border: `1px solid ${sm.color}18`,
          }}>
            <StepIcon size={13} color={sm.color} style={{
              flexShrink: 0, marginTop: 1,
              ...(step.status === 'running' ? { animation: 'spin 1s linear infinite' } : {}),
            }} />
            <div style={{ minWidth: 0, flex: 1 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                <span style={{ fontSize: 11, fontWeight: 700, color: sm.color }}>#{step.order ?? i + 1}</span>
                <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>
                  {step.expertName || step.label || step.id}
                </span>
                {step.tokensUsed != null && step.tokensUsed > 0 && (
                  <span style={{ fontSize: 10, color: 'var(--text-4)', marginLeft: 'auto', flexShrink: 0 }}>
                    {fmtTokens(step.tokensUsed)} tok
                  </span>
                )}
              </div>
              {step.taskDescription && (
                <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 2, lineHeight: 1.4 }}>
                  {step.taskDescription.slice(0, 120)}{step.taskDescription.length > 120 ? '...' : ''}
                </div>
              )}
              {step.error && (
                <div style={{ fontSize: 11, color: '#ef4444', marginTop: 3, padding: '3px 7px', borderRadius: 4, background: '#ef444410' }}>
                  {step.error}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

/* ── Logs Panel ────────────────────────────────────── */
function LogsPanel({ runId, workflowId }: { runId: string; workflowId: string }) {
  const [expanded, setExpanded] = useState(false);
  const [logs, setLogs] = useState<Array<{ timestamp: string; level: string; message: string }>>([]);
  const [loading, setLoading] = useState(false);

  const fetchLogs = useCallback(async () => {
    if (logs.length > 0) return;
    setLoading(true);
    try {
      const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';
      const res = await fetch(`${ENGINE_URL}/api/logs/run/${workflowId}/${runId}`);
      if (res.ok) {
        const data = await res.json();
        // Engine returns { log: "..." } — plain text with newline-delimited entries
        if (typeof data.log === 'string' && data.log.trim()) {
          const parsed = data.log.split('\n').filter(Boolean).map((line: string) => {
            const match = line.match(/^\[(.+?)\]\s+(\w+)\s+(.+)$/);
            if (match) return { timestamp: match[1], level: match[2], message: match[3] };
            // Try alternate format: "TIMESTAMP | LEVEL | message"
            const alt = line.match(/^(.+?)\s*\|\s*(\w+)\s*\|\s*(.+)$/);
            if (alt) return { timestamp: alt[1], level: alt[2], message: alt[3] };
            return { timestamp: '', level: 'info', message: line };
          });
          setLogs(parsed);
        } else if (Array.isArray(data.logs ?? data.entries)) {
          setLogs(data.logs ?? data.entries ?? []);
        }
      }
    } catch {
      // Fallback to frontend logs
      try {
        const res = await fetch(`/api/monitoring/logs?runId=${runId}&limit=100`);
        if (res.ok) {
          const data = await res.json();
          setLogs(data.logs ?? []);
        }
      } catch {
        // Non-critical
      }
    } finally {
      setLoading(false);
    }
  }, [runId, workflowId, logs.length]);

  const toggleLogs = () => {
    if (!expanded) fetchLogs();
    setExpanded(!expanded);
  };

  const levelColor = (level: string) => {
    switch (level.toLowerCase()) {
      case 'error': return '#ef4444';
      case 'warn': case 'warning': return '#f59e0b';
      case 'info': return '#3b82f6';
      default: return 'var(--text-4)';
    }
  };

  return (
    <div style={{ borderTop: '1px solid var(--border-sm)' }}>
      <button onClick={toggleLogs} style={{
        display: 'flex', alignItems: 'center', gap: 6, width: '100%',
        padding: '10px 20px', background: 'none', border: 'none', cursor: 'pointer',
        fontSize: 11, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase',
        letterSpacing: '0.06em',
      }}>
        <ScrollText size={12} />
        Execution Logs
        {expanded ? <ChevronUp size={11} /> : <ChevronDown size={11} />}
      </button>
      <AnimatePresence>
        {expanded && (
          <motion.div
            initial={{ opacity: 0, height: 0 }}
            animate={{ opacity: 1, height: 'auto' }}
            exit={{ opacity: 0, height: 0 }}
            transition={{ duration: 0.2 }}
            style={{ overflow: 'hidden' }}
          >
            <div style={{
              margin: '0 20px 16px', padding: '10px 12px',
              background: 'var(--bg-canvas, rgba(0,0,0,0.03))',
              border: '1px solid var(--border-sm)', borderRadius: 6,
              maxHeight: 240, overflowY: 'auto',
              fontFamily: 'var(--font-mono, monospace)', fontSize: 11, lineHeight: 1.6,
            }}>
              {loading && (
                <div style={{ display: 'flex', alignItems: 'center', gap: 6, color: 'var(--text-4)', padding: '8px 0' }}>
                  <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> Loading logs...
                </div>
              )}
              {!loading && logs.length === 0 && (
                <div style={{ color: 'var(--text-4)', fontStyle: 'italic', padding: '8px 0' }}>
                  No logs available for this run.
                </div>
              )}
              {logs.map((log, i) => (
                <div key={i} style={{ padding: '2px 0', borderBottom: i < logs.length - 1 ? '1px solid var(--border-sm)' : 'none' }}>
                  <span style={{ color: 'var(--text-4)', marginRight: 8 }}>
                    {log.timestamp ? (() => { try { return new Date(log.timestamp).toLocaleTimeString(); } catch { return log.timestamp; } })() : '—'}
                  </span>
                  <span style={{ color: levelColor(log.level || 'info'), fontWeight: 600, marginRight: 8, textTransform: 'uppercase', fontSize: 9 }}>
                    {log.level || 'INFO'}
                  </span>
                  <span style={{ color: 'var(--text-2)' }}>{log.message}</span>
                </div>
              ))}
            </div>
          </motion.div>
        )}
      </AnimatePresence>
    </div>
  );
}

/* ── Expanded row panel ─────────────────────────────── */
function ExpandedPanel({ run }: { run: WorkflowRun }) {
  return (
    <motion.div
      initial={{ opacity: 0, height: 0 }} animate={{ opacity: 1, height: 'auto' }}
      exit={{ opacity: 0, height: 0 }}
      transition={{ duration: 0.22, ease: [0.25, 0.46, 0.45, 0.94] as const }}
      style={{ overflow: 'hidden' }}
    >
      <div style={{
        padding: '16px 20px 20px',
        borderTop: '1px solid var(--border-sm)',
        display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20,
      }}>
        {/* Left col — input + error/output */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          <div>
            <div style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 6 }}>
              Full Input
            </div>
            <div style={{
              fontSize: 12, color: 'var(--text-2)', lineHeight: 1.55,
              padding: '10px 12px', borderRadius: 7,
              background: 'var(--bg-canvas, rgba(0,0,0,0.03))',
              border: '1px solid var(--border-sm)',
              maxHeight: 140, overflowY: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
            }}>
              {run.input || <em style={{ color: 'var(--text-4)' }}>No input recorded</em>}
            </div>
          </div>

          {run.error && (
            <div>
              <div style={{ fontSize: 11, fontWeight: 700, color: '#ef4444', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 6 }}>
                Error
              </div>
              <div style={{
                fontSize: 12, color: '#ef4444', lineHeight: 1.5,
                padding: '10px 12px', borderRadius: 7,
                background: '#ef444410', border: '1px solid #ef444425',
              }}>
                {run.error}
              </div>
            </div>
          )}

          {run.output && run.status === 'completed' && (
            <div>
              <div style={{ fontSize: 11, fontWeight: 700, color: '#10b981', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 6 }}>
                Output Preview
              </div>
              <div style={{
                fontSize: 12, color: 'var(--text-2)', lineHeight: 1.55,
                padding: '10px 12px', borderRadius: 7,
                background: '#10b98108', border: '1px solid #10b98120',
                maxHeight: 140, overflowY: 'auto', whiteSpace: 'pre-wrap', wordBreak: 'break-word',
              }}>
                {run.output.slice(0, 600)}{run.output.length > 600 ? '...' : ''}
              </div>
            </div>
          )}
        </div>

        {/* Right col — step breakdown */}
        <div>
          <div style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 8 }}>
            Step Execution ({run.steps?.length ?? 0} steps)
          </div>
          <StepBreakdown steps={run.steps ?? []} />
        </div>
      </div>

      {/* View assets button */}
      <div style={{ padding: '0 20px 12px', display: 'flex', gap: 8 }}>
        <a
          href={`/data?tab=assets&runId=${run.id}`}
          style={{
            display: 'inline-flex', alignItems: 'center', gap: 6,
            padding: '7px 14px', borderRadius: 7,
            border: `1px solid ${SECTION_COLOR}40`, background: `${SECTION_COLOR}08`,
            color: SECTION_COLOR, fontSize: 12, fontWeight: 600,
            textDecoration: 'none', transition: 'all 0.15s',
          }}
          onMouseEnter={e => { e.currentTarget.style.background = `${SECTION_COLOR}18`; }}
          onMouseLeave={e => { e.currentTarget.style.background = `${SECTION_COLOR}08`; }}
        >
          <FolderOpen size={13} /> View Generated Assets
        </a>
      </div>

      {/* Logs section */}
      <LogsPanel runId={run.id} workflowId={run.workflowId} />

      {/* Metadata footer */}
      <div style={{
        padding: '8px 20px', borderTop: '1px solid var(--border-sm)',
        display: 'flex', gap: 20, flexWrap: 'wrap',
        fontSize: 11, color: 'var(--text-4)',
      }}>
        <span>ID: <code style={{ fontSize: 10, color: 'var(--text-3)' }}>{run.id}</code></span>
        {run.startedAt   && <span>Started: {fmtDateTime(run.startedAt)}</span>}
        {run.completedAt && <span>Completed: {fmtDateTime(run.completedAt)}</span>}
      </div>
    </motion.div>
  );
}

/* ── Run row ────────────────────────────────────────── */
function RunRow({ run, expanded, onToggle, onDelete, onCancel, onViewDetails, sysStats }: {
  run: WorkflowRun; expanded: boolean; onToggle: () => void; onDelete: () => void;
  onCancel: () => void; onViewDetails: () => void;
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  sysStats?: any;
}) {
  const status = (run.status as RunStatus) in STATUS_META ? (run.status as RunStatus) : 'cancelled';
  const meta   = STATUS_META[status];

  /* Live duration counter for running workflows */
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (status !== 'running' || !run.startedAt) return;
    const start = new Date(run.startedAt).getTime();
    const tick = () => setElapsed(Math.floor((Date.now() - start) / 1000));
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [status, run.startedAt]);

  const displayDuration = status === 'running'
    ? fmtDuration(elapsed)
    : run.durationSec != null ? fmtDuration(run.durationSec) : '—';

  /* CPU display */
  const cpuPct = status === 'running' && sysStats
    ? (sysStats.cpu_percent ?? 0)
    : null;

  return (
    <div style={{
      borderRadius: 9, border: `1px solid ${expanded ? `${SECTION_COLOR}30` : 'var(--border-sm)'}`,
      background: 'var(--bg-surface)', marginBottom: 4, overflow: 'hidden',
      transition: 'border-color 0.15s, box-shadow 0.15s',
      boxShadow: expanded ? `0 0 0 2px ${SECTION_COLOR}15` : 'none',
    }}>
      {/* Main row */}
      <div
        onClick={onToggle}
        style={{
          display: 'grid', gridTemplateColumns: GRID_COLS,
          gap: 12, alignItems: 'center', padding: '12px 16px', cursor: 'pointer',
          transition: 'background 0.12s',
        }}
        onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-hover, rgba(0,0,0,0.03))')}
        onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
      >
        {/* 1. Status icon */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
          {status === 'running' ? (
            <NeuralLoader />
          ) : (
            <div style={{ animation: meta.animation }}>
              <meta.icon size={14} color={meta.color} />
            </div>
          )}
        </div>

        {/* 2. Workflow name */}
        <div style={{ minWidth: 0 }}>
          <div style={{
            fontSize: 13, fontWeight: 700, color: 'var(--text-1)',
            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
          }}>
            {run.workflowName || 'Unnamed Workflow'}
          </div>
          <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 1 }}>
            {run.startedAt ? fmtElapsed(run.startedAt) : '—'}
          </div>
        </div>

        {/* 3. Input (truncated) */}
        <div style={{
          fontSize: 12, color: 'var(--text-2)', minWidth: 0,
          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
        }}>
          {run.input ? run.input.slice(0, 80) : <span style={{ color: 'var(--text-4)', fontStyle: 'italic' }}>No input</span>}
        </div>

        {/* 4. Expert Chain */}
        <div style={{ minWidth: 0, overflow: 'hidden' }}>
          {run.expertChain && run.expertChain.length > 0 ? (
            <div style={{ display: 'flex', alignItems: 'center', gap: 3, overflow: 'hidden' }}>
              {run.expertChain.slice(0, 3).map((e, i) => (
                <span key={i} style={{ display: 'flex', alignItems: 'center', gap: 3, flexShrink: 0 }}>
                  <span style={{
                    fontSize: 10, color: SECTION_COLOR,
                    padding: '1px 5px', borderRadius: 3,
                    background: `${SECTION_COLOR}12`,
                    whiteSpace: 'nowrap',
                  }}>{e}</span>
                  {i < run.expertChain.slice(0, 3).length - 1 && <ArrowRight size={8} color="var(--text-4)" />}
                </span>
              ))}
              {run.expertChain.length > 3 && (
                <span style={{ fontSize: 10, color: 'var(--text-4)' }}>+{run.expertChain.length - 3}</span>
              )}
            </div>
          ) : (
            <span style={{ fontSize: 11, color: 'var(--text-4)' }}>—</span>
          )}
        </div>

        {/* 5. Tokens (consumed + generated) */}
        <div style={{ fontSize: 12, fontVariantNumeric: 'tabular-nums', minWidth: 0 }}>
          {run.totalTokensUsed ? (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
              <span style={{ color: 'var(--text-2)', fontWeight: 600 }}>{fmtTokens(run.totalTokensUsed)}</span>
              <span style={{ fontSize: 9, color: 'var(--text-4)' }}>total</span>
            </div>
          ) : (
            <span style={{ color: 'var(--text-4)' }}>—</span>
          )}
        </div>

        {/* 6. CPU */}
        <div style={{ fontSize: 11, fontVariantNumeric: 'tabular-nums' }}>
          {cpuPct != null ? (
            <span style={{ color: cpuColor(cpuPct), fontWeight: 600, display: 'flex', alignItems: 'center', gap: 3 }}>
              <Cpu size={10} />
              {cpuPct.toFixed(0)}%
            </span>
          ) : (
            <span style={{ color: 'var(--text-4)' }}>—</span>
          )}
        </div>

        {/* 7. Duration / Time */}
        <div style={{
          fontSize: 12, fontVariantNumeric: 'tabular-nums',
          color: status === 'running' ? '#3b82f6' : 'var(--text-2)',
          fontWeight: status === 'running' ? 600 : 400,
        }}>
          {displayDuration}
        </div>

        {/* 8. Expand chevron */}
        <div style={{
          width: 20, height: 20, display: 'flex', alignItems: 'center', justifyContent: 'center',
          borderRadius: 4, color: 'var(--text-4)', flexShrink: 0,
        }}>
          {expanded ? <ChevronUp size={12} /> : <ChevronDown size={12} />}
        </div>

        {/* 9. Cancel button (running only) */}
        {status === 'running' ? (
          <button
            onClick={e => { e.stopPropagation(); onCancel(); }}
            title="Cancel run"
            style={{
              width: 26, height: 26, borderRadius: 6,
              border: '1px solid #ef444440',
              background: '#ef444410',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              cursor: 'pointer', color: '#ef4444', transition: 'all 0.15s',
            }}
          >
            <Ban size={11} />
          </button>
        ) : (
          <div style={{ width: 26 }} />
        )}

        {/* 10. Details button */}
        <button
          onClick={e => { e.stopPropagation(); onViewDetails(); }}
          title="View details"
          style={{
            width: 26, height: 26, borderRadius: 6,
            border: `1px solid ${SECTION_COLOR}40`,
            background: `${SECTION_COLOR}08`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            cursor: 'pointer', color: SECTION_COLOR, transition: 'all 0.15s',
          }}
        >
          <FileText size={11} />
        </button>

        {/* 10. Delete (far right) */}
        <button
          onClick={e => { e.stopPropagation(); onDelete(); }}
          title="Delete run"
          style={{
            width: 26, height: 26, borderRadius: 6, border: '1px solid transparent',
            background: 'transparent', display: 'flex', alignItems: 'center', justifyContent: 'center',
            cursor: 'pointer', color: 'var(--text-4)', transition: 'all 0.15s',
          }}
          onMouseEnter={e => { e.currentTarget.style.color = '#ef4444'; e.currentTarget.style.background = '#ef444410'; e.currentTarget.style.borderColor = '#ef444425'; }}
          onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-4)'; e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = 'transparent'; }}
        >
          <Trash2 size={12} />
        </button>
      </div>

      {/* Expanded detail panel */}
      <AnimatePresence>
        {expanded && <ExpandedPanel key="panel" run={run} />}
      </AnimatePresence>
    </div>
  );
}

/* ── Stat card ──────────────────────────────────────── */
function StatCard({ icon: Icon, label, value, sub }: { icon: React.ElementType; label: string; value: string; sub?: string }) {
  return (
    <div style={{
      padding: '14px 18px', borderRadius: 10, flex: 1,
      background: 'var(--bg-surface)', border: '1px solid var(--border-sm)',
      display: 'flex', alignItems: 'center', gap: 12,
    }}>
      <div style={{
        width: 34, height: 34, borderRadius: 8,
        background: `${SECTION_COLOR}15`, border: `1.5px solid ${SECTION_COLOR}25`,
        display: 'flex', alignItems: 'center', justifyContent: 'center', flexShrink: 0,
      }}>
        <Icon size={15} color={SECTION_COLOR} />
      </div>
      <div style={{ minWidth: 0 }}>
        <div style={{ fontSize: 11, color: 'var(--text-4)', fontWeight: 600, textTransform: 'uppercase', letterSpacing: '0.04em' }}>{label}</div>
        <div style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.1, marginTop: 2 }}>{value}</div>
        {sub && <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2 }}>{sub}</div>}
      </div>
    </div>
  );
}

/* ══════════════════════════════════════════════════════
   MAIN PAGE
══════════════════════════════════════════════════════ */
export default function WorkflowHistoryPage() {
  const [search,       setSearch]       = useState('');
  const [statusFilter, setStatusFilter] = useState('All');
  const [dateRange,    setDateRange]    = useState<DateRange>('Last 30 days');
  const [workflowFilter, setWorkflowFilter] = useState('all');
  const [page,         setPage]         = useState(1);
  const [expandedId,   setExpandedId]   = useState<string | null>(null);
  const [showFilters,  setShowFilters]  = useState(false);
  const [deletingRunId, setDeletingRunId] = useState<string | null>(null);
  const [deleting, setDeleting] = useState(false);
  const [detailRun, setDetailRun] = useState<WorkflowRun | null>(null);

  const { runs, total, isLoading, mutate } = useWorkflowRuns(undefined, 500);
  const { workflows } = useWorkflows();
  const hasRunning = (runs as WorkflowRun[]).some(r => r.status === 'running');
  const { data: sysStats } = useSWR(hasRunning ? '/api/system/stats' : null, statsFetcher, { refreshInterval: 3000 });

  /* ── Filtering ── */
  const filtered = useMemo(() => {
    let result = filterByDateRange(runs as WorkflowRun[], dateRange);

    if (statusFilter !== 'All') {
      result = result.filter(r => r.status === statusFilter.toLowerCase());
    }
    if (workflowFilter !== 'all') {
      result = result.filter(r => r.workflowId === workflowFilter);
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      result = result.filter(r =>
        r.workflowName.toLowerCase().includes(q) ||
        (r.input ?? '').toLowerCase().includes(q) ||
        r.id.toLowerCase().includes(q),
      );
    }
    return result;
  }, [runs, statusFilter, dateRange, workflowFilter, search]);

  /* ── Pagination ── */
  const totalPages  = Math.max(1, Math.ceil(filtered.length / PAGE_SIZE));
  const currentPage = Math.min(page, totalPages);
  const pageRuns    = filtered.slice((currentPage - 1) * PAGE_SIZE, currentPage * PAGE_SIZE);

  /* ── Stats ── */
  const stats = useMemo(() => {
    const r = filtered;
    const completed   = r.filter(x => x.status === 'completed').length;
    const successRate = r.length > 0 ? Math.round((completed / r.length) * 100) : 0;
    const avgDur      = r.filter(x => x.durationSec).length > 0
      ? Math.round(r.reduce((a, x) => a + (x.durationSec ?? 0), 0) / r.filter(x => x.durationSec).length)
      : 0;
    const totalTok    = r.reduce((a, x) => a + (x.totalTokensUsed ?? 0), 0);
    return { count: r.length, successRate, avgDur, totalTok };
  }, [filtered]);

  function toggleExpand(id: string) {
    setExpandedId(prev => prev === id ? null : id);
  }

  function handleExport() {
    const csv = [
      ['ID', 'Workflow', 'Status', 'Duration(s)', 'Tokens', 'Started'].join(','),
      ...filtered.map(r => [r.id, r.workflowName, r.status, r.durationSec ?? '', r.totalTokensUsed ?? '', r.startedAt].join(',')),
    ].join('\n');
    const blob = new Blob([csv], { type: 'text/csv' });
    const url  = URL.createObjectURL(blob);
    const a    = document.createElement('a');
    a.href = url; a.download = 'workflow-runs.csv'; a.click();
    URL.revokeObjectURL(url);
  }

  async function handleCancelRun(runId: string, workflowId: string) {
    try {
      await fetch('/api/workflows/stop', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ runId, workflowId }),
      });
      mutate();
    } catch (err) {
      console.error('Failed to cancel run:', err);
    }
  }

  async function handleDeleteRun() {
    if (!deletingRunId) return;
    setDeleting(true);
    try {
      await fetch(`/api/workflows/runs?id=${deletingRunId}`, { method: 'DELETE' });
      mutate();
    } catch (err) {
      console.error('Failed to delete run:', err);
    } finally {
      setDeleting(false);
      setDeletingRunId(null);
    }
  }

  /* ══════════════════════════════════════════════════
     RENDER
  ══════════════════════════════════════════════════ */
  return (
    <div style={{ padding: 28, maxWidth: 1200 }}>

      {/* ── Page header ── */}
      <motion.div
        initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.28 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 22, flexWrap: 'wrap', gap: 10 }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 38, height: 38, borderRadius: 9,
            background: `${SECTION_COLOR}18`, border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <History size={18} color={SECTION_COLOR} strokeWidth={2} />
          </div>
          <div>
            <h1 style={{ fontSize: 19, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>Run History</h1>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 3 }}>
              {total} total runs · auto-refreshes every 5s
            </p>
          </div>
        </div>

        <div style={{ display: 'flex', gap: 8 }}>
          {/* Date range */}
          <select
            value={dateRange}
            onChange={e => { setDateRange(e.target.value as DateRange); setPage(1); }}
            style={{
              padding: '7px 12px', borderRadius: 7, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', color: 'var(--text-2)', fontSize: 12, cursor: 'pointer',
              appearance: 'none' as const, paddingRight: 28,
              backgroundImage: `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='11' height='11' viewBox='0 0 24 24' fill='none' stroke='%236b7280' stroke-width='2'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E")`,
              backgroundRepeat: 'no-repeat', backgroundPosition: 'right 8px center',
            }}
          >
            {DATE_RANGES.map(r => <option key={r} value={r}>{r}</option>)}
          </select>

          {/* Export */}
          <button
            onClick={handleExport}
            style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '7px 14px', borderRadius: 7, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', cursor: 'pointer', fontSize: 12, color: 'var(--text-2)',
            }}
          >
            <Download size={12} /> Export
          </button>

          {/* Refresh */}
          <button
            onClick={() => mutate()}
            style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '7px 14px', borderRadius: 7, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', cursor: 'pointer', fontSize: 12, color: 'var(--text-2)',
            }}
          >
            <RefreshCw size={12} /> Refresh
          </button>
        </div>
      </motion.div>

      {/* ── Stats bar (no cost) ── */}
      <motion.div
        initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.08 }}
        style={{ display: 'flex', gap: 10, marginBottom: 20, flexWrap: 'wrap' }}
      >
        <StatCard icon={BarChart2}  label="Total Runs"   value={stats.count.toLocaleString()} />
        <StatCard icon={TrendingUp} label="Success Rate" value={`${stats.successRate}%`} sub={`${filtered.filter(r => r.status === 'completed').length} completed`} />
        <StatCard icon={Timer}      label="Avg Duration" value={stats.avgDur > 0 ? fmtDuration(stats.avgDur) : '—'} />
        <StatCard icon={Zap}        label="Total Tokens" value={fmtTokens(stats.totalTok)} />
      </motion.div>

      {/* ── Filter bar ── */}
      <motion.div
        initial={{ opacity: 0 }} animate={{ opacity: 1 }}
        transition={{ delay: 0.12 }}
        style={{ marginBottom: 14 }}
      >
        <div style={{ display: 'flex', gap: 8, alignItems: 'center', flexWrap: 'wrap' }}>
          {/* Status pills */}
          <div style={{ display: 'flex', gap: 4 }}>
            {STATUS_FILTERS.map(s => (
              <button
                key={s}
                onClick={() => { setStatusFilter(s); setPage(1); }}
                style={{
                  padding: '5px 11px', borderRadius: 20, fontSize: 11, cursor: 'pointer',
                  border: statusFilter === s ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
                  background: statusFilter === s ? `${SECTION_COLOR}15` : 'var(--bg-surface)',
                  color: statusFilter === s ? SECTION_COLOR : 'var(--text-3)',
                  fontWeight: statusFilter === s ? 700 : 400, transition: 'all 0.12s',
                }}
              >
                {s}
              </button>
            ))}
          </div>

          {/* Workflow filter */}
          <select
            value={workflowFilter}
            onChange={e => { setWorkflowFilter(e.target.value); setPage(1); }}
            style={{
              padding: '5px 10px', borderRadius: 7, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', color: 'var(--text-2)', fontSize: 12, cursor: 'pointer',
              appearance: 'none' as const, paddingRight: 24,
              backgroundImage: `url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10' viewBox='0 0 24 24' fill='none' stroke='%236b7280' stroke-width='2'%3E%3Cpolyline points='6 9 12 15 18 9'%3E%3C/polyline%3E%3C/svg%3E")`,
              backgroundRepeat: 'no-repeat', backgroundPosition: 'right 6px center',
            }}
          >
            <option value="all">All Workflows</option>
            {(workflows as Array<{ id: string; name: string }>).map(w => (
              <option key={w.id} value={w.id}>{w.name}</option>
            ))}
          </select>

          {/* Extra filters toggle */}
          <button
            onClick={() => setShowFilters(!showFilters)}
            style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '5px 10px', borderRadius: 7, fontSize: 11, cursor: 'pointer',
              border: showFilters ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
              background: showFilters ? `${SECTION_COLOR}10` : 'var(--bg-surface)',
              color: showFilters ? SECTION_COLOR : 'var(--text-3)',
            }}
          >
            <Filter size={11} /> Filters
          </button>

          {/* Search */}
          <div style={{
            marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 7,
            padding: '5px 12px', borderRadius: 8, border: '1px solid var(--border-md)',
            background: 'var(--bg-surface)',
          }}>
            <Search size={12} color="var(--text-4)" />
            <input
              value={search}
              onChange={e => { setSearch(e.target.value); setPage(1); }}
              placeholder="Search by name or input..."
              style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 12, color: 'var(--text-1)', width: 200 }}
            />
            {search && (
              <button onClick={() => setSearch('')} style={{ border: 'none', background: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-4)', lineHeight: 1 }}>x</button>
            )}
          </div>
        </div>

        {/* Active filter chips */}
        {(statusFilter !== 'All' || workflowFilter !== 'all' || search) && (
          <div style={{ display: 'flex', gap: 6, marginTop: 8, flexWrap: 'wrap', alignItems: 'center' }}>
            <span style={{ fontSize: 11, color: 'var(--text-4)' }}>{filtered.length} result{filtered.length !== 1 ? 's' : ''}</span>
            {statusFilter !== 'All' && (
              <span style={{
                display: 'flex', alignItems: 'center', gap: 4, fontSize: 11,
                padding: '2px 8px', borderRadius: 4, background: `${SECTION_COLOR}12`, color: SECTION_COLOR,
              }}>
                Status: {statusFilter}
                <button onClick={() => setStatusFilter('All')} style={{ border: 'none', background: 'none', cursor: 'pointer', color: SECTION_COLOR, padding: 0, lineHeight: 1, marginLeft: 2 }}>x</button>
              </span>
            )}
            {search && (
              <span style={{
                display: 'flex', alignItems: 'center', gap: 4, fontSize: 11,
                padding: '2px 8px', borderRadius: 4, background: `${SECTION_COLOR}12`, color: SECTION_COLOR,
              }}>
                &ldquo;{search}&rdquo;
                <button onClick={() => setSearch('')} style={{ border: 'none', background: 'none', cursor: 'pointer', color: SECTION_COLOR, padding: 0, lineHeight: 1, marginLeft: 2 }}>x</button>
              </span>
            )}
          </div>
        )}
      </motion.div>

      {/* ── Table header ── */}
      <div style={{
        display: 'grid', gridTemplateColumns: GRID_COLS,
        gap: 12, padding: '6px 16px', marginBottom: 6,
        fontSize: 10, fontWeight: 700, color: 'var(--text-4)',
        textTransform: 'uppercase', letterSpacing: '0.07em',
      }}>
        <div />
        <div>Workflow</div>
        <div>Input</div>
        <div>Expert Chain</div>
        <div>Tokens</div>
        <div>CPU</div>
        <div>Duration</div>
        <div />
        <div />
      </div>

      {/* ── Run rows ── */}
      {isLoading ? (
        <div>
          {Array.from({ length: 8 }).map((_, i) => <SkeletonRow key={i} />)}
        </div>
      ) : pageRuns.length === 0 ? (
        /* Empty state */
        <motion.div
          initial={{ opacity: 0, y: 10 }} animate={{ opacity: 1, y: 0 }}
          style={{
            display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 12,
            padding: '60px 20px', borderRadius: 12,
            border: '1px dashed var(--border-md)',
            background: 'var(--bg-surface)',
          }}
        >
          <div style={{
            width: 48, height: 48, borderRadius: '50%',
            background: `${SECTION_COLOR}12`, border: `1.5px solid ${SECTION_COLOR}25`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <FileText size={20} color={SECTION_COLOR} />
          </div>
          <div style={{ textAlign: 'center' }}>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', marginBottom: 5 }}>
              {search || statusFilter !== 'All' ? 'No matching runs' : 'No run history yet'}
            </div>
            <div style={{ fontSize: 13, color: 'var(--text-3)' }}>
              {search || statusFilter !== 'All'
                ? 'Try adjusting your filters or search query'
                : 'Run a workflow to see execution history here'}
            </div>
          </div>
          <a
            href="/workflow"
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 18px', borderRadius: 8,
              background: SECTION_COLOR, border: 'none',
              color: '#fff', fontSize: 13, fontWeight: 600, textDecoration: 'none',
            }}
          >
            <Workflow size={13} /> Go to Workflow Builder
          </a>
        </motion.div>
      ) : (
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }}
          transition={{ delay: 0.08 }}
        >
          {pageRuns.map((run) => (
            <RunRow
              key={(run as WorkflowRun).id}
              run={run as WorkflowRun}
              expanded={expandedId === (run as WorkflowRun).id}
              onToggle={() => toggleExpand((run as WorkflowRun).id)}
              onDelete={() => setDeletingRunId((run as WorkflowRun).id)}
              onCancel={() => handleCancelRun((run as WorkflowRun).id, (run as WorkflowRun).workflowId)}
              onViewDetails={() => setDetailRun(run as WorkflowRun)}
              sysStats={sysStats}
            />
          ))}
        </motion.div>
      )}

      {/* ── Pagination ── */}
      {!isLoading && totalPages > 1 && (
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.2 }}
          style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            marginTop: 16, padding: '10px 0', borderTop: '1px solid var(--border-sm)',
          }}
        >
          <div style={{ fontSize: 12, color: 'var(--text-4)' }}>
            Showing {(currentPage - 1) * PAGE_SIZE + 1}–{Math.min(currentPage * PAGE_SIZE, filtered.length)} of {filtered.length} runs
          </div>

          <div style={{ display: 'flex', gap: 4, alignItems: 'center' }}>
            <button
              onClick={() => setPage(p => Math.max(1, p - 1))}
              disabled={currentPage === 1}
              style={{
                display: 'flex', alignItems: 'center', gap: 4,
                padding: '6px 12px', borderRadius: 7, fontSize: 12,
                border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                color: currentPage === 1 ? 'var(--text-4)' : 'var(--text-2)',
                cursor: currentPage === 1 ? 'not-allowed' : 'pointer',
                opacity: currentPage === 1 ? 0.5 : 1,
              }}
            >
              <ChevronLeft size={12} /> Prev
            </button>

            {/* Page numbers */}
            {Array.from({ length: Math.min(5, totalPages) }, (_, i) => {
              let p: number;
              if (totalPages <= 5) p = i + 1;
              else if (currentPage <= 3) p = i + 1;
              else if (currentPage >= totalPages - 2) p = totalPages - 4 + i;
              else p = currentPage - 2 + i;
              return (
                <button
                  key={p}
                  onClick={() => setPage(p)}
                  style={{
                    width: 30, height: 30, borderRadius: 7, fontSize: 12,
                    border: currentPage === p ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
                    background: currentPage === p ? `${SECTION_COLOR}15` : 'var(--bg-surface)',
                    color: currentPage === p ? SECTION_COLOR : 'var(--text-2)',
                    fontWeight: currentPage === p ? 700 : 400,
                    cursor: 'pointer', transition: 'all 0.12s',
                  }}
                >{p}</button>
              );
            })}

            <button
              onClick={() => setPage(p => Math.min(totalPages, p + 1))}
              disabled={currentPage === totalPages}
              style={{
                display: 'flex', alignItems: 'center', gap: 4,
                padding: '6px 12px', borderRadius: 7, fontSize: 12,
                border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                color: currentPage === totalPages ? 'var(--text-4)' : 'var(--text-2)',
                cursor: currentPage === totalPages ? 'not-allowed' : 'pointer',
                opacity: currentPage === totalPages ? 0.5 : 1,
              }}
            >
              Next <ChevronRight size={12} />
            </button>
          </div>
        </motion.div>
      )}

      {/* ── Auto-refresh footer ── */}
      <motion.div
        initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.4 }}
        style={{ marginTop: 20, display: 'flex', alignItems: 'center', gap: 6, color: 'var(--text-4)', fontSize: 11 }}
      >
        <AlertCircle size={11} />
        Auto-refreshes every 5 seconds
        {isLoading && <Loader2 size={10} style={{ animation: 'spin 1s linear infinite', marginLeft: 4 }} />}
      </motion.div>

      {/* Delete confirmation dialog */}
      <AnimatePresence>
        {deletingRunId && (
          <motion.div
            initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
            onClick={() => !deleting && setDeletingRunId(null)}
            style={{
              position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
              zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}
          >
            <motion.div
              initial={{ opacity: 0, scale: 0.96, y: -8 }}
              animate={{ opacity: 1, scale: 1, y: 0 }}
              exit={{ opacity: 0, scale: 0.96, y: -8 }}
              transition={{ type: 'spring', damping: 25, stiffness: 300 }}
              onClick={e => e.stopPropagation()}
              style={{
                background: 'var(--bg-surface, #fff)', borderRadius: 14,
                border: '1px solid var(--border)', padding: '28px 32px',
                maxWidth: 400, width: '90%', textAlign: 'center',
              }}
            >
              <div style={{
                width: 40, height: 40, borderRadius: 10,
                background: 'var(--error-dim, rgba(220,38,38,0.08))',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                margin: '0 auto 14px',
              }}>
                <Trash2 size={18} color="var(--error, #DC2626)" />
              </div>
              <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', marginBottom: 8 }}>
                Delete Run
              </div>
              <div style={{ fontSize: 13, color: 'var(--text-2)', lineHeight: 1.5, marginBottom: 20 }}>
                This will permanently delete this run and its step executions. This action cannot be undone.
              </div>
              <div style={{ display: 'flex', gap: 8, justifyContent: 'center' }}>
                <button
                  onClick={() => setDeletingRunId(null)}
                  disabled={deleting}
                  style={{
                    padding: '8px 20px', borderRadius: 7, fontSize: 12, fontWeight: 600,
                    border: '1px solid var(--border-md)', background: 'var(--bg-elevated)',
                    color: 'var(--text-2)', cursor: 'pointer',
                  }}
                >
                  Cancel
                </button>
                <button
                  onClick={handleDeleteRun}
                  disabled={deleting}
                  style={{
                    padding: '8px 20px', borderRadius: 7, fontSize: 12, fontWeight: 700,
                    border: '1.5px solid #ef4444', background: '#ef444414', color: '#ef4444',
                    cursor: deleting ? 'wait' : 'pointer', opacity: deleting ? 0.6 : 1,
                    display: 'flex', alignItems: 'center', gap: 5,
                  }}
                >
                  {deleting && <Loader2 size={11} style={{ animation: 'spin 1s linear infinite' }} />}
                  Delete
                </button>
              </div>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      <style>{`
        @keyframes spin        { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
        @keyframes pulse       { 0%, 100% { opacity: 1; } 50% { opacity: 0.45; } }
        @keyframes fadeIn      { from { opacity: 0; transform: scale(0.8); } to { opacity: 1; transform: scale(1); } }
        @keyframes shake       { 0%, 100% { transform: translateX(0); } 20% { transform: translateX(-3px); } 40% { transform: translateX(3px); } 60% { transform: translateX(-2px); } 80% { transform: translateX(2px); } }
        @keyframes orbit       { from { transform: rotate(0deg); } to { transform: rotate(360deg); } }
        @keyframes neuralPulse { 0% { transform: translate(-50%, -50%) scale(0.7); opacity: 0.5; } 100% { transform: translate(-50%, -50%) scale(1.3); opacity: 1; } }
      `}</style>

      {/* Run Detail Dialog */}
      <RunDetailDialog
        run={detailRun}
        open={!!detailRun}
        onClose={() => setDetailRun(null)}
      />
    </div>
  );
}
