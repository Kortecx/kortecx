'use client';

import { useState } from 'react';
import Link from 'next/link';
import {
  Search, Filter, Plus, Star, Zap, Clock, TrendingUp,
  ChevronRight, RotateCcw, Play, Settings, Copy,
} from 'lucide-react';
import { EXPERTS, ROLE_META, PROVIDERS } from '@/lib/constants';
import type { Expert, ExpertRole, ExpertStatus } from '@/lib/types';

const ALL_ROLES: ExpertRole[] = [
  'researcher', 'analyst', 'writer', 'coder', 'reviewer',
  'planner', 'legal', 'financial', 'medical', 'creative', 'custom',
];

const STATUS_OPTIONS: ExpertStatus[] = ['active', 'idle', 'training', 'fine-tuning', 'offline'];

function statusBadge(status: ExpertStatus) {
  switch (status) {
    case 'active':
      return <span className="badge badge-success">Active</span>;
    case 'idle':
      return <span className="badge badge-neutral">Idle</span>;
    case 'training':
      return <span className="badge badge-amber">Training</span>;
    case 'fine-tuning':
      return <span className="badge badge-amber">Fine-tuning</span>;
    case 'deploying':
      return <span className="badge badge-info">Deploying</span>;
    case 'offline':
      return <span className="badge badge-error">Offline</span>;
    default:
      return <span className="badge badge-neutral">{status}</span>;
  }
}

function StarRating({ rating }: { rating: number }) {
  return (
    <div style={{ display: 'flex', alignItems: 'center', gap: 2 }}>
      {[1, 2, 3, 4, 5].map(i => (
        <Star
          key={i}
          size={11}
          fill={i <= Math.round(rating) ? 'var(--amber)' : 'none'}
          color={i <= Math.round(rating) ? 'var(--amber)' : 'var(--text-4)'}
        />
      ))}
      <span style={{ fontSize: 11, color: 'var(--text-3)', marginLeft: 3 }}>
        {rating.toFixed(1)}
      </span>
    </div>
  );
}

function ExpertCard({ expert }: { expert: Expert }) {
  const roleMeta = ROLE_META[expert.role];
  const provider = PROVIDERS.find(p => p.id === expert.providerId);

  return (
    <div className="expert-card" style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12 }}>
        <div
          className="expert-avatar"
          style={{ background: roleMeta.dimColor, color: roleMeta.color, fontSize: 18 }}
        >
          {roleMeta.emoji}
        </div>
        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 2 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>
              {expert.name}
            </span>
            {expert.isFinetuned && (
              <span className="badge badge-violet" style={{ fontSize: 9, padding: '1px 5px' }}>
                Fine-tuned
              </span>
            )}
            {expert.isPublic && (
              <span className="badge badge-neutral" style={{ fontSize: 9, padding: '1px 5px' }}>
                Public
              </span>
            )}
          </div>
          <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
            <span style={{
              fontSize: 10, fontWeight: 600, color: roleMeta.color,
              textTransform: 'uppercase', letterSpacing: '0.08em',
            }}>
              {roleMeta.label}
            </span>
            <span style={{ color: 'var(--text-4)' }}>·</span>
            <span style={{ fontSize: 11, color: 'var(--text-3)' }}>v{expert.version}</span>
          </div>
        </div>
        {statusBadge(expert.status)}
      </div>

      {/* Description */}
      <p style={{
        fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5,
        margin: 0,
        display: '-webkit-box',
        WebkitLineClamp: 2,
        WebkitBoxOrient: 'vertical',
        overflow: 'hidden',
      }}>
        {expert.description}
      </p>

      {/* Model */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8,
        padding: '7px 10px',
        background: 'var(--bg)',
        border: '1px solid var(--border)',
        borderRadius: 4,
      }}>
        <span style={{
          width: 8, height: 8, borderRadius: '50%',
          background: provider?.color ?? 'var(--text-4)',
          flexShrink: 0,
        }} />
        <span style={{ fontSize: 12, color: 'var(--text-2)', flex: 1 }}>
          {expert.modelName}
        </span>
        <span style={{ fontSize: 11, color: 'var(--text-3)' }}>
          {expert.providerName}
        </span>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 8 }}>
        {[
          {
            icon: Play, label: 'Runs', value: expert.stats.totalRuns.toLocaleString(),
            color: 'var(--text-2)',
          },
          {
            icon: Zap, label: 'Avg tokens', value: `${(expert.stats.avgTokensPerRun / 1000).toFixed(1)}k`,
            color: 'var(--amber)',
          },
          {
            icon: Clock, label: 'Latency', value: `${(expert.stats.avgLatencyMs / 1000).toFixed(1)}s`,
            color: 'var(--text-2)',
          },
        ].map(stat => (
          <div key={stat.label} style={{
            padding: '7px 8px',
            background: 'var(--bg)',
            border: '1px solid var(--border)',
            borderRadius: 4,
            textAlign: 'center',
          }}>
            <div style={{ fontSize: 13, fontWeight: 700, color: stat.color, fontVariantNumeric: 'tabular-nums' }}>
              {stat.value}
            </div>
            <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 2 }}>{stat.label}</div>
          </div>
        ))}
      </div>

      {/* Rating + Success */}
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
        <StarRating rating={expert.stats.rating} />
        <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
          <TrendingUp size={11} color="var(--success)" />
          <span style={{ fontSize: 11, color: 'var(--success)' }}>
            {(expert.stats.successRate * 100).toFixed(1)}% success
          </span>
        </div>
      </div>

      {/* Tags */}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4 }}>
        {expert.specializations.slice(0, 3).map(tag => (
          <span key={tag} className="badge badge-neutral" style={{ fontSize: 10 }}>
            {tag}
          </span>
        ))}
        {expert.specializations.length > 3 && (
          <span className="badge badge-neutral" style={{ fontSize: 10 }}>
            +{expert.specializations.length - 3}
          </span>
        )}
      </div>

      {/* Actions */}
      <div style={{ display: 'flex', gap: 6, marginTop: 'auto' }}>
        <Link href="/workflow" style={{ flex: 1 }}>
          <button className="btn btn-primary btn-sm" style={{ width: '100%', justifyContent: 'center' }}>
            <Play size={12} /> Use in Workflow
          </button>
        </Link>
        <button className="btn btn-secondary btn-icon btn-sm" title="Clone expert">
          <Copy size={13} />
        </button>
        <button className="btn btn-secondary btn-icon btn-sm" title="Configure">
          <Settings size={13} />
        </button>
      </div>
    </div>
  );
}

