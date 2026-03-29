'use client';

import { useState, useMemo, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Zap, Activity, TrendingUp, Clock, AlertCircle,
  Search, ChevronDown, ChevronRight,
  CheckCircle2, XCircle, Loader2, RotateCcw,
} from 'lucide-react';
import { fadeUp, stagger, hoverLift, filterTab, rowEntrance, emptyState } from '@/lib/motion';
import { useExpertRuns, useWorkflowRuns, useSynthesisJobs } from '@/lib/hooks/useApi';
import type { UnifiedRun } from '@/lib/types';

/* eslint-disable @typescript-eslint/no-explicit-any */

const SECTION_COLOR = '#DC2626';

const STATUS_CONFIG: Record<string, { color: string; bg: string; icon: React.ElementType }> = {
  running:   { color: '#3b82f6', bg: '#dbeafe', icon: Loader2 },
  completed: { color: '#22c55e', bg: '#dcfce7', icon: CheckCircle2 },
  failed:    { color: '#ef4444', bg: '#fef2f2', icon: XCircle },
  queued:    { color: '#f59e0b', bg: '#fef3c7', icon: Clock },
  cancelled: { color: '#6b7280', bg: '#f3f4f6', icon: AlertCircle },
  started:   { color: '#3b82f6', bg: '#dbeafe', icon: Loader2 },
};

type StatusFilter = 'all' | 'running' | 'completed' | 'failed' | 'queued';
type RunTypeFilter = 'all' | 'prism' | 'workflow' | 'synthesis';

const TYPE_BADGE: Record<string, { label: string; abbr: string; color: string }> = {
  prism:     { label: 'PRISM',     abbr: 'PR', color: '#D97706' },
  workflow:  { label: 'Workflows', abbr: 'WF', color: '#2563eb' },
  synthesis: { label: 'Synthesis', abbr: 'SY', color: '#0EA5E9' },
};

