'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  X, CheckCircle2, XCircle, Loader2, Clock, Zap,
  AlertCircle, FileText, Play, Ban, ScrollText,
  ChevronDown, ChevronUp, BarChart2,
} from 'lucide-react';
import useSWR from 'swr';
import type { WorkflowRun, WorkflowStep } from '@/lib/types';

const fetcher = (url: string) => fetch(url).then(r => r.json());
const SECTION_COLOR = '#06b6d4';

type RunStatus = 'completed' | 'failed' | 'running' | 'cancelled';

const STATUS_META: Record<RunStatus, { color: string; icon: typeof CheckCircle2; label: string }> = {
  completed: { color: '#10b981', icon: CheckCircle2, label: 'Completed' },
  failed:    { color: '#ef4444', icon: XCircle,      label: 'Failed' },
  running:   { color: '#3b82f6', icon: Loader2,      label: 'Running' },
  cancelled: { color: '#6b7280', icon: Ban,           label: 'Cancelled' },
};

type MetaLevel = 'critical' | 'important' | 'recommended';
const LEVEL_COLORS: Record<MetaLevel, { border: string; bg: string; accent: string }> = {
  critical:    { border: '#ef444440', bg: '#ef444408', accent: '#ef4444' },
  important:   { border: '#f59e0b40', bg: '#f59e0b08', accent: '#f59e0b' },
  recommended: { border: '#3b82f640', bg: '#3b82f608', accent: '#3b82f6' },
};

