'use client';

import { useState } from 'react';
import Link from 'next/link';
import { motion } from 'framer-motion';
import {
  LayoutTemplate, Search, RefreshCw, Play, Copy,
  ArrowRight, Loader2, Clock, Zap,
} from 'lucide-react';
import { useWorkflows } from '@/lib/hooks/useApi';

const SECTION_COLOR = '#06b6d4';

const fadeUp = {
  hidden: { opacity: 0, y: 12 },
  show:   { opacity: 1, y: 0, transition: { duration: 0.32, ease: [0.25, 0.46, 0.45, 0.94] as const } },
};
const stagger = { hidden: {}, show: { transition: { staggerChildren: 0.07 } } };

const CATEGORIES = ['All', 'Research', 'Engineering', 'Legal', 'Finance', 'Marketing', 'Operations'];

const CATEGORY_COLOR: Record<string, string> = {
  Research: '#8b5cf6', Engineering: '#3b82f6', Legal: '#6b7280',
  Finance: '#f59e0b', Marketing: '#ec4899', Operations: '#10b981',
};

function TemplateCard({ template }: { template: Record<string, unknown> }) {
  const cat = (template.templateCategory as string) ?? 'General';
  const color = CATEGORY_COLOR[cat] ?? '#6b7280';
  const successPct = template.totalRuns as number > 0
    ? Math.round(((template.successfulRuns as number) / (template.totalRuns as number)) * 100)
    : null;

  return (
    <motion.div
      variants={fadeUp}
      whileHover={{ y: -3, boxShadow: '0 12px 32px rgba(13,13,13,0.10)' }}
      transition={{ type: 'spring', stiffness: 400, damping: 30 }}
      style={{
        background: 'var(--bg-surface)', border: '1px solid var(--border-sm)',
        borderRadius: 12, padding: 20,
        display: 'flex', flexDirection: 'column', gap: 14,
      }}
    >
      {/* Category badge + name */}
      <div>
        <span style={{
          padding: '2px 8px', borderRadius: 4, fontSize: 10, fontWeight: 700,
          background: `${color}15`, color,
        }}>
          {cat}
        </span>
        <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', marginTop: 8, lineHeight: 1.3 }}>
          {template.name as string}
        </div>
        <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 4, lineHeight: 1.5 }}>
          {template.description as string}
        </div>
      </div>

      {/* Stats */}
      <div style={{
        display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 8,
        padding: '10px 0', borderTop: '1px solid var(--border-sm)',
      }}>
        {[
          { label: 'Runs', value: String(template.totalRuns ?? 0), icon: Play },
          {
            label: 'Success',
            value: successPct != null ? `${successPct}%` : '—',
            icon: ArrowRight,
          },
          {
            label: 'Est. Time',
            value: template.estimatedDurationSec
              ? `${Math.round((template.estimatedDurationSec as number) / 60)}m`
              : '—',
            icon: Clock,
          },
        ].map(({ label, value, icon: Icon }) => (
          <div key={label} style={{ textAlign: 'center' }}>
            <div style={{ display: 'flex', justifyContent: 'center', marginBottom: 2 }}>
              <Icon size={11} color="var(--text-4)" />
            </div>
            <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)' }}>{value}</div>
            <div style={{ fontSize: 10, color: 'var(--text-4)' }}>{label}</div>
          </div>
        ))}
      </div>

      {/* Est cost + tokens */}
      {!!(template.estimatedCostUsd || template.estimatedTokens) && (
        <div style={{ display: 'flex', gap: 10, fontSize: 11, color: 'var(--text-4)' }}>
          {!!template.estimatedCostUsd && (
            <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <Zap size={10} /> ~${parseFloat(String(template.estimatedCostUsd)).toFixed(2)}
            </span>
          )}
          {!!template.estimatedTokens && (
            <span>~{Math.round((template.estimatedTokens as number) / 1000)}k tokens</span>
          )}
        </div>
      )}

      {/* Tags */}
      {Array.isArray(template.tags) && (template.tags as string[]).length > 0 && (
        <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap' }}>
          {(template.tags as string[]).map(tag => (
            <span key={tag} style={{
              padding: '2px 7px', borderRadius: 4, fontSize: 10,
              background: 'var(--bg)', color: 'var(--text-3)',
              border: '1px solid var(--border-sm)',
            }}>{tag}</span>
          ))}
        </div>
      )}

      {/* Actions */}
      <div style={{ display: 'flex', gap: 8, paddingTop: 4, borderTop: '1px solid var(--border-sm)' }}>
        <Link href="/workflow" style={{
          flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
          padding: '8px 0', borderRadius: 8,
          background: SECTION_COLOR, border: 'none',
          color: '#fff', fontSize: 12, fontWeight: 600, textDecoration: 'none',
        }}>
          <Play size={11} /> Use Template
        </Link>
        <button style={{
          padding: '8px 14px', borderRadius: 8,
          border: '1px solid var(--border-md)',
          background: 'transparent', cursor: 'pointer',
          display: 'flex', alignItems: 'center', gap: 5,
          color: 'var(--text-3)', fontSize: 12,
        }}>
          <Copy size={11} /> Clone
        </button>
      </div>
    </motion.div>
  );
}

