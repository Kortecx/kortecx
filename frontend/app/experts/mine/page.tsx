'use client';

import { useState, useMemo } from 'react';
import Link from 'next/link';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Star, Search, Plus, Settings, TrendingUp,
  Loader2, Activity, Zap, Clock, BarChart2,
  ChevronDown, Play, Cpu, Tag,
} from 'lucide-react';
import { useExperts } from '@/lib/hooks/useApi';

const SECTION_COLOR = '#8b5cf6';

const fadeUp = {
  hidden: { opacity: 0, y: 14 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};
const stagger = { hidden: {}, show: { transition: { staggerChildren: 0.07 } } };

/* ─── Status config ─────────────────────────────────── */
const STATUS_CONFIG: Record<string, { color: string; bg: string; label: string; pulse: boolean }> = {
  active:      { color: '#10b981', bg: '#10b98112', label: 'Active',      pulse: true  },
  idle:        { color: '#6b7280', bg: '#6b728012', label: 'Idle',        pulse: false },
  training:    { color: '#f59e0b', bg: '#f59e0b12', label: 'Training',    pulse: true  },
  finetuning:  { color: '#8b5cf6', bg: '#8b5cf612', label: 'Fine-tuning', pulse: true  },
  offline:     { color: '#ef4444', bg: '#ef444412', label: 'Offline',     pulse: false },
  error:       { color: '#ef4444', bg: '#ef444412', label: 'Error',       pulse: false },
};

/* ─── Provider config ───────────────────────────────── */
const PROVIDER_CONFIG: Record<string, { color: string; label: string }> = {
  anthropic: { color: '#D97757', label: 'Anthropic' },
  openai:    { color: '#74AA9C', label: 'OpenAI'    },
  google:    { color: '#4285f4', label: 'Google'    },
};

/* ─── Role emoji ────────────────────────────────────── */
const ROLE_EMOJI: Record<string, string> = {
  researcher: '🔬', analyst: '📊', writer: '✍️', coder: '💻',
  reviewer: '🔍', planner: '🗂', legal: '⚖️', financial: '💰',
  medical: '🩺', coordinator: '🔄', 'data-engineer': '🛠', creative: '🎨',
  translator: '🌐', custom: '⚙️',
};

const ROLE_COLOR: Record<string, string> = {
  researcher: '#8b5cf6', analyst: '#3b82f6', writer: '#f59e0b', coder: '#10b981',
  reviewer: '#06b6d4', planner: '#6366f1', legal: '#ef4444', financial: '#f97316',
  medical: '#ec4899', coordinator: '#8b5cf6', 'data-engineer': '#14b8a6',
  creative: '#a855f7', translator: '#06b6d4', custom: '#6b7280',
};

const STATUS_FILTERS = ['all', 'active', 'idle', 'training', 'finetuning'] as const;
type StatusFilter = typeof STATUS_FILTERS[number];

const SORT_OPTIONS = [
  { value: 'rating', label: 'Rating'    },
  { value: 'runs',   label: 'Runs'      },
  { value: 'name',   label: 'Name'      },
  { value: 'cost',   label: 'Avg Cost'  },
] as const;
type SortOption = typeof SORT_OPTIONS[number]['value'];

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return String(n);
}

function pct(n: number) {
  return `${(n * 100).toFixed(1)}%`;
}

