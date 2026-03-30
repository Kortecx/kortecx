'use client';

import React, { useState, useMemo, useEffect, useCallback, useRef } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Workflow, Plus, Search, Play, Trash2, X, Square, RotateCcw, Pencil,
  ChevronDown, ChevronUp, ChevronRight, Loader2, AlertCircle, ArrowUpDown,
  Clock, Cpu, Zap, CheckCircle2, XCircle, Eye, ScrollText, ExternalLink,
  FileText, Lock, Unlock, Snowflake, Upload, Download, TrendingUp, Activity, FolderOpen,
} from 'lucide-react';
import { useWorkflows, useWorkflowRuns, useStepExecutions } from '@/lib/hooks/useApi';
import { useWorkflowWS } from '@/lib/hooks/useWorkflowWS';
import { ImportButton, SharedImportButton } from '@/components/ImportExportButtons';
import SharedConfigImportDialog from '@/components/SharedConfigImportDialog';
import { exportEntity } from '@/lib/config-export';
import WorkflowOutputDialog from './_components/WorkflowOutputDialog';
import { fadeUp, stagger, hoverLift } from '@/lib/motion';

const SECTION_COLOR = '#2563EB';
const staggerDefault = stagger();
const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

/* ── Helpers ──────────────────────────────────────────── */
function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function timeAgo(dateStr: string | null | undefined) {
  if (!dateStr) return '—';
  const d = new Date(dateStr);
  const now = Date.now();
  const sec = Math.floor((now - d.getTime()) / 1000);
  if (sec < 60) return 'just now';
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  if (sec < 604800) return `${Math.floor(sec / 86400)}d ago`;
  return d.toLocaleDateString();
}

function fmtElapsed(ms: number) {
  const s = Math.floor(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  const rs = s % 60;
  return rs > 0 ? `${m}m ${rs}s` : `${m}m`;
}

const STATUS_STYLE: Record<string, { color: string; bg: string; label: string }> = {
  idle:      { color: '#6b7280', bg: '#6b728012', label: 'Idle' },
  draft:     { color: '#6b7280', bg: '#6b728012', label: 'Draft' },
  ready:     { color: '#2563EB', bg: '#2563EB12', label: 'Ready' },
  running:   { color: '#f59e0b', bg: '#f59e0b12', label: 'Running' },
  completed: { color: '#10b981', bg: '#10b98112', label: 'Completed' },
  failed:    { color: '#ef4444', bg: '#ef444412', label: 'Failed' },
  cancelled: { color: '#6b7280', bg: '#6b728012', label: 'Cancelled' },
  paused:    { color: '#8b5cf6', bg: '#8b5cf612', label: 'Paused' },
};

type SortField = 'name' | 'updatedAt' | 'status' | 'totalRuns' | 'estimatedTokens';
type SortDir = 'asc' | 'desc';

/* ── Running Timer Component ─────────────────────────── */
function RunningTimer({ startedAt }: { startedAt: string }) {
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    const start = new Date(startedAt).getTime();
    const tick = () => setElapsed(Date.now() - start);
    tick();
    const id = setInterval(tick, 1000);
    return () => clearInterval(id);
  }, [startedAt]);
  return (
    <span className="mono" style={{ fontSize: 10, color: '#f59e0b', fontWeight: 600 }}>
      <Clock size={9} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 3 }} />
      {fmtElapsed(elapsed)}
    </span>
  );
}

/* ── Live Execution Panel ──────────────────────────────── */
const STEP_STATUS_STYLE: Record<string, { bg: string; color: string; label: string }> = {
  pending:   { bg: '#6b728010', color: '#6b7280', label: 'Pending' },
  running:   { bg: '#3b82f610', color: '#3b82f6', label: 'Running' },
  thinking:  { bg: '#8b5cf610', color: '#8b5cf6', label: 'Thinking' },
  spawned:   { bg: '#06b6d410', color: '#06b6d4', label: 'Spawned' },
  completed: { bg: '#10b98110', color: '#10b981', label: 'Done' },
  failed:    { bg: '#ef444410', color: '#ef4444', label: 'Failed' },
};

function LiveExecutionPanel({ agents, liveMetrics, events }: {
  agents: Record<string, { agentId: string; stepId: string; status: string; stepName?: string; tokensUsed?: number; durationMs?: number; model?: string; engine?: string; error?: string }>;
  liveMetrics: { cpuPercent: number; gpuPercent: number; memoryMb: number; totalTokensUsed: number; elapsedMs: number } | null;
  events: { event: string; agentId?: string; data: Record<string, unknown>; timestamp: string }[];
}) {
  const agentList = Object.values(agents);
  if (agentList.length === 0 && !liveMetrics) return null;

  return (
    <tr>
      <td colSpan={8} style={{ padding: 0, border: 'none' }}>
        <motion.div
          initial={{ opacity: 0, height: 0 }}
          animate={{ opacity: 1, height: 'auto' }}
          exit={{ opacity: 0, height: 0 }}
          style={{
            margin: '0 12px 8px', padding: '12px 14px',
            background: 'var(--bg)', borderRadius: 8,
            border: '1px solid var(--border)',
          }}
        >
          {/* System metrics bar */}
          {liveMetrics && (
            <div style={{ display: 'flex', gap: 16, marginBottom: 10, fontSize: 11, color: 'var(--text-3)' }}>
              <span className="mono"><Cpu size={10} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 3 }} />CPU {liveMetrics.cpuPercent.toFixed(0)}%</span>
              <span className="mono">GPU {liveMetrics.gpuPercent.toFixed(0)}%</span>
              <span className="mono">Mem {liveMetrics.memoryMb.toFixed(0)} MB</span>
              <span className="mono"><Zap size={10} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 3 }} />{fmt(liveMetrics.totalTokensUsed)} tok</span>
              <span className="mono"><Clock size={10} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 3 }} />{fmtElapsed(liveMetrics.elapsedMs)}</span>
            </div>
          )}

          {/* Step progress */}
          <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
            {agentList.map(agent => {
              const ss = STEP_STATUS_STYLE[agent.status] ?? STEP_STATUS_STYLE.pending;
              return (
                <div key={agent.agentId} style={{
                  display: 'flex', alignItems: 'center', gap: 10,
                  padding: '8px 10px', borderRadius: 6,
                  background: 'var(--bg-surface)', border: '1px solid var(--border)',
                }}>
                  <span style={{
                    padding: '2px 7px', borderRadius: 99, fontSize: 9, fontWeight: 700,
                    background: ss.bg, color: ss.color, border: `1px solid ${ss.color}28`,
                    flexShrink: 0, minWidth: 52, textAlign: 'center',
                  }}>{ss.label}</span>
                  <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)', flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                    {agent.stepName || agent.stepId}
                  </span>
                  {agent.model && (
                    <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>
                      {agent.engine ? `${agent.engine}:` : ''}{agent.model}
                    </span>
                  )}
                  {agent.tokensUsed ? (
                    <span className="mono" style={{ fontSize: 10, color: 'var(--text-3)' }}>
                      <Zap size={9} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />
                      {fmt(agent.tokensUsed)}
                    </span>
                  ) : null}
                  {agent.durationMs ? (
                    <span className="mono" style={{ fontSize: 10, color: 'var(--text-3)' }}>
                      {(agent.durationMs / 1000).toFixed(1)}s
                    </span>
                  ) : null}
                  {agent.status === 'running' || agent.status === 'thinking' ? (
                    <Loader2 size={12} color={ss.color} style={{ animation: 'spin 1s linear infinite', flexShrink: 0 }} />
                  ) : null}
                  {agent.error && (
                    <span style={{ fontSize: 10, color: '#ef4444', maxWidth: 200, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                      {agent.error}
                    </span>
                  )}
                </div>
              );
            })}
          </div>

          {/* Recent events log */}
          {events.length > 0 && (
            <div style={{ marginTop: 8, maxHeight: 100, overflowY: 'auto', fontSize: 10, fontFamily: 'var(--font-mono, monospace)', color: 'var(--text-4)', lineHeight: 1.6 }}>
              {events.slice(-8).map((ev, i) => (
                <div key={i} style={{ display: 'flex', gap: 8 }}>
                  <span style={{ color: 'var(--text-4)', flexShrink: 0 }}>{new Date(ev.timestamp).toLocaleTimeString()}</span>
                  <span style={{ color: STEP_STATUS_STYLE[ev.event.split('.').pop() || '']?.color || 'var(--text-3)' }}>{ev.event}</span>
                  {ev.agentId && <span style={{ color: 'var(--text-4)' }}>{ev.agentId}</span>}
                </div>
              ))}
            </div>
          )}
        </motion.div>
      </td>
    </tr>
  );
}

