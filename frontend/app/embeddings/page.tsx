/* eslint-disable @typescript-eslint/no-explicit-any */
'use client';

import { useState, useCallback } from 'react';
import useSWR from 'swr';
import { motion, AnimatePresence } from 'framer-motion';
import {
  Database, Search, Upload, Layers, RefreshCcw,
  Loader2, AlertTriangle, CheckCircle2, Hash, Box,
  ArrowRight, Sparkles, FileText,
} from 'lucide-react';
import GlowCard from '@/components/ui/GlowCard';

/* ── Helpers ───────────────────────────────────────────── */

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return `${n}`;
}

/* ── Tab definitions ───────────────────────────────────── */

type Tab = 'collections' | 'search' | 'embed' | 'upsert';

const TABS: { id: Tab; label: string; icon: typeof Database; color: string }[] = [
  { id: 'collections', label: 'Collections', icon: Database,  color: '#0EA5E9' },
  { id: 'search',      label: 'Search',      icon: Search,    color: '#7C3AED' },
  { id: 'embed',       label: 'Embed',       icon: Sparkles,  color: '#D97706' },
  { id: 'upsert',      label: 'Upsert',      icon: Upload,    color: '#059669' },
];

/* ── Collections Panel ─────────────────────────────────── */

function CollectionsPanel() {
  const { data, error, isLoading, mutate } = useSWR(
    '/api/embeddings?action=collections',
    fetcher,
    { refreshInterval: 30_000 },
  );

  const collections = data?.collections ?? (data?.name ? [data] : []);

  return (
    <div className="space-y-4">
      <div className="flex items-center justify-between">
        <h2 className="text-lg font-semibold text-[var(--text-1)] flex items-center gap-2">
          <Database size={18} className="text-sky-400" />
          Vector Collections
        </h2>
        <button
          onClick={() => mutate()}
          className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium text-[var(--text-3)]
                     hover:text-[var(--text-1)] bg-[var(--bg-elevated)] hover:bg-[var(--bg-elevated)] border border-[var(--border)]
                     rounded-lg transition-colors"
        >
          <RefreshCcw size={12} /> Refresh
        </button>
      </div>

      {isLoading && (
        <div className="flex items-center justify-center py-16 text-[var(--text-3)]">
          <Loader2 size={20} className="animate-spin mr-2" /> Loading collections...
        </div>
      )}

      {error && (
        <GlowCard glowColor="rgba(239,68,68,0.15)">
          <div className="flex items-center gap-2 text-red-400 text-sm">
            <AlertTriangle size={16} />
            Failed to load collections: {error.message}
          </div>
        </GlowCard>
      )}

      {!isLoading && !error && collections.length === 0 && (
        <GlowCard>
          <div className="text-center py-8 text-[var(--text-3)]">
            <Layers size={32} className="mx-auto mb-3 opacity-40" />
            <p className="text-sm">No collections found. Upsert some vectors to get started.</p>
          </div>
        </GlowCard>
      )}

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
        {collections.map((col: any, i: number) => (
          <GlowCard key={col.name || i} glowColor="rgba(14,165,233,0.15)">
            <div className="flex items-start justify-between mb-3">
              <div className="flex items-center gap-2">
                <div className="w-8 h-8 rounded-lg bg-sky-500/10 flex items-center justify-center">
                  <Database size={16} className="text-sky-400" />
                </div>
                <div>
                  <p className="text-sm font-semibold text-[var(--text-1)]">{typeof col.name === 'string' ? col.name : 'default'}</p>
                  <p className="text-xs text-[var(--text-3)]">Qdrant collection</p>
                </div>
              </div>
            </div>
            <div className="grid grid-cols-2 gap-3 mt-3">
              <div className="bg-[var(--bg-surface)] rounded-lg p-2.5 text-center">
                <p className="text-xs text-[var(--text-3)] mb-0.5">Vectors</p>
                <p className="text-lg font-bold text-[var(--text-1)]">
                  {fmt(col.vectors_count ?? col.points_count ?? 0)}
                </p>
              </div>
              <div className="bg-[var(--bg-surface)] rounded-lg p-2.5 text-center">
                <p className="text-xs text-[var(--text-3)] mb-0.5">Dimension</p>
                <p className="text-lg font-bold text-[var(--text-1)]">
                  {col.dimension ?? '—'}
                </p>
              </div>
            </div>
            {col.status && (
              <div className="mt-3 flex items-center gap-1.5 text-xs">
                <CheckCircle2 size={12} className={
                  col.status === 'green' ? 'text-emerald-400' : 'text-amber-400'
                } />
                <span className="text-[var(--text-2)]">Status: {col.status}</span>
              </div>
            )}
          </GlowCard>
        ))}
      </div>
    </div>
  );
}

