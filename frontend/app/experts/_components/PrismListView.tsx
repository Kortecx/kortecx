'use client';

import { useState, useMemo } from 'react';
import { ChevronDown, ChevronRight, Play, Settings, Trash2, Star, ArrowUpDown, Plus, X, Link2 } from 'lucide-react';
import type { SimilarityEdge } from './PrismGraph';

const ROLE_EMOJI: Record<string, string> = {
  researcher: '🔬', analyst: '📊', writer: '✍️', coder: '💻',
  reviewer: '🔍', planner: '🗂', legal: '⚖️', financial: '💰',
  medical: '🩺', coordinator: '🔄', 'data-engineer': '🛠', creative: '🎨',
  translator: '🌐', custom: '⚙️',
};

const ROLE_COLOR: Record<string, string> = {
  researcher: '#a78bfa', analyst: '#60a5fa', writer: '#fbbf24', coder: '#34d399',
  reviewer: '#22d3ee', planner: '#818cf8', legal: '#f87171', financial: '#fb923c',
  medical: '#f472b6', coordinator: '#c084fc', 'data-engineer': '#2dd4bf',
  creative: '#e879f9', translator: '#67e8f9', custom: '#94a3b8',
};

const STATUS_DOT: Record<string, string> = {
  active: '#10b981', idle: '#6b7280', running: '#3b82f6', completed: '#10b981',
  failed: '#ef4444', queued: '#f59e0b', training: '#f59e0b', error: '#ef4444', offline: '#ef4444',
};

type SortKey = 'name' | 'role' | 'category' | 'status' | 'totalRuns' | 'rating';
type GroupKey = 'none' | 'role' | 'category' | 'status';

interface PrismListViewProps {
  prisms: Array<Record<string, unknown>>;
  edges: SimilarityEdge[];
  onConfigure: (p: Record<string, unknown>) => void;
  onRun: (p: Record<string, unknown>) => void;
  onDelete: (p: Record<string, unknown>) => void;
  onCreateEdge: (sourceId: string, targetId: string) => void;
}

