'use client';

import { useState, useMemo } from 'react';
import { motion } from 'framer-motion';
import {
  Zap, Activity, TrendingUp, Clock, AlertCircle,
  Search, Network, List, ChevronDown, ChevronRight,
  Play, CheckCircle2, XCircle, Loader2,
} from 'lucide-react';
import { useExpertRuns, useWorkflowRuns } from '@/lib/hooks/useApi';
import type { UnifiedRun, PlanNode, PlanEdge } from '@/lib/types';
import RunGraph from './_components/RunGraph';

const SECTION_COLOR = '#DC2626';

const STATUS_CONFIG: Record<string, { color: string; bg: string; icon: React.ElementType }> = {
  running:   { color: '#3b82f6', bg: '#dbeafe', icon: Loader2 },
  completed: { color: '#22c55e', bg: '#dcfce7', icon: CheckCircle2 },
  failed:    { color: '#ef4444', bg: '#fef2f2', icon: XCircle },
  queued:    { color: '#f59e0b', bg: '#fef3c7', icon: Clock },
  cancelled: { color: '#6b7280', bg: '#f3f4f6', icon: AlertCircle },
  started:   { color: '#3b82f6', bg: '#dbeafe', icon: Play },
};

type TabKey = 'list' | 'graphs';
type StatusFilter = 'all' | 'running' | 'completed' | 'failed' | 'queued';

/* ── Demo plan for graph tab preview ── */
const DEMO_PLAN_NODES: PlanNode[] = [
  { id: 'n1', prismId: 'researcher', label: 'Research', position: { x: 250, y: 0 }, status: 'completed', tokensUsed: 1200, durationMs: 3400 },
  { id: 'n2', prismId: 'analyst', label: 'Analyze', position: { x: 100, y: 150 }, status: 'completed', tokensUsed: 800, durationMs: 2100 },
  { id: 'n3', prismId: 'writer', label: 'Draft', position: { x: 400, y: 150 }, status: 'running', tokensUsed: 450 },
  { id: 'n4', prismId: 'reviewer', label: 'Review', position: { x: 250, y: 300 }, status: 'pending' },
];
const DEMO_PLAN_EDGES: PlanEdge[] = [
  { id: 'e1-2', source: 'n1', target: 'n2' },
  { id: 'e1-3', source: 'n1', target: 'n3', animated: true },
  { id: 'e2-4', source: 'n2', target: 'n4' },
  { id: 'e3-4', source: 'n3', target: 'n4' },
];