/* ── Plan Dialog ────────────────────────────────────── */
function PlanDialog({
  wf,
  onClose,
}: {
  wf: Record<string, unknown>;
  onClose: () => void;
}) {
  const [tab, setTab] = useState<'upload' | 'prompt' | 'editor' | 'generated'>('editor');
  const [markdown, setMarkdown] = useState('');
  const [prompt, setPrompt] = useState('');
  const [generating, setGenerating] = useState(false);
  const [saving, setSaving] = useState(false);
  const [dagPreview, setDagPreview] = useState<{ nodes: unknown[]; edges: unknown[] } | null>(null);
  const [versions, setVersions] = useState<Record<string, unknown>[]>([]);
  const [maxVersions, setMaxVersions] = useState((wf.planMaxVersions as number) || 3);
  const [loadedPlan, setLoadedPlan] = useState<Record<string, unknown> | null>(null);
  const fileRef = useRef<HTMLInputElement>(null);

  const slug = ((wf.name as string) || 'untitled').toLowerCase().replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '');

  // Load existing plan and versions on open
  useEffect(() => {
    (async () => {
      try {
        const [plansRes, versionsRes] = await Promise.all([
          fetch(`/api/plans?workflowId=${wf.id}`),
          fetch(`/api/plans/versions?workflowId=${wf.id}`),
        ]);
        const plansData = await plansRes.json();
        const versionsData = await versionsRes.json();
        setVersions(versionsData.versions || []);
        const latest = plansData.plans?.[0];
        if (latest) {
          setLoadedPlan(latest);
          setMarkdown((latest.markdownContent as string) || '');
          const dag = latest.dag as { nodes: unknown[]; edges: unknown[] } | null;
          if (dag && dag.nodes) setDagPreview(dag);
        }
      } catch { /* ignore */ }
    })();
  }, [wf.id]);

  const handleFileUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = () => {
      setMarkdown(reader.result as string);
      setTab('editor');
    };
    reader.readAsText(file);
  };

  const handleGenerate = async () => {
    setGenerating(true);
    try {
      const res = await fetch('/api/plans/generate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          workflowId: wf.id,
          workflowSlug: slug,
          prompt: prompt || undefined,
          useGraph: true,
        }),
      });
      const data = await res.json();
      if (data.dag) {
        setDagPreview(data.dag);
        setTab('generated');
      }
    } catch (err) {
      console.error('Plan generation failed:', err);
    } finally {
      setGenerating(false);
    }
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      const dag = dagPreview || { nodes: [], edges: [] };
      const nextVersion = versions.length > 0
        ? Math.max(...versions.map(v => (v.version as number) || 0)) + 1
        : 1;

      // Save to DB
      const res = await fetch('/api/plans', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          workflowId: wf.id,
          name: `${wf.name} Plan v${nextVersion}`,
          description: `Plan version ${nextVersion}`,
          dag,
          markdownContent: markdown,
          sourceType: tab === 'upload' ? 'upload' : tab === 'prompt' ? 'prompt' : tab === 'generated' ? 'agent_generated' : 'manual',
          version: nextVersion,
          planType: 'live',
          generatedBy: tab === 'generated' || tab === 'prompt' ? 'model' : 'user',
        }),
      });
      const { plan } = await res.json();

      // Update workflow active plan
      await fetch('/api/workflows', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: wf.id, activePlanId: plan?.id }),
      });

      // Save to engine filesystem
      await fetch(`${ENGINE_URL}/api/plans/save`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workflowSlug: slug, dag, markdown, maxVersions }),
      });

      // Prune excess DB versions
      await fetch('/api/plans/versions', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workflowId: wf.id, maxVersions }),
      });

      onClose();
    } catch (err) {
      console.error('Save plan failed:', err);
    } finally {
      setSaving(false);
    }
  };

  const loadVersion = async (plan: Record<string, unknown>) => {
    setLoadedPlan(plan);
    setMarkdown((plan.markdownContent as string) || '');
    const dag = plan.dag as { nodes: unknown[]; edges: unknown[] } | null;
    if (dag && dag.nodes) setDagPreview(dag);
  };

  const tabs = [
    { key: 'upload' as const, label: 'Upload' },
    { key: 'prompt' as const, label: 'Prompt' },
    { key: 'editor' as const, label: 'Editor' },
    { key: 'generated' as const, label: 'Generated' },
  ];

  return (
    <motion.div
      initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 1000,
        background: 'rgba(7,7,26,0.85)', backdropFilter: 'blur(4px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.95, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.95, y: 20 }}
        transition={{ type: 'spring', damping: 25, stiffness: 300 }}
        onClick={e => e.stopPropagation()}
        style={{
          background: 'var(--bg-surface)', border: '1px solid var(--border)',
          borderRadius: 12, width: '90vw', maxWidth: 1000, maxHeight: '85vh',
          overflow: 'hidden', display: 'flex', flexDirection: 'column',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '16px 20px', borderBottom: '1px solid var(--border)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <FileText size={18} color="#8b5cf6" />
            <span style={{ fontSize: 16, fontWeight: 700, color: 'var(--text-1)' }}>
              Plan — {wf.name as string}
            </span>
            {loadedPlan && (
              <span style={{
                fontSize: 10, padding: '2px 8px', borderRadius: 4,
                background: (loadedPlan.planType === 'frozen') ? '#06b6d420' : '#8b5cf620',
                color: (loadedPlan.planType === 'frozen') ? '#06b6d4' : '#8b5cf6',
                fontWeight: 600,
              }}>
                {(loadedPlan.planType as string)?.toUpperCase() || 'LIVE'} v{(loadedPlan.version as number) || 1}
              </span>
            )}
          </div>
          <button onClick={onClose} style={{
            background: 'transparent', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', padding: 4,
          }}>
            <X size={18} />
          </button>
        </div>

        {/* Tabs */}
        <div style={{
          display: 'flex', gap: 0, borderBottom: '1px solid var(--border)',
          padding: '0 20px',
        }}>
          {tabs.map(t => (
            <button key={t.key} onClick={() => setTab(t.key)} style={{
              padding: '10px 16px', fontSize: 12, fontWeight: 600,
              background: 'transparent', border: 'none', cursor: 'pointer',
              color: tab === t.key ? '#8b5cf6' : 'var(--text-3)',
              borderBottom: tab === t.key ? '2px solid #8b5cf6' : '2px solid transparent',
            }}>
              {t.label}
            </button>
          ))}
        </div>

        {/* Body */}
        <div style={{ display: 'flex', flex: 1, overflow: 'hidden' }}>
          {/* Main content */}
          <div style={{ flex: 1, padding: 20, overflowY: 'auto' }}>
            {tab === 'upload' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                <div style={{
                  border: '2px dashed var(--border)', borderRadius: 8,
                  padding: 40, textAlign: 'center', cursor: 'pointer',
                }} onClick={() => fileRef.current?.click()}>
                  <Upload size={32} color="var(--text-4)" style={{ marginBottom: 8 }} />
                  <p style={{ fontSize: 13, color: 'var(--text-3)', margin: 0 }}>
                    Click to upload a Markdown (.md) plan file
                  </p>
                  <input ref={fileRef} type="file" accept=".md,.markdown,.txt"
                    style={{ display: 'none' }} onChange={handleFileUpload} />
                </div>
                {markdown && (
                  <pre style={{
                    background: 'var(--bg-1)', border: '1px solid var(--border)',
                    borderRadius: 8, padding: 16, fontSize: 12, color: 'var(--text-2)',
                    maxHeight: 300, overflow: 'auto', whiteSpace: 'pre-wrap',
                  }}>{markdown}</pre>
                )}
              </div>
            )}

            {tab === 'prompt' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                <label style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>
                  Describe the plan you want to generate
                </label>
                <textarea
                  value={prompt}
                  onChange={e => setPrompt(e.target.value)}
                  placeholder="e.g., Create a research workflow that first gathers data, then analyzes it, and finally generates a report..."
                  style={{
                    width: '100%', minHeight: 120, padding: 12, borderRadius: 8,
                    border: '1px solid var(--border)', background: 'var(--bg-1)',
                    color: 'var(--text-1)', fontSize: 13, fontFamily: 'inherit',
                    resize: 'vertical',
                  }}
                />
                <button onClick={handleGenerate} disabled={generating || !prompt.trim()} style={{
                  display: 'flex', alignItems: 'center', gap: 6, alignSelf: 'flex-start',
                  padding: '8px 16px', borderRadius: 6, fontSize: 12, fontWeight: 600,
                  background: generating ? '#8b5cf630' : '#8b5cf6', color: '#fff',
                  border: 'none', cursor: generating ? 'wait' : 'pointer',
                  opacity: (!prompt.trim() || generating) ? 0.5 : 1,
                }}>
                  {generating ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Zap size={12} />}
                  {generating ? 'Generating...' : 'Generate Plan'}
                </button>
              </div>
            )}

            {tab === 'editor' && (
              <textarea
                value={markdown}
                onChange={e => setMarkdown(e.target.value)}
                placeholder="# Execution Plan&#10;&#10;## Step 1: Research&#10;- Agent: Research Analyst&#10;- Task: Gather relevant data...&#10;&#10;## Step 2: Analysis&#10;..."
                style={{
                  width: '100%', minHeight: 400, padding: 16, borderRadius: 8,
                  border: '1px solid var(--border)', background: 'var(--bg-1)',
                  color: 'var(--text-1)', fontSize: 13, fontFamily: 'monospace',
                  resize: 'vertical', lineHeight: 1.6,
                }}
              />
            )}

            {tab === 'generated' && (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
                <button onClick={handleGenerate} disabled={generating} style={{
                  display: 'flex', alignItems: 'center', gap: 6, alignSelf: 'flex-start',
                  padding: '8px 16px', borderRadius: 6, fontSize: 12, fontWeight: 600,
                  background: generating ? '#8b5cf630' : '#8b5cf6', color: '#fff',
                  border: 'none', cursor: generating ? 'wait' : 'pointer',
                }}>
                  {generating ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Zap size={12} />}
                  {generating ? 'Generating...' : 'Generate from Agent Graph'}
                </button>
                {dagPreview && dagPreview.nodes.length > 0 && (
                  <div style={{
                    background: 'var(--bg-1)', border: '1px solid var(--border)',
                    borderRadius: 8, padding: 16, minHeight: 200,
                  }}>
                    <p style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)', margin: '0 0 8px' }}>
                      DAG Preview — {dagPreview.nodes.length} nodes, {dagPreview.edges.length} edges
                    </p>
                    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 8 }}>
                      {(dagPreview.nodes as Record<string, unknown>[]).map((n) => (
                        <div key={n.id as string} style={{
                          padding: '6px 12px', borderRadius: 6, fontSize: 11, fontWeight: 500,
                          background: '#8b5cf615', color: '#8b5cf6',
                          border: '1px solid #8b5cf630',
                        }}>
                          {(n.label as string) || (n.id as string)}
                          {n.connectionType === 'parallel' && (
                            <span style={{ marginLeft: 4, fontSize: 9, opacity: 0.7 }}>⟂</span>
                          )}
                        </div>
                      ))}
                    </div>
                  </div>
                )}
              </div>
            )}
          </div>

          {/* Version sidebar */}
          <div style={{
            width: 220, borderLeft: '1px solid var(--border)',
            padding: 16, overflowY: 'auto', background: 'var(--bg-1)',
          }}>
            <p style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-2)', marginTop: 0 }}>
              Versions
            </p>
            {versions.length === 0 && (
              <p style={{ fontSize: 11, color: 'var(--text-4)', fontStyle: 'italic' }}>
                No versions yet
              </p>
            )}
            {versions.map((v) => (
              <button key={v.id as string} onClick={() => loadVersion(v)} style={{
                width: '100%', textAlign: 'left', padding: '8px 10px', marginBottom: 4,
                borderRadius: 6, fontSize: 11, cursor: 'pointer',
                background: loadedPlan?.id === v.id ? '#8b5cf615' : 'transparent',
                border: loadedPlan?.id === v.id ? '1px solid #8b5cf640' : '1px solid transparent',
                color: 'var(--text-2)',
              }}>
                <div style={{ fontWeight: 600 }}>v{v.version as number}</div>
                <div style={{ fontSize: 10, color: 'var(--text-4)' }}>
                  {v.sourceType as string} • {timeAgo(v.createdAt as string)}
                </div>
              </button>
            ))}

            <div style={{ marginTop: 16, paddingTop: 12, borderTop: '1px solid var(--border)' }}>
              <label style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-3)' }}>
                Max Versions
              </label>
              <input
                type="number" min={1} max={50} value={maxVersions}
                onChange={e => setMaxVersions(Math.max(1, Math.min(50, Number(e.target.value))))}
                style={{
                  width: '100%', padding: '6px 8px', borderRadius: 4, marginTop: 4,
                  border: '1px solid var(--border)', background: 'var(--bg-2)',
                  color: 'var(--text-1)', fontSize: 12,
                }}
              />
            </div>
          </div>
        </div>

        {/* Footer */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 8,
          padding: '12px 20px', borderTop: '1px solid var(--border)',
        }}>
          <button onClick={onClose} style={{
            padding: '8px 16px', borderRadius: 6, fontSize: 12, fontWeight: 600,
            background: 'transparent', border: '1px solid var(--border)',
            color: 'var(--text-3)', cursor: 'pointer',
          }}>
            Cancel
          </button>
          <button onClick={handleSave} disabled={saving} style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '8px 20px', borderRadius: 6, fontSize: 12, fontWeight: 600,
            background: saving ? '#8b5cf650' : '#8b5cf6', color: '#fff',
            border: 'none', cursor: saving ? 'wait' : 'pointer',
          }}>
            {saving ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <CheckCircle2 size={12} />}
            {saving ? 'Saving...' : 'Save Plan'}
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}

