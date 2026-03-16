'use client';

import Link from 'next/link';
import { motion } from 'framer-motion';
import useSWR from 'swr';
import {
  Cpu, CheckCircle2, TrendingUp, Zap, ArrowRight,
  Circle, ChevronRight, Play, BarChart3, Users,
  Workflow, Activity, X,
} from 'lucide-react';
import { useMetrics, useTasks, useExperts, useWorkflowRuns } from '@/lib/hooks/useApi';
import type { QueuedTask, WorkflowRun, Expert, AIProvider } from '@/lib/types';

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

/* ── Animation variants ────────────────────────────────── */
const fadeUp = {
  hidden: { opacity: 0, y: 16 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.38, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};

const stagger = (delay = 0.07) => ({
  hidden: {},
  show:   { transition: { staggerChildren: delay } },
});

/* ── Helpers ───────────────────────────────────────────── */
function fmt(n: number | null | undefined): string {
  if (n == null) return '\u2014';
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function v(n: number | null | undefined, fallback = '\u2014'): string {
  return n != null ? String(n) : fallback;
}

function elapsed(iso: string): string {
  const diff = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (diff < 60) return `${diff}s ago`;
  if (diff < 3600) return `${Math.floor(diff / 60)}m ago`;
  return `${Math.floor(diff / 3600)}h ago`;
}

function priorityLabel(p: string) {
  const map: Record<string, string> = {
    critical: 'CRITICAL',
    high:     'HIGH',
    normal:   'NORMAL',
    low:      'LOW',
  };
  return map[p] ?? p.toUpperCase();
}

/* ── Skeleton Shimmer ─────────────────────────────────── */
function Skeleton({ width = '100%', height = 16 }: { width?: string | number; height?: number }) {
  return (
    <div
      style={{
        width,
        height,
        borderRadius: 4,
        background: 'linear-gradient(90deg, var(--bg-elevated) 25%, var(--border) 50%, var(--bg-elevated) 75%)',
        backgroundSize: '200% 100%',
        animation: 'shimmer 1.5s ease-in-out infinite',
      }}
    />
  );
}

function SkeletonCard() {
  return (
    <motion.div variants={fadeUp} className="metric-card" style={{ cursor: 'default' }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div style={{ flex: 1 }}>
          <Skeleton width={80} height={28} />
          <div style={{ marginTop: 8 }}><Skeleton width={100} height={12} /></div>
          <div style={{ marginTop: 6 }}><Skeleton width={120} height={10} /></div>
        </div>
        <Skeleton width={34} height={34} />
      </div>
    </motion.div>
  );
}

function SkeletonRow() {
  return (
    <div style={{ padding: '10px 16px', borderBottom: '1px solid var(--border)' }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <Skeleton width={8} height={8} />
        <div style={{ flex: 1 }}>
          <Skeleton width="70%" height={13} />
          <div style={{ marginTop: 4 }}><Skeleton width="50%" height={10} /></div>
        </div>
        <Skeleton width={40} height={12} />
      </div>
    </div>
  );
}

/* ── Metric Card ───────────────────────────────────────── */
function MetricCard({
  label, value, sub, icon: Icon,
}: {
  label: string; value: string; sub?: string; icon: React.ElementType;
}) {
  return (
    <motion.div
      variants={fadeUp}
      whileHover={{ y: -2, boxShadow: '0 8px 28px rgba(13,13,13,0.10)' }}
      transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      className="metric-card"
      style={{ cursor: 'default' }}
    >
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div>
          <div className="metric-value">{value}</div>
          <div className="metric-label">{label}</div>
          {sub && (
            <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 5 }}>{sub}</div>
          )}
        </div>
        <div style={{
          width: 34, height: 34, borderRadius: 6,
          background: 'var(--primary-dim)',
          border: '1px solid var(--border-md)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
        }}>
          <Icon size={15} color="var(--primary)" strokeWidth={2} />
        </div>
      </div>
    </motion.div>
  );
}

/* ── Task Row ───────────────────────────────────────────── */
function TaskRow({ task, index }: { task: QueuedTask; index: number }) {
  const isRunning = task.status === 'running';
  return (
    <motion.div
      initial={{ opacity: 0, x: -10 }}
      animate={{ opacity: 1, x: 0 }}
      transition={{ delay: index * 0.06 + 0.2, duration: 0.3, ease: 'easeOut' }}
      style={{
        padding: '10px 16px',
        borderBottom: '1px solid var(--border)',
        background: isRunning ? 'rgba(13,13,13,0.025)' : 'transparent',
      }}
    >
      <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
        <span style={{ flexShrink: 0 }}>
          {isRunning
            ? <span className="status-dot dot-online dot-pulse" />
            : <span className="status-dot dot-offline" />}
        </span>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{
            fontSize: 13, fontWeight: 500, color: 'var(--text-1)',
            whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
          }}>
            {task.name}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginTop: 3 }}>
            {task.currentExpert && (
              <span style={{ fontSize: 11, color: 'var(--text-2)' }}>→ {task.currentExpert}</span>
            )}
            {task.workflowName && (
              <span style={{ fontSize: 11, color: 'var(--text-3)' }}>{task.workflowName}</span>
            )}
            <span style={{
              fontSize: 10, color: 'var(--text-3)',
              textTransform: 'uppercase', fontWeight: 600, letterSpacing: '0.06em',
              background: 'var(--bg-elevated)',
              padding: '1px 5px', borderRadius: 3,
            }}>
              {priorityLabel(task.priority)}
            </span>
          </div>
        </div>
        <div style={{ textAlign: 'right', flexShrink: 0 }}>
          <div className="mono" style={{ fontSize: 11, color: 'var(--text-2)' }}>
            {task.currentStep}/{task.totalSteps}
          </div>
          <div className="mono" style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 1 }}>
            {fmt(task.tokensUsed)} tok
          </div>
        </div>
      </div>
      {isRunning && (
        <div style={{ marginTop: 8 }}>
          <div className="progress-track">
            <motion.div
              className="progress-fill"
              initial={{ width: 0 }}
              animate={{ width: `${task.progress}%` }}
              transition={{ duration: 1, ease: 'easeOut', delay: 0.4 }}
            />
          </div>
          <div style={{
            display: 'flex', justifyContent: 'space-between',
            fontSize: 10, color: 'var(--text-3)', marginTop: 3,
          }}>
            <span className="mono">{task.progress}% complete</span>
            <span className="mono">{fmt(task.estimatedTokens)} est.</span>
          </div>
        </div>
      )}
    </motion.div>
  );
}

