'use client';

import { useState } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Cpu, Activity, Zap, Clock, RefreshCw,
  TrendingUp, Loader2, ChevronRight, Settings,
  Database, Server,
} from 'lucide-react';
import { useAgents } from '@/lib/hooks/useApi';

const SECTION_COLOR = '#3b82f6';

const fadeUp = {
  hidden: { opacity: 0, y: 14 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};
const stagger = { hidden: {}, show: { transition: { staggerChildren: 0.07 } } };

const ROLE_COLOR: Record<string, string> = {
  researcher:      '#8b5cf6',
  analyst:         '#3b82f6',
  writer:          '#f59e0b',
  coder:           '#10b981',
  reviewer:        '#06b6d4',
  planner:         '#6366f1',
  legal:           '#ef4444',
  financial:       '#f97316',
  medical:         '#ec4899',
  coordinator:     '#8b5cf6',
  'data-engineer': '#14b8a6',
  creative:        '#a855f7',
  translator:      '#06b6d4',
  custom:          '#6b7280',
};

const ROLE_EMOJI: Record<string, string> = {
  researcher: '🔬', analyst: '📊', writer: '✍️', coder: '💻',
  reviewer: '🔍', planner: '🗂', legal: '⚖️', financial: '💰',
  medical: '🩺', coordinator: '🔄', 'data-engineer': '🛠', creative: '🎨',
  translator: '🌐', custom: '⚙️',
};

const STATUS_CONFIG: Record<string, { color: string; label: string; pulse: boolean }> = {
  active:   { color: '#10b981', label: 'Active',   pulse: true  },
  idle:     { color: '#6b7280', label: 'Idle',     pulse: false },
  training: { color: '#f59e0b', label: 'Training', pulse: true  },
  error:    { color: '#ef4444', label: 'Error',    pulse: false },
};

const FILTER_TABS = ['all', 'active', 'idle', 'training'] as const;
type FilterTab = typeof FILTER_TABS[number];

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
        <div className="skeleton" style={{ width: 40, height: 40, borderRadius: 9 }} />
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', gap: 7 }}>
          <div className="skeleton" style={{ height: 13, width: '60%', borderRadius: 6 }} />
          <div className="skeleton" style={{ height: 10, width: '40%', borderRadius: 5 }} />
        </div>
        <div className="skeleton" style={{ width: 54, height: 20, borderRadius: 99 }} />
      </div>
      <div className="skeleton" style={{ height: 46, borderRadius: 8, width: '100%' }} />
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        {[0, 1, 2].map(i => (
          <div key={i} className="skeleton" style={{ height: 52, borderRadius: 7 }} />
        ))}
      </div>
      <div style={{ display: 'flex', gap: 8, paddingTop: 4 }}>
        <div className="skeleton" style={{ height: 28, flex: 1, borderRadius: 7 }} />
        <div className="skeleton" style={{ height: 28, width: 80, borderRadius: 7 }} />
      </div>
    </div>
  );
}