export default function RunsPage() {
  const [tab, setTab] = useState<TabKey>('list');
  const [statusFilter, setStatusFilter] = useState<StatusFilter>('all');
  const [searchQuery, setSearchQuery] = useState('');
  const [expandedRun, setExpandedRun] = useState<string | null>(null);
  const [selectedGraphRun, setSelectedGraphRun] = useState<string | null>(null);

  const { runs: expertRuns, isLoading: erLoading } = useExpertRuns();
  const { runs: workflowRuns, isLoading: wrLoading } = useWorkflowRuns();
  const isLoading = erLoading || wrLoading;

  // Unify runs from both sources
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

    // Sort newest first
    unified.sort((a, b) => {
      const at = a.startedAt ? new Date(a.startedAt).getTime() : 0;
      const bt = b.startedAt ? new Date(b.startedAt).getTime() : 0;
      return bt - at;
    });

    return unified;
  }, [expertRuns, workflowRuns]);

  // Filter
  const filtered = useMemo(() => {
    let list = allRuns;
    if (statusFilter !== 'all') list = list.filter(r => r.status === statusFilter);
    if (searchQuery) {
      const q = searchQuery.toLowerCase();
      list = list.filter(r => r.name.toLowerCase().includes(q) || r.id.toLowerCase().includes(q));
    }
    return list;
  }, [allRuns, statusFilter, searchQuery]);

  // Metrics
  const running   = allRuns.filter(r => r.status === 'running' || r.status === 'started').length;
  const completed = allRuns.filter(r => r.status === 'completed').length;
  const failed    = allRuns.filter(r => r.status === 'failed').length;
  const totalTokens = allRuns.reduce((s, r) => s + (r.tokensUsed ?? 0), 0);

  const fmtTime = (ms?: number) => {
    if (!ms) return '—';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  };

  return (
    <div style={{ padding: '28px 32px', maxWidth: 1200, margin: '0 auto' }}>
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        style={{ display: 'flex', alignItems: 'center', gap: 14, marginBottom: 24 }}
      >
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
            Unified view of all PRISM and workflow executions
          </p>
        </div>
      </motion.div>

      {/* Metrics bar */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.05 }}
        style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10, marginBottom: 20 }}
      >
        {[
          { label: 'Running',   value: String(running),   color: '#3b82f6', icon: Activity },
          { label: 'Completed', value: String(completed), color: '#22c55e', icon: TrendingUp },
          { label: 'Failed',    value: String(failed),    color: '#ef4444', icon: AlertCircle },
          { label: 'Total Tokens', value: totalTokens > 1000 ? `${(totalTokens / 1000).toFixed(1)}k` : String(totalTokens), color: '#f59e0b', icon: Clock },
        ].map(({ label, value, color, icon: Icon }) => (
          <div key={label} style={{
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
          </div>
        ))}
      </motion.div>

      {/* Tabs + search + filters */}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center', marginBottom: 16, flexWrap: 'wrap' }}>
        {/* Tabs */}
        <div style={{ display: 'flex', gap: 2, background: 'var(--bg-surface)', borderRadius: 8, border: '1px solid var(--border)', padding: 2 }}>
          {([
            { key: 'list' as TabKey, icon: List, label: 'All Runs' },
            { key: 'graphs' as TabKey, icon: Network, label: 'Graphs' },
          ]).map(({ key, icon: Icon, label }) => (
            <button
              key={key}
              onClick={() => setTab(key)}
              style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '6px 14px', borderRadius: 6, border: 'none',
                background: tab === key ? `${SECTION_COLOR}14` : 'transparent',
                color: tab === key ? SECTION_COLOR : 'var(--text-3)',
                fontSize: 12, fontWeight: tab === key ? 700 : 400,
                cursor: 'pointer', transition: 'all 0.15s',
              }}
            >
              <Icon size={13} />
              {label}
            </button>
          ))}
        </div>

        {/* Search */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 6,
          padding: '6px 12px', borderRadius: 7, border: '1px solid var(--border)',
          background: 'var(--bg-surface)', flex: 1, maxWidth: 280,
        }}>
          <Search size={12} color="var(--text-4)" />
          <input
            value={searchQuery}
            onChange={e => setSearchQuery(e.target.value)}
            placeholder="Search runs..."
            style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 12, color: 'var(--text-1)', width: '100%' }}
          />
        </div>

        {/* Status filters */}
        <div style={{ display: 'flex', gap: 4 }}>
          {(['all', 'running', 'completed', 'failed'] as StatusFilter[]).map(s => (
            <button
              key={s}
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
            </button>
          ))}
        </div>
      </div>

      {/* ── List Tab ── */}
      {tab === 'list' && (
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
            <div style={{ width: 80, textAlign: 'center' }}>Graph</div>
          </div>

          {isLoading && (
            <div style={{ padding: '40px 0', textAlign: 'center', color: 'var(--text-4)' }}>
              <Loader2 size={20} style={{ animation: 'spin 1s linear infinite', margin: '0 auto 8px' }} />
              Loading runs...
            </div>
          )}

          {!isLoading && filtered.length === 0 && (
            <div style={{ padding: '60px 0', textAlign: 'center', color: 'var(--text-4)', fontSize: 13 }}>
              No runs found
            </div>
          )}

          {filtered.map(run => {
            const sc = STATUS_CONFIG[run.status] ?? STATUS_CONFIG.queued;
            const StatusIcon = sc.icon;
            const isExpanded = expandedRun === run.id;

            return (
              <div key={run.id}>
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
                      background: run.type === 'workflow' ? '#2563eb14' : '#D9770614',
                      color: run.type === 'workflow' ? '#2563eb' : '#D97706',
                      textTransform: 'uppercase',
                    }}>
                      {run.type === 'workflow' ? 'WF' : 'PR'}
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

                  {/* View Graph */}
                  <div style={{ width: 80, textAlign: 'center' }}>
                    {run.type === 'workflow' && (
                      <button
                        onClick={e => { e.stopPropagation(); setSelectedGraphRun(run.id); setTab('graphs'); }}
                        style={{
                          padding: '4px 10px', borderRadius: 5, fontSize: 10, fontWeight: 600,
                          border: '1px solid #2563eb30', background: '#2563eb08',
                          color: '#2563eb', cursor: 'pointer',
                        }}
                      >
                        <Network size={10} style={{ marginRight: 3 }} /> Graph
                      </button>
                    )}
                  </div>
                </div>

                {/* Expanded details */}
                {isExpanded && (
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
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* ── Graphs Tab ── */}
      {tab === 'graphs' && (
        <div>
          <div style={{ fontSize: 13, color: 'var(--text-3)', marginBottom: 16 }}>
            {selectedGraphRun
              ? `Showing execution graph for run ${selectedGraphRun.slice(0, 12)}…`
              : 'Select a workflow run from the list, or view the demo graph below.'}
          </div>

          <RunGraph
            planNodes={DEMO_PLAN_NODES}
            planEdges={DEMO_PLAN_EDGES}
            editable
            onSave={(nodes, edges) => {
              console.log('Saved graph:', { nodes, edges });
            }}
          />

          <div style={{ marginTop: 12, fontSize: 11, color: 'var(--text-4)' }}>
            Toggle Edit to rearrange nodes, add connections, then Save to persist changes.
          </div>
        </div>
      )}
    </div>
  );
}