/* ── Run Row ────────────────────────────────────────────── */
function RunRow({ run, index }: { run: WorkflowRun; index: number }) {
  const isOk = run.status === 'completed';
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ delay: index * 0.06 + 0.35 }}
      style={{
        display: 'flex', alignItems: 'center', gap: 12,
        padding: '9px 16px', borderBottom: '1px solid var(--border)',
      }}
    >
      <span style={{ flexShrink: 0 }}>
        {isOk
          ? <CheckCircle2 size={14} color="var(--primary)" />
          : run.status === 'failed'
          ? <X size={14} color="var(--text-2)" />
          : <Circle size={14} color="var(--text-3)" />
        }
      </span>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{
          fontSize: 13, color: 'var(--text-1)',
          whiteSpace: 'nowrap', overflow: 'hidden', textOverflow: 'ellipsis',
        }}>
          {run.input}
        </div>
        <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 2, display: 'flex', gap: 6 }}>
          <span>{run.workflowName}</span>
          <span style={{ color: 'var(--border-strong)' }}>·</span>
          <span className="mono">{run.expertChain.join(' → ')}</span>
        </div>
      </div>
      <div style={{ textAlign: 'right', flexShrink: 0 }}>
        <div className="mono" style={{ fontSize: 11, color: 'var(--text-2)' }}>
          {fmt(run.totalTokensUsed)} tok
        </div>
        <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 1 }}>
          {elapsed(run.startedAt)}
        </div>
      </div>
    </motion.div>
  );
}