/* ── Search Panel ──────────────────────────────────────── */

function SearchPanel() {
  const [query, setQuery] = useState('');
  const [collection, setCollection] = useState('');
  const [limit, setLimit] = useState(10);
  const [results, setResults] = useState<any[]>([]);
  const [searching, setSearching] = useState(false);
  const [searchError, setSearchError] = useState('');
  const [hasSearched, setHasSearched] = useState(false);

  const handleSearch = useCallback(async () => {
    if (!query.trim()) return;
    setSearching(true);
    setSearchError('');
    setHasSearched(true);
    try {
      const params = new URLSearchParams({
        action: 'search',
        query: query.trim(),
        limit: String(limit),
      });
      if (collection.trim()) params.set('collection', collection.trim());
      const res = await fetch(`/api/embeddings?${params}`);
      const data = await res.json();
      if (data.error) throw new Error(data.error);
      setResults(data.results ?? []);
    } catch (e: any) {
      setSearchError(e.message);
      setResults([]);
    } finally {
      setSearching(false);
    }
  }, [query, collection, limit]);

  return (
    <div className="space-y-4">
      <h2 className="text-lg font-semibold text-[var(--text-1)] flex items-center gap-2">
        <Search size={18} className="text-violet-400" />
        Semantic Search
      </h2>

      <GlowCard glowColor="rgba(124,58,237,0.15)">
        <div className="space-y-3">
          <div>
            <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">Query</label>
            <textarea
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder="Enter your search query..."
              rows={3}
              className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                         text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                         focus:border-[var(--border-focus)] resize-none transition-colors"
              onKeyDown={e => {
                if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) handleSearch();
              }}
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div>
              <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">Collection (optional)</label>
              <input
                value={collection}
                onChange={e => setCollection(e.target.value)}
                placeholder="default"
                className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                           text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                           focus:border-[var(--border-focus)] transition-colors"
              />
            </div>
            <div>
              <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">Limit</label>
              <input
                type="number"
                value={limit}
                onChange={e => setLimit(Math.max(1, parseInt(e.target.value) || 10))}
                min={1}
                max={100}
                className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                           text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                           focus:border-[var(--border-focus)] transition-colors"
              />
            </div>
          </div>

          <button
            onClick={handleSearch}
            disabled={searching || !query.trim()}
            className="flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg
                       bg-violet-600 hover:bg-violet-500 disabled:bg-[var(--bg-elevated)] disabled:text-[var(--text-3)]
                       text-white transition-colors"
          >
            {searching ? <Loader2 size={14} className="animate-spin" /> : <Search size={14} />}
            {searching ? 'Searching...' : 'Search'}
          </button>
        </div>
      </GlowCard>

      {searchError && (
        <GlowCard glowColor="rgba(239,68,68,0.15)">
          <div className="flex items-center gap-2 text-red-400 text-sm">
            <AlertTriangle size={16} />
            {searchError}
          </div>
        </GlowCard>
      )}

      {hasSearched && !searching && !searchError && results.length === 0 && (
        <div className="text-center py-8 text-[var(--text-3)] text-sm">
          No results found for your query.
        </div>
      )}

      <AnimatePresence mode="popLayout">
        {results.map((r: any, i: number) => (
          <motion.div
            key={r.id ?? i}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -8 }}
            transition={{ delay: i * 0.04 }}
          >
            <GlowCard glowColor="rgba(124,58,237,0.10)" className="mb-3">
              <div className="flex items-start justify-between gap-4">
                <div className="flex-1 min-w-0">
                  <div className="flex items-center gap-2 mb-1.5">
                    <Hash size={12} className="text-[var(--text-3)] shrink-0" />
                    <span className="text-xs text-[var(--text-3)] font-mono">
                      {r.id ?? `result-${i}`}
                    </span>
                  </div>
                  {r.payload?.text && (
                    <p className="text-sm text-[var(--text-2)] leading-relaxed line-clamp-3">
                      {r.payload.text}
                    </p>
                  )}
                  {r.payload && !r.payload.text && (
                    <pre className="text-xs text-[var(--text-2)] bg-[var(--bg-surface)] rounded-md p-2 mt-1 overflow-auto max-h-24">
                      {JSON.stringify(r.payload, null, 2)}
                    </pre>
                  )}
                </div>
                <div className="shrink-0">
                  <div className={`px-2.5 py-1 rounded-full text-xs font-bold ${
                    (r.score ?? 0) >= 0.8
                      ? 'bg-emerald-500/15 text-emerald-400 border border-emerald-500/20'
                      : (r.score ?? 0) >= 0.5
                        ? 'bg-amber-500/15 text-amber-400 border border-amber-500/20'
                        : 'bg-[var(--bg-elevated)] text-[var(--text-2)] border border-[var(--border)]'
                  }`}>
                    {((r.score ?? 0) * 100).toFixed(1)}%
                  </div>
                </div>
              </div>
            </GlowCard>
          </motion.div>
        ))}
      </AnimatePresence>
    </div>
  );
}