export default function PrismListView({ prisms, edges, onConfigure, onRun, onDelete, onCreateEdge }: PrismListViewProps) {
  const [sortBy, setSortBy] = useState<SortKey>('name');
  const [sortDir, setSortDir] = useState<'asc' | 'desc'>('asc');
  const [groupBy, setGroupBy] = useState<GroupKey>('role');
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const [connectingId, setConnectingId] = useState<string | null>(null);
  const [connectInput, setConnectInput] = useState('');

  // Build a map of connections per PRISM
  const connectionsMap = useMemo(() => {
    const map = new Map<string, string[]>();
    const nameMap = new Map<string, string>();
    for (const p of prisms) nameMap.set(p.id as string, (p.name as string) ?? '');
    for (const e of edges) {
      const sn = nameMap.get(e.source);
      const tn = nameMap.get(e.target);
      if (sn !== undefined) {
        if (!map.has(e.source)) map.set(e.source, []);
        if (tn) map.get(e.source)!.push(tn);
      }
      if (tn !== undefined) {
        if (!map.has(e.target)) map.set(e.target, []);
        if (sn) map.get(e.target)!.push(sn);
      }
    }
    return map;
  }, [prisms, edges]);

  const toggleSort = (key: SortKey) => {
    if (sortBy === key) setSortDir(d => d === 'asc' ? 'desc' : 'asc');
    else { setSortBy(key); setSortDir('asc'); }
  };

  const toggleGroup = (key: string) => {
    setCollapsed(prev => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key); else next.add(key);
      return next;
    });
  };

  const handleConnect = (sourceId: string) => {
    const target = prisms.find(p => (p.name as string)?.toLowerCase() === connectInput.trim().toLowerCase());
    if (!target) return;
    onCreateEdge(sourceId, target.id as string);
    setConnectingId(null);
    setConnectInput('');
  };

  const sorted = useMemo(() => {
    const list = [...prisms];
    list.sort((a, b) => {
      const av = a[sortBy] as string | number ?? '';
      const bv = b[sortBy] as string | number ?? '';
      const cmp = typeof av === 'number' && typeof bv === 'number' ? av - bv : String(av).localeCompare(String(bv));
      return sortDir === 'asc' ? cmp : -cmp;
    });
    return list;
  }, [prisms, sortBy, sortDir]);

  const groups = useMemo(() => {
    if (groupBy === 'none') return [{ key: 'all', label: 'All PRISMs', items: sorted }];
    const map = new Map<string, Array<Record<string, unknown>>>();
    for (const p of sorted) {
      const gk = (p[groupBy] as string) ?? 'unknown';
      if (!map.has(gk)) map.set(gk, []);
      map.get(gk)!.push(p);
    }
    return Array.from(map.entries()).map(([key, items]) => ({
      key,
      label: `${ROLE_EMOJI[key] ?? ''} ${key.charAt(0).toUpperCase() + key.slice(1)}`.trim(),
      items,
    }));
  }, [sorted, groupBy]);

  const colStyle = (flex: number): React.CSSProperties => ({
    flex, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
  });

  const headerBtn = (key: SortKey, label: string, flex: number) => (
    <button
      key={key}
      onClick={() => toggleSort(key)}
      style={{
        ...colStyle(flex), display: 'flex', alignItems: 'center', gap: 4,
        background: 'none', border: 'none', cursor: 'pointer',
        fontSize: 10, fontWeight: 700, color: sortBy === key ? '#D97706' : 'var(--text-4)',
        textTransform: 'uppercase', letterSpacing: '0.05em', padding: '0 4px',
      }}
    >
      {label}
      {sortBy === key && <ArrowUpDown size={10} />}
    </button>
  );

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 8 }}>
      {/* Group-by selector */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 4 }}>
        <span style={{ fontSize: 11, color: 'var(--text-4)', fontWeight: 600 }}>Group by:</span>
        {(['none', 'role', 'category', 'status'] as GroupKey[]).map(g => (
          <button
            key={g}
            onClick={() => setGroupBy(g)}
            style={{
              padding: '4px 10px', borderRadius: 6, fontSize: 11, cursor: 'pointer',
              border: groupBy === g ? '1.5px solid #D97706' : '1px solid var(--border-md)',
              background: groupBy === g ? 'rgba(217,119,6,0.1)' : 'var(--bg-surface)',
              color: groupBy === g ? '#D97706' : 'var(--text-3)',
              fontWeight: groupBy === g ? 700 : 400,
            }}
          >
            {g === 'none' ? 'None' : g.charAt(0).toUpperCase() + g.slice(1)}
          </button>
        ))}
      </div>

      {/* Header row */}
      <div style={{
        display: 'flex', alignItems: 'center', gap: 8, padding: '8px 12px',
        borderRadius: 8, background: 'var(--bg-elevated)', border: '1px solid var(--border)',
      }}>
        {headerBtn('name', 'Name', 2.5)}
        {headerBtn('role', 'Role', 1.2)}
        {headerBtn('status', 'Status', 0.8)}
        <div style={{ ...colStyle(2.5), fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.05em', display: 'flex', alignItems: 'center', gap: 4 }}>
          <Link2 size={9} /> Connections
        </div>
        {headerBtn('totalRuns', 'Runs', 0.6)}
        {headerBtn('rating', 'Rating', 0.6)}
        <div style={{ width: 84, fontSize: 10, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', textAlign: 'center' }}>Actions</div>
      </div>

      {/* Groups + rows */}
      {groups.map(group => (
        <div key={group.key}>
          {groupBy !== 'none' && (
            <button
              onClick={() => toggleGroup(group.key)}
              style={{
                display: 'flex', alignItems: 'center', gap: 8, width: '100%',
                padding: '8px 12px', marginBottom: 2, borderRadius: 6,
                background: 'var(--bg-surface)', border: '1px solid var(--border)',
                cursor: 'pointer', fontSize: 13, fontWeight: 600, color: 'var(--text-2)',
              }}
            >
              {collapsed.has(group.key) ? <ChevronRight size={14} /> : <ChevronDown size={14} />}
              <span style={{ color: ROLE_COLOR[group.key] ?? 'var(--text-2)' }}>{group.label}</span>
              <span style={{ fontSize: 11, color: 'var(--text-4)', fontWeight: 400 }}>({group.items.length})</span>
            </button>
          )}

          {!collapsed.has(group.key) && group.items.map(p => {
            const id = p.id as string;
            const role = (p.role as string) ?? 'custom';
            const status = (p.status as string) ?? 'idle';
            const rating = Number(p.rating) || 0;
            const totalRuns = (p.totalRuns as number) ?? 0;
            const connections = connectionsMap.get(id) ?? [];
            const isConnecting = connectingId === id;

            return (
              <div key={id} style={{ marginLeft: groupBy !== 'none' ? 16 : 0 }}>
                <div
                  onClick={() => onConfigure(p)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 8,
                    padding: '9px 12px', borderRadius: 6, cursor: 'pointer',
                    border: '1px solid transparent', transition: 'all 0.12s',
                  }}
                  onMouseEnter={e => { e.currentTarget.style.background = 'var(--bg-surface)'; e.currentTarget.style.borderColor = 'var(--border)'; }}
                  onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.borderColor = 'transparent'; }}
                >
                  {/* Name */}
                  <div style={{ ...colStyle(2.5), display: 'flex', alignItems: 'center', gap: 8 }}>
                    <span style={{ fontSize: 15 }}>{ROLE_EMOJI[role] ?? '⚙️'}</span>
                    <div style={{
                      width: 8, height: 8, borderRadius: '50%',
                      background: ROLE_COLOR[role] ?? '#94a3b8',
                      boxShadow: `0 0 6px ${ROLE_COLOR[role] ?? '#94a3b8'}60`,
                      flexShrink: 0,
                    }} />
                    <span style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)' }}>{p.name as string}</span>
                  </div>

                  {/* Role */}
                  <div style={{ ...colStyle(1.2), fontSize: 12, color: ROLE_COLOR[role] ?? 'var(--text-3)', fontWeight: 500 }}>
                    {role}
                  </div>

                  {/* Status */}
                  <div style={{ ...colStyle(0.8), display: 'flex', alignItems: 'center', gap: 5 }}>
                    <div style={{ width: 6, height: 6, borderRadius: '50%', background: STATUS_DOT[status] ?? '#6b7280' }} />
                    <span style={{ fontSize: 11, color: 'var(--text-3)' }}>{status}</span>
                  </div>

                  {/* Connections */}
                  <div style={{ ...colStyle(2.5), display: 'flex', alignItems: 'center', gap: 4, overflow: 'hidden', flexWrap: 'wrap' }} onClick={e => e.stopPropagation()}>
                    {connections.slice(0, 3).map(name => (
                      <span key={name} style={{
                        fontSize: 10, padding: '1px 7px', borderRadius: 4,
                        background: 'rgba(99,102,241,0.1)', color: '#818cf8',
                        fontWeight: 500, whiteSpace: 'nowrap', border: '1px solid rgba(99,102,241,0.2)',
                      }}>
                        {name}
                      </span>
                    ))}
                    {connections.length > 3 && (
                      <span style={{ fontSize: 10, color: 'var(--text-4)' }}>+{connections.length - 3}</span>
                    )}
                    <button
                      onClick={(e) => { e.stopPropagation(); setConnectingId(isConnecting ? null : id); setConnectInput(''); }}
                      title="Connect to another PRISM"
                      style={{
                        display: 'flex', alignItems: 'center', justifyContent: 'center',
                        width: 20, height: 20, borderRadius: 4, flexShrink: 0,
                        border: '1px solid rgba(217,119,6,0.3)', background: isConnecting ? 'rgba(217,119,6,0.15)' : 'transparent',
                        color: '#D97706', cursor: 'pointer', transition: 'all 0.12s',
                      }}
                    >
                      {isConnecting ? <X size={10} /> : <Plus size={10} />}
                    </button>
                  </div>

                  {/* Runs */}
                  <div style={{ ...colStyle(0.6), fontSize: 12, color: 'var(--text-3)', textAlign: 'center' }}>
                    {totalRuns}
                  </div>

                  {/* Rating */}
                  <div style={{ ...colStyle(0.6), display: 'flex', alignItems: 'center', gap: 3, justifyContent: 'center' }}>
                    <Star size={10} fill={rating > 0 ? '#f59e0b' : 'none'} color="#f59e0b" />
                    <span style={{ fontSize: 11, color: 'var(--text-3)' }}>{rating > 0 ? rating.toFixed(1) : '—'}</span>
                  </div>

                  {/* Actions */}
                  <div style={{ width: 84, display: 'flex', gap: 4, justifyContent: 'center' }}>
                    {[
                      { icon: Play, color: '#10b981', title: 'Run', action: onRun },
                      { icon: Settings, color: '#6b7280', title: 'Configure', action: onConfigure },
                      { icon: Trash2, color: '#ef4444', title: 'Delete', action: onDelete },
                    ].map(({ icon: Icon, color, title, action }) => (
                      <button
                        key={title}
                        onClick={e => { e.stopPropagation(); action(p); }}
                        title={title}
                        style={{
                          display: 'flex', alignItems: 'center', justifyContent: 'center',
                          width: 24, height: 24, borderRadius: 5,
                          border: `1px solid ${color}30`, background: `${color}08`,
                          color, cursor: 'pointer', transition: 'all 0.12s',
                        }}
                        onMouseEnter={e => { e.currentTarget.style.background = `${color}18`; }}
                        onMouseLeave={e => { e.currentTarget.style.background = `${color}08`; }}
                      >
                        <Icon size={11} />
                      </button>
                    ))}
                  </div>
                </div>

                {/* Inline connect input */}
                {isConnecting && (
                  <div style={{
                    display: 'flex', gap: 6, alignItems: 'center',
                    padding: '6px 12px 10px', marginLeft: 40,
                  }}>
                    <Link2 size={12} color="#818cf8" />
                    <input
                      value={connectInput}
                      onChange={e => setConnectInput(e.target.value)}
                      placeholder="Type PRISM name to connect…"
                      list={`connect-list-${id}`}
                      autoFocus
                      style={{
                        flex: 1, maxWidth: 260, padding: '5px 10px', borderRadius: 6, fontSize: 12,
                        border: '1px solid var(--border-md)', background: 'var(--bg-surface)',
                        color: 'var(--text-1)', outline: 'none',
                      }}
                      onKeyDown={e => { if (e.key === 'Enter') handleConnect(id); if (e.key === 'Escape') { setConnectingId(null); setConnectInput(''); } }}
                    />
                    <datalist id={`connect-list-${id}`}>
                      {prisms.filter(pp => (pp.id as string) !== id).map(pp => (
                        <option key={pp.id as string} value={pp.name as string} />
                      ))}
                    </datalist>
                    <button
                      onClick={() => handleConnect(id)}
                      disabled={!connectInput.trim()}
                      style={{
                        padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: 600,
                        border: '1.5px solid #818cf8', background: connectInput.trim() ? 'rgba(99,102,241,0.12)' : 'transparent',
                        color: '#818cf8', cursor: connectInput.trim() ? 'pointer' : 'default',
                        opacity: connectInput.trim() ? 1 : 0.4,
                      }}
                    >
                      Connect
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      ))}

      {prisms.length === 0 && (
        <div style={{ textAlign: 'center', padding: '40px 0', color: 'var(--text-4)', fontSize: 13 }}>
          No PRISMs to display
        </div>
      )}
    </div>
  );
}