/* ─── Skeleton card ─────────────────────────────────── */
function SkeletonCard() {
  return (
    <div style={{
      background: 'var(--bg-surface)',
      border: '1px solid var(--border)',
      borderRadius: 12, padding: 20,
      display: 'flex', flexDirection: 'column', gap: 14,
    }}>
      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
        <div style={{ display: 'flex', flexDirection: 'column', gap: 8, flex: 1 }}>
          <div className="skeleton" style={{ height: 14, width: '55%', borderRadius: 6 }} />
          <div style={{ display: 'flex', gap: 6 }}>
            <div className="skeleton" style={{ height: 20, width: 58, borderRadius: 99 }} />
            <div className="skeleton" style={{ height: 20, width: 68, borderRadius: 99 }} />
          </div>
        </div>
        <div className="skeleton" style={{ width: 36, height: 36, borderRadius: 8 }} />
      </div>
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8 }}>
        {[0, 1, 2].map(i => (
          <div key={i} className="skeleton" style={{ height: 56, borderRadius: 8 }} />
        ))}
      </div>
      <div className="skeleton" style={{ height: 22, width: '70%', borderRadius: 99 }} />
      <div style={{ display: 'flex', gap: 7 }}>
        {[0, 1, 2].map(i => (
          <div key={i} className="skeleton" style={{ height: 30, flex: 1, borderRadius: 7 }} />
        ))}
      </div>
    </div>
  );
}

