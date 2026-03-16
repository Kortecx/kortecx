'use client';

import { useState, useMemo } from 'react';
import Link from 'next/link';
import { useRouter } from 'next/navigation';
import {
  Workflow, Plus, Search, Play, Trash2, X, Clock, Zap,
  ChevronDown, ChevronUp, Loader2, CheckCircle2, AlertCircle,
  Tag, ArrowUpDown, Calendar, Layers, Filter,
} from 'lucide-react';
import { useWorkflows } from '@/lib/hooks/useApi';

const SECTION_COLOR = '#2563EB';

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

const STATUS_STYLE: Record<string, { color: string; bg: string; label: string }> = {
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

/* ── Create Workflow Dialog ──────────────────────────── */
function CreateWorkflowDialog({
  onClose,
  onCreate,
}: {
  onClose: () => void;
  onCreate: (data: { name: string; description: string; goalStatement: string; tags: string[] }) => Promise<void>;
}) {
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [goalStatement, setGoalStatement] = useState('');
  const [tagsStr, setTagsStr] = useState('');
  const [saving, setSaving] = useState(false);
  const [errors, setErrors] = useState<{ name?: string; goal?: string; general?: string }>({});

  const handleCreate = async () => {
    const errs: typeof errors = {};
    if (!name.trim()) errs.name = 'Workflow name is required';
    if (!goalStatement.trim()) errs.goal = 'Task goal is required';
    if (Object.keys(errs).length > 0) { setErrors(errs); return; }

    setSaving(true);
    setErrors({});
    try {
      await onCreate({
        name: name.trim(),
        description: description.trim(),
        goalStatement: goalStatement.trim(),
        tags: tagsStr.split(',').map(t => t.trim()).filter(Boolean),
      });
    } catch (err) {
      setErrors({ general: err instanceof Error ? err.message : 'Create failed' });
      setSaving(false);
    }
  };

  const LABEL: React.CSSProperties = {
    fontSize: 11, fontWeight: 600, color: 'var(--text-3)',
    display: 'block', marginBottom: 4,
  };

  return (
    <div style={{
      position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
      zIndex: 200, display: 'flex', alignItems: 'flex-start', justifyContent: 'center',
      paddingTop: 60, overflowY: 'auto',
    }}>
      <div style={{
        width: 560, maxWidth: '92vw',
        background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
        borderRadius: 12, overflow: 'hidden',
        boxShadow: '0 24px 80px rgba(0,0,0,0.2)',
        marginBottom: 40,
      }}>
        {/* Header */}
        <div style={{
          padding: '18px 22px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
            <Plus size={16} color={SECTION_COLOR} />
            <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>Create New Workflow</span>
          </div>
          <button onClick={onClose} style={{
            background: 'none', border: 'none', cursor: 'pointer',
            color: 'var(--text-3)', display: 'flex', padding: 4,
          }}><X size={16} /></button>
        </div>

        {/* Form */}
        <div style={{ padding: '20px 22px', display: 'flex', flexDirection: 'column', gap: 16 }}>
          <div>
            <label style={LABEL}>Workflow Name <span style={{ color: '#ef4444' }}>*</span></label>
            <input className="input" style={{
              width: '100%', fontSize: 13,
              borderColor: errors.name ? 'var(--error)' : undefined,
            }}
              placeholder="e.g. Research & Summarize Pipeline"
              value={name} onChange={e => { setName(e.target.value); if (e.target.value.trim()) setErrors(p => { const { name: _, ...r } = p; return r; }); }} />
            {errors.name && <div style={{ fontSize: 11, color: 'var(--error)', marginTop: 3, display: 'flex', alignItems: 'center', gap: 3 }}>
              <AlertCircle size={10} /> {errors.name}
            </div>}
          </div>

          <div>
            <label style={LABEL}>Description</label>
            <textarea className="textarea" style={{ width: '100%', minHeight: 60, fontSize: 12 }}
              placeholder="What does this workflow accomplish?"
              value={description} onChange={e => setDescription(e.target.value)} />
          </div>

          <div>
            <label style={LABEL}>Task Goal <span style={{ color: '#ef4444' }}>*</span></label>
            <textarea className="textarea" style={{
              width: '100%', minHeight: 100, fontSize: 12,
              fontFamily: 'var(--font-mono, monospace)', lineHeight: 1.5,
              borderColor: errors.goal ? 'var(--error)' : undefined,
            }}
              placeholder={"## Objective\nDescribe what you want to accomplish...\n\n## Requirements\n- Requirement 1\n- Requirement 2"}
              value={goalStatement} onChange={e => { setGoalStatement(e.target.value); if (e.target.value.trim()) setErrors(p => { const { goal: _, ...r } = p; return r; }); }} />
            {errors.goal && <div style={{ fontSize: 11, color: 'var(--error)', marginTop: 3, display: 'flex', alignItems: 'center', gap: 3 }}>
              <AlertCircle size={10} /> {errors.goal}
            </div>}
          </div>

          <div>
            <label style={LABEL}>Tags (comma-separated)</label>
            <input className="input" style={{ width: '100%', fontSize: 12 }}
              placeholder="e.g. research, production, weekly"
              value={tagsStr} onChange={e => setTagsStr(e.target.value)} />
          </div>

          {errors.general && <div style={{ fontSize: 11, color: 'var(--error)', display: 'flex', alignItems: 'center', gap: 4 }}>
            <AlertCircle size={11} /> {errors.general}
          </div>}
        </div>

        {/* Footer */}
        <div style={{
          padding: '14px 22px', borderTop: '1px solid var(--border)',
          display: 'flex', gap: 8, justifyContent: 'space-between', alignItems: 'center',
        }}>
          <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
            You can add expert steps after creating the workflow.
          </span>
          <div style={{ display: 'flex', gap: 8 }}>
            <button onClick={onClose} style={{
              padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 500,
              border: '1px solid var(--border-md)', background: 'transparent',
              color: 'var(--text-3)', cursor: 'pointer',
            }}>Cancel</button>
            <button onClick={handleCreate} disabled={saving} style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 18px', borderRadius: 7, fontSize: 12, fontWeight: 700,
              border: `1.5px solid ${SECTION_COLOR}`,
              background: `${SECTION_COLOR}14`, color: SECTION_COLOR,
              cursor: saving ? 'wait' : 'pointer', opacity: saving ? 0.6 : 1,
            }}>
              {saving ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Plus size={12} />}
              Create Workflow
            </button>
          </div>
        </div>
      </div>
    </div>
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
    <div style={{
      position: 'fixed', inset: 0, background: 'rgba(7,7,26,0.85)',
      zIndex: 200, display: 'flex', alignItems: 'center', justifyContent: 'center',
    }}>
      <div style={{
        width: 400, background: 'var(--bg-surface)', border: '1px solid var(--border-md)',
        borderRadius: 10, padding: 24,
        boxShadow: '0 20px 60px rgba(0,0,0,0.2)',
      }}>
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
      </div>
    </div>
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
  const [showCreate, setShowCreate] = useState(false);
  const [deletingWf, setDeletingWf] = useState<Record<string, unknown> | null>(null);

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

  const SortIcon = ({ field }: { field: SortField }) => {
    if (sortField !== field) return <ArrowUpDown size={10} color="var(--text-4)" />;
    return sortDir === 'asc' ? <ChevronUp size={10} color={SECTION_COLOR} /> : <ChevronDown size={10} color={SECTION_COLOR} />;
  };

  /* Create handler */
  const handleCreate = async (data: { name: string; description: string; goalStatement: string; tags: string[] }) => {
    const res = await fetch('/api/workflows', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(data),
    });
    if (!res.ok) {
      const body = await res.json();
      throw new Error(body.error || 'Create failed');
    }
    setShowCreate(false);
    mutate();
  };

  /* Delete handler */
  const handleDelete = async (id: string) => {
    await fetch(`/api/workflows?id=${id}`, { method: 'DELETE' });
    setDeletingWf(null);
    mutate();
  };

  /* Status counts */
  const statusCounts = useMemo(() => {
    const counts: Record<string, number> = { all: workflows.length };
    for (const w of workflows) {
      const s = (w as Record<string, unknown>).status as string;
      counts[s] = (counts[s] ?? 0) + 1;
    }
    return counts;
  }, [workflows]);

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
      <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
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
            <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
              {total} workflow{total !== 1 ? 's' : ''} · Build and manage agent pipelines
            </p>
          </div>
        </div>
        <button onClick={() => setShowCreate(true)} style={{
          display: 'flex', alignItems: 'center', gap: 6,
          padding: '9px 18px', borderRadius: 8,
          border: `1.5px solid ${SECTION_COLOR}`,
          background: `${SECTION_COLOR}14`,
          color: SECTION_COLOR, fontSize: 13, fontWeight: 700,
          cursor: 'pointer',
        }}>
          <Plus size={14} strokeWidth={2.5} /> Create New Workflow
        </button>
      </div>

      {/* Filters bar */}
      <div style={{ display: 'flex', gap: 8, marginBottom: 16, alignItems: 'center', flexWrap: 'wrap' }}>
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
        {['all', 'draft', 'ready', 'running', 'completed', 'failed'].map(s => {
          const count = statusCounts[s] ?? 0;
          if (s !== 'all' && count === 0) return null;
          const active = statusFilter === s;
          const st = s === 'all' ? { color: SECTION_COLOR, bg: `${SECTION_COLOR}12`, label: 'All' } : (STATUS_STYLE[s] ?? STATUS_STYLE.draft);
          return (
            <button key={s} onClick={() => setStatusFilter(s)} style={{
              padding: '5px 12px', borderRadius: 20, fontSize: 11, cursor: 'pointer',
              border: `1px solid ${active ? st.color : 'var(--border)'}`,
              background: active ? st.bg : 'transparent',
              color: active ? st.color : 'var(--text-3)',
              fontWeight: active ? 700 : 400, transition: 'all 0.12s',
            }}>
              {st.label}
              <span style={{ marginLeft: 4, fontSize: 10, opacity: 0.7 }}>({count})</span>
            </button>
          );
        })}
      </div>

      {/* Table */}
      {isLoading ? (
        <div style={{ textAlign: 'center', padding: '60px 0', color: 'var(--text-4)' }}>
          <Loader2 size={20} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite' }} />
          <div style={{ fontSize: 13 }}>Loading workflows...</div>
        </div>
      ) : filtered.length === 0 ? (
        <div style={{
          textAlign: 'center', padding: '80px 20px',
          border: '1px dashed var(--border-md)', borderRadius: 10,
        }}>
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
            <button onClick={() => setShowCreate(true)} style={{
              display: 'inline-flex', alignItems: 'center', gap: 6,
              padding: '10px 20px', borderRadius: 8,
              border: `1.5px solid ${SECTION_COLOR}`,
              background: `${SECTION_COLOR}14`,
              color: SECTION_COLOR, fontSize: 13, fontWeight: 700,
              cursor: 'pointer',
            }}>
              <Plus size={14} /> Create Your First Workflow
            </button>
          )}
        </div>
      ) : (
        <div style={{
          background: 'var(--bg-surface)', border: '1px solid var(--border)',
          borderRadius: 10, overflow: 'hidden',
        }}>
          <table style={{ width: '100%', borderCollapse: 'collapse' }}>
            <thead>
              <tr style={{ background: 'var(--bg-elevated)' }}>
                <th style={TH} onClick={() => toggleSort('name')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Name <SortIcon field="name" /></span>
                </th>
                <th style={TH} onClick={() => toggleSort('status')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Status <SortIcon field="status" /></span>
                </th>
                <th style={TH}>Goal</th>
                <th style={TH} onClick={() => toggleSort('totalRuns')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Runs <SortIcon field="totalRuns" /></span>
                </th>
                <th style={TH} onClick={() => toggleSort('estimatedTokens')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Tokens <SortIcon field="estimatedTokens" /></span>
                </th>
                <th style={TH}>Tags</th>
                <th style={TH} onClick={() => toggleSort('updatedAt')}>
                  <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>Updated <SortIcon field="updatedAt" /></span>
                </th>
                <th style={{ ...TH, textAlign: 'right' }}>Actions</th>
              </tr>
            </thead>
            <tbody>
              {filtered.map(wf => {
                const st = STATUS_STYLE[(wf.status as string) ?? 'draft'] ?? STATUS_STYLE.draft;
                const tags = ((wf.tags as string[]) ?? []).slice(0, 3);
                const goal = (wf.goalStatement as string) ?? '';
                const runs = (wf.totalRuns as number) ?? 0;
                const tokens = (wf.estimatedTokens as number) ?? 0;

                return (
                  <tr key={wf.id as string}
                    style={{ transition: 'background 0.1s', cursor: 'pointer' }}
                    onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-elevated)')}
                    onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                    onClick={() => router.push(`/workflow/builder?id=${wf.id}`)}
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
                      <span style={{
                        padding: '3px 8px', borderRadius: 99, fontSize: 10, fontWeight: 700,
                        background: st.bg, color: st.color, border: `1px solid ${st.color}28`,
                      }}>{st.label}</span>
                    </td>
                    <td style={{ ...TD, maxWidth: 200 }}>
                      <div style={{ fontSize: 12, color: 'var(--text-2)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontStyle: goal ? 'normal' : 'italic' }}>
                        {goal ? goal.slice(0, 80) + (goal.length > 80 ? '...' : '') : 'No goal set'}
                      </div>
                    </td>
                    <td style={TD}>
                      <span className="mono" style={{ fontSize: 12, fontWeight: 600 }}>{runs}</span>
                    </td>
                    <td style={TD}>
                      <span className="mono" style={{ fontSize: 12 }}>{tokens > 0 ? fmt(tokens) : '—'}</span>
                    </td>
                    <td style={TD}>
                      <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
                        {tags.map(tag => (
                          <span key={tag} style={{
                            padding: '2px 7px', borderRadius: 4, fontSize: 10, fontWeight: 500,
                            background: `${SECTION_COLOR}10`, color: SECTION_COLOR,
                            border: `1px solid ${SECTION_COLOR}25`,
                          }}>{tag}</span>
                        ))}
                        {tags.length === 0 && <span style={{ fontSize: 11, color: 'var(--text-3)', fontStyle: 'italic' }}>none</span>}
                      </div>
                    </td>
                    <td style={TD}>
                      <span style={{ fontSize: 11, color: 'var(--text-2)', fontWeight: 500 }}>
                        {timeAgo(wf.updatedAt as string)}
                      </span>
                    </td>
                    <td style={{ ...TD, textAlign: 'right' }} onClick={e => e.stopPropagation()}>
                      <div style={{ display: 'flex', gap: 4, justifyContent: 'flex-end' }}>
                        <Link href={`/workflow/builder?id=${wf.id}`} style={{
                          display: 'flex', alignItems: 'center', gap: 4,
                          padding: '5px 10px', borderRadius: 5, fontSize: 11, fontWeight: 600,
                          border: `1px solid ${SECTION_COLOR}50`,
                          background: `${SECTION_COLOR}10`, color: SECTION_COLOR,
                          textDecoration: 'none',
                        }}>
                          <Play size={10} /> Open
                        </Link>
                        <button onClick={() => setDeletingWf(wf)} style={{
                          display: 'flex', alignItems: 'center',
                          padding: '5px 8px', borderRadius: 5,
                          border: '1px solid var(--border)', background: 'transparent',
                          color: 'var(--text-4)', cursor: 'pointer', fontSize: 11,
                        }}>
                          <Trash2 size={10} />
                        </button>
                      </div>
                    </td>
                  </tr>
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
        </div>
      )}

      {/* Modals */}
      {showCreate && (
        <CreateWorkflowDialog onClose={() => setShowCreate(false)} onCreate={handleCreate} />
      )}
      {deletingWf && (
        <DeleteConfirmDialog
          name={deletingWf.name as string}
          onClose={() => setDeletingWf(null)}
          onConfirm={() => handleDelete(deletingWf.id as string)}
        />
      )}
    </div>
  );
}
