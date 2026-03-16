'use client';

import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  ListOrdered, Search, RefreshCw, X, Play, Clock,
  AlertTriangle, CheckCircle2, Loader2, ChevronDown,
} from 'lucide-react';
import { useTasks } from '@/lib/hooks/useApi';
import type { QueuedTask } from '@/lib/types';

const SECTION_COLOR = '#3b82f6';

const fadeUp = {
  hidden: { opacity: 0, y: 12 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};
const stagger = { hidden: {}, show: { transition: { staggerChildren: 0.06 } } };

const STATUS_FILTERS = ['all', 'queued', 'running', 'completed', 'failed', 'cancelled'];
const PRIORITY_COLOR: Record<string, string> = {
  critical: '#ef4444',
  high:     '#f59e0b',
  normal:   '#3b82f6',
  low:      '#6b7280',
};

function StatusIcon({ status }: { status: string }) {
  switch (status) {
    case 'running':   return <Loader2 size={13} className="spin" color="#3b82f6" />;
    case 'completed': return <CheckCircle2 size={13} color="#10b981" />;
    case 'failed':    return <X size={13} color="#ef4444" />;
    case 'queued':    return <Clock size={13} color="#f59e0b" />;
    default:          return <Clock size={13} color="var(--text-4)" />;
  }
}

function elapsed(iso: string) {
  const d = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (d < 60) return `${d}s`;
  if (d < 3600) return `${Math.floor(d / 60)}m`;
  return `${Math.floor(d / 3600)}h`;
}

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

export default function TasksPage() {
  const [statusFilter, setStatusFilter] = useState('all');
  const [search, setSearch] = useState('');

  const { tasks, total, isLoading, mutate } = useTasks(
    statusFilter === 'all' ? undefined : statusFilter, 100,
  );

  const filtered = tasks.filter((t: QueuedTask) =>
    !search || t.name.toLowerCase().includes(search.toLowerCase()),
  );

  const counts = STATUS_FILTERS.reduce((acc: Record<string, number>, s) => {
    acc[s] = s === 'all' ? tasks.length : tasks.filter((t: QueuedTask) => t.status === s).length;
    return acc;
  }, {});

  return (
    <div style={{ padding: 28, maxWidth: 1100 }}>
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 36, height: 36, borderRadius: 8,
            background: `${SECTION_COLOR}18`,
            border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <ListOrdered size={18} color={SECTION_COLOR} strokeWidth={2} />
          </div>
          <div>
            <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>
              Task Queue
            </h1>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 3 }}>
              {total} total · {counts.running ?? 0} running · {counts.queued ?? 0} queued
            </p>
          </div>
        </div>
        <button
          onClick={() => mutate()}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '7px 14px', borderRadius: 7, border: '1px solid var(--border-md)',
            background: 'var(--bg-surface)', cursor: 'pointer',
            fontSize: 12, color: 'var(--text-2)',
          }}
        >
          <RefreshCw size={12} />
          Refresh
        </button>
      </motion.div>

      {/* Filters */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.1 }}
        style={{ display: 'flex', gap: 8, flexWrap: 'wrap', marginBottom: 18, alignItems: 'center' }}
      >
        {STATUS_FILTERS.map(s => (
          <button
            key={s}
            onClick={() => setStatusFilter(s)}
            style={{
              padding: '5px 12px', borderRadius: 20, fontSize: 12, cursor: 'pointer',
              border: statusFilter === s ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
              background: statusFilter === s ? `${SECTION_COLOR}15` : 'var(--bg-surface)',
              color: statusFilter === s ? SECTION_COLOR : 'var(--text-3)',
              fontWeight: statusFilter === s ? 600 : 400,
              transition: 'all 0.15s',
            }}
          >
            {s.charAt(0).toUpperCase() + s.slice(1)}
            {counts[s] > 0 && (
              <span style={{ marginLeft: 5, fontSize: 10, opacity: 0.7 }}>({counts[s]})</span>
            )}
          </button>
        ))}
        <div style={{
          marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8,
          padding: '6px 12px', borderRadius: 8, border: '1px solid var(--border-md)',
          background: 'var(--bg-surface)',
        }}>
          <Search size={13} color="var(--text-4)" />
          <input
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder="Search tasks…"
            style={{
              border: 'none', outline: 'none', background: 'transparent',
              fontSize: 13, color: 'var(--text-1)', width: 180,
            }}
          />
        </div>
      </motion.div>

      {/* Task list */}
      {isLoading ? (
        <div style={{ textAlign: 'center', padding: '60px 0', color: 'var(--text-4)' }}>
          <Loader2 size={22} className="spin" style={{ margin: '0 auto 8px' }} />
          <div style={{ fontSize: 13 }}>Loading tasks…</div>
        </div>
      ) : (
        <motion.div variants={stagger} initial="hidden" animate="show"
          style={{ display: 'flex', flexDirection: 'column', gap: 6 }}
        >
          <AnimatePresence>
            {filtered.length === 0 ? (
              <motion.div variants={fadeUp}
                style={{
                  textAlign: 'center', padding: '60px 0',
                  color: 'var(--text-4)', fontSize: 14,
                }}
              >
                No tasks match the current filter.
              </motion.div>
            ) : (
              filtered.map((task: QueuedTask) => (
                <motion.div
                  key={task.id as string}
                  variants={fadeUp}
                  layout
                  style={{
                    background: 'var(--bg-surface)',
                    border: '1px solid var(--border-sm)',
                    borderRadius: 10,
                    padding: '14px 18px',
                    display: 'grid',
                    gridTemplateColumns: '1fr auto',
                    gap: 12,
                    alignItems: 'center',
                  }}
                >
                  <div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 5 }}>
                      <StatusIcon status={task.status as string} />
                      <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
                        {task.name as string}
                      </span>
                      <span style={{
                        padding: '1px 7px', borderRadius: 4, fontSize: 10, fontWeight: 700,
                        color: PRIORITY_COLOR[(task.priority as string) ?? 'normal'] ?? '#6b7280',
                        background: `${PRIORITY_COLOR[(task.priority as string) ?? 'normal']}15`,
                        border: `1px solid ${PRIORITY_COLOR[(task.priority as string) ?? 'normal']}30`,
                      }}>
                        {String(task.priority ?? 'normal').toUpperCase()}
                      </span>
                    </div>

                    <div style={{ display: 'flex', gap: 16, alignItems: 'center' }}>
                      {task.workflowName && (
                        <span style={{ fontSize: 11, color: 'var(--text-3)' }}>
                          {task.workflowName as string}
                        </span>
                      )}
                      {task.currentExpert && (
                        <span style={{ fontSize: 11, color: SECTION_COLOR }}>
                          via {task.currentExpert as string}
                        </span>
                      )}
                      <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                        Step {task.currentStep as number}/{task.totalSteps as number}
                      </span>
                      {task.tokensUsed as number > 0 && (
                        <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                          {fmt(task.tokensUsed as number)} tokens
                        </span>
                      )}
                      {task.createdAt && (
                        <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                          {elapsed(task.createdAt as string)} ago
                        </span>
                      )}
                    </div>

                    {/* Progress bar */}
                    {(task.status === 'running' || (task.progress as number) > 0) && (
                      <div style={{ marginTop: 8 }}>
                        <div style={{
                          height: 3, background: 'var(--border-sm)', borderRadius: 99, overflow: 'hidden',
                        }}>
                          <motion.div
                            initial={{ width: 0 }}
                            animate={{ width: `${task.progress ?? 0}%` }}
                            transition={{ duration: 0.8, ease: 'easeOut' }}
                            style={{ height: '100%', background: SECTION_COLOR, borderRadius: 99 }}
                          />
                        </div>
                        <span style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2, display: 'block' }}>
                          {task.progress as number}% complete
                        </span>
                      </div>
                    )}
                  </div>

                  <div style={{ display: 'flex', gap: 6 }}>
                    {task.status === 'queued' && (
                      <button style={{
                        display: 'flex', alignItems: 'center', gap: 4,
                        padding: '5px 10px', borderRadius: 6,
                        border: `1px solid ${SECTION_COLOR}40`,
                        background: `${SECTION_COLOR}10`,
                        color: SECTION_COLOR, fontSize: 11, cursor: 'pointer',
                      }}>
                        <Play size={10} /> Run
                      </button>
                    )}
                    {(task.status === 'queued' || task.status === 'running') && (
                      <button style={{
                        padding: '5px 10px', borderRadius: 6,
                        border: '1px solid var(--border-md)',
                        background: 'transparent',
                        color: 'var(--text-3)', fontSize: 11, cursor: 'pointer',
                      }}>
                        Cancel
                      </button>
                    )}
                    {task.status === 'failed' && (
                      <button style={{
                        display: 'flex', alignItems: 'center', gap: 4,
                        padding: '5px 10px', borderRadius: 6,
                        border: '1px solid var(--border-md)',
                        background: 'transparent',
                        color: 'var(--text-3)', fontSize: 11, cursor: 'pointer',
                      }}>
                        <RefreshCw size={10} /> Retry
                      </button>
                    )}
                    <button style={{
                      padding: '5px 8px', borderRadius: 6,
                      border: '1px solid var(--border-md)',
                      background: 'transparent',
                      color: 'var(--text-4)', fontSize: 11, cursor: 'pointer',
                    }}>
                      <ChevronDown size={12} />
                    </button>
                  </div>
                </motion.div>
              ))
            )}
          </AnimatePresence>
        </motion.div>
      )}

      {/* Empty-state helper */}
      {!isLoading && tasks.length === 0 && (
        <motion.div initial={{ opacity: 0, y: 12 }} animate={{ opacity: 1, y: 0 }}
          style={{
            marginTop: 24, padding: 28, borderRadius: 12,
            border: `1.5px dashed ${SECTION_COLOR}30`,
            background: `${SECTION_COLOR}08`, textAlign: 'center',
          }}
        >
          <AlertTriangle size={22} color={SECTION_COLOR} style={{ margin: '0 auto 8px' }} />
          <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>
            Queue is empty
          </div>
          <div style={{ fontSize: 12, color: 'var(--text-4)' }}>
            Run a workflow from the Builder or Dashboard to queue tasks.
          </div>
        </motion.div>
      )}
    </div>
  );
}