/* ─── Expert card ───────────────────────────────────── */
function ExpertCard({ expert }: { expert: Record<string, unknown> }) {
  const role        = (expert.role as string) ?? 'custom';
  const roleColor   = ROLE_COLOR[role] ?? '#6b7280';
  const roleEmoji   = ROLE_EMOJI[role] ?? '⚙️';
  const statusKey   = (expert.status as string) ?? 'idle';
  const status      = STATUS_CONFIG[statusKey] ?? STATUS_CONFIG.idle;
  const providerId  = (expert.providerId as string) ?? '';
  const provider    = PROVIDER_CONFIG[providerId.toLowerCase()] ?? { color: '#6b7280', label: expert.providerName as string ?? 'Unknown' };

  const stats       = (expert.stats as Record<string, number>) ?? {};
  const totalRuns   = (expert.totalRuns as number) ?? stats.totalRuns ?? 0;
  const successRate = (expert.successRate as number) ?? stats.successRate ?? 0;
  const avgCost     = stats.avgCostPerRun ?? 0;
  const avgLatency  = (expert.avgLatencyMs as number) ?? stats.avgLatencyMs ?? 0;
  const rating      = stats.rating ?? 0;
  const tags        = ((expert.tags as string[]) ?? []).slice(0, 3);
  const isFinetuned = (expert.isFinetuned as boolean) ?? false;

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

      {/* Header: name + role icon */}
      <div style={{ display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between', marginTop: 4 }}>
        <div style={{ flex: 1, minWidth: 0 }}>
          {/* Name + version */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' }}>
            <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
              {expert.name as string}
            </span>
            <span style={{
              padding: '1px 7px', borderRadius: 99, fontSize: 10, fontWeight: 700,
              background: 'var(--bg-elevated)', color: 'var(--text-3)',
              border: '1px solid var(--border-md)',
            }}>
              v{expert.version as string}
            </span>
          </div>

          {/* Status + badges row */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginTop: 7, flexWrap: 'wrap' }}>
            {/* Status dot + label */}
            <div style={{
              display: 'flex', alignItems: 'center', gap: 5,
              padding: '3px 8px', borderRadius: 99,
              background: status.bg, border: `1px solid ${status.color}28`,
            }}>
              <div style={{ position: 'relative', width: 6, height: 6 }}>
                {status.pulse && (
                  <div className="dot-pulse" style={{
                    position: 'absolute', inset: -3,
                    borderRadius: '50%', background: `${status.color}30`,
                  }} />
                )}
                <div style={{
                  width: 6, height: 6, borderRadius: '50%',
                  background: status.color, position: 'relative', zIndex: 1,
                }} />
              </div>
              <span style={{ fontSize: 10, fontWeight: 700, color: status.color }}>
                {status.label}
              </span>
            </div>

            {/* Provider badge */}
            <span style={{
              padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
              background: `${provider.color}12`, color: provider.color,
              border: `1px solid ${provider.color}28`,
            }}>
              {provider.label}
            </span>

            {/* Fine-tuned badge */}
            {isFinetuned && (
              <span style={{
                padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
                background: `${SECTION_COLOR}12`, color: SECTION_COLOR,
                border: `1px solid ${SECTION_COLOR}30`,
              }}>
                ✦ Fine-tuned
              </span>
            )}
          </div>
        </div>

        {/* Role emoji icon */}
        <div style={{
          width: 40, height: 40, borderRadius: 9, flexShrink: 0, marginLeft: 8,
          background: `${roleColor}12`, border: `1.5px solid ${roleColor}25`,
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          fontSize: 18,
        }}>
          {roleEmoji}
        </div>
      </div>

      {/* Role name */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 6,
        fontSize: 11, color: 'var(--text-3)', fontWeight: 500,
        marginTop: -6,
      }}>
        <Cpu size={10} color="var(--text-4)" />
        <span style={{ textTransform: 'capitalize' }}>{role}</span>
        <span style={{ color: 'var(--text-4)' }}>·</span>
        <span style={{ color: 'var(--text-4)' }}>{(expert.modelName ?? expert.modelId) as string}</span>
      </div>

      {/* Stats grid */}
      <div style={{
        display: 'grid', gridTemplateColumns: '1fr 1fr 1fr',
        gap: 8,
        paddingTop: 10, borderTop: '1px solid var(--border)',
      }}>
        {/* Total runs */}
        <div style={{
          textAlign: 'center', padding: '9px 4px', borderRadius: 8,
          background: `${SECTION_COLOR}06`, border: `1px solid ${SECTION_COLOR}12`,
        }}>
          <div style={{ fontSize: 18, fontWeight: 800, color: SECTION_COLOR, lineHeight: 1 }}>
            {fmt(totalRuns)}
          </div>
          <div style={{ fontSize: 9, color: 'var(--text-4)', marginTop: 3, fontWeight: 500 }}>
            Total Runs
          </div>
        </div>

        {/* Success rate */}
        <div style={{
          textAlign: 'center', padding: '9px 4px', borderRadius: 8,
          background: '#10b98106', border: '1px solid #10b98112',
        }}>
          <div style={{ fontSize: 18, fontWeight: 800, color: '#10b981', lineHeight: 1 }}>
            {successRate > 0 ? pct(successRate) : '—'}
          </div>
          <div style={{ fontSize: 9, color: 'var(--text-4)', marginTop: 3, fontWeight: 500 }}>
            Success Rate
          </div>
        </div>

        {/* Avg cost */}
        <div style={{
          textAlign: 'center', padding: '9px 4px', borderRadius: 8,
          background: '#f59e0b06', border: '1px solid #f59e0b12',
        }}>
          <div style={{ fontSize: 18, fontWeight: 800, color: '#f59e0b', lineHeight: 1 }}>
            {avgCost > 0 ? `$${avgCost.toFixed(3)}` : '—'}
          </div>
          <div style={{ fontSize: 9, color: 'var(--text-4)', marginTop: 3, fontWeight: 500 }}>
            Avg Cost / Run
          </div>
        </div>
      </div>

      {/* Latency + rating row */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        fontSize: 11,
      }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 5, color: 'var(--text-3)' }}>
          <Clock size={10} color="var(--text-4)" />
          <span style={{ fontWeight: 500 }}>
            {avgLatency > 0 ? `${avgLatency.toLocaleString()} ms avg latency` : 'No latency data'}
          </span>
        </div>
        {rating > 0 && (
          <div style={{ display: 'flex', alignItems: 'center', gap: 4, color: '#f59e0b' }}>
            <Star size={11} fill="#f59e0b" strokeWidth={0} />
            <span style={{ fontWeight: 700, fontSize: 12 }}>{rating.toFixed(1)}</span>
          </div>
        )}
      </div>

      {/* Tags */}
      {tags.length > 0 && (
        <div style={{ display: 'flex', alignItems: 'center', gap: 6, flexWrap: 'wrap' }}>
          <Tag size={10} color="var(--text-4)" />
          {tags.map(tag => (
            <span key={tag} style={{
              padding: '2px 8px', borderRadius: 99, fontSize: 10, fontWeight: 500,
              background: 'var(--bg-elevated)', color: 'var(--text-3)',
              border: '1px solid var(--border)',
            }}>
              {tag}
            </span>
          ))}
        </div>
      )}

      {/* Action buttons */}
      <div style={{
        display: 'flex', gap: 7, paddingTop: 4,
        borderTop: '1px solid var(--border)', marginTop: 'auto',
      }}>
        <button style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '8px 10px', borderRadius: 7, cursor: 'pointer',
          border: `1.5px solid ${SECTION_COLOR}50`,
          background: `${SECTION_COLOR}12`,
          color: SECTION_COLOR, fontSize: 11, fontWeight: 700,
          transition: 'all 0.15s',
        }}>
          <Play size={10} fill={SECTION_COLOR} strokeWidth={0} />
          Run
        </button>
        <button style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '8px 10px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid var(--border-md)',
          background: 'transparent',
          color: 'var(--text-3)', fontSize: 11, fontWeight: 500,
          transition: 'all 0.15s',
        }}>
          <Settings size={10} />
          Configure
        </button>
        <button style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 5,
          padding: '8px 10px', borderRadius: 7, cursor: 'pointer',
          border: '1px solid var(--border-md)',
          background: 'transparent',
          color: 'var(--text-3)', fontSize: 11, fontWeight: 500,
          transition: 'all 0.15s',
        }}>
          <BarChart2 size={10} />
          View Stats
        </button>
      </div>
    </motion.div>
  );
}

