'use client';

import { useState, useEffect, useRef } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Cpu, Activity, Zap, Clock, RefreshCw,
  TrendingUp, ListOrdered, Loader2,
} from 'lucide-react';
import { useExpertRuns, useLiveMetrics, useExpertStats } from '@/lib/hooks/useApi';
import ExpertRunCard from './_components/ExpertRunCard';
import type { ExpertRun } from './_components/ExpertRunCard';
import ExpertRunDetailDialog from './_components/ExpertRunDetailDialog';
import TaskList from './_components/TaskList';

const SECTION_COLOR = '#D97706';

const stagger = { hidden: {}, show: { transition: { staggerChildren: 0.07 } } };

const RUN_FILTERS = ['all', 'running', 'queued', 'completed', 'failed'] as const;
type RunFilter = typeof RUN_FILTERS[number];

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function SkeletonCard() {
  return (
    <div style={{
      background: 'var(--bg-surface)',
      border: '1px solid var(--border)',
      borderRadius: 12, padding: 20,
      display: 'flex', flexDirection: 'column', gap: 14,
    }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 7 }}>
          <div className="skeleton" style={{ height: 13, width: '60%', borderRadius: 6 }} />
          <div className="skeleton" style={{ height: 10, width: '40%', borderRadius: 5 }} />
        </div>
        <div className="skeleton" style={{ width: 54, height: 20, borderRadius: 99 }} />
      </div>
      <div className="skeleton" style={{ height: 36, borderRadius: 8, width: '100%' }} />
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        {[0, 1, 2].map(i => (
          <div key={i} className="skeleton" style={{ height: 48, borderRadius: 7 }} />
        ))}
      </div>
    </div>
  );
}

type SectionTab = 'experts' | 'tasks';