export default function WorkflowTemplatesPage() {
  const [category, setCategory] = useState('All');
  const [search, setSearch] = useState('');
  const { workflows, total, isLoading, mutate } = useWorkflows(true);

  const filtered = workflows.filter((w: Record<string, unknown>) => {
    const matchCat = category === 'All' || w.templateCategory === category;
    const matchSearch = !search || (w.name as string).toLowerCase().includes(search.toLowerCase());
    return matchCat && matchSearch;
  });

  return (
    <div style={{ padding: 28, maxWidth: 1200 }}>
      {/* Header */}
      <motion.div
        initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.3 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 24 }}
      >
        <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
          <div style={{
            width: 36, height: 36, borderRadius: 8,
            background: `${SECTION_COLOR}18`, border: `1.5px solid ${SECTION_COLOR}30`,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <LayoutTemplate size={18} color={SECTION_COLOR} strokeWidth={2} />
          </div>
          <div>
            <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1 }}>
              Workflow Templates
            </h1>
            <p style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 3 }}>
              {total} templates · Pre-built pipelines ready to run
            </p>
          </div>
        </div>
        <button onClick={() => mutate()} style={{
          display: 'flex', alignItems: 'center', gap: 6,
          padding: '7px 14px', borderRadius: 7, border: '1px solid var(--border-md)',
          background: 'var(--bg-surface)', cursor: 'pointer', fontSize: 12, color: 'var(--text-2)',
        }}>
          <RefreshCw size={12} /> Refresh
        </button>
      </motion.div>

      {/* Category + search filters */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.1 }}
        style={{ display: 'flex', gap: 8, marginBottom: 20, alignItems: 'center', flexWrap: 'wrap' }}
      >
        {CATEGORIES.map(c => (
          <button key={c} onClick={() => setCategory(c)}
            style={{
              padding: '5px 12px', borderRadius: 20, fontSize: 12, cursor: 'pointer',
              border: category === c ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border-md)',
              background: category === c ? `${SECTION_COLOR}15` : 'var(--bg-surface)',
              color: category === c ? SECTION_COLOR : 'var(--text-3)',
              fontWeight: category === c ? 600 : 400, transition: 'all 0.15s',
            }}
          >{c}</button>
        ))}
        <div style={{
          marginLeft: 'auto', display: 'flex', alignItems: 'center', gap: 8,
          padding: '6px 12px', borderRadius: 8, border: '1px solid var(--border-md)',
          background: 'var(--bg-surface)',
        }}>
          <Search size={13} color="var(--text-4)" />
          <input value={search} onChange={e => setSearch(e.target.value)}
            placeholder="Search templates…"
            style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 13, color: 'var(--text-1)', width: 180 }}
          />
        </div>
      </motion.div>

      {isLoading ? (
        <div style={{ textAlign: 'center', padding: '60px 0', color: 'var(--text-4)' }}>
          <Loader2 size={22} className="spin" style={{ margin: '0 auto 8px' }} />
          <div style={{ fontSize: 13 }}>Loading templates…</div>
        </div>
      ) : (
        <motion.div variants={stagger} initial="hidden" animate="show"
          style={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(320px, 1fr))',
            gap: 16,
          }}
        >
          {filtered.map((template: Record<string, unknown>) => (
            <TemplateCard key={template.id as string} template={template} />
          ))}
        </motion.div>
      )}
    </div>
  );
}
