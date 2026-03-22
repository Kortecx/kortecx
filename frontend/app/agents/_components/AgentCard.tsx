'use client';

import { motion } from 'framer-motion';
import {
  Zap, Clock, Activity, Database, Server,
  ChevronRight, Settings,
} from 'lucide-react';

const SECTION_COLOR = '#3b82f6';

const fadeUp = {
  hidden: { opacity: 0, y: 14 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};

const ROLE_COLOR: Record<string, string> = {
  researcher: '#8b5cf6', analyst: '#3b82f6', writer: '#f59e0b', coder: '#10b981',
  reviewer: '#06b6d4', planner: '#6366f1', legal: '#ef4444', financial: '#f97316',
  medical: '#ec4899', coordinator: '#8b5cf6', 'data-engineer': '#14b8a6',
  creative: '#a855f7', translator: '#06b6d4', custom: '#6b7280',
};

const ROLE_EMOJI: Record<string, string> = {
  researcher: '\u{1F52C}', analyst: '\u{1F4CA}', writer: '\u270D\uFE0F', coder: '\u{1F4BB}',
  reviewer: '\u{1F50D}', planner: '\u{1F5C2}', legal: '\u2696\uFE0F', financial: '\u{1F4B0}',
  medical: '\u{1FA7A}', coordinator: '\u{1F504}', 'data-engineer': '\u{1F6E0}', creative: '\u{1F3A8}',
  translator: '\u{1F310}', custom: '\u2699\uFE0F',
};

const STATUS_CONFIG: Record<string, { color: string; label: string; pulse: boolean }> = {
  active:    { color: '#10b981', label: 'Active',    pulse: true  },
  idle:      { color: '#6b7280', label: 'Idle',      pulse: false },
  running:   { color: '#3b82f6', label: 'Running',   pulse: true  },
  completed: { color: '#10b981', label: 'Completed', pulse: false },
  failed:    { color: '#ef4444', label: 'Failed',    pulse: false },
  training:  { color: '#f59e0b', label: 'Training',  pulse: true  },
  error:     { color: '#ef4444', label: 'Error',     pulse: false },
};

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

interface AgentCardProps {
  agent: Record<string, unknown>;
  expertStats: Record<string, unknown>[];
}

export default function AgentCard({ agent, expertStats }: AgentCardProps) {
  const role      = (agent.role as string) ?? 'custom';
  const roleColor = ROLE_COLOR[role] ?? '#6b7280';
  const emoji     = ROLE_EMOJI[role] ?? '\u2699\uFE0F';
  const statusKey = (agent.status as string) ?? 'idle';
  const status    = STATUS_CONFIG[statusKey] ?? STATUS_CONFIG.idle;
  const isActive  = statusKey === 'active' || statusKey === 'running';

  const agentPerf  = expertStats.find((e: Record<string, unknown>) => e.step_id === agent.id);
  const tokensUsed = (agentPerf?.total_tokens as number) ?? (agent.tokensUsed as number) ?? 0;
  const tokenMax   = 8000;
  const tokenPct   = Math.min((tokensUsed / tokenMax) * 100, 100);
  const tokenColor = tokenPct > 80 ? '#ef4444' : tokenPct > 50 ? '#f59e0b' : SECTION_COLOR;

  const provider      = (agent.provider as string) ?? '';
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
          { label: 'Tokens',   value: tokensUsed > 0 ? fmt(tokensUsed) : '\u2014', icon: Zap,      color: SECTION_COLOR },
          { label: 'Requests', value: fmt((agentPerf?.successful_runs as number) ?? (agent.requestsHandled as number) ?? 0), icon: Activity, color: '#10b981' },
          { label: 'Latency',  value: (agentPerf?.avg_latency_ms as number) > 0 ? `${Math.round(agentPerf?.avg_latency_ms as number)}ms` : (agent.uptimeMin as number) > 0 ? `${agent.uptimeMin}m` : '\u2014', icon: Clock, color: '#f59e0b' },
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

      {/* Token progress bar */}
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