function AgentCard({ agent }: { agent: Record<string, unknown> }) {
  const role      = (agent.role as string) ?? 'custom';
  const roleColor = ROLE_COLOR[role] ?? '#6b7280';
  const emoji     = ROLE_EMOJI[role] ?? '⚙️';
  const statusKey = (agent.status as string) ?? 'idle';
  const status    = STATUS_CONFIG[statusKey] ?? STATUS_CONFIG.idle;
  const isActive  = statusKey === 'active';

  const tokensUsed = (agent.tokensUsed as number) ?? 0;
  const tokenMax   = 8000;
  const tokenPct   = Math.min((tokensUsed / tokenMax) * 100, 100);
  const tokenColor = tokenPct > 80 ? '#ef4444' : tokenPct > 50 ? '#f59e0b' : SECTION_COLOR;

  const provider = (agent.provider as string) ?? '';
  const providerColor =
    provider.toLowerCase().includes('anthropic') ? '#D97757' :
    provider.toLowerCase().includes('openai')    ? '#74AA9C' :
    provider.toLowerCase().includes('google')    ? '#4285f4' : '#6b7280';

  return (
    <motion.div
      variants={fadeUp}
      whileHover={{ y: -3, boxShadow: '0 10px 32px rgba(13,13,13,0.09)' }}
      transition={{ type: 'spring', stiffness: 380, damping: 28 }}
      style={{
        background: 'var(--bg-surface)',
        border: '1px solid var(--border)',
        borderRadius: 12, padding: 20,
        display: 'flex', flexDirection: 'column', gap: 14,
        position: 'relative', overflow: 'hidden',
      }}
    >
      {/* Top accent stripe */}
      <div style={{
        position: 'absolute', top: 0, left: 0, right: 0, height: 2,
        background: `linear-gradient(90deg, ${roleColor}, ${roleColor}50)`,
        borderRadius: '12px 12px 0 0',
      }} />

      {/* Header: icon + name + status */}
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', marginTop: 4 }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 11 }}>
          <div style={{
            width: 40, height: 40, borderRadius: 9,
            background: `${roleColor}14`,
            border: `1.5px solid ${roleColor}28`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            fontSize: 18,
          }}>
            {emoji}
          </div>
          <div>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
              {agent.name as string}
            </div>
            <div style={{
              display: 'inline-flex', alignItems: 'center', marginTop: 4,
              padding: '2px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
              textTransform: 'uppercase', letterSpacing: '0.05em',
              background: `${roleColor}14`, color: roleColor,
              border: `1px solid ${roleColor}28`,
            }}>
              {role}
            </div>
          </div>
        </div>

        {/* Status badge */}
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <div style={{ position: 'relative', width: 8, height: 8 }}>
            {status.pulse && (
              <div style={{
                position: 'absolute', inset: -3,
                borderRadius: '50%',
                background: `${status.color}30`,
                animation: 'pulse-dot 2s ease-in-out infinite',
              }} />
            )}
            <div style={{
              width: 8, height: 8, borderRadius: '50%',
              background: status.color,
              position: 'relative', zIndex: 1,
            }} />
          </div>
          <span style={{ fontSize: 11, fontWeight: 600, color: status.color }}>
            {status.label}
          </span>
        </div>
      </div>

      {/* Current task */}
      {isActive && agent.taskName ? (
        <div style={{
          padding: '10px 12px', borderRadius: 8,
          background: `${SECTION_COLOR}08`,
          border: `1px solid ${SECTION_COLOR}20`,
        }}>
          <div style={{
            fontSize: 9, color: SECTION_COLOR, fontWeight: 700,
            letterSpacing: '0.07em', textTransform: 'uppercase', marginBottom: 4,
          }}>
            Current Task
          </div>
          <div style={{ fontSize: 12, color: 'var(--text-2)', lineHeight: 1.45, fontWeight: 500 }}>
            {agent.taskName as string}
          </div>
        </div>
      ) : (
        <div style={{
          padding: '10px 12px', borderRadius: 8,
          background: 'var(--bg-elevated)',
          border: '1px solid var(--border)',
        }}>
          <div style={{ fontSize: 12, color: 'var(--text-4)', fontStyle: 'italic' }}>
            — No active task
          </div>
        </div>
      )}

      {/* Model + provider */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8,
        paddingBottom: 12, borderBottom: '1px solid var(--border)',
      }}>
        <Server size={11} color="var(--text-4)" />
        <span style={{ fontSize: 12, color: 'var(--text-3)', fontWeight: 500 }}>
          {agent.model as string}
        </span>
        <div style={{
          padding: '1px 7px', borderRadius: 99, fontSize: 10, fontWeight: 700,
          background: `${providerColor}14`, color: providerColor,
          border: `1px solid ${providerColor}28`, marginLeft: 'auto',
        }}>
          {provider}
        </div>
      </div>

      {/* Stats row */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        {[
          { label: 'Tokens',   value: tokensUsed > 0 ? fmt(tokensUsed) : '—', icon: Zap,      color: SECTION_COLOR },
          { label: 'Requests', value: fmt((agent.requestsHandled as number) ?? 0), icon: Activity, color: '#10b981' },
          { label: 'Uptime',   value: (agent.uptimeMin as number) > 0 ? `${agent.uptimeMin}m` : '—', icon: Clock, color: '#f59e0b' },
        ].map(({ label, value, icon: Icon, color }) => (
          <div key={label} style={{
            textAlign: 'center', padding: '8px 4px', borderRadius: 8,
            background: `${color}06`, border: `1px solid ${color}12`,
          }}>
            <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 4 }}>
              <Icon size={11} color={color} strokeWidth={2.5} />
            </div>
            <div style={{ fontSize: 14, fontWeight: 800, color: 'var(--text-1)', lineHeight: 1 }}>
              {value}
            </div>
            <div style={{ fontSize: 9, color: 'var(--text-4)', marginTop: 2, fontWeight: 500 }}>
              {label}
            </div>
          </div>
        ))}
      </div>

      {/* Token progress bar (only if active and has token usage) */}
      {isActive && tokensUsed > 0 && (
        <div>
          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            marginBottom: 5,
          }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 5 }}>
              <Database size={9} color="var(--text-4)" />
              <span style={{ fontSize: 9, color: 'var(--text-4)', fontWeight: 600 }}>
                TOKEN USAGE
              </span>
            </div>
            <span style={{ fontSize: 9, color: tokenColor, fontWeight: 700 }}>
              {tokenPct.toFixed(0)}%
            </span>
          </div>
          <div style={{
            height: 4, background: 'var(--border)',
            borderRadius: 99, overflow: 'hidden',
          }}>
            <motion.div
              initial={{ width: 0 }}
              animate={{ width: `${tokenPct}%` }}
              transition={{ duration: 0.8, ease: 'easeOut', delay: 0.2 }}
              style={{
                height: '100%', borderRadius: 99,
                background: `linear-gradient(90deg, ${tokenColor}90, ${tokenColor})`,
              }}
            />
          </div>
          <div style={{ fontSize: 9, color: 'var(--text-4)', marginTop: 3 }}>
            {fmt(tokensUsed)} / {fmt(tokenMax)} ctx tokens
          </div>
        </div>
      )}

      {/* Actions */}
      <div style={{ display: 'flex', gap: 8, marginTop: 'auto' }}>
        {isActive && !!(agent.taskId as string) && (
          <button style={{
            flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
            padding: '7px 12px', borderRadius: 7, cursor: 'pointer',
            border: `1.5px solid ${SECTION_COLOR}40`,
            background: `${SECTION_COLOR}10`,
            color: SECTION_COLOR, fontSize: 11, fontWeight: 600,
          }}>
            <ChevronRight size={11} strokeWidth={2.5} />
            View Task
          </button>
        )}
        <button style={{
          display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '7px 12px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid var(--border-md)',
          background: 'transparent',
          color: 'var(--text-3)', fontSize: 11, fontWeight: 500,
          flex: isActive && (agent.taskId as string) ? '0 0 auto' : 1,
        }}>
          <Settings size={11} />
          Configure
        </button>
      </div>
    </motion.div>
  );
}

