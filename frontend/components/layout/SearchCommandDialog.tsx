'use client';

import { useState, useEffect, useRef, useCallback } from 'react';
import { useRouter } from 'next/navigation';
import {
  Search, X, Brain, GitBranch, ListTodo,
  Database, AlertTriangle, FolderOpen, Loader2, ArrowRight,
  Clock,
} from 'lucide-react';

/* ─── Types ──────────────────────────────────────────── */
interface SearchResult {
  id: string;
  type: 'expert' | 'workflow' | 'task' | 'dataset' | 'alert' | 'project';
  name: string;
  description: string | null;
  status: string | null;
  meta: Record<string, unknown>;
  href: string;
  updatedAt: string | null;
}

type ResultCategory = SearchResult['type'];

const CATEGORY_META: Record<ResultCategory, { label: string; icon: typeof Brain; color: string }> = {
  expert:   { label: 'Experts',       icon: Brain,           color: '#D97706' },
  workflow: { label: 'Workflows',     icon: GitBranch,       color: '#2563EB' },
  task:     { label: 'Tasks',         icon: ListTodo,        color: '#F04500' },
  dataset:  { label: 'Datasets',      icon: Database,        color: '#7C3AED' },
  alert:    { label: 'Alerts',        icon: AlertTriangle,   color: '#DC2626' },
  project:  { label: 'Projects',      icon: FolderOpen,      color: '#059669' },
};

const TYPE_FILTERS: { value: string; label: string }[] = [
  { value: '',         label: 'All' },
  { value: 'expert',   label: 'Experts' },
  { value: 'workflow', label: 'Workflows' },
  { value: 'task',     label: 'Tasks' },
  { value: 'dataset',  label: 'Datasets' },
  { value: 'alert',    label: 'Alerts' },
  { value: 'project',  label: 'Projects' },
];

const STATUS_COLORS: Record<string, string> = {
  active: '#10b981', idle: '#6b7280', running: '#2563EB', completed: '#10b981',
  failed: '#DC2626', error: '#DC2626', training: '#7C3AED', draft: '#6b7280',
  ready: '#10b981', queued: '#D97706', critical: '#DC2626', warning: '#D97706',
  info: '#2563EB', offline: '#6b7280', cancelled: '#6b7280',
};

/* ─── Component ──────────────────────────────────────── */
interface Props {
  open: boolean;
  onClose: () => void;
}