export default function ExpertsAndTasksPage() {
  const [runFilter, setRunFilter] = useState<RunFilter>('all');
  const [section, setSection] = useState<SectionTab>('experts');
  const [selectedRun, setSelectedRun] = useState<ExpertRun | null>(null);

  const { runs, total, isLoading, mutate } = useExpertRuns();
  const { metrics: liveMetrics } = useLiveMetrics();
  const { experts: expertStats } = useExpertStats();

  // Cleanup stale runs on mount and every 60s
  const cleanupDone = useRef(false);
  useEffect(() => {
    const doCleanup = () => {
      fetch('/api/experts/run/cleanup', { method: 'POST' })
        .then(() => mutate())
        .catch(() => {});
    };
    if (!cleanupDone.current) { doCleanup(); cleanupDone.current = true; }
    const interval = setInterval(doCleanup, 60_000);
    return () => clearInterval(interval);
  }, [mutate]);

  const running   = runs.filter((r: ExpertRun) => r.status === 'running').length;
  const queued    = runs.filter((r: ExpertRun) => r.status === 'queued').length;
  const completed = runs.filter((r: ExpertRun) => r.status === 'completed').length;
  const failed    = runs.filter((r: ExpertRun) => r.status === 'failed').length;

  const totalTokens = runs.reduce(
    (sum: number, r: ExpertRun) => sum + (r.tokensUsed ?? 0), 0,
  );

  const filteredRuns = runFilter === 'all'
    ? runs
    : runs.filter((r: ExpertRun) => r.status === runFilter);

  const runCounts: Record<RunFilter, number> = {
    all:       runs.length,
    running,
    queued,
    completed,
    failed,
  };

  const [lastUpdated, setLastUpdated] = useState('--:--:--');
  useEffect(() => {
    const update = () => {
      const now = new Date();
      setLastUpdated(
        `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`,
      );
    };
    update();
    const interval = setInterval(update, 1000);
    return () => clearInterval(interval);
  }, []);

  return (
    <div style={{ padding: 28, maxWidth: 1200 }}>

      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        style={{
          display: 'flex', alignItems: 'center',
          justifyContent: 'space-between', marginBottom: 24,
        }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 38, height: 38, borderRadius: 9,
            background: `${SECTION_COLOR}15`,
            border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Cpu size={19} color={SECTION_COLOR} strokeWidth={2} />
          </div>
          <div>
            <div style={{ display: 'flex', alignItems: 'center', gap: 9 }}>
              <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>
                Experts & Tasks
              </h1>
              {running > 0 && (
                <div style={{
                  display: 'flex', alignItems: 'center', gap: 5,
                  padding: '3px 9px', borderRadius: 99,
                  background: '#3b82f618',
                  border: '1px solid #3b82f630',
                }}>
                  <div className="dot-pulse" style={{
                    width: 6, height: 6, borderRadius: '50%', background: '#3b82f6',
                  }} />
                  <span style={{ fontSize: 11, fontWeight: 700, color: '#3b82f6' }}>
                    {running} running
                  </span>
                </div>
              )}
            </div>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4 }}>
              {total} expert runs · auto-refreshes every 5s
            </p>
          </div>
        </div>

        <button
          onClick={() => mutate()}
          style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '8px 15px', borderRadius: 8, border: '1px solid var(--border-md)',
            background: 'var(--bg-surface)', cursor: 'pointer',
            fontSize: 12, fontWeight: 500, color: 'var(--text-2)',
            transition: 'all 0.15s',
          }}
        >
          <RefreshCw size={12} />
          Refresh
        </button>
      </motion.div>

      {/* Metric summary bar */}
      <motion.div
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ delay: 0.08 }}
        style={{
          display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)',
          gap: 10, marginBottom: 22,
        }}
      >
        {[
          { label: 'Running',     value: running,                           color: '#3b82f6', icon: Activity,   desc: 'active expert runs' },
          { label: 'Completed',   value: completed,                         color: '#10b981', icon: TrendingUp, desc: 'successful runs' },
          { label: 'Failed',      value: failed,                            color: '#ef4444', icon: Zap,        desc: 'errored runs' },
          { label: 'Total Tokens',value: totalTokens,                       color: '#f59e0b', icon: Clock,      desc: 'tokens consumed' },
        ].map(({ label, value, color, icon: Icon, desc }) => (
          <div key={label} style={{
            background: 'var(--bg-surface)',
            border: '1px solid var(--border)',
            borderRadius: 11, padding: '15px 18px',
            display: 'flex', alignItems: 'center', gap: 13,
          }}>
            <div style={{
              width: 36, height: 36, borderRadius: 8, flexShrink: 0,
              background: `${color}12`, border: `1.5px solid ${color}22`,
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <Icon size={16} color={color} strokeWidth={2} />
            </div>
            <div>
              <div style={{ fontSize: 22, fontWeight: 800, color, lineHeight: 1 }}>
                {fmt(value)}
              </div>
              <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', marginTop: 2 }}>
                {label}
              </div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1 }}>{desc}</div>
            </div>
          </div>
        ))}
      </motion.div>

      {/* Section toggle */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.12 }}
        style={{
          display: 'flex', gap: 4, marginBottom: 22,
          padding: 3, borderRadius: 10,
          background: 'var(--bg-elevated)',
          border: '1px solid var(--border)',
          width: 'fit-content',
        }}
      >
        {([
          { key: 'experts' as SectionTab, label: 'Expert Runs',  icon: Cpu },
          { key: 'tasks' as SectionTab,   label: 'Task Queue',   icon: ListOrdered },
        ]).map(({ key, label, icon: Icon }) => (
          <button
            key={key}
            onClick={() => setSection(key)}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 18px', borderRadius: 8, fontSize: 13, cursor: 'pointer',
              border: 'none',
              background: section === key ? 'var(--bg-surface)' : 'transparent',
              color: section === key ? 'var(--text-1)' : 'var(--text-3)',
              fontWeight: section === key ? 700 : 400,
              boxShadow: section === key ? '0 1px 4px rgba(0,0,0,0.06)' : 'none',
              transition: 'all 0.15s',
            }}
          >
            <Icon size={14} />
            {label}
          </button>
        ))}
      </motion.div>

      {/* Section content */}
      <AnimatePresence mode="wait">
        {section === 'experts' ? (
          <motion.div
            key="experts"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.2 }}
          >
            {/* Run filter tabs */}
            <div style={{ display: 'flex', gap: 6, marginBottom: 20 }}>
              {RUN_FILTERS.map(t => (
                <button
                  key={t}
                  onClick={() => setRunFilter(t)}
                  style={{
                    padding: '6px 14px', borderRadius: 99, fontSize: 12, cursor: 'pointer',
                    border: runFilter === t ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
                    background: runFilter === t ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
                    color: runFilter === t ? SECTION_COLOR : 'var(--text-3)',
                    fontWeight: runFilter === t ? 700 : 400,
                    transition: 'all 0.15s',
                  }}
                >
                  {t.charAt(0).toUpperCase() + t.slice(1)}
                  <span style={{ marginLeft: 5, fontSize: 10, opacity: 0.75 }}>
                    ({runCounts[t]})
                  </span>
                </button>
              ))}
            </div>

            {/* Expert runs grid */}
            {isLoading ? (
              <div style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
                gap: 16,
              }}>
                {[0, 1, 2].map(i => <SkeletonCard key={i} />)}
              </div>
            ) : (
              <AnimatePresence mode="wait">
                {filteredRuns.length === 0 ? (
                  <motion.div
                    key="empty"
                    initial={{ opacity: 0, y: 12 }}
                    animate={{ opacity: 1, y: 0 }}
                    exit={{ opacity: 0 }}
                    style={{
                      textAlign: 'center', padding: '80px 0',
                      display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 12,
                    }}
                  >
                    <div style={{
                      width: 52, height: 52, borderRadius: 12,
                      background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                      display: 'flex', alignItems: 'center', justifyContent: 'center',
                    }}>
                      <Cpu size={22} color="var(--text-4)" />
                    </div>
                    <div style={{ fontSize: 15, fontWeight: 600, color: 'var(--text-2)' }}>
                      No expert runs found
                    </div>
                    <div style={{ fontSize: 12, color: 'var(--text-4)', maxWidth: 320 }}>
                      {runFilter === 'all'
                        ? 'Run an expert from the Experts page to see runs here.'
                        : `No ${runFilter} runs. Switch tabs to see other runs.`}
                    </div>
                  </motion.div>
                ) : (
                  <motion.div
                    key={runFilter}
                    variants={stagger}
                    initial="hidden"
                    animate="show"
                    style={{
                      display: 'grid',
                      gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
                      gap: 16,
                    }}
                  >
                    {filteredRuns.map((run: ExpertRun) => (
                      <ExpertRunCard
                        key={run.id}
                        run={run}
                        onClick={() => setSelectedRun(run)}
                      />
                    ))}
                  </motion.div>
                )}
              </AnimatePresence>
            )}
          </motion.div>
        ) : (
          <motion.div
            key="tasks"
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ duration: 0.2 }}
          >
            <TaskList />
          </motion.div>
        )}
      </AnimatePresence>

      {/* Footer */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.5 }}
        style={{
          marginTop: 28, display: 'flex', alignItems: 'center', gap: 8,
          color: 'var(--text-4)', fontSize: 11,
        }}
      >
        <div className="dot-pulse" style={{
          width: 6, height: 6, borderRadius: '50%', background: SECTION_COLOR,
        }} />
        Live · Auto-refreshes every 5 seconds · Last updated {lastUpdated}
      </motion.div>

      {/* Expert Run Detail Dialog */}
      <ExpertRunDetailDialog
        run={selectedRun}
        open={!!selectedRun}
        onClose={() => setSelectedRun(null)}
      />
    </div>
  );
}