/* ── View Plan Dialog (read-only) ──────────────────── */
function ViewPlanDialog({
  wf,
  onClose,
}: {
  wf: Record<string, unknown>;
  onClose: () => void;
}) {
  const [plan, setPlan] = useState<Record<string, unknown> | null>(null);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    (async () => {
      try {
        const res = await fetch(`/api/plans?workflowId=${wf.id}`);
        const data = await res.json();
        setPlan(data.plans?.[0] || null);
      } catch { /* ignore */ }
      setLoading(false);
    })();
  }, [wf.id]);

  const dag = plan?.dag as { nodes: Record<string, unknown>[]; edges: Record<string, unknown>[] } | null;

  return (
    <motion.div
      initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, zIndex: 1000,
        background: 'rgba(7,7,26,0.85)', backdropFilter: 'blur(4px)',
        display: 'flex', alignItems: 'center', justifyContent: 'center',
      }}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.95, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.95, y: 20 }}
        transition={{ type: 'spring', damping: 25, stiffness: 300 }}
        onClick={e => e.stopPropagation()}
        style={{
          background: 'var(--bg-surface)', border: '1px solid var(--border)',
          borderRadius: 12, width: '80vw', maxWidth: 800, maxHeight: '80vh',
          overflow: 'hidden', display: 'flex', flexDirection: 'column',
        }}
      >
        {/* Header */}
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '16px 20px', borderBottom: '1px solid var(--border)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <Eye size={18} color="var(--text-3)" />
            <span style={{ fontSize: 16, fontWeight: 700, color: 'var(--text-1)' }}>
              View Plan — {wf.name as string}
            </span>
            {plan && (
              <span style={{
                fontSize: 10, padding: '2px 8px', borderRadius: 4,
                background: (plan.planType === 'frozen') ? '#06b6d420' : '#8b5cf620',
                color: (plan.planType === 'frozen') ? '#06b6d4' : '#8b5cf6',
                fontWeight: 600,
              }}>
                {(plan.planType as string)?.toUpperCase() || 'LIVE'} v{(plan.version as number) || 1}
              </span>
            )}
          </div>
          <button onClick={onClose} style={{
            background: 'transparent', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', padding: 4,
          }}>
            <X size={18} />
          </button>
        </div>

        {/* Body */}
        <div style={{ flex: 1, padding: 20, overflowY: 'auto' }}>
          {loading && (
            <div style={{ textAlign: 'center', padding: 40, color: 'var(--text-4)' }}>
              <Loader2 size={24} style={{ animation: 'spin 1s linear infinite' }} />
            </div>
          )}

          {!loading && !plan && (
            <div style={{ textAlign: 'center', padding: 40, color: 'var(--text-4)' }}>
              <p style={{ fontSize: 13 }}>No plan exists for this workflow yet.</p>
              <p style={{ fontSize: 11 }}>Click the PLAN button to create one.</p>
            </div>
          )}

          {!loading && plan && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {/* Plan info */}
              <div style={{ display: 'flex', gap: 16, flexWrap: 'wrap' }}>
                <div>
                  <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600 }}>Name</span>
                  <p style={{ fontSize: 13, color: 'var(--text-1)', margin: '2px 0 0' }}>{String(plan.name ?? '')}</p>
                </div>
                <div>
                  <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600 }}>Source</span>
                  <p style={{ fontSize: 13, color: 'var(--text-1)', margin: '2px 0 0' }}>{String(plan.sourceType ?? '')}</p>
                </div>
                <div>
                  <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600 }}>Updated</span>
                  <p style={{ fontSize: 13, color: 'var(--text-1)', margin: '2px 0 0' }}>{timeAgo(plan.updatedAt as string)}</p>
                </div>
                {Boolean(wf.planFrozen) && (
                  <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                    <Snowflake size={12} color="#06b6d4" />
                    <span style={{ fontSize: 11, color: '#06b6d4', fontWeight: 600 }}>FROZEN</span>
                  </div>
                )}
              </div>

              {/* Markdown content */}
              {Boolean(plan.markdownContent) && (
                <div>
                  <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600 }}>Plan Content</span>
                  <pre style={{
                    background: 'var(--bg-1)', border: '1px solid var(--border)',
                    borderRadius: 8, padding: 16, fontSize: 12, color: 'var(--text-2)',
                    maxHeight: 300, overflow: 'auto', whiteSpace: 'pre-wrap',
                    marginTop: 4,
                  }}>{String(plan.markdownContent)}</pre>
                </div>
              )}

              {/* DAG visualization */}
              {dag && dag.nodes && dag.nodes.length > 0 && (
                <div>
                  <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600 }}>
                    DAG — {dag.nodes.length} nodes, {dag.edges.length} edges
                  </span>
                  <div style={{
                    background: 'var(--bg-1)', border: '1px solid var(--border)',
                    borderRadius: 8, padding: 16, marginTop: 4,
                    display: 'flex', flexWrap: 'wrap', gap: 8,
                  }}>
                    {dag.nodes.map((n) => (
                      <div key={n.id as string} style={{
                        padding: '6px 12px', borderRadius: 6, fontSize: 11, fontWeight: 500,
                        background: n.status === 'completed' ? '#10b98115' : n.status === 'running' ? '#f59e0b15' : '#8b5cf615',
                        color: n.status === 'completed' ? '#10b981' : n.status === 'running' ? '#f59e0b' : '#8b5cf6',
                        border: `1px solid ${n.status === 'completed' ? '#10b98130' : n.status === 'running' ? '#f59e0b30' : '#8b5cf630'}`,
                      }}>
                        {(n.label as string) || (n.id as string)}
                        {n.connectionType === 'parallel' && (
                          <span style={{ marginLeft: 4, fontSize: 9, opacity: 0.7 }}>⟂</span>
                        )}
                      </div>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
        </div>
      </motion.div>
    </motion.div>
  );
}

/* ── Delete Confirm Dialog ───────────────────────────── */
function DeleteConfirmDialog({
  name,
  onClose,
  onConfirm,
}: {
  name: string;
  onClose: () => void;
  onConfirm: () => void;
}) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
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
        style={{
          width: 400, background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
          borderRadius: 10, padding: 24,
          boxShadow: '0 20px 60px rgba(0,0,0,0.2)',
        }}
      >
        <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', marginBottom: 8 }}>Delete Workflow</div>
        <div style={{ fontSize: 13, color: 'var(--text-3)', marginBottom: 20, lineHeight: 1.5 }}>
          Are you sure you want to delete <strong style={{ color: 'var(--text-1)' }}>{name}</strong>? This will also remove all steps and cannot be undone.
        </div>
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button onClick={onClose} style={{
            padding: '8px 16px', borderRadius: 7, fontSize: 12,
            border: '1px solid var(--border-md)', background: 'transparent',
            color: 'var(--text-3)', cursor: 'pointer',
          }}>Cancel</button>
          <button onClick={onConfirm} style={{
            display: 'flex', alignItems: 'center', gap: 5,
            padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 700,
            border: '1.5px solid #ef4444', background: '#ef444412', color: '#ef4444',
            cursor: 'pointer',
          }}>
            <Trash2 size={12} /> Delete
          </button>
        </div>
      </motion.div>
    </motion.div>
  );
}