/* ── Embed Panel ───────────────────────────────────────── */

function EmbedPanel() {
  const [texts, setTexts] = useState('');
  const [embedding, setEmbedding] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [embedError, setEmbedError] = useState('');

  const handleEmbed = useCallback(async () => {
    const lines = texts.split('\n').map(l => l.trim()).filter(Boolean);
    if (lines.length === 0) return;
    setEmbedding(true);
    setEmbedError('');
    setResult(null);
    try {
      const res = await fetch('/api/embeddings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ action: 'embed', texts: lines }),
      });
      const data = await res.json();
      if (data.error) throw new Error(data.error);
      setResult(data);
    } catch (e: any) {
      setEmbedError(e.message);
    } finally {
      setEmbedding(false);
    }
  }, [texts]);

  return (
    <div className="space-y-4">
      <h2 className="text-lg font-semibold text-[var(--text-1)] flex items-center gap-2">
        <Sparkles size={18} className="text-amber-400" />
        Generate Embeddings
      </h2>

      <GlowCard glowColor="rgba(217,119,6,0.15)">
        <div className="space-y-3">
          <div>
            <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">
              Texts (one per line)
            </label>
            <textarea
              value={texts}
              onChange={e => setTexts(e.target.value)}
              placeholder={"Enter texts to embed, one per line...\nExample: The quick brown fox\nAnother sentence here"}
              rows={6}
              className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                         text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                         focus:border-[var(--border-focus)] resize-none font-mono transition-colors"
            />
          </div>

          <div className="flex items-center gap-3">
            <button
              onClick={handleEmbed}
              disabled={embedding || !texts.trim()}
              className="flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg
                         bg-amber-600 hover:bg-amber-500 disabled:bg-[var(--bg-elevated)] disabled:text-[var(--text-3)]
                         text-white transition-colors"
            >
              {embedding ? <Loader2 size={14} className="animate-spin" /> : <Sparkles size={14} />}
              {embedding ? 'Generating...' : 'Generate Embeddings'}
            </button>
            {texts.trim() && (
              <span className="text-xs text-[var(--text-3)]">
                {texts.split('\n').filter(l => l.trim()).length} text(s)
              </span>
            )}
          </div>
        </div>
      </GlowCard>

      {embedError && (
        <GlowCard glowColor="rgba(239,68,68,0.15)">
          <div className="flex items-center gap-2 text-red-400 text-sm">
            <AlertTriangle size={16} />
            {embedError}
          </div>
        </GlowCard>
      )}

      {result && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
          <GlowCard glowColor="rgba(217,119,6,0.12)">
            <div className="flex items-center gap-2 mb-3">
              <CheckCircle2 size={16} className="text-emerald-400" />
              <span className="text-sm font-medium text-[var(--text-1)]">
                Generated {result.count} vector(s)
              </span>
            </div>
            <div className="grid grid-cols-2 gap-3">
              <div className="bg-[var(--bg-surface)] rounded-lg p-2.5 text-center">
                <p className="text-xs text-[var(--text-3)] mb-0.5">Model</p>
                <p className="text-xs font-mono text-[var(--text-2)] truncate">{result.model}</p>
              </div>
              <div className="bg-[var(--bg-surface)] rounded-lg p-2.5 text-center">
                <p className="text-xs text-[var(--text-3)] mb-0.5">Dimensions</p>
                <p className="text-sm font-bold text-[var(--text-1)]">
                  {result.vectors?.[0]?.length ?? '—'}
                </p>
              </div>
            </div>
            <details className="mt-3">
              <summary className="text-xs text-[var(--text-3)] cursor-pointer hover:text-[var(--text-2)] transition-colors">
                View raw vectors
              </summary>
              <pre className="mt-2 text-xs text-[var(--text-2)] bg-[var(--bg-surface)] rounded-md p-2 overflow-auto max-h-48 font-mono">
                {JSON.stringify(result.vectors?.map((v: number[]) =>
                  `[${v.slice(0, 5).map(n => n.toFixed(4)).join(', ')}... (${v.length}d)]`
                ), null, 2)}
              </pre>
            </details>
          </GlowCard>
        </motion.div>
      )}
    </div>
  );
}

