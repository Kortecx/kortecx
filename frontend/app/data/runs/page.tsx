'use client';

import { useState } from 'react';
import {
  History, Search, ChevronDown, ChevronUp, Clock,
  CheckCircle2, AlertTriangle, Loader2, XCircle,
} from 'lucide-react';
import { useSynthesisJobs, useWorkflowRuns } from '@/lib/hooks/useApi';

/* eslint-disable @typescript-eslint/no-explicit-any */

type RunType = 'all' | 'synthesis' | 'workflow';
type RunStatus = 'all' | 'completed' | 'running' | 'failed' | 'queued' | 'cancelled';

interface UnifiedRun {
  id: string;
  name: string;
  type: 'synthesis' | 'workflow';
  status: string;
  progress: number;
  tokensUsed: number;
  duration: string;
  startedAt: string;
  completedAt: string;
  model?: string;
  source?: string;
  error?: string;
  meta: Record<string, unknown>;
}

function formatDuration(startedAt?: string, completedAt?: string): string {
  if (!startedAt) return '—';
  const end = completedAt ? new Date(completedAt) : new Date();
  const secs = Math.round((end.getTime() - new Date(startedAt).getTime()) / 1000);
  if (secs < 60) return `${secs}s`;
  if (secs < 3600) return `${Math.floor(secs / 60)}m ${secs % 60}s`;
  return `${(secs / 3600).toFixed(1)}h`;
}

function timeAgo(d: string): string {
  if (!d) return '';
  const secs = Math.round((Date.now() - new Date(d).getTime()) / 1000);
  if (secs < 60) return 'just now';
  if (secs < 3600) return `${Math.floor(secs / 60)}m ago`;
  if (secs < 86400) return `${Math.floor(secs / 3600)}h ago`;
  return `${Math.floor(secs / 86400)}d ago`;
}