/* ── Run Log Panel (expandable step executions) ─────── */
function RunLogPanel({ runId }: { runId: string }) {
  const { executions, isLoading } = useStepExecutions(runId);

  if (isLoading) {
    return (
      <div style={{ padding: '16px 14px', borderTop: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 6, color: 'var(--text-4)', fontSize: 11 }}>
        <Loader2 size={10} style={{ animation: 'spin 1s linear infinite' }} /> Loading step logs...
      </div>
    );
  }

  if (!executions || executions.length === 0) {
    return (
      <div style={{ padding: '16px 14px', borderTop: '1px solid var(--border)', color: 'var(--text-4)', fontSize: 11, textAlign: 'center' }}>
        No step executions recorded for this run.
      </div>
    );
  }

  return (
    <div style={{ borderTop: '1px solid var(--border)', maxHeight: 280, overflowY: 'auto' }}>
      {executions.map((step: Record<string, unknown>, i: number) => {
        const stepSt = STEP_STATUS_STYLE[(step.status as string) ?? 'pending'] ?? STEP_STATUS_STYLE.pending;
        return (
          <div key={(step.id as string) || i} style={{
            padding: '8px 14px 8px 33px',
            borderBottom: i < executions.length - 1 ? '1px solid rgba(255,255,255,0.04)' : 'none',
            fontSize: 11,
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
              <span style={{
                padding: '1px 5px', borderRadius: 3, fontSize: 8, fontWeight: 700,
                background: stepSt.bg, color: stepSt.color,
              }}>{stepSt.label}</span>
              <span style={{ fontWeight: 600, color: 'var(--text-2)' }}>
                {(step.stepName as string) || (step.stepId as string) || `Step ${i + 1}`}
              </span>
              {(step.model as string) && (
                <span className="mono" style={{ fontSize: 9, color: 'var(--text-4)', background: 'var(--bg-surface)', padding: '1px 4px', borderRadius: 3 }}>
                  {step.model as string}
                </span>
              )}
            </div>
            <div style={{ display: 'flex', gap: 12, color: 'var(--text-4)', fontSize: 10 }}>
              {(step.tokensUsed as number) > 0 && (
                <span className="mono"><Zap size={8} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />{fmt(step.tokensUsed as number)} tok</span>
              )}
              {(step.durationMs as number) > 0 && (
                <span className="mono"><Clock size={8} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />{fmtElapsed(step.durationMs as number)}</span>
              )}
              {(step.completedAt as string) && (
                <span>{new Date(step.completedAt as string).toLocaleTimeString()}</span>
              )}
            </div>
            {(step.errorMessage as string) && (
              <div style={{ marginTop: 4, padding: '4px 8px', borderRadius: 4, background: '#ef444410', color: '#ef4444', fontSize: 10, fontFamily: 'var(--font-mono, monospace)', whiteSpace: 'pre-wrap', maxHeight: 80, overflow: 'auto' }}>
                {step.errorMessage as string}
              </div>
            )}
            {(step.responsePreview as string) && (
              <div style={{ marginTop: 4, padding: '4px 8px', borderRadius: 4, background: 'rgba(255,255,255,0.03)', color: 'var(--text-3)', fontSize: 10, fontFamily: 'var(--font-mono, monospace)', whiteSpace: 'pre-wrap', maxHeight: 60, overflow: 'auto' }}>
                {(step.responsePreview as string).slice(0, 300)}
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}

/* ── Workflow Detail Dialog ──────────────────────────── */
function WorkflowDetailDialog({
  wf,
  onClose,
  onRun,
  onStop,
  onRestart,
  onDelete,
  onEdit,
  runningWf,
}: {
  wf: Record<string, unknown>;
  onClose: () => void;
  onRun: (id: string) => void;
  onStop: (id: string) => void;
  onRestart: (id: string) => void;
  onDelete: (id: string) => void;
  onEdit: (id: string) => void;
  runningWf: boolean;
}) {
  const router = useRouter();
  const { runs } = useWorkflowRuns(wf.id as string, 10);
  const [activeTab, setActiveTab] = useState<'details' | 'runs'>('details');
  const [expandedRunId, setExpandedRunId] = useState<string | null>(null);
  const status = (wf.status as string) || 'idle';
  const st = STATUS_STYLE[status] ?? STATUS_STYLE.idle;
  const canRun = ['idle', 'draft', 'ready', 'completed', 'failed', 'cancelled'].includes(status);
  const canStop = status === 'running';
  const canRestart = ['completed', 'failed', 'cancelled'].includes(status);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      onClick={onClose}
      style={{
        position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
        zIndex: 200, display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
        paddingTop: 40, overflowY: 'auto',
      }}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: -8 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        exit={{ opacity: 0, scale: 0.96, y: -8 }}
        transition={{ type: 'spring', damping: 25, stiffness: 300 }}
        onClick={e => e.stopPropagation()}
        style={{
          width: 700, maxWidth: '94vw', maxHeight: '85vh',
          background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
          borderRadius: 12, overflow: 'hidden', display: 'flex', flexDirection: 'column',
          boxShadow: '0 24px 80px rgba(0,0,0,0.3)',
          marginBottom: 40,
        }}
      >
        {/* Header */}
        <div style={{
          padding: '18px 22px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10, minWidth: 0, flex: 1 }}>
            <Workflow size={16} color={SECTION_COLOR} />
            <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
              {wf.name as string}
            </span>
            <span style={{
              padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
              background: st.bg, color: st.color, border: `1px solid ${st.color}28`,
              flexShrink: 0,
            }}>{st.label}</span>
            {status === 'running' && (wf.lastRunAt as string) ? (
              <RunningTimer startedAt={wf.lastRunAt as string} />
            ) : null}
          </div>
          <button onClick={onClose} style={{
            background: 'none', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', display: 'flex', padding: 4,
          }}><X size={16} /></button>
        </div>

        {/* Tabs */}
        <div style={{ display: 'flex', borderBottom: '1px solid var(--border)', padding: '0 22px' }}>
          {(['details', 'runs'] as const).map(tab => (
            <button key={tab} onClick={() => setActiveTab(tab)} style={{
              padding: '10px 16px', fontSize: 12, fontWeight: activeTab === tab ? 700 : 400,
              color: activeTab === tab ? SECTION_COLOR : 'var(--text-3)',
              background: 'none', borderTop: 'none', borderLeft: 'none', borderRight: 'none',
              borderBottom: activeTab === tab ? `2px solid ${SECTION_COLOR}` : '2px solid transparent',
              cursor: 'pointer', textTransform: 'capitalize',
            }}>{tab === 'runs' ? 'Run History' : tab}</button>
          ))}
        </div>

        {/* Body */}
        <div style={{ padding: '20px 22px', overflowY: 'auto', flex: 1 }}>
          {activeTab === 'details' && (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {(wf.description as string) ? (
                <div>
                  <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 4 }}>Description</div>
                  <div style={{ fontSize: 13, color: 'var(--text-2)', lineHeight: 1.5 }}>{wf.description as string}</div>
                </div>
              ) : null}
              {(wf.goalStatement as string) ? (
                <div>
                  <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 4 }}>Goal</div>
                  <div style={{ fontSize: 12, color: 'var(--text-2)', lineHeight: 1.5, fontFamily: 'var(--font-mono, monospace)', whiteSpace: 'pre-wrap', maxHeight: 200, overflow: 'auto', background: 'var(--bg)', padding: '10px 12px', borderRadius: 6, border: '1px solid var(--border)' }}>
                    {wf.goalStatement as string}
                  </div>
                </div>
              ) : null}
              <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 10 }}>
                {[
                  { label: 'Total Runs', value: String((wf.totalRuns as number) ?? 0), icon: Play },
                  { label: 'Tokens', value: fmt((wf.estimatedTokens as number) ?? 0), icon: Zap },
                  { label: 'Status', value: st.label, icon: CheckCircle2 },
                  { label: 'Updated', value: timeAgo(wf.updatedAt as string), icon: Clock },
                ].map(item => (
                  <div key={item.label} style={{ padding: '10px 12px', background: 'var(--bg)', borderRadius: 6, border: '1px solid var(--border)' }}>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 4, marginBottom: 4 }}>
                      <item.icon size={10} color="var(--text-4)" />
                      <span style={{ fontSize: 10, color: 'var(--text-4)', fontWeight: 600, textTransform: 'uppercase' }}>{item.label}</span>
                    </div>
                    <div className="mono" style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>{item.value}</div>
                  </div>
                ))}
              </div>
              {((wf.tags as string[]) ?? []).length > 0 && (
                <div>
                  <div style={{ fontSize: 10, fontWeight: 600, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.06em', marginBottom: 6 }}>Tags</div>
                  <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                    {((wf.tags as string[]) ?? []).map(tag => (
                      <span key={tag} style={{ padding: '3px 8px', borderRadius: 4, fontSize: 11, fontWeight: 500, background: `${SECTION_COLOR}10`, color: SECTION_COLOR, border: `1px solid ${SECTION_COLOR}25` }}>{tag}</span>
                    ))}
                  </div>
                </div>
              )}
            </div>
          )}
          {activeTab === 'runs' && (
            <div>
              {(!runs || runs.length === 0) ? (
                <div style={{ textAlign: 'center', padding: '40px 0', color: 'var(--text-4)', fontSize: 13 }}>
                  No run history yet. Run the workflow to see results here.
                </div>
              ) : (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {runs.map((run: Record<string, unknown>) => {
                    const runId = run.id as string;
                    const runSt = STATUS_STYLE[(run.status as string) ?? 'idle'] ?? STATUS_STYLE.idle;
                    const isExpanded = expandedRunId === runId;
                    return (
                      <div key={runId} style={{ background: 'var(--bg)', borderRadius: 6, border: '1px solid var(--border)', overflow: 'hidden' }}>
                        <div
                          onClick={() => setExpandedRunId(prev => prev === runId ? null : runId)}
                          style={{ padding: '12px 14px', cursor: 'pointer', transition: 'background 0.15s' }}
                        >
                          <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                              <ChevronRight size={11} color="var(--text-4)" style={{ transform: isExpanded ? 'rotate(90deg)' : 'none', transition: 'transform 0.15s', flexShrink: 0 }} />
                              <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>{runId}</span>
                              <span style={{ padding: '2px 6px', borderRadius: 99, fontSize: 9, fontWeight: 700, background: runSt.bg, color: runSt.color }}>{runSt.label}</span>
                              {run.status === 'running' && <Loader2 size={10} color="#f59e0b" style={{ animation: 'spin 1s linear infinite' }} />}
                            </div>
                            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                              <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                                {run.startedAt ? new Date(run.startedAt as string).toLocaleString() : '—'}
                              </span>
                              <button
                                onClick={(e) => { e.stopPropagation(); router.push(`/monitoring/logs?category=workflows&runId=${runId}`); }}
                                title="View in Logs"
                                style={{
                                  display: 'flex', alignItems: 'center', gap: 3,
                                  padding: '3px 7px', borderRadius: 4, fontSize: 9, fontWeight: 600,
                                  border: '1px solid var(--border)', background: 'transparent',
                                  color: 'var(--text-4)', cursor: 'pointer',
                                }}
                              >
                                <ExternalLink size={8} /> Logs
                              </button>
                            </div>
                          </div>
                          <div style={{ display: 'flex', gap: 16, fontSize: 11, color: 'var(--text-3)', paddingLeft: 19 }}>
                            {(run.durationSec as number) > 0 && <span className="mono"><Clock size={9} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />{run.durationSec as number}s</span>}
                            {(run.totalTokensUsed as number) > 0 && <span className="mono"><Zap size={9} style={{ display: 'inline', verticalAlign: 'middle', marginRight: 2 }} />{fmt(run.totalTokensUsed as number)} tok</span>}
                            {(run.errorMessage as string) ? <span style={{ color: '#ef4444', fontSize: 11 }}>{(run.errorMessage as string).slice(0, 100)}</span> : null}
                          </div>
                        </div>
                        {isExpanded && <RunLogPanel runId={runId} />}
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}
        </div>

        {/* Footer Actions */}
        <div style={{
          padding: '14px 22px', borderTop: '1px solid var(--border)',
          display: 'flex', gap: 8, justifyContent: 'space-between', alignItems: 'center',
        }}>
          <div style={{ display: 'flex', gap: 6 }}>
            {canRun && (
              <button onClick={() => onRun(wf.id as string)} disabled={runningWf} style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 700,
                border: '1.5px solid #10b981', background: '#10b98114', color: '#10b981',
                cursor: runningWf ? 'wait' : 'pointer', opacity: runningWf ? 0.5 : 1,
              }}>
                {runningWf ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Play size={12} />} Run
              </button>
            )}
            {canStop && (
              <button onClick={() => onStop(wf.id as string)} style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 700,
                border: '1.5px solid #ef4444', background: '#ef444414', color: '#ef4444',
                cursor: 'pointer',
              }}>
                <Square size={12} /> Stop
              </button>
            )}
            {canRestart && (
              <button onClick={() => onRestart(wf.id as string)} style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '8px 14px', borderRadius: 7, fontSize: 12, fontWeight: 600,
                border: '1px solid #f59e0b50', background: '#f59e0b10', color: '#f59e0b',
                cursor: 'pointer',
              }}>
                <RotateCcw size={11} /> Restart
              </button>
            )}
          </div>
          <div style={{ display: 'flex', gap: 6 }}>
            <button onClick={() => onEdit(wf.id as string)} style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '8px 14px', borderRadius: 7, fontSize: 12, fontWeight: 600,
              border: `1px solid ${SECTION_COLOR}50`, background: `${SECTION_COLOR}10`, color: SECTION_COLOR,
              cursor: 'pointer',
            }}>
              <Pencil size={11} /> Edit in Builder
            </button>
            <button onClick={() => onDelete(wf.id as string)} style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '8px 12px', borderRadius: 7, fontSize: 12,
              border: '1px solid var(--border)', background: 'transparent',
              color: 'var(--text-4)', cursor: 'pointer',
            }}>
              <Trash2 size={11} />
            </button>
          </div>
        </div>
      </motion.div>
    </motion.div>
  );
}

/* ── Main Page ───────────────────────────────────────── */
export default function WorkflowsPage() {
  const router = useRouter();
  const { workflows, total, isLoading, mutate } = useWorkflows();
  const [search, setSearch] = useState('');
  const [statusFilter, setStatusFilter] = useState<string>('all');
  const [sortField, setSortField] = useState<SortField>('updatedAt');
  const [sortDir, setSortDir] = useState<SortDir>('desc');
  const [deletingWf, setDeletingWf] = useState<Record<string, unknown> | null>(null);
  const [selectedWf, setSelectedWf] = useState<Record<string, unknown> | null>(null);
  const [runningIds, setRunningIds] = useState<Set<string>>(new Set());
  const runGuardRef = useRef<Set<string>>(new Set());
  const [runError, setRunError] = useState<string | null>(null);
  const [planDialogWf, setPlanDialogWf] = useState<Record<string, unknown> | null>(null);
  const [viewPlanWf, setViewPlanWf] = useState<Record<string, unknown> | null>(null);
  const [freezingId, setFreezingId] = useState<string | null>(null);
  const [outputWf, setOutputWf] = useState<Record<string, unknown> | null>(null);
  const [showSharedImport, setShowSharedImport] = useState(false);
  const ws = useWorkflowWS();

  // Check if any workflow is running for faster polling
  const hasRunning = useMemo(() => workflows.some((w: Record<string, unknown>) => w.status === 'running'), [workflows]);

  // Faster refresh when workflows are running
  useEffect(() => {
    if (!hasRunning) return;
    const id = setInterval(() => mutate(), 5000);
    return () => clearInterval(id);
  }, [hasRunning, mutate]);

  // Auto-subscribe to running workflow runs for live updates
  useEffect(() => {
    if (!hasRunning) return;
    const runningWfs = workflows.filter((w: Record<string, unknown>) => w.status === 'running');
    // Find run IDs for running workflows via latest runs
    const fetchRuns = async () => {
      for (const wf of runningWfs) {
        try {
          const res = await fetch(`/api/workflows/runs?workflowId=${wf.id}&limit=1`);
          if (res.ok) {
            const { runs } = await res.json();
            const activeRun = runs?.find((r: Record<string, unknown>) => r.status === 'running');
            if (activeRun) {
              ws.connect(activeRun.id as string);
              return; // Connect to first running workflow
            }
          }
        } catch { /* ignore */ }
      }
    };
    fetchRuns();
  }, [hasRunning]); // eslint-disable-line react-hooks/exhaustive-deps

  /* Filter + sort */
  const filtered = useMemo(() => {
    let list = [...workflows] as Record<string, unknown>[];

    if (statusFilter !== 'all') {
      list = list.filter(w => w.status === statusFilter);
    }
    if (search.trim()) {
      const q = search.toLowerCase();
      list = list.filter(w =>
        (w.name as string).toLowerCase().includes(q) ||
        ((w.description as string) ?? '').toLowerCase().includes(q) ||
        ((w.tags as string[]) ?? []).some(t => t.toLowerCase().includes(q)),
      );
    }

    list.sort((a, b) => {
      let av: string | number, bv: string | number;
      switch (sortField) {
        case 'name':
          av = (a.name as string).toLowerCase();
          bv = (b.name as string).toLowerCase();
          break;
        case 'updatedAt':
          av = new Date((a.updatedAt as string) ?? 0).getTime();
          bv = new Date((b.updatedAt as string) ?? 0).getTime();
          break;
        case 'status':
          av = (a.status as string) ?? '';
          bv = (b.status as string) ?? '';
          break;
        case 'totalRuns':
          av = (a.totalRuns as number) ?? 0;
          bv = (b.totalRuns as number) ?? 0;
          break;
        case 'estimatedTokens':
          av = (a.estimatedTokens as number) ?? 0;
          bv = (b.estimatedTokens as number) ?? 0;
          break;
        default:
          av = 0; bv = 0;
      }
      if (av < bv) return sortDir === 'asc' ? -1 : 1;
      if (av > bv) return sortDir === 'asc' ? 1 : -1;
      return 0;
    });

    return list;
  }, [workflows, search, statusFilter, sortField, sortDir]);

  const toggleSort = (field: SortField) => {
    if (sortField === field) {
      setSortDir(d => d === 'asc' ? 'desc' : 'asc');
    } else {
      setSortField(field);
      setSortDir(field === 'name' ? 'asc' : 'desc');
    }
  };

  const renderSortIcon = (field: SortField) => {
    if (sortField !== field) return <ArrowUpDown size={10} color="var(--text-4)" />;
    return sortDir === 'asc' ? <ChevronUp size={10} color={SECTION_COLOR} /> : <ChevronDown size={10} color={SECTION_COLOR} />;
  };

  /* Delete handler */
  const handleDelete = async (id: string) => {
    await fetch(`/api/workflows?id=${id}`, { method: 'DELETE' });
    setDeletingWf(null);
    setSelectedWf(null);
    mutate();
  };

  /* Freeze / unfreeze handler */
  const handleFreeze = useCallback(async (workflowId: string) => {
    const wf = workflows.find((w: Record<string, unknown>) => (w.id as string) === workflowId);
    const isFrozen = (wf?.planFrozen as boolean) || false;
    setFreezingId(workflowId);
    try {
      await fetch('/api/plans/freeze', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ workflowId, action: isFrozen ? 'unfreeze' : 'freeze' }),
      });
      mutate();
    } catch (err) {
      console.error('Freeze toggle failed:', err);
    } finally {
      setFreezingId(null);
    }
  }, [workflows, mutate]);

  /* Run workflow handler */
  const handleRun = useCallback(async (workflowId: string) => {
    // Synchronous ref guard — prevents double-invocation before React re-renders
    if (runGuardRef.current.has(workflowId)) {
      console.warn('[handleRun] blocked duplicate invocation for', workflowId);
      return;
    }
    runGuardRef.current.add(workflowId);
    console.trace('[handleRun] called', workflowId, new Date().toISOString());
    setRunningIds(prev => new Set(prev).add(workflowId));
    try {
      // Fetch workflow with steps
      const wfRes = await fetch(`/api/workflows?id=${workflowId}`);
      if (!wfRes.ok) throw new Error('Failed to fetch workflow');
      const { workflow: wfData, steps: wfSteps } = await wfRes.json();

      if (!wfSteps || wfSteps.length === 0) {
        setRunError('This workflow has no steps. Edit it in the builder first.');
        return;
      }

      // Upload goal file
      const formData = new FormData();
      if (wfData.goalStatement) {
        const blob = new Blob([wfData.goalStatement], { type: 'text/markdown' });
        formData.append('files', blob, 'goal.md');
      }
      let goalFileUrl = '';
      try {
        const uploadResp = await fetch(`${ENGINE_URL}/api/orchestrator/upload`, { method: 'POST', body: formData });
        if (uploadResp.ok) {
          const uploadData = await uploadResp.json();
          const uploaded = uploadData.files || [];
          if (uploaded.length > 0) goalFileUrl = uploaded[0].url;
        }
      } catch {
        // Upload failure is non-critical for local goals
      }

      // Update workflow status to running
      await fetch('/api/workflows', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: workflowId, status: 'running', lastRunAt: new Date().toISOString() }),
      });

      // Build step configs
      const stepConfigs = wfSteps.map((s: Record<string, unknown>, i: number) => ({
        stepId: s.id || `step-${i + 1}`,
        name: s.name || null,
        expertId: s.expertId || null,
        taskDescription: s.taskDescription || '',
        systemInstructions: s.systemInstructions || '',
        voiceCommand: s.voiceCommand || '',
        fileLocations: s.fileLocations || [],
        stepFileNames: s.stepFileUrls || [],
        stepImageNames: s.stepImageUrls || [],
        modelSource: s.modelSource || 'local',
        localModel: s.localModelConfig || null,
        temperature: s.temperature ?? 0.7,
        maxTokens: s.maxTokens ?? 4096,
        connectionType: s.connectionType || 'sequential',
        shareMemory: s.shareMemory !== false,
        integrations: s.integrations || [],
        stepType: s.stepType || 'agent',
        actionConfig: s.actionConfig || null,
      }));

      // Create run record in NeonDB via REST (reliable DB insert)
      const runId = `run-${Date.now()}`;
      await fetch('/api/workflows/run', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          runId,
          workflowId,
          name: wfData.name,
          goalFileUrl,
          inputFileUrls: [],
          steps: stepConfigs,
        }),
      });

      // Submit execution to engine via WebSocket for real-time updates
      ws.submitWorkflow(runId, {
        workflowId,
        name: wfData.name,
        goalFileUrl,
        inputFileUrls: [],
        steps: stepConfigs,
      });

      mutate();
    } catch (err) {
      console.error('Failed to run workflow:', err);
      // Reset workflow status on failure
      await fetch('/api/workflows', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: workflowId, status: 'idle' }),
      }).catch(() => {});
    } finally {
      runGuardRef.current.delete(workflowId);
      setRunningIds(prev => {
        const next = new Set(prev);
        next.delete(workflowId);
        return next;
      });
      mutate();
    }
  }, [mutate, ws.submitWorkflow]);

  /* Stop workflow handler */
  const handleStop = useCallback(async (workflowId: string) => {
    try {
      // Find the latest running run for this workflow
      const runsRes = await fetch(`/api/workflows/runs?workflowId=${workflowId}&limit=1`);
      if (!runsRes.ok) return;
      const { runs } = await runsRes.json();
      const activeRun = runs?.find((r: Record<string, unknown>) => r.status === 'running');
      if (!activeRun) return;

      await fetch('/api/workflows/stop', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ runId: activeRun.id, workflowId }),
      });
      ws.disconnect();
      mutate();
    } catch (err) {
      console.error('Failed to stop workflow:', err);
    }
  }, [mutate, ws]);

  /* Restart workflow handler */
  const handleRestart = useCallback(async (workflowId: string) => {
    try {
      // Find the latest run
      const runsRes = await fetch(`/api/workflows/runs?workflowId=${workflowId}&limit=1`);
      if (!runsRes.ok) return;
      const { runs } = await runsRes.json();
      if (!runs?.length) {
        // No previous run — just do a fresh run
        handleRun(workflowId);
        return;
      }

      await fetch('/api/workflows/restart', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ runId: runs[0].id, workflowId }),
      });

      // Update workflow status
      await fetch('/api/workflows', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id: workflowId, status: 'running', lastRunAt: new Date().toISOString() }),
      });
      mutate();
    } catch (err) {
      console.error('Failed to restart workflow:', err);
    }
  }, [mutate, handleRun]);

  /* Status counts */
  const statusCounts = useMemo(() => {
    const counts: Record<string, number> = { all: workflows.length };
    for (const w of workflows) {
      const s = (w as Record<string, unknown>).status as string;
      counts[s] = (counts[s] ?? 0) + 1;
    }
    return counts;
  }, [workflows]);

  /* Stats for panels */
  const runningCt = statusCounts['running'] ?? 0;
  const completedCt = statusCounts['completed'] ?? 0;
  const failedCt = statusCounts['failed'] ?? 0;
  const avgSuccessRate = useMemo(() => {
    const done = completedCt + failedCt;
    return done > 0 ? completedCt / done : 0;
  }, [completedCt, failedCt]);
  const totalRunsAll = useMemo(() => {
    return workflows.reduce((sum: number, w: Record<string, unknown>) => sum + ((w.totalRuns as number) ?? 0), 0);
  }, [workflows]);

  /* Sort dropdown state */
  const [sortOpen, setSortOpen] = useState(false);
  const SORT_LABELS: Record<SortField, string> = {
    name: 'Name', updatedAt: 'Last Modified', status: 'Status', totalRuns: 'Runs', estimatedTokens: 'Tokens',
  };

  const TH: React.CSSProperties = {
    padding: '10px 14px', fontSize: 10, fontWeight: 700, color: 'var(--text-3)',
    textTransform: 'uppercase', letterSpacing: '0.06em', textAlign: 'left',
    borderBottom: '1px solid var(--border)', cursor: 'pointer',
    userSelect: 'none', whiteSpace: 'nowrap',
  };

  const TD: React.CSSProperties = {
    padding: '12px 14px', fontSize: 13, color: 'var(--text-2)',
    borderBottom: '1px solid var(--border)',
  };

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{
            width: 36, height: 36, borderRadius: 8,
            background: `${SECTION_COLOR}15`, border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Workflow size={18} color={SECTION_COLOR} />
          </div>
          <div>
            <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>Workflows</h1>
            <p style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 3, margin: '3px 0 0', letterSpacing: '0.5px', textTransform: 'uppercase' }}>
              Wiring &middot; Orchestration &middot; Routing &middot; Knowledge &middot; Flow
            </p>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4, margin: '4px 0 0', maxWidth: 420 }}>
              Build autonomous agent pipelines — orchestrate multi-step reasoning,
              code generation, and domain tasks into workflows that execute,
              monitor, and scale automatically.
            </p>
          </div>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <ImportButton entityType="workflow" onImported={() => mutate()} size="md" />
          <SharedImportButton onClick={() => setShowSharedImport(true)} size="md" />
          <button onClick={() => router.push('/workflow/builder')} style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '9px 18px', borderRadius: 8,
            border: `1.5px solid ${SECTION_COLOR}`,
            background: `${SECTION_COLOR}14`,
            color: SECTION_COLOR, fontSize: 13, fontWeight: 700,
            cursor: 'pointer',
          }}>
            <Plus size={14} strokeWidth={2.5} /> Create New
          </button>
        </div>
      </motion.div>

      {/* Stats panels */}
      <motion.div
        variants={stagger(0.06)}
        initial="hidden"
        animate="show"
        style={{
          display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)',
          gap: 10, marginBottom: 22,
        }}
      >
        {[
          { label: 'Total Workflows', value: String(total),       color: SECTION_COLOR, icon: Workflow,     sub: 'pipelines'      },
          { label: 'Running',         value: String(runningCt),    color: '#f59e0b',     icon: Activity,     sub: 'executing now'  },
          { label: 'Total Runs',      value: fmt(totalRunsAll),    color: '#10b981',     icon: Play,         sub: 'all time'       },
          { label: 'Success Rate',    value: avgSuccessRate > 0 ? `${(avgSuccessRate * 100).toFixed(1)}%` : '—', color: '#06b6d4', icon: TrendingUp, sub: 'completed / total' },
        ].map(({ label, value, color, icon: Icon, sub }) => (
          <motion.div
            key={label}
            variants={fadeUp}
            whileHover={hoverLift.whileHover}
            transition={hoverLift.transition}
            style={{
              background: 'var(--bg-surface)',
              border: '1px solid var(--border)',
              borderRadius: 11, padding: '15px 18px',
              display: 'flex', alignItems: 'center', gap: 13,
              cursor: 'default',
            }}
          >
            <div style={{
              width: 36, height: 36, borderRadius: 8, flexShrink: 0,
              background: `${color}12`, border: `1.5px solid ${color}22`,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Icon size={16} color={color} strokeWidth={2} />
            </div>
            <div>
              <div style={{ fontSize: 22, fontWeight: 800, color, lineHeight: 1 }}>{value}</div>
              <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', marginTop: 2 }}>{label}</div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1 }}>{sub}</div>
            </div>
          </motion.div>
        ))}
      </motion.div>

      {/* Filters bar */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.08, duration: 0.3 }}
        style={{ display: 'flex', gap: 8, marginBottom: 16, alignItems: 'center', flexWrap: 'wrap' }}
      >
        {/* Search */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
          background: 'var(--bg-surface)',
        }}>
          <Search size={13} color="var(--text-4)" />
          <input value={search} onChange={e => setSearch(e.target.value)}
            placeholder="Search name, description, tags..."
            style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 13, color: 'var(--text-1)', width: 220 }} />
          {search && (
            <button onClick={() => setSearch('')} style={{
              background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 0,
            }}><X size={12} /></button>
          )}
        </div>

        {/* Status filters */}
        {['all', 'idle', 'draft', 'ready', 'running', 'completed', 'failed'].map(s => {
          const count = statusCounts[s] ?? 0;
          if (s !== 'all' && count === 0) return null;
          const active = statusFilter === s;
          const stl = s === 'all' ? { color: SECTION_COLOR, bg: `${SECTION_COLOR}12`, label: 'All' } : (STATUS_STYLE[s] ?? STATUS_STYLE.idle);
          return (
            <button key={s} onClick={() => setStatusFilter(s)} style={{
              padding: '5px 12px', borderRadius: 20, fontSize: 11, cursor: 'pointer',
              border: `1px solid ${active ? stl.color : 'var(--border)'}`,
              background: active ? stl.bg : 'transparent',
              color: active ? stl.color : 'var(--text-3)',
              fontWeight: active ? 700 : 400, transition: 'all 0.12s',
            }}>
              {stl.label}
              <span style={{ marginLeft: 4, fontSize: 10, opacity: 0.7 }}>({count})</span>
            </button>
          );
        })}

        {/* Sort dropdown */}
        <div style={{ position: 'relative', marginLeft: 'auto' }}>
          <button
            onClick={() => setSortOpen(o => !o)}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', cursor: 'pointer',
              fontSize: 12, color: 'var(--text-2)', fontWeight: 500,
            }}
          >
            <ArrowUpDown size={12} />
            Sort: {SORT_LABELS[sortField]}
            <ChevronDown size={11} style={{ transition: 'transform 0.15s', transform: sortOpen ? 'rotate(180deg)' : 'none' }} />
          </button>
          <AnimatePresence>
            {sortOpen && (
              <motion.div
                initial={{ opacity: 0, y: -6, scale: 0.97 }}
                animate={{ opacity: 1, y: 0, scale: 1 }}
                exit={{ opacity: 0, y: -4, scale: 0.97 }}
                transition={{ duration: 0.14 }}
                style={{
                  position: 'absolute', top: 'calc(100% + 6px)', right: 0, zIndex: 50,
                  background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
                  borderRadius: 9, padding: 6, minWidth: 150,
                  boxShadow: '0 8px 24px rgba(13,13,13,0.10)',
                }}
              >
                {(Object.keys(SORT_LABELS) as SortField[]).map(field => (
                  <button
                    key={field}
                    onClick={() => { setSortField(field); setSortDir(field === 'name' ? 'asc' : 'desc'); setSortOpen(false); }}
                    style={{
                      display: 'block', width: '100%', textAlign: 'left',
                      padding: '7px 12px', borderRadius: 6, cursor: 'pointer',
                      fontSize: 12, fontWeight: sortField === field ? 700 : 400,
                      background: sortField === field ? `${SECTION_COLOR}12` : 'transparent',
                      color: sortField === field ? SECTION_COLOR : 'var(--text-2)',
                      border: 'none',
                    }}
                  >
                    {SORT_LABELS[field]}
                  </button>
                ))}
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </motion.div>

      {/* Table */}
      {isLoading ? (
        <div style={{ textAlign: 'center', padding: '60px 0', color: 'var(--text-4)' }}>
          <Loader2 size={20} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite' }} />
          <div style={{ fontSize: 13 }}>Loading workflows...</div>
        </div>
      ) : filtered.length === 0 ? (
        <motion.div
          initial={{ opacity: 0, y: 12 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ duration: 0.3 }}
          style={{
            textAlign: 'center', padding: '80px 20px',
            border: '1px dashed var(--border-md)', borderRadius: 10,
          }}
        >
          <Workflow size={28} color="var(--text-4)" style={{ margin: '0 auto 12px' }} />
          <div style={{ fontSize: 15, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>
            {search || statusFilter !== 'all' ? 'No workflows match your filters' : 'No workflows yet'}
          </div>
          <div style={{ fontSize: 13, color: 'var(--text-3)', marginBottom: 16 }}>
            {search || statusFilter !== 'all'
              ? 'Try adjusting your search or clearing filters.'
              : 'Create your first workflow to start building agent pipelines.'}
          </div>
          {!search && statusFilter === 'all' && (
            <button onClick={() => router.push('/workflow/builder')} style={{
              display: 'inline-flex', alignItems: 'center', gap: 6,
              padding: '10px 20px', borderRadius: 8,
              border: `1.5px solid ${SECTION_COLOR}`,
              background: `${SECTION_COLOR}14`,
              color: SECTION_COLOR, fontSize: 13, fontWeight: 700,
              cursor: 'pointer',
            }}>
              <Plus size={14} /> Create Your First
            </button>
          )}
        </motion.div>
      ) : (
        <motion.div
          initial={{ opacity: 0, y: 8 }}
          animate={{ opacity: 1, y: 0 }}
          transition={{ delay: 0.12, duration: 0.3 }}
          style={{
            background: 'var(--bg-surface)', border: '1px solid var(--border)',
            borderRadius: 10, overflow: 'hidden',
          }}
        >
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr style={{ background: 'var(--bg-elevated)' }}>
                <th style={TH} onClick={() => toggleSort('name')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Name {renderSortIcon('name')}</span>
                </th>
                <th style={TH} onClick={() => toggleSort('status')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Status {renderSortIcon('status')}</span>
                </th>
                <th style={TH}>Goal</th>
                <th style={TH} onClick={() => toggleSort('updatedAt')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Updated {renderSortIcon('updatedAt')}</span>
                </th>
                <th style={{ ...TH, textAlign: 'right' }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map((wf, index) => {
                const status = (wf.status as string) ?? 'idle';
                const st = STATUS_STYLE[status] ?? STATUS_STYLE.idle;
                const tags = ((wf.tags as string[]) ?? []).slice(0, 3);
                const goal = (wf.goalStatement as string) ?? '';
                const runs = (wf.totalRuns as number) ?? 0;
                const tokens = (wf.estimatedTokens as number) ?? 0;
                const isRunning = status === 'running';
                const canRun = ['idle', 'draft', 'ready', 'completed', 'failed', 'cancelled'].includes(status);
                const canStop = isRunning;
                const canRestart = ['completed', 'failed', 'cancelled'].includes(status);
                const isStarting = runningIds.has(wf.id as string);
                const isFrozen = (wf.planFrozen as boolean) || false;

                return (
                  <React.Fragment key={wf.id as string}>
                  <motion.tr
                    initial={{ opacity: 0, y: 8 }}
                    animate={{ opacity: 1, y: 0 }}
                    transition={{ delay: index * 0.03, duration: 0.3 }}
                    style={{ transition: 'background 0.1s', cursor: 'pointer' }}
                    onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-elevated)')}
                    onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                    onClick={() => setSelectedWf(wf)}
                  >
                    <td style={TD}>
                      <div style={{ fontWeight: 600, color: 'var(--text-1)', fontSize: 13 }}>
                        {wf.name as string}
                      </div>
                      {(wf.description as string) ? (
                        <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 2, maxWidth: 240, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
                          {wf.description as string}
                        </div>
                      ) : null}
                    </td>
                    <td style={TD}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <motion.span
                          initial={{ scale: 0.9 }}
                          animate={{ scale: 1 }}
                          transition={{ delay: index * 0.03 + 0.1, type: 'spring', damping: 20 }}
                          style={{
                            display: 'inline-flex', alignItems: 'center', gap: 4,
                            padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
                            background: st.bg, color: st.color, border: `1px solid ${st.color}28`,
                          }}
                        >
                          {isRunning && (
                            <span style={{
                              width: 6, height: 6, borderRadius: '50%',
                              background: '#f59e0b',
                              animation: 'pulse 1.5s ease-in-out infinite',
                            }} />
                          )}
                          {st.label}
                        </motion.span>
                        {isRunning && (wf.lastRunAt as string) ? (
                          <RunningTimer startedAt={wf.lastRunAt as string} />
                        ) : null}
                        {isFrozen && (
                          <span title="Plan Frozen" style={{ display: 'inline-flex', alignItems: 'center', gap: 2, padding: '2px 6px', borderRadius: 99, fontSize: 9, fontWeight: 700, background: '#06b6d412', color: '#06b6d4', border: '1px solid #06b6d428' }}>
                            <Snowflake size={8} /> FROZEN
                          </span>
                        )}
                      </div>
                    </td>
                    <td style={{ ...TD, maxWidth: 200 }}>
                      <div style={{ fontSize: 12, color: 'var(--text-2)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontStyle: goal ? 'normal' : 'italic' }}>
                        {goal ? goal.slice(0, 80) + (goal.length > 80 ? '...' : '') : 'No goal set'}
                      </div>
                    </td>
                    <td style={TD}>
                      <span style={{ fontSize: 11, color: 'var(--text-2)', fontWeight: 500 }}>
                        {timeAgo(wf.updatedAt as string)}
                      </span>
                    </td>
                    <td style={{ ...TD, textAlign: 'right', whiteSpace: 'nowrap' }} onClick={e => e.stopPropagation()}>
                      <div style={{ display: 'flex', gap: 3, justifyContent: 'flex-end', flexWrap: 'nowrap' }}>
                        {/* Fixed-width container for conditional Run/Stop/Restart */}
                        <div style={{ display: 'flex', gap: 3, width: 80, justifyContent: 'flex-end', flexShrink: 0 }}>
                        {canRun && (
                          <button onClick={() => handleRun(wf.id as string)} disabled={isStarting} style={{
                            display: 'flex', alignItems: 'center', gap: 4,
                            padding: '5px 10px', borderRadius: 5, fontSize: 11, fontWeight: 600,
                            border: '1px solid #10b98150',
                            background: '#10b98110', color: '#10b981',
                            cursor: isStarting ? 'wait' : 'pointer',
                            opacity: isStarting ? 0.5 : 1,
                          }}>
                            {isStarting ? <Loader2 size={10} style={{ animation: 'spin 1s linear infinite' }} /> : <Play size={10} />} Run
                          </button>
                        )}
                        {canStop && (
                          <button onClick={() => handleStop(wf.id as string)} style={{
                            display: 'flex', alignItems: 'center', gap: 4,
                            padding: '5px 10px', borderRadius: 5, fontSize: 11, fontWeight: 600,
                            border: '1px solid #ef444450',
                            background: '#ef444410', color: '#ef4444',
                            cursor: 'pointer',
                          }}>
                            <Square size={10} /> Stop
                          </button>
                        )}
                        {canRestart && (
                          <button onClick={() => handleRestart(wf.id as string)} style={{
                            display: 'flex', alignItems: 'center', gap: 4,
                            padding: '5px 8px', borderRadius: 5, fontSize: 11, fontWeight: 600,
                            border: '1px solid #f59e0b40',
                            background: '#f59e0b08', color: '#f59e0b',
                            cursor: 'pointer',
                          }}>
                            <RotateCcw size={10} />
                          </button>
                        )}
                        </div>
                        <button onClick={() => setPlanDialogWf(wf)} title="Edit Plan" style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          padding: '4px 6px', borderRadius: 5,
                          border: '1px solid #8b5cf640',
                          background: '#8b5cf608', color: '#8b5cf6',
                          cursor: 'pointer',
                        }}>
                          <FileText size={10} />
                        </button>
                        <button onClick={() => setViewPlanWf(wf)} title="View Plan" style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          padding: '4px 6px', borderRadius: 5,
                          border: '1px solid var(--border)',
                          background: 'transparent', color: 'var(--text-3)',
                          cursor: 'pointer',
                        }}>
                          <Eye size={10} />
                        </button>
                        <button
                          onClick={() => handleFreeze(wf.id as string)}
                          disabled={freezingId === (wf.id as string)}
                          title={isFrozen ? 'Unfreeze Plan' : 'Freeze Plan'}
                          style={{
                            display: 'flex', alignItems: 'center', justifyContent: 'center',
                            padding: '4px 6px', borderRadius: 5,
                            border: isFrozen ? '1px solid #06b6d450' : '1px solid var(--border)',
                            background: isFrozen ? '#06b6d412' : 'transparent',
                            color: isFrozen ? '#06b6d4' : 'var(--text-4)',
                            cursor: freezingId === (wf.id as string) ? 'wait' : 'pointer',
                          }}
                        >
                          {freezingId === (wf.id as string) ? (
                            <Loader2 size={10} style={{ animation: 'spin 1s linear infinite' }} />
                          ) : isFrozen ? (
                            <Lock size={10} />
                          ) : (
                            <Unlock size={10} />
                          )}
                        </button>
                        <Link href={`/workflow/builder?id=${wf.id}`} title="Edit" style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          padding: '4px 6px', borderRadius: 5,
                          border: `1px solid ${SECTION_COLOR}40`,
                          background: `${SECTION_COLOR}08`, color: SECTION_COLOR,
                          textDecoration: 'none',
                        }}>
                          <Pencil size={10} />
                        </Link>
                        <button onClick={() => exportEntity('workflow', wf.id as string, wf.name as string)} title="Export" style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          padding: '4px 6px', borderRadius: 5,
                          border: '1px solid var(--border)', background: 'transparent',
                          color: 'var(--text-4)', cursor: 'pointer',
                        }}>
                          <Download size={10} />
                        </button>
                        <button onClick={() => setOutputWf(wf)} title="View Outputs" style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          padding: '4px 6px', borderRadius: 5,
                          border: `1px solid ${SECTION_COLOR}40`,
                          background: `${SECTION_COLOR}08`, color: SECTION_COLOR,
                          cursor: 'pointer',
                        }}>
                          <FolderOpen size={10} />
                        </button>
                        <button onClick={() => setDeletingWf(wf)} title="Delete" style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          padding: '4px 6px', borderRadius: 5,
                          border: '1px solid var(--border)', background: 'transparent',
                          color: 'var(--text-4)', cursor: 'pointer',
                        }}>
                          <Trash2 size={10} />
                        </button>
                      </div>
                    </td>
                  </motion.tr>
                  {isRunning && ws.status === 'running' && (
                    <LiveExecutionPanel
                      agents={ws.agents}
                      liveMetrics={ws.liveMetrics}
                      events={ws.events}
                    />
                  )}
                  </React.Fragment>
                );
              })}
            </tbody>
          </table>
          <div style={{
            padding: '10px 14px', borderTop: '1px solid var(--border)',
            fontSize: 11, color: 'var(--text-4)', display: 'flex', justifyContent: 'space-between',
          }}>
            <span>Showing {filtered.length} of {total} workflows</span>
            <span>Sorted by {sortField} ({sortDir})</span>
          </div>
        </motion.div>
      )}

      {/* Pulse animation for running status */}
      <style>{`
        @keyframes pulse {
          0%, 100% { opacity: 1; }
          50% { opacity: 0.4; }
        }
      `}</style>

      {/* Modals */}
      <AnimatePresence>
        {deletingWf && (
          <DeleteConfirmDialog
            name={deletingWf.name as string}
            onClose={() => setDeletingWf(null)}
            onConfirm={() => handleDelete(deletingWf.id as string)}
          />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {planDialogWf && (
          <PlanDialog wf={planDialogWf} onClose={() => { setPlanDialogWf(null); mutate(); }} />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {viewPlanWf && (
          <ViewPlanDialog wf={viewPlanWf} onClose={() => setViewPlanWf(null)} />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {selectedWf && (
          <WorkflowDetailDialog
            wf={selectedWf}
            onClose={() => setSelectedWf(null)}
            onRun={handleRun}
            onStop={handleStop}
            onRestart={handleRestart}
            onDelete={handleDelete}
            onEdit={(id) => router.push(`/workflow/builder?id=${id}`)}
            runningWf={runningIds.has(selectedWf.id as string)}
          />
        )}
      </AnimatePresence>
      <AnimatePresence>
        {runError && (
          <motion.div
            initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
            onClick={() => setRunError(null)}
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
                maxWidth: 420, width: '90%', textAlign: 'center',
              }}
            >
              <div style={{
                width: 40, height: 40, borderRadius: 10,
                background: 'var(--error-dim, rgba(220,38,38,0.08))',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
                margin: '0 auto 14px',
              }}>
                <AlertCircle size={20} color="var(--error, #DC2626)" />
              </div>
              <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', marginBottom: 8 }}>
                Cannot Run Workflow
              </div>
              <div style={{ fontSize: 13, color: 'var(--text-2)', lineHeight: 1.5, marginBottom: 20 }}>
                {runError}
              </div>
              <button
                onClick={() => setRunError(null)}
                style={{
                  padding: '8px 24px', borderRadius: 7, fontSize: 12, fontWeight: 700,
                  border: '1px solid var(--border-md)', background: 'var(--bg-elevated)',
                  color: 'var(--text-1)', cursor: 'pointer',
                }}
              >
                Dismiss
              </button>
            </motion.div>
          </motion.div>
        )}
      </AnimatePresence>

      {/* Shared Config Import Dialog */}
      <SharedConfigImportDialog
        open={showSharedImport}
        onClose={() => setShowSharedImport(false)}
        onImported={() => { mutate(); setShowSharedImport(false); }}
        filterType="workflow"
      />

      {/* Workflow Output Dialog */}
      <WorkflowOutputDialog
        workflowName={(outputWf?.name as string) ?? ''}
        open={!!outputWf}
        onClose={() => setOutputWf(null)}
      />
    </div>
  );
}