/* ── Upsert Panel ──────────────────────────────────────── */

function UpsertPanel() {
  const [texts, setTexts] = useState('');
  const [collection, setCollection] = useState('');
  const [metadata, setMetadata] = useState('');
  const [upserting, setUpserting] = useState(false);
  const [result, setResult] = useState<any>(null);
  const [upsertError, setUpsertError] = useState('');

  const handleUpsert = useCallback(async () => {
    const lines = texts.split('\n').map(l => l.trim()).filter(Boolean);
    if (lines.length === 0) return;
    setUpserting(true);
    setUpsertError('');
    setResult(null);

    let payloads: any[] | undefined;
    if (metadata.trim()) {
      try {
        payloads = JSON.parse(metadata.trim());
        if (!Array.isArray(payloads)) throw new Error('Payloads must be an array');
      } catch (e: any) {
        setUpsertError(`Invalid metadata JSON: ${e.message}`);
        setUpserting(false);
        return;
      }
    }

    try {
      const body: any = { action: 'upsert', texts: lines };
      if (collection.trim()) body.collection = collection.trim();
      if (payloads) body.payloads = payloads;

      const res = await fetch('/api/embeddings', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      const data = await res.json();
      if (data.error) throw new Error(data.error);
      setResult(data);
    } catch (e: any) {
      setUpsertError(e.message);
    } finally {
      setUpserting(false);
    }
  }, [texts, collection, metadata]);

  return (
    <div className="space-y-4">
      <h2 className="text-lg font-semibold text-[var(--text-1)] flex items-center gap-2">
        <Upload size={18} className="text-emerald-400" />
        Upsert to Vector Store
      </h2>

      <GlowCard glowColor="rgba(5,150,105,0.15)">
        <div className="space-y-3">
          <div>
            <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">
              Texts (one per line)
            </label>
            <textarea
              value={texts}
              onChange={e => setTexts(e.target.value)}
              placeholder="Enter texts to embed and store, one per line..."
              rows={5}
              className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                         text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                         focus:border-[var(--border-focus)] resize-none font-mono transition-colors"
            />
          </div>

          <div>
            <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">
              Collection (optional)
            </label>
            <input
              value={collection}
              onChange={e => setCollection(e.target.value)}
              placeholder="default"
              className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                         text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                         focus:border-[var(--border-focus)] transition-colors"
            />
          </div>

          <div>
            <label className="text-xs font-medium text-[var(--text-2)] mb-1 block">
              Metadata (optional JSON array)
            </label>
            <textarea
              value={metadata}
              onChange={e => setMetadata(e.target.value)}
              placeholder={'[{"source": "doc1.pdf", "page": 1}, {"source": "doc1.pdf", "page": 2}]'}
              rows={3}
              className="w-full bg-[var(--bg-surface)] border border-[var(--border)] rounded-lg px-3 py-2
                         text-sm text-[var(--text-1)] placeholder-[var(--text-4)] focus:outline-none
                         focus:border-[var(--border-focus)] resize-none font-mono transition-colors"
            />
          </div>

          <div className="flex items-center gap-3">
            <button
              onClick={handleUpsert}
              disabled={upserting || !texts.trim()}
              className="flex items-center gap-2 px-4 py-2 text-sm font-medium rounded-lg
                         bg-emerald-600 hover:bg-emerald-500 disabled:bg-[var(--bg-elevated)] disabled:text-[var(--text-3)]
                         text-white transition-colors"
            >
              {upserting ? <Loader2 size={14} className="animate-spin" /> : <Upload size={14} />}
              {upserting ? 'Upserting...' : 'Upsert Vectors'}
            </button>
            {texts.trim() && (
              <span className="text-xs text-[var(--text-3)]">
                {texts.split('\n').filter(l => l.trim()).length} text(s)
              </span>
            )}
          </div>
        </div>
      </GlowCard>

      {upsertError && (
        <GlowCard glowColor="rgba(239,68,68,0.15)">
          <div className="flex items-center gap-2 text-red-400 text-sm">
            <AlertTriangle size={16} />
            {upsertError}
          </div>
        </GlowCard>
      )}

      {result && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
          <GlowCard glowColor="rgba(5,150,105,0.12)">
            <div className="flex items-center gap-2">
              <CheckCircle2 size={16} className="text-emerald-400" />
              <span className="text-sm font-medium text-[var(--text-1)]">
                Upserted {result.upserted} vector(s) using {result.model}
              </span>
            </div>
          </GlowCard>
        </motion.div>
      )}
    </div>
  );
}