export default function ExpertsPage() {
  const [search, setSearch] = useState('');
  const [selectedRole, setSelectedRole] = useState<ExpertRole | 'all'>('all');
  const [selectedStatus, setSelectedStatus] = useState<ExpertStatus | 'all'>('all');
  const [sortBy, setSortBy] = useState<'rating' | 'runs' | 'name'>('rating');

  const filtered = EXPERTS
    .filter(e => {
      if (search && !e.name.toLowerCase().includes(search.toLowerCase()) &&
        !e.description.toLowerCase().includes(search.toLowerCase())) return false;
      if (selectedRole !== 'all' && e.role !== selectedRole) return false;
      if (selectedStatus !== 'all' && e.status !== selectedStatus) return false;
      return true;
    })
    .sort((a, b) => {
      if (sortBy === 'rating') return b.stats.rating - a.stats.rating;
      if (sortBy === 'runs') return b.stats.totalRuns - a.stats.totalRuns;
      return a.name.localeCompare(b.name);
    });

  const activeCount    = EXPERTS.filter(e => e.status === 'active').length;
  const trainingCount  = EXPERTS.filter(e => e.status === 'training').length;

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'flex-start', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Expert Catalog
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            {EXPERTS.length} experts — {activeCount} active, {trainingCount} in training
          </p>
        </div>
        <div style={{ display: 'flex', gap: 8 }}>
          <Link href="/experts/deploy">
            <button className="btn btn-primary btn-sm">
              <Plus size={13} /> Deploy New Expert
            </button>
          </Link>
        </div>
      </div>

      {/* Filters */}
      <div style={{
        display: 'flex', gap: 10, flexWrap: 'wrap',
        marginBottom: 20,
        padding: '14px 16px',
        background: 'var(--bg-card)',
        border: '1px solid var(--border)',
        borderRadius: 6,
      }}>
        {/* Search */}
        <div style={{
          display: 'flex', alignItems: 'center', gap: 8,
          background: 'var(--bg)', border: '1px solid var(--border-md)',
          borderRadius: 4, padding: '6px 10px', flex: 1, minWidth: 200,
        }}>
          <Search size={13} color="var(--text-3)" />
          <input
            className="input"
            style={{ background: 'none', border: 'none', padding: 0, fontSize: 13 }}
            placeholder="Search experts..."
            value={search}
            onChange={e => setSearch(e.target.value)}
          />
        </div>

        {/* Role filter */}
        <select
          className="input"
          style={{ width: 'auto', minWidth: 140 }}
          value={selectedRole}
          onChange={e => setSelectedRole(e.target.value as ExpertRole | 'all')}
        >
          <option value="all">All Roles</option>
          {ALL_ROLES.map(r => (
            <option key={r} value={r}>{ROLE_META[r].label}</option>
          ))}
        </select>

        {/* Status filter */}
        <select
          className="input"
          style={{ width: 'auto', minWidth: 130 }}
          value={selectedStatus}
          onChange={e => setSelectedStatus(e.target.value as ExpertStatus | 'all')}
        >
          <option value="all">All Statuses</option>
          {STATUS_OPTIONS.map(s => (
            <option key={s} value={s}>{s.charAt(0).toUpperCase() + s.slice(1)}</option>
          ))}
        </select>

        {/* Sort */}
        <select
          className="input"
          style={{ width: 'auto', minWidth: 130 }}
          value={sortBy}
          onChange={e => setSortBy(e.target.value as typeof sortBy)}
        >
          <option value="rating">Sort: Rating</option>
          <option value="runs">Sort: Most Used</option>
          <option value="name">Sort: Name</option>
        </select>

        {(search || selectedRole !== 'all' || selectedStatus !== 'all') && (
          <button
            className="btn btn-ghost btn-sm"
            onClick={() => { setSearch(''); setSelectedRole('all'); setSelectedStatus('all'); }}
          >
            <RotateCcw size={12} /> Reset
          </button>
        )}
      </div>

      {/* Role pills */}
      <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 20 }}>
        <button
          onClick={() => setSelectedRole('all')}
          style={{
            padding: '4px 12px',
            borderRadius: 20,
            fontSize: 12,
            fontWeight: 500,
            border: '1px solid',
            cursor: 'pointer',
            background: selectedRole === 'all' ? 'var(--primary-dim)' : 'var(--bg-card)',
            borderColor: selectedRole === 'all' ? 'var(--primary)' : 'var(--border)',
            color: selectedRole === 'all' ? 'var(--primary-text)' : 'var(--text-2)',
          }}
        >
          All
        </button>
        {ALL_ROLES.slice(0, 8).map(role => {
          const meta = ROLE_META[role];
          const active = selectedRole === role;
          return (
            <button
              key={role}
              onClick={() => setSelectedRole(active ? 'all' : role)}
              style={{
                padding: '4px 12px',
                borderRadius: 20,
                fontSize: 12,
                fontWeight: 500,
                border: '1px solid',
                cursor: 'pointer',
                background: active ? `${meta.dimColor}` : 'var(--bg-card)',
                borderColor: active ? meta.color : 'var(--border)',
                color: active ? meta.color : 'var(--text-2)',
                transition: 'all 0.12s',
              }}
            >
              {meta.emoji} {meta.label}
            </button>
          );
        })}
      </div>

      {/* Results count */}
      {filtered.length !== EXPERTS.length && (
        <div style={{ fontSize: 12, color: 'var(--text-3)', marginBottom: 14 }}>
          Showing {filtered.length} of {EXPERTS.length} experts
        </div>
      )}

      {/* Grid */}
      {filtered.length === 0 ? (
        <div style={{
          textAlign: 'center', padding: 60,
          color: 'var(--text-3)', fontSize: 14,
        }}>
          No experts match your filters.
        </div>
      ) : (
        <div style={{
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(300px, 1fr))',
          gap: 12,
        }}>
          {filtered.map(expert => (
            <ExpertCard key={expert.id} expert={expert} />
          ))}

          {/* Add custom expert card */}
          <Link href="/experts/deploy" style={{ textDecoration: 'none' }}>
            <div style={{
              border: '1px dashed var(--border-md)',
              borderRadius: 6,
              padding: 20,
              display: 'flex',
              flexDirection: 'column',
              alignItems: 'center',
              justifyContent: 'center',
              gap: 12,
              cursor: 'pointer',
              minHeight: 280,
              transition: 'border-color 0.15s, background 0.15s',
              background: 'transparent',
            }}
              onMouseEnter={e => {
                e.currentTarget.style.borderColor = 'var(--primary)';
                e.currentTarget.style.background = 'var(--primary-dim)';
              }}
              onMouseLeave={e => {
                e.currentTarget.style.borderColor = 'var(--border-md)';
                e.currentTarget.style.background = 'transparent';
              }}
            >
              <div style={{
                width: 48, height: 48,
                borderRadius: 8,
                background: 'var(--bg-elevated)',
                border: '1px solid var(--border-md)',
                display: 'flex', alignItems: 'center', justifyContent: 'center',
              }}>
                <Plus size={20} color="var(--text-3)" />
              </div>
              <div style={{ textAlign: 'center' }}>
                <div style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>
                  Deploy New Expert
                </div>
                <div style={{ fontSize: 12, color: 'var(--text-3)', lineHeight: 1.5 }}>
                  Configure a new specialized agent
                  <br />from any provider or model
                </div>
              </div>
              <div style={{ display: 'flex', alignItems: 'center', gap: 4, fontSize: 12, color: 'var(--primary-text)' }}>
                Get started <ChevronRight size={12} />
              </div>
            </div>
          </Link>
        </div>
      )}
    </div>
  );
}