/* ── Dashboard ──────────────────────────────────────────── */
export default function OpsDashboard() {
  const { metrics, isLoading: metricsLoading } = useMetrics();
  const { tasks: liveTasks, isLoading: tasksLoading } = useTasks(undefined, 20);
  const { experts, isLoading: expertsLoading } = useExperts();
  const { runs: recentRuns, isLoading: runsLoading } = useWorkflowRuns(undefined, 10);
  const { data: providerData, isLoading: providersLoading } = useSWR<{ providers: AIProvider[] }>(
    '/api/providers',
    fetcher,
    { refreshInterval: 30_000 },
  );

  const providers = providerData?.providers ?? [];
  const activeTasks: QueuedTask[] = liveTasks as QueuedTask[];
  const runningTasks  = activeTasks.filter(t => t.status === 'running').length;
  const queuedTasks   = activeTasks.filter(t => t.status === 'queued').length;

  const activeExperts   = experts.filter((e: Expert) => e.status === 'active').length;
  const idleExperts     = experts.filter((e: Expert) => e.status === 'idle').length;
  const trainingExperts = experts.filter((e: Expert) => e.status === 'training').length;

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>

      {/* Shimmer keyframes — injected once */}
      <style>{`
        @keyframes shimmer {
          0% { background-position: 200% 0; }
          100% { background-position: -200% 0; }
        }
      `}</style>

      {/* ── Page Header ──────────────────────────────────── */}
      <motion.div
        initial={{ opacity: 0, y: -10 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.4, ease: 'easeOut' }}
        style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          marginBottom: 28,
        }}
      >
        <div>
          <h1 style={{
            fontSize: 22, fontWeight: 700, color: 'var(--text-1)',
            margin: 0, letterSpacing: '-0.03em',
          }}>
            Ops Dashboard
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Real-time overview · {runningTasks} running · {queuedTasks} queued
          </p>
        </div>
        <motion.div
          initial={{ opacity: 0, x: 10 }}
          animate={{ opacity: 1, x: 0 }}
          transition={{ delay: 0.15 }}
          style={{ display: 'flex', gap: 8 }}
        >
          <Link href="/workflow/builder">
            <button className="btn btn-secondary btn-sm">
              <Workflow size={13} /> New Workflow
            </button>
          </Link>
          <Link href="/workflow">
            <button className="btn btn-primary btn-sm">
              <Play size={13} /> Quick Run
            </button>
          </Link>
        </motion.div>
      </motion.div>

      {/* ── Metrics ──────────────────────────────────────── */}
      <motion.div
        variants={stagger(0.08)}
        initial="hidden"
        animate="show"
        style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 20 }}
      >
        {metricsLoading ? (
          <>
            <SkeletonCard />
            <SkeletonCard />
            <SkeletonCard />
            <SkeletonCard />
          </>
        ) : (
          <>
            <MetricCard
              label="ACTIVE AGENTS"
              value={v(metrics?.activeAgents)}
              sub={`${runningTasks} running · ${queuedTasks} queued`}
              icon={Cpu}
            />
            <MetricCard
              label="TASKS TODAY"
              value={v(metrics?.tasksToday)}
              sub={metrics?.successRate != null
                ? `${(metrics.successRate * 100).toFixed(1)}% success rate`
                : '\u2014'}
              icon={CheckCircle2}
            />
            <MetricCard
              label="TOKENS USED"
              value={fmt(metrics?.tokensUsedToday)}
              sub={metrics?.tokenBudgetDaily != null
                ? `of ${fmt(metrics.tokenBudgetDaily)} daily budget`
                : '\u2014'}
              icon={Zap}
            />
            <MetricCard
              label="AVG LATENCY"
              value={metrics?.avgLatencyMs != null
                ? `${(metrics.avgLatencyMs / 1000).toFixed(1)}s`
                : '\u2014'}
              sub={metrics?.costToday != null
                ? `$${metrics.costToday.toFixed(2)} spent today`
                : '\u2014'}
              icon={TrendingUp}
            />
          </>
        )}
      </motion.div>

      {/* ── Main grid ────────────────────────────────────── */}
      <motion.div
        variants={stagger(0.1)}
        initial="hidden"
        animate="show"
        style={{ display: 'grid', gridTemplateColumns: '1fr 300px', gap: 12, marginBottom: 12 }}
      >
        {/* Task Queue */}
        <motion.div variants={fadeUp} className="card" style={{ overflow: 'hidden' }}>
          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            padding: '13px 16px', borderBottom: '1px solid var(--border)',
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
              <Activity size={14} color="var(--primary)" />
              <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
                Active Task Queue
              </span>
              <span className="badge badge-primary">{activeTasks.length}</span>
            </div>
            <Link href="/tasks">
              <button className="btn btn-ghost btn-sm">
                View All <ChevronRight size={12} />
              </button>
            </Link>
          </div>
          {tasksLoading ? (
            <>
              <SkeletonRow />
              <SkeletonRow />
              <SkeletonRow />
            </>
          ) : activeTasks.length === 0 ? (
            <div style={{ padding: '24px 16px', textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>
              No active tasks
            </div>
          ) : (
            activeTasks.map((task, i) => <TaskRow key={task.id} task={task} index={i} />)
          )}
        </motion.div>

        {/* Right panels */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
          {/* Provider Health */}
          <motion.div variants={fadeUp} className="card">
            <div style={{
              padding: '13px 16px 10px', borderBottom: '1px solid var(--border)',
              fontSize: 13, fontWeight: 600, color: 'var(--text-1)',
              display: 'flex', alignItems: 'center', gap: 8,
            }}>
              <Cpu size={13} color="var(--text-2)" /> Provider Health
            </div>
            <div style={{ padding: '6px 0' }}>
              {providersLoading ? (
                <>
                  <div style={{ padding: '7px 16px' }}><Skeleton width="80%" height={13} /></div>
                  <div style={{ padding: '7px 16px' }}><Skeleton width="60%" height={13} /></div>
                  <div style={{ padding: '7px 16px' }}><Skeleton width="70%" height={13} /></div>
                </>
              ) : (
                <>
                  {providers.filter((p: AIProvider) => p.connected).map((p: AIProvider) => (
                    <div key={p.id} style={{
                      display: 'flex', alignItems: 'center', gap: 10,
                      padding: '7px 16px',
                    }}>
                      <span className={`status-dot ${
                        p.status === 'operational' ? 'dot-online' :
                        p.status === 'degraded'    ? 'dot-training' : 'dot-error'
                      }`} />
                      <span style={{ flex: 1, fontSize: 13, color: 'var(--text-1)' }}>{p.name}</span>
                      <span className="mono" style={{
                        fontSize: 11,
                        color: p.status === 'operational' ? 'var(--text-2)' : 'var(--text-1)',
                        fontWeight: p.status !== 'operational' ? 600 : 400,
                      }}>
                        {p.status === 'operational' ? `${p.latencyMs ?? '\u2014'}ms`
                          : p.status === 'degraded' ? `${p.latencyMs ?? '\u2014'}ms ↑` : 'Outage'}
                      </span>
                    </div>
                  ))}
                  {providers.filter((p: AIProvider) => !p.connected).slice(0, 2).map((p: AIProvider) => (
                    <div key={p.id} style={{
                      display: 'flex', alignItems: 'center', gap: 10,
                      padding: '7px 16px', opacity: 0.30,
                    }}>
                      <span className="status-dot dot-offline" />
                      <span style={{ flex: 1, fontSize: 13, color: 'var(--text-2)' }}>{p.name}</span>
                      <span style={{ fontSize: 11, color: 'var(--text-3)' }}>Not connected</span>
                    </div>
                  ))}
                </>
              )}
            </div>
          </motion.div>

          {/* Expert Pool */}
          <motion.div variants={fadeUp} className="card">
            <div style={{
              padding: '13px 16px 10px', borderBottom: '1px solid var(--border)',
              fontSize: 13, fontWeight: 600, color: 'var(--text-1)',
              display: 'flex', alignItems: 'center', gap: 8,
            }}>
              <Users size={13} color="var(--text-2)" /> Expert Pool
            </div>
            <div style={{ padding: '10px 16px' }}>
              {expertsLoading ? (
                <>
                  <div style={{ marginBottom: 10 }}><Skeleton width="100%" height={18} /></div>
                  <div style={{ marginBottom: 10 }}><Skeleton width="100%" height={18} /></div>
                  <div style={{ marginBottom: 10 }}><Skeleton width="100%" height={18} /></div>
                </>
              ) : (
                <>
                  {[
                    { label: 'Active',   count: activeExperts,   dot: 'dot-online'   },
                    { label: 'Idle',     count: idleExperts,     dot: 'dot-idle'     },
                    { label: 'Training', count: trainingExperts, dot: 'dot-training' },
                  ].map((item, i) => (
                    <motion.div
                      key={item.label}
                      initial={{ opacity: 0, x: 8 }}
                      animate={{ opacity: 1, x: 0 }}
                      transition={{ delay: i * 0.08 + 0.5 }}
                      style={{
                        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                        marginBottom: 10,
                      }}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        <span className={`status-dot ${item.dot}`} />
                        <span style={{ fontSize: 13, color: 'var(--text-2)' }}>{item.label}</span>
                      </div>
                      <span style={{
                        fontSize: 18, fontWeight: 700, color: 'var(--text-1)',
                        fontVariantNumeric: 'tabular-nums', letterSpacing: '-0.03em',
                      }}>
                        {item.count}
                      </span>
                    </motion.div>
                  ))}
                </>
              )}
              <div className="divider" style={{ margin: '8px 0 10px' }} />
              <Link href="/experts">
                <motion.button
                  whileHover={{ scale: 1.01 }}
                  whileTap={{ scale: 0.99 }}
                  className="btn btn-secondary btn-sm"
                  style={{ width: '100%', justifyContent: 'center' }}
                >
                  Manage Experts <ArrowRight size={12} />
                </motion.button>
              </Link>
            </div>
          </motion.div>
        </div>
      </motion.div>

      {/* ── Recent Runs ────────────────────────────────────── */}
      <motion.div
        variants={fadeUp}
        initial="hidden"
        animate="show"
        transition={{ delay: 0.3 }}
        className="card"
        style={{ marginBottom: 12, overflow: 'hidden' }}
      >
        <div style={{
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
          padding: '13px 16px', borderBottom: '1px solid var(--border)',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <BarChart3 size={14} color="var(--text-2)" />
            <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>
              Recent Workflow Runs
            </span>
          </div>
          <Link href="/workflow/history">
            <button className="btn btn-ghost btn-sm">
              Full History <ChevronRight size={12} />
            </button>
          </Link>
        </div>
        {runsLoading ? (
          <>
            <SkeletonRow />
            <SkeletonRow />
            <SkeletonRow />
          </>
        ) : recentRuns.length === 0 ? (
          <div style={{ padding: '24px 16px', textAlign: 'center', color: 'var(--text-3)', fontSize: 13 }}>
            No recent runs
          </div>
        ) : (
          recentRuns.map((run: WorkflowRun, i: number) => <RunRow key={run.id} run={run} index={i} />)
        )}
      </motion.div>

      {/* ── Quick Start ─────────────────────────────────────── */}
      <motion.div
        variants={stagger(0.1)}
        initial="hidden"
        animate="show"
        transition={{ delay: 0.4 }}
        style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 12 }}
      >
        {[
          {
            href:  '/workflow',
            icon:  Workflow,
            title: 'Build a Workflow',
            desc:  'Chain experts to tackle complex multi-step tasks with minimal tokens and energy.',
          },
          {
            href:  '/experts',
            icon:  Users,
            title: 'Browse Expert Catalog',
            desc:  'Discover and deploy specialized AI agents for any domain or task type.',
          },
          {
            href:  '/training',
            icon:  Zap,
            title: 'Train a New Expert',
            desc:  'Fine-tune foundation models on your data to create domain-specific agents.',
          },
        ].map(card => (
          <motion.div key={card.href} variants={fadeUp}>
            <Link href={card.href} style={{ textDecoration: 'none' }}>
              <motion.div
                whileHover={{ y: -3, boxShadow: '0 10px 32px rgba(13,13,13,0.10)' }}
                whileTap={{ scale: 0.99 }}
                transition={{ type: 'spring', stiffness: 400, damping: 28 }}
                className="card"
                style={{ padding: 20, cursor: 'pointer', overflow: 'hidden', position: 'relative' }}
              >
                {/* dot-grid pattern */}
                <div
                  className="dot-grid"
                  style={{
                    position: 'absolute', inset: 0, opacity: 0.5,
                    pointerEvents: 'none',
                  }}
                />
                <div style={{ position: 'relative' }}>
                  <div style={{
                    width: 36, height: 36, borderRadius: 8,
                    background: 'var(--primary-dim)',
                    border: '1px solid var(--border-md)',
                    display: 'flex', alignItems: 'center', justifyContent: 'center',
                    marginBottom: 14,
                  }}>
                    <card.icon size={17} color="var(--primary)" strokeWidth={2} />
                  </div>
                  <div style={{
                    fontSize: 14, fontWeight: 600, color: 'var(--text-1)', marginBottom: 6,
                    letterSpacing: '-0.01em',
                  }}>
                    {card.title}
                  </div>
                  <div style={{ fontSize: 12, color: 'var(--text-3)', lineHeight: 1.6 }}>
                    {card.desc}
                  </div>
                  <div style={{
                    display: 'flex', alignItems: 'center', gap: 4,
                    marginTop: 14, fontSize: 12, color: 'var(--text-2)', fontWeight: 500,
                  }}>
                    Get started <ArrowRight size={11} />
                  </div>
                </div>
              </motion.div>
            </Link>
          </motion.div>
        ))}
      </motion.div>
    </div>
  );
}