export default function SearchCommandDialog({ open, onClose }: Props) {
  const router = useRouter();
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [query, setQuery] = useState('');
  const [typeFilter, setTypeFilter] = useState('');
  const [results, setResults] = useState<SearchResult[]>([]);
  const [total, setTotal] = useState(0);
  const [loading, setLoading] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const debounceRef = useRef<ReturnType<typeof setTimeout>>(undefined);

  // Focus input on open
  useEffect(() => {
    if (open) {
      setQuery('');
      setResults([]);
      setTotal(0);
      setSelectedIndex(0);
      setTypeFilter('');
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  // Debounced search
  const doSearch = useCallback(async (q: string, type: string) => {
    if (!q.trim()) {
      setResults([]);
      setTotal(0);
      return;
    }
    setLoading(true);
    try {
      const params = new URLSearchParams({ q: q.trim(), limit: '25' });
      if (type) params.set('type', type);
      const res = await fetch(`/api/search?${params}`);
      const data = await res.json();
      setResults(data.results ?? []);
      setTotal(data.total ?? 0);
      setSelectedIndex(0);
    } catch {
      setResults([]);
      setTotal(0);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => doSearch(query, typeFilter), 200);
    return () => clearTimeout(debounceRef.current);
  }, [query, typeFilter, doSearch]);

  // Keyboard navigation
  const flatResults = results;
  const handleKeyDown = useCallback((e: React.KeyboardEvent) => {
    if (e.key === 'ArrowDown') {
      e.preventDefault();
      setSelectedIndex(prev => Math.min(prev + 1, flatResults.length - 1));
    } else if (e.key === 'ArrowUp') {
      e.preventDefault();
      setSelectedIndex(prev => Math.max(prev - 1, 0));
    } else if (e.key === 'Enter' && flatResults[selectedIndex]) {
      e.preventDefault();
      navigateTo(flatResults[selectedIndex]);
    } else if (e.key === 'Escape') {
      onClose();
    }
  }, [flatResults, selectedIndex, onClose]);

  // Scroll selected item into view
  useEffect(() => {
    const el = listRef.current?.querySelector(`[data-index="${selectedIndex}"]`);
    el?.scrollIntoView({ block: 'nearest' });
  }, [selectedIndex]);

  const navigateTo = (result: SearchResult) => {
    onClose();
    router.push(result.href);
  };

  // Group results by type
  const grouped = flatResults.reduce<Record<string, SearchResult[]>>((acc, r) => {
    (acc[r.type] ??= []).push(r);
    return acc;
  }, {});

  // Build a flat index for keyboard navigation
  let runningIndex = -1;

  if (!open) return null;

  return (
    <>
      {/* Backdrop */}
      <div
        onClick={onClose}
        style={{
          position: 'fixed',
          inset: 0,
          background: 'rgba(0,0,0,0.5)',
          backdropFilter: 'blur(4px)',
          zIndex: 9998,
          animation: 'fadeIn 0.15s ease',
        }}
      />

      {/* Dialog */}
      <div
        style={{
          position: 'fixed',
          top: '12%',
          left: '50%',
          transform: 'translateX(-50%)',
          width: 620,
          maxHeight: '70vh',
          background: 'var(--bg-surface)',
          border: '1px solid var(--border)',
          borderRadius: 12,
          boxShadow: '0 24px 64px rgba(0,0,0,0.3), 0 0 0 1px rgba(255,255,255,0.05)',
          zIndex: 9999,
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
          animation: 'slideDown 0.15s ease',
        }}
        onKeyDown={handleKeyDown}
      >
        {/* Search input */}
        <div style={{
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '14px 16px',
          borderBottom: '1px solid var(--border)',
        }}>
          {loading
            ? <Loader2 size={18} color="var(--text-3)" style={{ animation: 'spin 1s linear infinite' }} />
            : <Search size={18} color="var(--text-3)" />
          }
          <input
            ref={inputRef}
            value={query}
            onChange={e => setQuery(e.target.value)}
            placeholder="Search experts, workflows, tasks, datasets..."
            style={{
              flex: 1,
              background: 'none',
              border: 'none',
              outline: 'none',
              fontSize: 15,
              color: 'var(--text-1)',
              fontFamily: 'inherit',
            }}
          />
          <button
            onClick={onClose}
            style={{
              background: 'var(--bg-elevated)',
              border: '1px solid var(--border)',
              borderRadius: 4,
              padding: '2px 6px',
              fontSize: 11,
              color: 'var(--text-4)',
              cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            ESC
          </button>
        </div>

        {/* Type filters */}
        <div style={{
          display: 'flex',
          gap: 4,
          padding: '8px 16px',
          borderBottom: '1px solid var(--border)',
          overflowX: 'auto',
        }}>
          {TYPE_FILTERS.map(f => (
            <button
              key={f.value}
              onClick={() => setTypeFilter(f.value)}
              style={{
                padding: '3px 10px',
                borderRadius: 4,
                fontSize: 11,
                fontWeight: 500,
                fontFamily: 'inherit',
                cursor: 'pointer',
                border: '1px solid',
                borderColor: typeFilter === f.value ? 'var(--border-strong)' : 'transparent',
                background: typeFilter === f.value ? 'var(--bg-elevated)' : 'none',
                color: typeFilter === f.value ? 'var(--text-1)' : 'var(--text-3)',
                transition: 'all 0.15s',
                whiteSpace: 'nowrap',
              }}
            >
              {f.label}
            </button>
          ))}
        </div>

        {/* Results */}
        <div
          ref={listRef}
          style={{
            flex: 1,
            overflowY: 'auto',
            padding: '8px 0',
          }}
        >
          {/* Empty state */}
          {!loading && query.trim() && flatResults.length === 0 && (
            <div style={{
              padding: '40px 16px',
              textAlign: 'center',
              color: 'var(--text-4)',
              fontSize: 13,
            }}>
              <Search size={32} style={{ marginBottom: 8, opacity: 0.3 }} />
              <div>No results found for &ldquo;{query}&rdquo;</div>
              <div style={{ fontSize: 11, marginTop: 4 }}>
                Try different keywords or remove filters
              </div>
            </div>
          )}

          {/* Initial state */}
          {!query.trim() && (
            <div style={{
              padding: '32px 16px',
              textAlign: 'center',
              color: 'var(--text-4)',
              fontSize: 13,
            }}>
              <div style={{ fontSize: 12, marginBottom: 12 }}>Search across your workspace</div>
              <div style={{ display: 'flex', gap: 8, justifyContent: 'center', flexWrap: 'wrap' }}>
                {['expert', 'workflow', 'task', 'dataset', 'alert', 'project'].map(t => {
                  const meta = CATEGORY_META[t as ResultCategory];
                  const Icon = meta.icon;
                  return (
                    <span key={t} style={{
                      display: 'inline-flex', alignItems: 'center', gap: 4,
                      padding: '3px 8px', borderRadius: 4, fontSize: 11,
                      background: 'var(--bg-elevated)', color: meta.color,
                      border: '1px solid var(--border)',
                    }}>
                      <Icon size={12} /> {meta.label}
                    </span>
                  );
                })}
              </div>
            </div>
          )}

          {/* Grouped results */}
          {Object.entries(grouped).map(([type, items]) => {
            const meta = CATEGORY_META[type as ResultCategory];
            const Icon = meta.icon;
            return (
              <div key={type}>
                {/* Category header */}
                <div style={{
                  padding: '6px 16px',
                  fontSize: 11,
                  fontWeight: 600,
                  color: meta.color,
                  textTransform: 'uppercase',
                  letterSpacing: '0.05em',
                  display: 'flex',
                  alignItems: 'center',
                  gap: 6,
                }}>
                  <Icon size={12} />
                  {meta.label}
                  <span style={{ color: 'var(--text-4)', fontWeight: 400 }}>
                    ({items.length})
                  </span>
                </div>

                {/* Items */}
                {items.map(item => {
                  runningIndex++;
                  const idx = runningIndex;
                  const isSelected = idx === selectedIndex;
                  return (
                    <div
                      key={item.id}
                      data-index={idx}
                      onClick={() => navigateTo(item)}
                      onMouseEnter={() => setSelectedIndex(idx)}
                      style={{
                        padding: '8px 16px',
                        margin: '0 8px',
                        borderRadius: 6,
                        cursor: 'pointer',
                        display: 'flex',
                        alignItems: 'center',
                        gap: 10,
                        background: isSelected ? 'var(--bg-elevated)' : 'transparent',
                        transition: 'background 0.1s',
                      }}
                    >
                      {/* Type icon */}
                      <div style={{
                        width: 32, height: 32, borderRadius: 6,
                        display: 'flex', alignItems: 'center', justifyContent: 'center',
                        background: `${meta.color}14`,
                        flexShrink: 0,
                      }}>
                        <Icon size={16} color={meta.color} />
                      </div>

                      {/* Content */}
                      <div style={{ flex: 1, minWidth: 0 }}>
                        <div style={{
                          fontSize: 13,
                          fontWeight: 500,
                          color: 'var(--text-1)',
                          whiteSpace: 'nowrap',
                          overflow: 'hidden',
                          textOverflow: 'ellipsis',
                        }}>
                          {highlightMatch(item.name, query)}
                        </div>
                        {item.description && (
                          <div style={{
                            fontSize: 11,
                            color: 'var(--text-3)',
                            whiteSpace: 'nowrap',
                            overflow: 'hidden',
                            textOverflow: 'ellipsis',
                            marginTop: 1,
                          }}>
                            {highlightMatch(item.description, query)}
                          </div>
                        )}
                      </div>

                      {/* Status badge */}
                      {item.status && (
                        <span style={{
                          fontSize: 10,
                          fontWeight: 600,
                          padding: '2px 6px',
                          borderRadius: 3,
                          background: `${STATUS_COLORS[item.status] ?? '#6b7280'}18`,
                          color: STATUS_COLORS[item.status] ?? '#6b7280',
                          textTransform: 'uppercase',
                          letterSpacing: '0.03em',
                          whiteSpace: 'nowrap',
                        }}>
                          {item.status}
                        </span>
                      )}

                      {/* Timestamp */}
                      {item.updatedAt && (
                        <span style={{
                          fontSize: 10,
                          color: 'var(--text-4)',
                          whiteSpace: 'nowrap',
                          display: 'flex',
                          alignItems: 'center',
                          gap: 3,
                        }}>
                          <Clock size={10} />
                          {formatRelativeTime(item.updatedAt)}
                        </span>
                      )}

                      {/* Navigate arrow */}
                      {isSelected && (
                        <ArrowRight size={14} color="var(--text-3)" style={{ flexShrink: 0 }} />
                      )}
                    </div>
                  );
                })}
              </div>
            );
          })}
        </div>

        {/* Footer */}
        {flatResults.length > 0 && (
          <div style={{
            padding: '8px 16px',
            borderTop: '1px solid var(--border)',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'space-between',
            fontSize: 11,
            color: 'var(--text-4)',
          }}>
            <span>{total} result{total !== 1 ? 's' : ''}</span>
            <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
              <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                <kbd style={kbdStyle}>↑</kbd>
                <kbd style={kbdStyle}>↓</kbd>
                navigate
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                <kbd style={kbdStyle}>↵</kbd>
                open
              </span>
              <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                <kbd style={kbdStyle}>esc</kbd>
                close
              </span>
            </div>
          </div>
        )}
      </div>

      {/* Global animations */}
      <style>{`
        @keyframes fadeIn { from { opacity: 0 } to { opacity: 1 } }
        @keyframes slideDown {
          from { opacity: 0; transform: translateX(-50%) translateY(-12px) }
          to { opacity: 1; transform: translateX(-50%) translateY(0) }
        }
        @keyframes spin { from { transform: rotate(0deg) } to { transform: rotate(360deg) } }
      `}</style>
    </>
  );
}

/* ─── Helpers ────────────────────────────────────────── */
const kbdStyle: React.CSSProperties = {
  display: 'inline-flex',
  alignItems: 'center',
  justifyContent: 'center',
  minWidth: 18,
  height: 18,
  padding: '0 4px',
  fontSize: 10,
  fontFamily: 'inherit',
  borderRadius: 3,
  border: '1px solid var(--border)',
  background: 'var(--bg-elevated)',
  color: 'var(--text-3)',
};

function highlightMatch(text: string, query: string): React.ReactNode {
  if (!query.trim()) return text;
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const regex = new RegExp(`(${escaped})`, 'gi');
  const parts = text.split(regex);
  return parts.map((part, i) =>
    regex.test(part)
      ? <mark key={i} style={{ background: 'rgba(37,99,235,0.2)', color: 'inherit', borderRadius: 2, padding: '0 1px' }}>{part}</mark>
      : part,
  );
}

function formatRelativeTime(iso: string): string {
  const diff = Date.now() - new Date(iso).getTime();
  const mins = Math.floor(diff / 60_000);
  if (mins < 1) return 'just now';
  if (mins < 60) return `${mins}m ago`;
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return `${hrs}h ago`;
  const days = Math.floor(hrs / 24);
  if (days < 30) return `${days}d ago`;
  return `${Math.floor(days / 30)}mo ago`;
}