export default function AgentsPage() {
  const [tab, setTab] = useState<FilterTab>('all');
  const { agents, total, active, isLoading, mutate } = useAgents();

  const idle       = agents.filter((a: Record<string, unknown>) => a.status === 'idle').length;
  const training   = agents.filter((a: Record<string, unknown>) => a.status === 'training').length;

  // Estimate total requests today from requestsHandled sum
  const totalRequests = agents.reduce(
    (sum: number, a: Record<string, unknown>) => sum + ((a.requestsHandled as number) ?? 0), 0,
  );

  const filtered = tab === 'all'
    ? agents
    : agents.filter((a: Record<string, unknown>) => a.status === tab);

  const tabCounts: Record<FilterTab, number> = {
    all:      agents.length,
    active,
    idle,
    training,
  };

  const now = new Date();
  const lastUpdated = `${now.getHours().toString().padStart(2, '0')}:${now.getMinutes().toString().padStart(2, '0')}:${now.getSeconds().toString().padStart(2, '0')}`;

  return (
    <div style={{ padding: 28, maxWidth: 1200 }}>

      {/* ── Header ── */}
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
                Active Agents
              </h1>
              {/* Live count badge */}
              <div style={{
                display: 'flex', alignItems: 'center', gap: 5,
                padding: '3px 9px', borderRadius: 99,
                background: active > 0 ? '#10b98118' : 'var(--bg-elevated)',
                border: `1px solid ${active > 0 ? '#10b98130' : 'var(--border)'}`,
              }}>
                {active > 0 && (
                  <div className="dot-pulse" style={{
                    width: 6, height: 6, borderRadius: '50%', background: '#10b981',
                  }} />
                )}
                <span style={{
                  fontSize: 11, fontWeight: 700,
                  color: active > 0 ? '#10b981' : 'var(--text-3)',
                }}>
                  {active} live
                </span>
              </div>
            </div>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4 }}>
              {total} agents registered · auto-refreshes every 5s
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

      {/* ── Metric summary bar ── */}
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
          { label: 'Active Now',          value: active,         color: '#10b981', icon: Activity,  desc: 'processing tasks'     },
          { label: 'Idle',                value: idle,           color: '#6b7280', icon: Clock,     desc: 'awaiting tasks'       },
          { label: 'In Training',         value: training,       color: '#f59e0b', icon: Zap,       desc: 'fine-tuning'          },
          { label: 'Requests Today',      value: totalRequests,  color: SECTION_COLOR, icon: TrendingUp, desc: 'total handled' },
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

      {/* ── Filter tabs ── */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.14 }}
        style={{ display: 'flex', gap: 6, marginBottom: 20 }}
      >
        {FILTER_TABS.map(t => (
          <button
            key={t}
            onClick={() => setTab(t)}
            style={{
              padding: '6px 14px', borderRadius: 99, fontSize: 12, cursor: 'pointer',
              border: tab === t ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
              background: tab === t ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
              color: tab === t ? SECTION_COLOR : 'var(--text-3)',
              fontWeight: tab === t ? 700 : 400,
              transition: 'all 0.15s',
            }}
          >
            {t.charAt(0).toUpperCase() + t.slice(1)}
            <span style={{ marginLeft: 5, fontSize: 10, opacity: 0.75 }}>
              ({tabCounts[t]})
            </span>
          </button>
        ))}
      </motion.div>

      {/* ── Agent grid ── */}
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
          {filtered.length === 0 ? (
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
                No agents in this view
              </div>
              <div style={{ fontSize: 12, color: 'var(--text-4)', maxWidth: 320 }}>
                {tab === 'all'
                  ? 'No agents are registered yet. Deploy an expert to create an agent.'
                  : `No agents are currently ${tab}. Switch tabs to see other agents.`}
              </div>
            </motion.div>
          ) : (
            <motion.div
              key={tab}
              variants={stagger}
              initial="hidden"
              animate="show"
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
                gap: 16,
              }}
            >
              {filtered.map((agent: Record<string, unknown>) => (
                <AgentCard key={agent.id as string} agent={agent} />
              ))}
            </motion.div>
          )}
        </AnimatePresence>
      )}

      {/* ── Footer / last updated ── */}
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
    </div>
  );
}