export default function RunsPage() {
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [expandedRun, setExpandedRun] = useState<string | null>(null);
  const [runTypeFilter, setRunTypeFilter] = useState<RunTypeFilter>('all');

  const { runs: expertRuns, isLoading: erLoading, mutate: mutateER } = useExpertRuns();
  const { runs: workflowRuns, isLoading: wrLoading, mutate: mutateWR } = useWorkflowRuns();
  const { jobs: synthJobs, isLoading: sjLoading, mutate: mutateSJ } = useSynthesisJobs();
  const isLoading = erLoading || wrLoading || sjLoading;

  /* LiveSync — manual refresh that revalidates all SWR caches */
  const [syncing, setSyncing] = useState(false);
  const liveSync = useCallback(async () => {
    setSyncing(true);
    await Promise.all([mutateER(), mutateWR(), mutateSJ()]);
    setSyncing(false);
  }, [mutateER, mutateWR, mutateSJ]);

  // Unify runs from all three sources
  const allRuns: UnifiedRun[] = useMemo(() => {
    const unified: UnifiedRun[] = [];

    for (const r of (expertRuns as Array<Record<string, unknown>>)) {
      unified.push({
        id: r.id as string,
        type: 'prism',
        name: (r.expertName as string) ?? 'PRISM Run',
        status: (r.status as string) ?? 'unknown',
        startedAt: r.startedAt as string,
        completedAt: r.completedAt as string,
        durationMs: r.durationMs as number,
        tokensUsed: r.tokensUsed as number,
        model: r.model as string,
        engine: r.engine as string,
        errorMessage: r.errorMessage as string,
      });
    }

    for (const r of (workflowRuns as Array<Record<string, unknown>>)) {
      unified.push({
        id: r.id as string,
        type: 'workflow',
        name: (r.workflowName as string) ?? 'Workflow Run',
        status: (r.status as string) ?? 'unknown',
        startedAt: r.startedAt as string,
        completedAt: r.completedAt as string,
        durationMs: r.durationSec ? (r.durationSec as number) * 1000 : undefined,
        tokensUsed: r.totalTokensUsed as number,
        costUsd: r.totalCostUsd ? Number(r.totalCostUsd) : undefined,
        planId: r.planId as string,
        errorMessage: r.errorMessage as string,
      });
    }

    for (const j of (synthJobs as Array<Record<string, any>>)) {
      const startedAt = j.startedAt || j.createdAt;
      const completedAt = j.completedAt;
      let durationMs: number | undefined;
      if (startedAt) {
        const end = completedAt ? new Date(completedAt) : new Date();
        durationMs = end.getTime() - new Date(startedAt).getTime();
      }
      unified.push({
        id: j.id as string,
        type: 'synthesis',
        name: (j.name as string) || 'Synthesis Job',
        status: (j.status as string) ?? 'unknown',
        startedAt,
        completedAt,
        durationMs,
        tokensUsed: j.tokensUsed as number,
        model: j.model as string,
        errorMessage: j.error as string,
      });
    }

    unified.sort((a, b) => {
      const at = a.startedAt ? new Date(a.startedAt).getTime() : 0;
      const bt = b.startedAt ? new Date(b.startedAt).getTime() : 0;
      return bt - at;
    });

    return unified;
  }, [expertRuns, workflowRuns, synthJobs]);

  // Filter
  const filtered = useMemo(() => {
    let list = allRuns;
    if (runTypeFilter !== 'all') list = list.filter(r => r.type === runTypeFilter);
    if (statusFilter !== 'all') list = list.filter(r => r.status === statusFilter);
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      list = list.filter(r => r.name.toLowerCase().includes(q) || r.id.toLowerCase().includes(q));
    }
    return list;
  }, [allRuns, statusFilter, searchQuery, runTypeFilter]);

  // Metrics
  const running   = allRuns.filter(r => r.status === 'running' || r.status === 'started').length;
  const completed = allRuns.filter(r => r.status === 'completed').length;
  const failed    = allRuns.filter(r => r.status === 'failed').length;
  const totalTokens = allRuns.reduce((s, r) => s + (r.tokensUsed ?? 0), 0);

  const fmtTime = (ms?: number) => {
    if (!ms) return '—';
    if (ms < 1000) return `${ms}ms`;
    if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`;
    return `${(ms / 60000).toFixed(1)}m`;
  };

  return (
    <div style={{ padding: '28px 32px', maxWidth: 1200, margin: '0 auto' }}>
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 14 }}>
          <div style={{
            width: 40, height: 40, borderRadius: 10,
            background: `${SECTION_COLOR}12`, border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Zap size={19} color={SECTION_COLOR} />
          </div>
          <div>
            <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>Runs</h1>
            <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
              All PRISM, workflow, and data synthesis executions
            </p>
          </div>
        </div>
        <button
          onClick={liveSync}
          disabled={syncing}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '8px 15px', borderRadius: 8, border: '1px solid var(--border-md)',
            background: 'var(--bg-surface)', cursor: syncing ? 'default' : 'pointer',
            fontSize: 12, fontWeight: 500, color: 'var(--text-2)',
            opacity: syncing ? 0.7 : 1, transition: 'all 0.15s',
          }}
        >
          <RotateCcw size={12} style={syncing ? { animation: 'spin 1s linear infinite' } : undefined} />
          {syncing ? 'Syncing...' : 'Refresh'}
        </button>
      </motion.div>

      {/* Metrics bar */}
      <motion.div
        variants={stagger(0.08)}
        initial="hidden"
        animate="show"
        style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10, marginBottom: 20 }}
      >
        {[
          { label: 'Running',   value: String(running),   color: '#3b82f6', icon: Activity },
          { label: 'Completed', value: String(completed), color: '#22c55e', icon: TrendingUp },
          { label: 'Failed',    value: String(failed),    color: '#ef4444', icon: AlertCircle },
          { label: 'Total Tokens', value: totalTokens > 1000 ? `${(totalTokens / 1000).toFixed(1)}k` : String(totalTokens), color: '#f59e0b', icon: Clock },
        ].map(({ label, value, color, icon: Icon }) => (
          <motion.div key={label} variants={fadeUp} {...hoverLift} style={{
            background: 'var(--bg-surface)', border: '1px solid var(--border)',
            borderRadius: 10, padding: '14px 16px',
            display: 'flex', alignItems: 'center', gap: 12,
          }}>
            <div style={{
              width: 36, height: 36, borderRadius: 8,
              background: `${color}12`, display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Icon size={16} color={color} />
            </div>
            <div>
              <div style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)' }}>{value}</div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>{label}</div>
            </div>
          </motion.div>
        ))}
      </motion.div>

      {/* Search + filters */}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 16, flexWrap: 'wrap' }}>
        {/* Search */}
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.1 }}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '6px 12px', borderRadius: 7, border: '1px solid var(--border)',
            background: 'var(--bg-surface)', flex: 1, maxWidth: 280,
          }}
        >
          <Search size={12} color="var(--text-4)" />
          <input
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            placeholder="Search runs..."
            style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 12, color: 'var(--text-1)', width: '100%' }}
          />
        </motion.div>

        {/* Run type filter */}
        <div style={{ display: 'flex', gap: 2, background: 'var(--bg-surface)', borderRadius: 8, border: '1px solid var(--border)', padding: 2 }}>
          {([
            { key: 'all' as RunTypeFilter, label: 'All' },
            { key: 'prism' as RunTypeFilter, label: 'PRISM' },
            { key: 'workflow' as RunTypeFilter, label: 'Workflows' },
            { key: 'synthesis' as RunTypeFilter, label: 'Synthesis' },
          ]).map(({ key, label }) => (
            <motion.button
              key={key}
              {...filterTab}
              onClick={() => setRunTypeFilter(key)}
              style={{
                padding: '5px 12px', borderRadius: 6, fontSize: 11, cursor: 'pointer',
                border: 'none',
                background: runTypeFilter === key ? `${SECTION_COLOR}14` : 'transparent',
                color: runTypeFilter === key ? SECTION_COLOR : 'var(--text-3)',
                fontWeight: runTypeFilter === key ? 700 : 400,
              }}
            >
              {label}
            </motion.button>
          ))}
        </div>

        {/* Status filters */}
        <div style={{ display: 'flex', gap: 4 }}>
          {(['all', 'running', 'completed', 'failed'] as StatusFilter[]).map(s => (
            <motion.button
              key={s}
              {...filterTab}
              onClick={() => setStatusFilter(s)}
              style={{
                padding: '5px 11px', borderRadius: 6, fontSize: 11, cursor: 'pointer',
                border: statusFilter === s ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
                background: statusFilter === s ? `${SECTION_COLOR}10` : 'var(--bg-surface)',
                color: statusFilter === s ? SECTION_COLOR : 'var(--text-3)',
                fontWeight: statusFilter === s ? 700 : 400,
              }}
            >
              {s === 'all' ? 'All' : s.charAt(0).toUpperCase() + s.slice(1)}
            </motion.button>
          ))}
        </div>
      </div>

      {/* Runs list */}
      <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8, padding: '8px 14px',
          borderRadius: 8, background: 'var(--bg-elevated)', border: '1px solid var(--border)',
          fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.04em',
        }}>
          <div style={{ flex: 0.5 }}>Type</div>
          <div style={{ flex: 3 }}>Name</div>
          <div style={{ flex: 1 }}>Status</div>
          <div style={{ flex: 1 }}>Duration</div>
          <div style={{ flex: 1 }}>Tokens</div>
          <div style={{ flex: 1.5 }}>Started</div>
        </div>

        {isLoading && (
          <motion.div {...emptyState} style={{ padding: '40px 0', textAlign: 'center', color: 'var(--text-4)' }}>
            <Loader2 size={20} style={{ animation: 'spin 1s linear infinite', margin: '0 auto 8px' }} />
            Loading runs...
          </motion.div>
        )}

        {!isLoading && filtered.length === 0 && (
          <motion.div {...emptyState} style={{ padding: '60px 0', textAlign: 'center', color: 'var(--text-4)', fontSize: 13 }}>
            No runs found
          </motion.div>
        )}

        {filtered.map((run, index) => {
          const sc = STATUS_CONFIG[run.status] ?? STATUS_CONFIG.queued;
          const StatusIcon = sc.icon;
          const isExpanded = expandedRun === run.id;
          const badge = TYPE_BADGE[run.type] ?? TYPE_BADGE.prism;

          return (
            <motion.div key={run.id} {...rowEntrance(index)}>
              <div
                onClick={() => setExpandedRun(isExpanded ? null : run.id)}
                style={{
                  display: 'flex', alignItems: 'center', gap: 8, padding: '10px 14px',
                  borderRadius: 7, cursor: 'pointer', border: '1px solid transparent',
                  transition: 'all 0.12s',
                }}
                onMouseEnter={e => { e.currentTarget.style.background = 'var(--bg-surface)'; e.currentTarget.style.borderColor = 'var(--border)'; }}
                onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = 'transparent'; }}
              >
                {/* Expand arrow */}
                {isExpanded ? <ChevronDown size={12} color="var(--text-4)" /> : <ChevronRight size={12} color="var(--text-4)" />}

                {/* Type badge */}
                <div style={{ flex: 0.5 }}>
                  <span style={{
                    fontSize: 9, padding: '2px 6px', borderRadius: 4, fontWeight: 600,
                    background: `${badge.color}14`,
                    color: badge.color,
                    textTransform: 'uppercase',
                  }}>
                    {badge.abbr}
                  </span>
                </div>

                {/* Name */}
                <div style={{ flex: 3, fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
                  {run.name}
                  <span style={{ fontSize: 10, color: 'var(--text-4)', marginLeft: 8, fontWeight: 400 }}>{run.id.slice(0, 12)}</span>
                </div>

                {/* Status */}
                <div style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 5 }}>
                  <StatusIcon size={12} color={sc.color} style={run.status === 'running' ? { animation: 'spin 1s linear infinite' } : undefined} />
                  <span style={{ fontSize: 11, color: sc.color, fontWeight: 500 }}>{run.status}</span>
                </div>

                {/* Duration */}
                <div style={{ flex: 1, fontSize: 12, color: 'var(--text-3)' }}>
                  {fmtTime(run.durationMs)}
                </div>

                {/* Tokens */}
                <div style={{ flex: 1, fontSize: 12, color: 'var(--text-3)' }}>
                  {run.tokensUsed ?? '—'}
                </div>

                {/* Started */}
                <div style={{ flex: 1.5, fontSize: 11, color: 'var(--text-4)' }}>
                  {run.startedAt ? new Date(run.startedAt).toLocaleString(undefined, { month: 'short', day: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—'}
                </div>
              </div>

              {/* Expanded details */}
              <AnimatePresence>
                {isExpanded && (
                  <motion.div
                    initial={{ height: 0, opacity: 0 }}
                    animate={{ height: 'auto', opacity: 1 }}
                    exit={{ height: 0, opacity: 0 }}
                    transition={{ duration: 0.25 }}
                    style={{ overflow: 'hidden' }}
                  >
                    <div style={{
                      margin: '0 14px 8px 30px', padding: '12px 16px',
                      background: 'var(--bg-surface)', borderRadius: 8,
                      border: '1px solid var(--border)', fontSize: 12,
                    }}>
                      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12, color: 'var(--text-3)' }}>
                        <div><strong style={{ color: 'var(--text-2)' }}>Model:</strong> {run.model ?? '—'}</div>
                        <div><strong style={{ color: 'var(--text-2)' }}>Engine:</strong> {run.engine ?? '—'}</div>
                        <div><strong style={{ color: 'var(--text-2)' }}>Cost:</strong> {run.costUsd ? `$${run.costUsd.toFixed(4)}` : '—'}</div>
                      </div>
                      {run.errorMessage && (
                        <div style={{ marginTop: 8, padding: '8px 10px', borderRadius: 6, background: '#fef2f2', border: '1px solid #fecaca', color: '#b91c1c', fontSize: 11 }}>
                          {run.errorMessage}
                        </div>
                      )}
                    </div>
                  </motion.div>
                )}
              </AnimatePresence>
            </motion.div>
          );
        })}
      </div>
    </div>
  );
}