/* ── Main Page ─────────────────────────────────────────── */

export default function EmbeddingsPage() {
  const [tab, setTab] = useState<Tab>('collections');

  return (
    <div className="min-h-screen bg-[var(--bg)] text-[var(--text-1)]">
      {/* Header */}
      <div className="border-b border-[var(--border)] bg-[var(--bg-surface)] backdrop-blur-sm">
        <div className="max-w-7xl mx-auto px-6 py-5">
          <div className="flex items-center gap-3 mb-1">
            <div className="w-9 h-9 rounded-lg bg-sky-500/10 flex items-center justify-center">
              <Layers size={20} className="text-sky-400" />
            </div>
            <div>
              <h1 className="text-xl font-bold tracking-tight">Embeddings & RAG</h1>
              <p className="text-xs text-[var(--text-3)] mt-0.5">
                Manage vector collections, search semantically, and embed text
              </p>
            </div>
          </div>
        </div>
      </div>

      {/* Tab Navigation */}
      <div className="border-b border-[var(--border)] bg-[var(--bg-surface)]">
        <div className="max-w-7xl mx-auto px-6">
          <div className="flex gap-1">
            {TABS.map(t => {
              const Icon = t.icon;
              const active = tab === t.id;
              return (
                <button
                  key={t.id}
                  onClick={() => setTab(t.id)}
                  className={`flex items-center gap-2 px-4 py-3 text-sm font-medium
                              border-b-2 transition-all ${
                    active
                      ? 'border-current text-[var(--text-1)]'
                      : 'border-transparent text-[var(--text-3)] hover:text-[var(--text-2)]'
                  }`}
                  style={active ? { color: t.color } : undefined}
                >
                  <Icon size={15} />
                  {t.label}
                </button>
              );
            })}
          </div>
        </div>
      </div>

      {/* Content */}
      <div className="max-w-7xl mx-auto px-6 py-6">
        <AnimatePresence mode="wait">
          <motion.div
            key={tab}
            initial={{ opacity: 0, y: 12 }}
            animate={{ opacity: 1, y: 0 }}
            exit={{ opacity: 0, y: -12 }}
            transition={{ duration: 0.2 }}
          >
            {tab === 'collections' && <CollectionsPanel />}
            {tab === 'search' && <SearchPanel />}
            {tab === 'embed' && <EmbedPanel />}
            {tab === 'upsert' && <UpsertPanel />}
          </motion.div>
        </AnimatePresence>
      </div>
    </div>
  );
}