/* ─── Page ──────────────────────────────────────────── */
export default function MyExpertsPage() {
  const [search, setSearch]       = useState('');
  const [statusFilter, setFilter] = useState<StatusFilter>('all');
  const [sortBy, setSortBy]       = useState<SortOption>('rating');
  const [sortOpen, setSortOpen]   = useState(false);

  const { experts, total, isLoading, mutate } = useExperts();

  /* Derived counts */
  const activeCt  = experts.filter((e: Record<string, unknown>) => e.status === 'active').length;
  const fineTunedCt = experts.filter((e: Record<string, unknown>) => e.isFinetuned).length;
  const avgSuccess = useMemo(() => {
    const rates = experts
      .map((e: Record<string, unknown>) => {
        const s = e.stats as Record<string, number> | undefined;
        return (e.successRate as number) ?? s?.successRate ?? 0;
      })
      .filter((r: number) => r > 0);
    if (rates.length === 0) return 0;
    return rates.reduce((a: number, b: number) => a + b, 0) / rates.length;
  }, [experts]);

  /* Filter + sort */
  const filtered = useMemo(() => {
    let list = [...experts] as Record<string, unknown>[];
    if (statusFilter !== 'all') {
      list = list.filter(e => e.status === statusFilter);
    }
    if (search.trim()) {
      const q = search.trim().toLowerCase();
      list = list.filter(e =>
        (e.name as string).toLowerCase().includes(q) ||
        (e.role as string).toLowerCase().includes(q) ||
        ((e.tags as string[]) ?? []).some(t => t.toLowerCase().includes(q)),
      );
    }
    if (sortBy === 'rating') {
      list.sort((a, b) => {
        const ra = (a.stats as Record<string, number>)?.rating ?? 0;
        const rb = (b.stats as Record<string, number>)?.rating ?? 0;
        return rb - ra;
      });
    } else if (sortBy === 'runs') {
      list.sort((a, b) => {
        const ra = (a.stats as Record<string, number>)?.totalRuns ?? (a.totalRuns as number) ?? 0;
        const rb = (b.stats as Record<string, number>)?.totalRuns ?? (b.totalRuns as number) ?? 0;
        return rb - ra;
      });
    } else if (sortBy === 'name') {
      list.sort((a, b) => (a.name as string).localeCompare(b.name as string));
    } else if (sortBy === 'cost') {
      list.sort((a, b) => {
        const ca = (a.stats as Record<string, number>)?.avgCostPerRun ?? 0;
        const cb = (b.stats as Record<string, number>)?.avgCostPerRun ?? 0;
        return cb - ca;
      });
    }
    return list;
  }, [experts, statusFilter, search, sortBy]);

  const currentSortLabel = SORT_OPTIONS.find(o => o.value === sortBy)?.label ?? 'Sort';

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
            <Star size={19} color={SECTION_COLOR} strokeWidth={2} />
          </div>
          <div>
            <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>
              My Experts
            </h1>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4 }}>
              {total} deployed · {activeCt} active · {fineTunedCt} fine-tuned
            </p>
          </div>
        </div>

        <div style={{ display: 'flex', gap: 8 }}>
          <button
            onClick={() => mutate()}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 15px', borderRadius: 8, border: '1px solid var(--border-md)',
              background: 'var(--bg-surface)', cursor: 'pointer',
              fontSize: 12, fontWeight: 500, color: 'var(--text-2)',
            }}
          >
            <Activity size={12} />
            Refresh
          </button>
          <Link href="/experts/deploy" style={{
            display: 'flex', alignItems: 'center', gap: 6,
            padding: '8px 15px', borderRadius: 8,
            border: `1.5px solid ${SECTION_COLOR}`,
            background: `${SECTION_COLOR}14`,
            color: SECTION_COLOR, fontSize: 12, fontWeight: 700,
            textDecoration: 'none',
          }}>
            <Plus size={13} strokeWidth={2.5} />
            Deploy New
          </Link>
        </div>
      </motion.div>

      {/* ── Stats bar ── */}
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
          { label: 'Total Experts',    value: String(total),             color: SECTION_COLOR, icon: Star,      sub: 'deployed'        },
          { label: 'Active',           value: String(activeCt),          color: '#10b981',     icon: Activity,  sub: 'processing'      },
          { label: 'Fine-tuned',       value: String(fineTunedCt),       color: '#f97316',     icon: Zap,       sub: 'custom models'   },
          { label: 'Avg Success Rate', value: avgSuccess > 0 ? `${(avgSuccess * 100).toFixed(1)}%` : '—', color: '#06b6d4', icon: TrendingUp, sub: 'across all experts' },
        ].map(({ label, value, color, icon: Icon, sub }) => (
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
              <div style={{ fontSize: 22, fontWeight: 800, color, lineHeight: 1 }}>{value}</div>
              <div style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', marginTop: 2 }}>{label}</div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1 }}>{sub}</div>
            </div>
          </div>
        ))}
      </motion.div>

      {/* ── Search + filters + sort ── */}
      <motion.div
        initial={{ opacity: 0 }}
        animate={{ opacity: 1 }}
        transition={{ delay: 0.13 }}
        style={{ display: 'flex', gap: 8, flexWrap: 'wrap', alignItems: 'center', marginBottom: 20 }}
      >
        {/* Search */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          padding: '7px 13px', borderRadius: 8, border: '1px solid var(--border-md)',
          background: 'var(--bg-surface)',
        }}>
          <Search size={13} color="var(--text-4)" />
          <input
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder="Search name, role, or tag…"
            style={{
              border: 'none', outline: 'none', background: 'transparent',
              fontSize: 13, color: 'var(--text-1)', width: 200,
            }}
          />
        </div>

        {/* Status filter tabs */}
        <div style={{ display: 'flex', gap: 5 }}>
          {STATUS_FILTERS.map(f => {
            const cnt = f === 'all'
              ? experts.length
              : experts.filter((e: Record<string, unknown>) => e.status === f).length;
            return (
              <button
                key={f}
                onClick={() => setFilter(f)}
                style={{
                  padding: '6px 13px', borderRadius: 99, fontSize: 12, cursor: 'pointer',
                  border: statusFilter === f ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
                  background: statusFilter === f ? `${SECTION_COLOR}14` : 'var(--bg-surface)',
                  color: statusFilter === f ? SECTION_COLOR : 'var(--text-3)',
                  fontWeight: statusFilter === f ? 700 : 400,
                  transition: 'all 0.15s',
                }}
              >
                {f.charAt(0).toUpperCase() + f.slice(1)}
                <span style={{ marginLeft: 4, fontSize: 10, opacity: 0.7 }}>({cnt})</span>
              </button>
            );
          })}
        </div>

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
            <Clock size={12} />
            Sort: {currentSortLabel}
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
                  borderRadius: 9, padding: 6, minWidth: 140,
                  boxShadow: '0 8px 24px rgba(13,13,13,0.10)',
                }}
              >
                {SORT_OPTIONS.map(opt => (
                  <button
                    key={opt.value}
                    onClick={() => { setSortBy(opt.value); setSortOpen(false); }}
                    style={{
                      display: 'block', width: '100%', textAlign: 'left',
                      padding: '7px 12px', borderRadius: 6, cursor: 'pointer',
                      fontSize: 12, fontWeight: sortBy === opt.value ? 700 : 400,
                      background: sortBy === opt.value ? `${SECTION_COLOR}12` : 'transparent',
                      color: sortBy === opt.value ? SECTION_COLOR : 'var(--text-2)',
                      border: 'none',
                    }}
                  >
                    {opt.label}
                  </button>
                ))}
              </motion.div>
            )}
          </AnimatePresence>
        </div>
      </motion.div>

      {/* ── Expert cards grid ── */}
      {isLoading ? (
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
          gap: 16,
        }}>
          {[0, 1, 2, 3].map(i => <SkeletonCard key={i} />)}
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
                display: 'flex', flexDirection: 'column', alignItems: 'center', gap: 14,
              }}
            >
              <div style={{
                width: 56, height: 56, borderRadius: 14,
                background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <Star size={24} color="var(--text-4)" />
              </div>
              <div>
                <div style={{ fontSize: 15, fontWeight: 600, color: 'var(--text-2)', marginBottom: 6 }}>
                  {search || statusFilter !== 'all' ? 'No experts match your filters' : 'No experts deployed yet'}
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-4)', maxWidth: 340 }}>
                  {search || statusFilter !== 'all'
                    ? 'Try adjusting your search terms or clearing the status filter.'
                    : 'Deploy your first expert to get started building AI-powered workflows.'}
                </div>
              </div>
              {!search && statusFilter === 'all' && (
                <Link href="/experts/deploy" style={{
                  display: 'inline-flex', alignItems: 'center', gap: 7,
                  padding: '9px 20px', borderRadius: 8,
                  border: `1.5px solid ${SECTION_COLOR}`,
                  background: `${SECTION_COLOR}14`,
                  color: SECTION_COLOR, fontSize: 13, fontWeight: 700,
                  textDecoration: 'none', marginTop: 4,
                }}>
                  <Plus size={14} strokeWidth={2.5} />
                  Deploy Your First Expert
                </Link>
              )}
            </motion.div>
          ) : (
            <motion.div
              key={`${statusFilter}-${sortBy}`}
              variants={stagger}
              initial="hidden"
              animate="show"
              style={{
                display: 'grid',
                gridTemplateColumns: 'repeat(auto-fill, minmax(280px, 1fr))',
                gap: 16,
              }}
            >
              {filtered.map((expert: Record<string, unknown>) => (
                <ExpertCard key={expert.id as string} expert={expert} />
              ))}
            </motion.div>
          )}
        </AnimatePresence>
      )}

      {/* ── Footer ── */}
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
        Auto-refreshes every 20 seconds
        {filtered.length !== total && (
          <span style={{ marginLeft: 8 }}>· Showing {filtered.length} of {total}</span>
        )}
      </motion.div>
    </div>
  );
}