export default function RunsHistoryPage() {
  const { jobs: synthJobs } = useSynthesisJobs();
  const { runs: workflowRuns } = useWorkflowRuns(undefined, 200);

  const [typeFilter, setTypeFilter] = useState<RunType>('all');
  const [statusFilter, setStatusFilter] = useState<RunStatus>('all');
  const [search, setSearch] = useState('');
  const [expandedId, setExpandedId] = useState<string | null>(null);

  // Unify all runs into a single list
  const allRuns: UnifiedRun[] = [
    ...synthJobs.map((j: any) => ({
      id: j.id,
      name: j.name || 'Synthesis Job',
      type: 'synthesis' as const,
      status: j.status,
      progress: j.targetSamples > 0 ? Math.round((j.currentSamples / j.targetSamples) * 100) : j.progress ?? 0,
      tokensUsed: j.tokensUsed ?? 0,
      duration: formatDuration(j.startedAt, j.completedAt),
      startedAt: j.startedAt || j.createdAt,
      completedAt: j.completedAt || '',
      model: j.model,
      source: j.source,
      error: j.error,
      meta: { targetSamples: j.targetSamples, currentSamples: j.currentSamples, outputFormat: j.outputFormat, outputPath: j.outputPath, tags: j.tags },
    })),
    ...workflowRuns.map((r: any) => ({
      id: r.id,
      name: r.workflowName || 'Workflow Run',
      type: 'workflow' as const,
      status: r.status,
      progress: r.status === 'completed' ? 100 : r.status === 'failed' ? 0 : 50,
      tokensUsed: r.totalTokensUsed ?? 0,
      duration: formatDuration(r.startedAt, r.completedAt),
      startedAt: r.startedAt || r.createdAt,
      completedAt: r.completedAt || '',
      error: r.errorMessage,
      meta: { durationSec: r.durationSec, totalCostUsd: r.totalCostUsd, expertChain: r.expertChain, input: r.input },
    })),
  ].sort((a, b) => new Date(b.startedAt).getTime() - new Date(a.startedAt).getTime());

  // Apply filters
  const filtered = allRuns.filter(r => {
    if (typeFilter !== 'all' && r.type !== typeFilter) return false;
    if (statusFilter !== 'all' && r.status !== statusFilter) return false;
    if (search && !r.name.toLowerCase().includes(search.toLowerCase()) && !r.id.includes(search)) return false;
    return true;
  });

  const statusCounts = {
    all: allRuns.length,
    completed: allRuns.filter(r => r.status === 'completed').length,
    running: allRuns.filter(r => r.status === 'running').length,
    failed: allRuns.filter(r => r.status === 'failed').length,
    queued: allRuns.filter(r => r.status === 'queued').length,
    cancelled: allRuns.filter(r => r.status === 'cancelled').length,
  };

  const typeColor = (t: string) => t === 'synthesis' ? '#0EA5E9' : '#2563EB';
  const statusColor = (s: string) => s === 'completed' ? '#059669' : s === 'running' ? '#D97706' : s === 'failed' ? '#DC2626' : s === 'queued' ? '#6B7280' : '#9CA3AF';
  const StatusIcon = (s: string) => s === 'completed' ? CheckCircle2 : s === 'running' ? Loader2 : s === 'failed' ? XCircle : s === 'queued' ? Clock : AlertTriangle;

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>
      {/* Header */}
      <div style={{ marginBottom: 24 }}>
        <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 10 }}>
          <History size={20} /> Runs History
        </h1>
        <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
          All synthesis and workflow runs across the platform — {allRuns.length} total
        </p>
      </div>

      {/* Filters */}
      <div style={{ display: 'flex', gap: 8, marginBottom: 16, flexWrap: 'wrap', alignItems: 'center' }}>
        {/* Type filter */}
        <div style={{ display: 'flex', gap: 4 }}>
          {(['all', 'synthesis', 'workflow'] as const).map(t => (
            <button key={t} onClick={() => setTypeFilter(t)} style={{
              padding: '5px 12px', borderRadius: 6, fontSize: 12, fontWeight: typeFilter === t ? 600 : 400,
              border: `1px solid ${typeFilter === t ? (t === 'all' ? 'var(--primary)' : typeColor(t)) : 'var(--border)'}`,
              background: typeFilter === t ? (t === 'all' ? 'var(--primary-dim)' : `${typeColor(t)}12`) : 'transparent',
              color: typeFilter === t ? (t === 'all' ? 'var(--primary)' : typeColor(t)) : 'var(--text-3)',
              cursor: 'pointer', textTransform: 'capitalize',
            }}>{t}</button>
          ))}
        </div>

        <span style={{ width: 1, height: 20, background: 'var(--border)' }} />

        {/* Status filter */}
        <div style={{ display: 'flex', gap: 4 }}>
          {(['all', 'completed', 'running', 'failed', 'queued'] as const).map(s => (
            <button key={s} onClick={() => setStatusFilter(s)} style={{
              padding: '5px 10px', borderRadius: 6, fontSize: 11, fontWeight: statusFilter === s ? 600 : 400,
              border: `1px solid ${statusFilter === s ? statusColor(s) : 'var(--border)'}`,
              background: statusFilter === s ? `${statusColor(s)}12` : 'transparent',
              color: statusFilter === s ? statusColor(s) : 'var(--text-3)',
              cursor: 'pointer', textTransform: 'capitalize', display: 'flex', alignItems: 'center', gap: 4,
            }}>
              {s}
              {statusCounts[s] > 0 && <span style={{ fontSize: 9, fontWeight: 700, opacity: 0.7 }}>({statusCounts[s]})</span>}
            </button>
          ))}
        </div>

        <div style={{ flex: 1 }} />

        {/* Search */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 6,
          padding: '5px 10px', borderRadius: 6,
          border: '1px solid var(--border)', background: 'var(--bg-surface)',
        }}>
          <Search size={12} color="var(--text-4)" />
          <input
            className="input"
            style={{ border: 'none', padding: 0, fontSize: 12, width: 160, background: 'none' }}
            placeholder="Search runs..."
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
        </div>
      </div>

      {/* Runs list */}
      {filtered.length === 0 ? (
        <div style={{ padding: '48px 0', textAlign: 'center', color: 'var(--text-4)', fontSize: 13 }}>
          <History size={32} style={{ opacity: 0.2, marginBottom: 8 }} />
          <div>No runs found</div>
        </div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {filtered.map(run => {
            const expanded = expandedId === run.id;
            const SIcon = StatusIcon(run.status);
            return (
              <div key={run.id} className="card" style={{ overflow: 'hidden' }}>
                {/* Run header row */}
                <div
                  onClick={() => setExpandedId(expanded ? null : run.id)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 10, padding: '12px 16px',
                    cursor: 'pointer', position: 'relative',
                  }}
                >
                  {/* Running shimmer */}
                  {run.status === 'running' && (
                    <div style={{
                      position: 'absolute', top: 0, left: 0, right: 0, height: 2,
                      background: 'linear-gradient(90deg, transparent, #D97706, transparent)',
                      animation: 'shimmer 1.5s infinite',
                    }} />
                  )}

                  {/* Status icon */}
                  <SIcon size={14} color={statusColor(run.status)} className={run.status === 'running' ? 'spin' : ''} />

                  {/* Type badge */}
                  <span style={{
                    fontSize: 9, fontWeight: 700, padding: '2px 7px', borderRadius: 10,
                    background: `${typeColor(run.type)}10`, color: typeColor(run.type),
                    textTransform: 'uppercase', letterSpacing: '0.04em',
                  }}>{run.type}</span>

                  {/* Name */}
                  <div style={{ flex: 1, minWidth: 0 }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: '#0d0d0d', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {run.name}
                    </div>
                  </div>

                  {/* Metrics */}
                  <div style={{ display: 'flex', gap: 12, alignItems: 'center', fontSize: 11, color: 'var(--text-3)', flexShrink: 0 }}>
                    {run.model && (
                      <span style={{ display: 'flex', alignItems: 'center', gap: 3 }} className="mono">
                        {run.model}
                      </span>
                    )}
                    {run.tokensUsed > 0 && (
                      <span>{run.tokensUsed.toLocaleString()} tok</span>
                    )}
                    <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                      <Clock size={10} /> {run.duration}
                    </span>
                    <span style={{ color: 'var(--text-4)', fontSize: 10 }}>
                      {timeAgo(run.startedAt)}
                    </span>
                  </div>

                  {/* Progress */}
                  {(run.status === 'running' || run.status === 'queued') && (
                    <div style={{ width: 50, textAlign: 'right' }}>
                      <span style={{ fontSize: 11, fontWeight: 600, color: statusColor(run.status) }}>{run.progress}%</span>
                    </div>
                  )}

                  {/* Expand chevron */}
                  {expanded ? <ChevronUp size={14} color="var(--text-4)" /> : <ChevronDown size={14} color="var(--text-4)" />}
                </div>

                {/* Expanded details */}
                {expanded && (
                  <div style={{ padding: '0 16px 14px', borderTop: '1px solid var(--border)' }}>
                    <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(180px, 1fr))', gap: 10, marginTop: 12 }}>
                      {/* Common fields */}
                      <div>
                        <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Status</div>
                        <div style={{ fontSize: 12, fontWeight: 600, color: statusColor(run.status), textTransform: 'capitalize' }}>{run.status}</div>
                      </div>
                      <div>
                        <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Duration</div>
                        <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>{run.duration}</div>
                      </div>
                      {run.tokensUsed > 0 && (
                        <div>
                          <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Tokens</div>
                          <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>{run.tokensUsed.toLocaleString()}</div>
                        </div>
                      )}

                      {/* Synthesis-specific */}
                      {run.type === 'synthesis' && (
                        <>
                          {run.meta.currentSamples !== undefined && (
                            <div>
                              <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Samples</div>
                              <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>{String(run.meta.currentSamples)} / {String(run.meta.targetSamples)}</div>
                            </div>
                          )}
                          {run.meta.outputFormat && (
                            <div>
                              <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Format</div>
                              <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>{String(run.meta.outputFormat)}</div>
                            </div>
                          )}
                        </>
                      )}

                      {/* Workflow-specific */}
                      {run.type === 'workflow' && (
                        <>
                          {run.meta.totalCostUsd && (
                            <div>
                              <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Cost</div>
                              <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }}>${Number(run.meta.totalCostUsd).toFixed(4)}</div>
                            </div>
                          )}
                          {run.meta.expertChain && (
                            <div>
                              <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Expert Chain</div>
                              <div style={{ fontSize: 11, fontWeight: 500, color: 'var(--text-3)' }}>{(run.meta.expertChain as string[]).join(' → ')}</div>
                            </div>
                          )}
                        </>
                      )}

                      {run.model && (
                        <div>
                          <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 2 }}>Model</div>
                          <div style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-2)' }} className="mono">{run.model}</div>
                        </div>
                      )}
                    </div>

                    {/* Error message */}
                    {run.error && (
                      <div style={{
                        marginTop: 10, padding: '8px 12px', borderRadius: 6,
                        background: 'rgba(220,38,38,0.05)', border: '1px solid rgba(220,38,38,0.15)',
                      }}>
                        <div style={{ fontSize: 10, fontWeight: 700, color: '#DC2626', textTransform: 'uppercase', marginBottom: 2 }}>Error</div>
                        <div style={{ fontSize: 11, color: '#DC2626', fontFamily: 'var(--font-mono, monospace)', whiteSpace: 'pre-wrap' }}>{run.error}</div>
                      </div>
                    )}

                    {/* Run ID */}
                    <div style={{ marginTop: 8, fontSize: 10, color: 'var(--text-4)' }} className="mono">
                      ID: {run.id}
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