function fmtTokens(n: number): string {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function fmtDuration(secs: number): string {
  if (!secs) return '—';
  if (secs < 60) return `${secs}s`;
  const m = Math.floor(secs / 60);
  const s = secs % 60;
  return s > 0 ? `${m}m ${s}s` : `${m}m`;
}

type TabKey = 'steps' | 'output' | 'logs';

interface RunDetailDialogProps {
  run: WorkflowRun | null;
  open: boolean;
  onClose: () => void;
}

export default function RunDetailDialog({ run, open, onClose }: RunDetailDialogProps) {
  const [tab, setTab] = useState<TabKey>('steps');
  const [logsExpanded, setLogsExpanded] = useState(false);
  const [logs, setLogs] = useState<Array<{ timestamp: string; level: string; message: string }>>([]);
  const [logsLoading, setLogsLoading] = useState(false);

  // Reactive polling for running workflows
  const isRunning = run?.status === 'running';
  const { data: liveData } = useSWR(
    isRunning && run ? `/api/workflows/runs?workflowId=${run.workflowId}&limit=1` : null,
    fetcher,
    { refreshInterval: 3000 },
  );

  // Merge live data if available
  const liveRun = liveData?.runs?.[0];
  const displayRun = (isRunning && liveRun?.id === run?.id) ? { ...run, ...liveRun } : run;

  // Fetch step executions
  const { data: execData } = useSWR(
    open && run ? `/api/workflows/executions?runId=${run.id}` : null,
    fetcher,
    { refreshInterval: isRunning ? 3000 : 0 },
  );
  const stepExecs = execData?.executions ?? [];

  // Live duration counter
  const [elapsed, setElapsed] = useState(0);
  useEffect(() => {
    if (!isRunning || !displayRun?.startedAt) return;
    const tick = () => setElapsed(Math.floor((Date.now() - new Date(displayRun.startedAt).getTime()) / 1000));
    tick();
    const iv = setInterval(tick, 1000);
    return () => clearInterval(iv);
  }, [isRunning, displayRun?.startedAt]);

  // Fetch logs
  const fetchLogs = async () => {
    if (!run) return;
    setLogsLoading(true);
    try {
      const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';
      const res = await fetch(`${ENGINE_URL}/api/logs/run/${run.workflowId}/${run.id}`);
      if (res.ok) {
        const data = await res.json();
        if (typeof data.log === 'string' && data.log.trim()) {
          const parsed = data.log.split('\n').filter(Boolean).map((line: string) => {
            const match = line.match(/^\[(.+?)\]\s+(\w+)\s+(.+)$/);
            if (match) return { timestamp: match[1], level: match[2], message: match[3] };
            return { timestamp: '', level: 'info', message: line };
          });
          setLogs(parsed);
        }
      }
    } catch { /* ignore */ }
    setLogsLoading(false);
  };

  useEffect(() => {
    if (open && tab === 'logs' && logs.length === 0) fetchLogs();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, tab]);

  // Reset on close
  useEffect(() => {
    if (!open) { setTab('steps'); setLogs([]); setLogsExpanded(false); }
  }, [open]);

  if (!open || !run || !displayRun) return null;

  const st = STATUS_META[(displayRun.status as RunStatus)] ?? STATUS_META.running;
  const StatusIcon = st.icon;
  const duration = isRunning ? elapsed : (displayRun.durationSec ?? 0);
  const stepsCompleted = stepExecs.filter((s: Record<string, unknown>) => s.status === 'completed').length;
  const totalSteps = displayRun.steps?.length ?? stepExecs.length ?? 0;

  // Metadata cards organized by importance
  const metaCards: Array<{ label: string; value: string; level: MetaLevel; show: boolean }> = [
    { label: 'Status', value: st.label, level: 'critical', show: true },
    { label: 'Duration', value: fmtDuration(duration), level: 'critical', show: true },
    { label: 'Error', value: displayRun.error?.slice(0, 100) ?? '', level: 'critical', show: !!displayRun.error },
    { label: 'Total Tokens', value: fmtTokens(displayRun.totalTokensUsed ?? 0), level: 'important', show: true },
    { label: 'Cost', value: `$${(Number(displayRun.totalCostUsd) || 0).toFixed(4)}`, level: 'important', show: true },
    { label: 'Steps', value: `${stepsCompleted}/${totalSteps}`, level: 'important', show: totalSteps > 0 },
    { label: 'Expert Chain', value: (displayRun.expertChain ?? []).join(' → ') || '—', level: 'important', show: true },
    { label: 'Started', value: displayRun.startedAt ? new Date(displayRun.startedAt).toLocaleString() : '—', level: 'recommended', show: true },
    { label: 'Completed', value: displayRun.completedAt ? new Date(displayRun.completedAt).toLocaleString() : '—', level: 'recommended', show: !!displayRun.completedAt },
    { label: 'Plan ID', value: displayRun.planId ?? '—', level: 'recommended', show: !!displayRun.planId },
    { label: 'Run ID', value: displayRun.id, level: 'recommended', show: true },
  ];

  const visibleCards = metaCards.filter(c => c.show);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
          style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
          }}
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 20 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={e => e.stopPropagation()}
            style={{
              background: 'var(--bg-surface)', border: '1px solid var(--border)',
              borderRadius: 16, width: '100%', maxWidth: 800,
              maxHeight: '90vh', display: 'flex', flexDirection: 'column',
              overflow: 'hidden',
            }}
          >
            {/* Header */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '18px 24px', borderBottom: '1px solid var(--border)',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <StatusIcon size={20} color={st.color} className={isRunning ? 'spin' : ''} />
                <div>
                  <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
                    {displayRun.workflowName}
                  </div>
                  <div style={{ fontSize: 11, color: 'var(--text-4)', fontFamily: 'monospace', marginTop: 2 }}>
                    {displayRun.id}
                  </div>
                </div>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                <div style={{
                  padding: '4px 12px', borderRadius: 99, fontSize: 12, fontWeight: 700,
                  background: st.color + '15', color: st.color,
                  border: `1px solid ${st.color}30`,
                }}>
                  {st.label}
                  {isRunning && ` · ${fmtDuration(elapsed)}`}
                </div>
                <button onClick={onClose} style={{
                  width: 30, height: 30, borderRadius: 8, border: '1px solid var(--border)',
                  background: 'transparent', cursor: 'pointer', color: 'var(--text-3)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <X size={14} />
                </button>
              </div>
            </div>

            {/* Metadata cards */}
            <div style={{
              padding: '14px 24px', borderBottom: '1px solid var(--border)',
              display: 'flex', flexWrap: 'wrap', gap: 8,
            }}>
              {visibleCards.map(card => {
                const lc = LEVEL_COLORS[card.level];
                return (
                  <div key={card.label} style={{
                    padding: '6px 12px', borderRadius: 8,
                    border: `1px solid ${lc.border}`,
                    background: lc.bg,
                    minWidth: card.label === 'Error' || card.label === 'Expert Chain' ? '100%' : undefined,
                  }}>
                    <div style={{ fontSize: 9, fontWeight: 700, color: lc.accent, textTransform: 'uppercase', letterSpacing: '0.05em' }}>
                      {card.label}
                    </div>
                    <div style={{
                      fontSize: 12, fontWeight: 600, color: 'var(--text-1)', marginTop: 2,
                      fontFamily: card.label === 'Run ID' || card.label === 'Plan ID' ? 'monospace' : undefined,
                      wordBreak: 'break-all',
                    }}>
                      {card.value}
                    </div>
                  </div>
                );
              })}
            </div>

            {/* Tabs */}
            <div style={{
              display: 'flex', gap: 2, padding: '0 24px',
              borderBottom: '1px solid var(--border)',
            }}>
              {([
                { key: 'steps' as TabKey, label: 'Steps', icon: BarChart2 },
                { key: 'output' as TabKey, label: 'Output', icon: FileText },
                { key: 'logs' as TabKey, label: 'Logs', icon: ScrollText },
              ]).map(({ key, label, icon: Icon }) => (
                <button
                  key={key}
                  onClick={() => setTab(key)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 6,
                    padding: '10px 14px', fontSize: 12, cursor: 'pointer',
                    border: 'none', background: 'transparent',
                    color: tab === key ? 'var(--text-1)' : 'var(--text-3)',
                    fontWeight: tab === key ? 700 : 400,
                    borderBottom: tab === key ? `2px solid ${SECTION_COLOR}` : '2px solid transparent',
                    transition: 'all 0.15s',
                  }}
                >
                  <Icon size={13} />
                  {label}
                </button>
              ))}
            </div>

            {/* Content */}
            <div style={{ flex: 1, overflow: 'auto', padding: '16px 24px' }}>
              {/* Steps tab */}
              {tab === 'steps' && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
                  {stepExecs.length === 0 && (
                    <div style={{ padding: 20, textAlign: 'center', color: 'var(--text-4)', fontSize: 13 }}>
                      {isRunning ? 'Waiting for step executions...' : 'No step execution data available.'}
                    </div>
                  )}
                  {stepExecs.map((step: Record<string, unknown>, i: number) => {
                    const stepSt = STATUS_META[(step.status as RunStatus)] ?? STATUS_META.running;
                    const StepIcon = stepSt.icon;
                    return (
                      <div key={step.id as string ?? i} style={{
                        padding: '12px 14px', borderRadius: 10,
                        border: `1px solid ${stepSt.color}25`,
                        background: `${stepSt.color}06`,
                      }}>
                        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                            <StepIcon size={13} color={stepSt.color} className={step.status === 'running' ? 'spin' : ''} />
                            <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
                              {step.stepName as string ?? `Step ${i + 1}`}
                            </span>
                          </div>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                            {(step.tokensUsed as number) > 0 && (
                              <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                                <Zap size={10} style={{ display: 'inline', verticalAlign: 'middle' }} /> {fmtTokens(step.tokensUsed as number)}
                              </span>
                            )}
                            {(step.durationMs as number) > 0 && (
                              <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                                <Clock size={10} style={{ display: 'inline', verticalAlign: 'middle' }} /> {((step.durationMs as number) / 1000).toFixed(1)}s
                              </span>
                            )}
                          </div>
                        </div>
                        {(step.model as string) && (
                          <div style={{ fontSize: 11, color: 'var(--text-4)', marginTop: 4 }}>
                            Model: {step.model as string} · Engine: {step.engine as string ?? 'ollama'}
                          </div>
                        )}
                        {(step.responsePreview as string) && (
                          <div style={{
                            marginTop: 8, padding: '8px 10px', borderRadius: 6,
                            background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                            fontSize: 11, color: 'var(--text-2)', lineHeight: 1.5,
                            maxHeight: 100, overflow: 'auto', whiteSpace: 'pre-wrap',
                          }}>
                            {step.responsePreview as string}
                          </div>
                        )}
                        {(step.errorMessage as string) && (
                          <div style={{
                            marginTop: 6, padding: '6px 10px', borderRadius: 6,
                            background: '#ef444410', border: '1px solid #ef444420',
                            fontSize: 11, color: '#ef4444',
                          }}>
                            {step.errorMessage as string}
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              )}

              {/* Output tab */}
              {tab === 'output' && (
                <div>
                  {displayRun.output ? (
                    <pre style={{
                      margin: 0, padding: 16, borderRadius: 10,
                      background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                      fontSize: 12, color: 'var(--text-2)', lineHeight: 1.6,
                      whiteSpace: 'pre-wrap', wordBreak: 'break-word',
                      maxHeight: 500, overflow: 'auto',
                    }}>
                      {displayRun.output}
                    </pre>
                  ) : (
                    <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-4)', fontSize: 13 }}>
                      {isRunning ? 'Output will appear when the workflow completes...' : 'No output available for this run.'}
                    </div>
                  )}
                </div>
              )}

              {/* Logs tab */}
              {tab === 'logs' && (
                <div style={{
                  padding: '10px 12px', borderRadius: 8,
                  background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                  maxHeight: 400, overflow: 'auto',
                  fontFamily: 'monospace', fontSize: 11, lineHeight: 1.6,
                }}>
                  {logsLoading && (
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6, color: 'var(--text-4)', padding: '8px 0' }}>
                      <Loader2 size={12} className="spin" /> Loading logs...
                    </div>
                  )}
                  {!logsLoading && logs.length === 0 && (
                    <div style={{ color: 'var(--text-4)', fontStyle: 'italic', padding: '8px 0' }}>
                      No logs available.
                    </div>
                  )}
                  {logs.map((log, i) => (
                    <div key={i} style={{ padding: '2px 0', borderBottom: i < logs.length - 1 ? '1px solid var(--border)' : 'none' }}>
                      <span style={{ color: 'var(--text-4)', marginRight: 8 }}>
                        {log.timestamp ? (() => { try { return new Date(log.timestamp).toLocaleTimeString(); } catch { return log.timestamp; } })() : '—'}
                      </span>
                      <span style={{
                        color: log.level.toLowerCase() === 'error' ? '#ef4444'
                          : log.level.toLowerCase().startsWith('warn') ? '#f59e0b'
                          : '#3b82f6',
                        fontWeight: 600, marginRight: 8,
                      }}>
                        {log.level.toUpperCase()}
                      </span>
                      <span style={{ color: 'var(--text-2)' }}>{log.message}</span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}
