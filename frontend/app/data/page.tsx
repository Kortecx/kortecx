'use client';

import { useState, useEffect, useCallback } from 'react';
import useSWR from 'swr';
import {
  Database, Plus, Play, Download, Sparkles, RefreshCcw,
  CheckCircle2, Clock, AlertTriangle, BarChart3, FileText,
  Zap, Filter, Search, Loader2, Heart, ArrowDownToLine,
  ExternalLink, Trash2, Eye, HardDrive, Rows3, Columns3, Key,
  Cpu, Server, Sparkle, ChevronDown, ChevronRight, X,
} from 'lucide-react';
import Link from 'next/link';
import type { Dataset } from '@/lib/types';

/* ── Helpers ───────────────────────────────────────────── */

function fmt(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(0)}k`;
  return `${n}`;
}

function fmtNum(n: number) {
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}k`;
  return String(n);
}

function fmtBytes(b: number) {
  if (b >= 1_073_741_824) return `${(b / 1_073_741_824).toFixed(1)} GB`;
  if (b >= 1_048_576) return `${(b / 1_048_576).toFixed(0)} MB`;
  if (b >= 1024) return `${(b / 1024).toFixed(0)} KB`;
  return `${b} B`;
}

/** Estimate download time assuming ~10 MB/s average broadband speed. */
function estimateDownloadTime(bytes: number): string {
  const speedBps = 10 * 1_048_576; // 10 MB/s baseline
  const seconds = bytes / speedBps;
  if (seconds < 5) return '<5s';
  if (seconds < 60) return `${Math.round(seconds)}s`;
  if (seconds < 3600) return `${Math.ceil(seconds / 60)}m`;
  return `${(seconds / 3600).toFixed(1)}h`;
}

const fetcher = (url: string) => fetch(url).then(r => {
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
});

/* ── Dataset Card (My Datasets tab) ────────────────────── */

function DatasetCard({ ds }: { ds: Dataset }) {
  const statusColor = ds.status === 'ready' ? 'var(--success)'
    : ds.status === 'generating' ? 'var(--amber)'
    : ds.status === 'failed' ? 'var(--error)'
    : 'var(--text-3)';

  const StatusIcon = ds.status === 'ready' ? CheckCircle2
    : ds.status === 'generating' ? RefreshCcw
    : ds.status === 'failed' ? AlertTriangle
    : Clock;

  return (
    <div className="card" style={{ padding: 18 }}>
      <div style={{ display: 'flex', alignItems: 'flex-start', gap: 12, marginBottom: 14 }}>
        <div style={{
          width: 38, height: 38, borderRadius: 6,
          background: 'var(--teal-dim)',
          border: '1px solid rgba(12,166,120,0.25)',
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          flexShrink: 0,
        }}>
          <Database size={17} color="var(--teal)" />
        </div>

        <div style={{ flex: 1, minWidth: 0 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 3 }}>
            <span style={{ fontSize: 14, fontWeight: 600, color: 'var(--text-1)' }}>{ds.name}</span>
            <span style={{
              display: 'flex', alignItems: 'center', gap: 4,
              fontSize: 10, fontWeight: 600, color: statusColor,
              textTransform: 'uppercase', letterSpacing: '0.06em',
            }}>
              <StatusIcon size={10} />
              {ds.status}
            </span>
          </div>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: 0, lineHeight: 1.4 }}>
            {ds.description}
          </p>
        </div>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(3,1fr)', gap: 8, marginBottom: 14 }}>
        {[
          { label: 'Samples', value: ds.sampleCount.toLocaleString() },
          { label: 'Size', value: `${(ds.sizeBytes / 1_000_000).toFixed(0)} MB` },
          { label: 'Format', value: ds.format.toUpperCase() },
        ].map(stat => (
          <div key={stat.label} style={{
            padding: '7px 10px',
            background: 'var(--bg)',
            border: '1px solid var(--border)',
            borderRadius: 4,
            textAlign: 'center',
          }}>
            <div className="mono" style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-1)' }}>
              {stat.value}
            </div>
            <div style={{ fontSize: 10, color: 'var(--text-3)', marginTop: 1 }}>{stat.label}</div>
          </div>
        ))}
      </div>

      {/* Quality score */}
      {ds.qualityScore !== undefined && (
        <div style={{ marginBottom: 14 }}>
          <div style={{
            display: 'flex', justifyContent: 'space-between',
            fontSize: 11, color: 'var(--text-3)', marginBottom: 5,
          }}>
            <span>Quality Score</span>
            <span className="mono" style={{
              color: ds.qualityScore >= 90 ? 'var(--success)' : ds.qualityScore >= 75 ? 'var(--warning)' : 'var(--error)',
            }}>
              {ds.qualityScore}%
            </span>
          </div>
          <div className="progress-track">
            <div
              className="progress-fill"
              style={{
                width: `${ds.qualityScore}%`,
                background: ds.qualityScore >= 90 ? 'var(--success)' : ds.qualityScore >= 75 ? 'var(--warning)' : 'var(--error)',
              }}
            />
          </div>
        </div>
      )}

      {/* Categories */}
      <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginBottom: 14 }}>
        {ds.categories.map(cat => (
          <span key={cat} className="badge badge-teal" style={{ fontSize: 10 }}>{cat}</span>
        ))}
        {ds.tags.map(tag => (
          <span key={tag} className="badge badge-neutral" style={{ fontSize: 10 }}>{tag}</span>
        ))}
      </div>

      {/* Actions */}
      <div style={{ display: 'flex', gap: 8 }}>
        <button className="btn btn-primary btn-sm">
          <Play size={12} /> Train Expert
        </button>
        <button className="btn btn-secondary btn-sm">
          <Download size={12} /> Export
        </button>
        <button className="btn btn-ghost btn-sm">
          <BarChart3 size={12} /> Inspect
        </button>
      </div>
    </div>
  );
}

/* ── HuggingFace Hub Tab ───────────────────────────────── */

function HuggingFaceHubTab() {
  const [hfQuery, setHfQuery] = useState('');
  const [hfSort, setHfSort] = useState('downloads');
  const [hfResults, setHfResults] = useState<any[]>([]);
  const [hfLoading, setHfLoading] = useState(false);
  const [downloadingIds, setDownloadingIds] = useState<Set<string>>(new Set());
  const [hfConfigured, setHfConfigured] = useState<boolean | null>(null);

  // Check HF connection status on mount
  useEffect(() => {
    fetch('/api/datasets/hf/status')
      .then(r => r.json())
      .then(d => setHfConfigured(d.configured))
      .catch(() => setHfConfigured(false));
  }, []);

  // Downloaded datasets from DB — track if any are still downloading for faster refresh
  const [hasDownloading, setHasDownloading] = useState(false);
  const { data: dlData, mutate: mutateDl } = useSWR('/api/datasets', fetcher, {
    refreshInterval: hasDownloading ? 3_000 : 10_000,
    onSuccess: (data) => {
      const any = (data?.datasets ?? []).some((d: any) => d.status === 'downloading');
      setHasDownloading(any);
    },
  });
  const downloadedDatasets: any[] = dlData?.datasets ?? [];

  // Debounced HF search — skip if no API key configured
  useEffect(() => {
    if (!hfQuery.trim() || hfConfigured === false) {
      setHfResults([]);
      setHfLoading(false);
      return;
    }

    setHfLoading(true);
    const timer = setTimeout(async () => {
      try {
        const params = new URLSearchParams({ q: hfQuery.trim(), sort: hfSort, limit: '30' });
        const res = await fetch(`/api/datasets/hf?${params}`);
        if (!res.ok) throw new Error(`HTTP ${res.status}`);
        const json = await res.json();
        setHfResults(json.datasets ?? json ?? []);
      } catch {
        setHfResults([]);
      } finally {
        setHfLoading(false);
      }
    }, 300);

    return () => clearTimeout(timer);
  }, [hfQuery, hfSort, hfConfigured]);

  const handleDownload = useCallback(async (ds: any) => {
    const hfId = ds.id ?? ds.hfId;
    if (downloadingIds.has(hfId)) return;

    setDownloadingIds(prev => new Set(prev).add(hfId));
    try {
      const res = await fetch('/api/datasets', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          hfId,
          author: ds.author ?? hfId.split('/')[0] ?? '',
          name: hfId.split('/').pop() ?? hfId,
          description: ds.description ?? '',
          tags: ds.tags ?? [],
          downloads: ds.downloads ?? 0,
          likes: ds.likes ?? 0,
        }),
      });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      mutateDl();
    } catch (err) {
      console.error('Download failed:', err);
    } finally {
      setDownloadingIds(prev => {
        const next = new Set(prev);
        next.delete(hfId);
        return next;
      });
    }
  }, [downloadingIds, mutateDl]);

  const handleRetry = useCallback(async (ds: any) => {
    // Remove the failed record, then re-download
    try {
      await fetch(`/api/datasets?id=${ds.id}`, { method: 'DELETE' });
      await handleDownload({ id: ds.hfId, hfId: ds.hfId, author: ds.author, tags: ds.tags, downloads: ds.downloads, likes: ds.likes, description: ds.description });
    } catch (err) {
      console.error('Retry failed:', err);
    }
  }, [handleDownload]);

  const handleRemove = useCallback(async (id: string) => {
    try {
      await fetch(`/api/datasets?id=${id}`, { method: 'DELETE' });
      mutateDl();
    } catch (err) {
      console.error('Remove failed:', err);
    }
  }, [mutateDl]);

  const sortedDownloaded = [...downloadedDatasets].sort(
    (a, b) => new Date(b.downloadedAt ?? b.createdAt ?? 0).getTime() - new Date(a.downloadedAt ?? a.createdAt ?? 0).getTime()
  );

  return (
    <div>
      {hfConfigured === false && (
        <div style={{
          padding: '14px 18px', marginBottom: 16, borderRadius: 8,
          background: '#FFD21E10', border: '1px solid #FFD21E30',
          display: 'flex', alignItems: 'center', gap: 12,
          fontSize: 13,
        }}>
          <Key size={16} color="#FFD21E" />
          <div style={{ flex: 1 }}>
            <div style={{ fontWeight: 600, color: 'var(--text-1)' }}>HuggingFace API key not configured</div>
            <div style={{ fontSize: 12, color: 'var(--text-3)', marginTop: 2 }}>
              Connect your HuggingFace account to browse and download datasets
            </div>
          </div>
          <Link href="/providers" style={{
            padding: '6px 14px', borderRadius: 6, fontSize: 12, fontWeight: 600,
            background: '#FFD21E18', border: '1px solid #FFD21E40', color: '#B8860B',
            textDecoration: 'none',
          }}>
            Configure API Key
          </Link>
        </div>
      )}

    <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20, alignItems: 'start' }}>
      {/* ── Left: Browse HuggingFace ─────────────── */}
      <div>
        <div style={{
          fontSize: 14, fontWeight: 700, color: 'var(--text-1)', marginBottom: 14,
          display: 'flex', alignItems: 'center', gap: 8,
        }}>
          <Search size={15} color="#059669" />
          Browse HuggingFace
        </div>

        {/* Search + Sort */}
        <div style={{ display: 'flex', gap: 10, marginBottom: 16 }}>
          <div style={{ flex: 1, position: 'relative' }}>
            <Search
              size={14}
              color="var(--text-3)"
              style={{ position: 'absolute', left: 10, top: '50%', transform: 'translateY(-50%)' }}
            />
            <input
              className="input"
              placeholder={hfConfigured === false ? "Configure HuggingFace API key to search..." : "Search datasets... (e.g. alpaca, code, medical)"}
              value={hfQuery}
              onChange={e => setHfQuery(e.target.value)}
              disabled={hfConfigured === false}
              style={{ paddingLeft: 32, opacity: hfConfigured === false ? 0.5 : 1 }}
            />
          </div>
          <select
            className="input"
            value={hfSort}
            onChange={e => setHfSort(e.target.value)}
            style={{ width: 170, flexShrink: 0 }}
          >
            <option value="downloads">Most Downloads</option>
            <option value="likes">Most Likes</option>
            <option value="lastModified">Recently Updated</option>
          </select>
        </div>

        {/* Results */}
        {hfLoading ? (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr', gap: 10 }}>
            {Array.from({ length: 6 }).map((_, i) => (
              <div key={i} className="card" style={{ padding: 16 }}>
                <div style={{ display: 'flex', gap: 12, marginBottom: 10 }}>
                  <div style={{ width: 160, height: 14, borderRadius: 4, background: 'var(--border)', animation: 'pulse 1.5s infinite' }} />
                  <div style={{ width: 80, height: 14, borderRadius: 4, background: 'var(--border)', animation: 'pulse 1.5s infinite' }} />
                </div>
                <div style={{ display: 'flex', gap: 6, marginBottom: 10 }}>
                  {[60, 50, 70].map((w, j) => (
                    <div key={j} style={{ width: w, height: 18, borderRadius: 9, background: 'var(--border)', animation: 'pulse 1.5s infinite' }} />
                  ))}
                </div>
                <div style={{ width: '100%', height: 12, borderRadius: 4, background: 'var(--border)', animation: 'pulse 1.5s infinite' }} />
              </div>
            ))}
          </div>
        ) : hfResults.length > 0 ? (
          <div style={{ display: 'grid', gridTemplateColumns: '1fr', gap: 10 }}>
            {hfResults.map((ds: any) => {
              const hfId = ds.id ?? ds.hfId ?? '';
              const author = ds.author ?? hfId.split('/')[0] ?? '';
              const name = hfId.split('/').pop() ?? hfId;
              const tags: string[] = ds.tags ?? [];
              const isDownloading = downloadingIds.has(hfId);

              const sizeBytes = ds.size_bytes ?? 0;
              const hfUrl = `https://huggingface.co/datasets/${hfId}`;

              return (
                <div key={hfId} style={{
                  background: 'var(--bg-card, var(--bg-surface, var(--bg-2)))',
                  border: '1px solid var(--border)',
                  borderRadius: 8,
                  padding: 16,
                }}>
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 8 }}>
                    <div style={{ minWidth: 0, flex: 1 }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 2 }}>
                        <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)' }}>
                          {name}
                        </span>
                        <a
                          href={hfUrl}
                          target="_blank"
                          rel="noopener noreferrer"
                          title="View on HuggingFace Hub"
                          style={{ display: 'flex', color: 'var(--text-4)', flexShrink: 0 }}
                        >
                          <ExternalLink size={12} />
                        </a>
                      </div>
                      <div style={{ fontSize: 11, color: 'var(--text-3)' }}>{author}</div>
                    </div>
                    <button
                      className="btn btn-sm"
                      disabled={isDownloading || hfConfigured === false}
                      onClick={() => handleDownload(ds)}
                      style={{
                        background: isDownloading ? 'var(--bg-2)' : '#059669',
                        color: isDownloading ? 'var(--text-3)' : '#fff',
                        border: 'none',
                        display: 'flex', alignItems: 'center', gap: 5,
                        flexShrink: 0,
                      }}
                    >
                      {isDownloading
                        ? <><Loader2 size={12} className="spin" /> Downloading</>
                        : <><ArrowDownToLine size={12} /> Download</>
                      }
                    </button>
                  </div>

                  {/* Description */}
                  {ds.description && (
                    <p style={{
                      fontSize: 11, color: 'var(--text-3)', margin: '0 0 8px',
                      lineHeight: 1.4,
                      display: '-webkit-box', WebkitLineClamp: 2,
                      WebkitBoxOrient: 'vertical', overflow: 'hidden',
                    }}>
                      {ds.description}
                    </p>
                  )}

                  {/* Tags */}
                  {tags.length > 0 && (
                    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginBottom: 10 }}>
                      {tags.slice(0, 5).map(tag => (
                        <span key={tag} className="badge badge-neutral" style={{ fontSize: 10 }}>{tag}</span>
                      ))}
                      {tags.length > 5 && (
                        <span style={{ fontSize: 10, color: 'var(--text-3)' }}>+{tags.length - 5}</span>
                      )}
                    </div>
                  )}

                  {/* Stats — downloads, likes, size, est. time */}
                  <div style={{ display: 'flex', gap: 14, fontSize: 11, color: 'var(--text-3)', flexWrap: 'wrap' }}>
                    <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                      <ArrowDownToLine size={11} /> {fmtNum(ds.downloads ?? 0)}
                    </span>
                    <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                      <Heart size={11} /> {fmtNum(ds.likes ?? 0)}
                    </span>
                    {sizeBytes > 0 && (
                      <span style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                        <HardDrive size={11} /> {fmtBytes(sizeBytes)}
                      </span>
                    )}
                    {sizeBytes > 0 && (
                      <span style={{ display: 'flex', alignItems: 'center', gap: 4, color: 'var(--text-4)' }}>
                        <Clock size={11} /> ~{estimateDownloadTime(sizeBytes)}
                      </span>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        ) : hfQuery.trim() ? (
          <div style={{
            textAlign: 'center', padding: '48px 24px',
            color: 'var(--text-3)', fontSize: 13,
          }}>
            <Database size={32} style={{ opacity: 0.3, marginBottom: 12 }} />
            <div>No datasets found for &ldquo;{hfQuery}&rdquo;</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>Try a different search term</div>
          </div>
        ) : (
          <div style={{
            textAlign: 'center', padding: '48px 24px',
            color: 'var(--text-3)', fontSize: 13,
          }}>
            <Search size={32} style={{ opacity: 0.3, marginBottom: 12 }} />
            <div>Search for datasets on HuggingFace Hub</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>Type a query above to get started</div>
          </div>
        )}
      </div>

      {/* ── Right: Downloaded Datasets ────────────── */}
      <div>
        <div style={{
          fontSize: 14, fontWeight: 700, color: 'var(--text-1)', marginBottom: 14,
          display: 'flex', alignItems: 'center', gap: 8,
        }}>
          <HardDrive size={15} color="#059669" />
          Downloaded Datasets
          {sortedDownloaded.length > 0 && (
            <span className="badge badge-teal" style={{ fontSize: 10, marginLeft: 4 }}>
              {sortedDownloaded.length}
            </span>
          )}
        </div>

        {sortedDownloaded.length === 0 ? (
          <div className="card" style={{
            padding: '40px 20px', textAlign: 'center',
            color: 'var(--text-3)', fontSize: 13,
          }}>
            <ArrowDownToLine size={28} style={{ opacity: 0.3, marginBottom: 10 }} />
            <div>No downloaded datasets yet</div>
            <div style={{ fontSize: 11, marginTop: 4 }}>Search and download datasets from HuggingFace</div>
          </div>
        ) : (
          <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
            {sortedDownloaded.map((ds: any) => {
              const statusColor = ds.status === 'downloaded' || ds.status === 'ready'
                ? 'var(--success)'
                : ds.status === 'downloading'
                ? 'var(--amber)'
                : ds.status === 'error'
                ? 'var(--error)'
                : 'var(--text-3)';

              const StatusIcon = ds.status === 'downloaded' || ds.status === 'ready'
                ? CheckCircle2
                : ds.status === 'downloading'
                ? Loader2
                : ds.status === 'error'
                ? AlertTriangle
                : Clock;

              return (
                <div key={ds.id} className="card" style={{ padding: 14, position: 'relative', overflow: 'hidden' }}>
                  {ds.status === 'downloading' && (
                    <div style={{
                      position: 'absolute', top: 0, left: 0, right: 0, height: 2,
                      background: 'linear-gradient(90deg, transparent, #D97706, transparent)',
                      animation: 'shimmer 1.5s infinite',
                      borderRadius: '8px 8px 0 0',
                    }} />
                  )}
                  {/* Header */}
                  <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 8 }}>
                    <div style={{ minWidth: 0, flex: 1 }}>
                      <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', marginBottom: 2 }}>
                        {ds.name ?? ds.hfId}
                      </div>
                      {ds.hfId && (
                        <div style={{ fontSize: 11, color: 'var(--text-3)', display: 'flex', alignItems: 'center', gap: 4 }}>
                          <ExternalLink size={10} /> {ds.hfId}
                        </div>
                      )}
                    </div>
                    <span style={{
                      display: 'inline-flex', alignItems: 'center', gap: 4,
                      fontSize: 10, fontWeight: 600, color: statusColor,
                      textTransform: 'uppercase', letterSpacing: '0.05em',
                      flexShrink: 0,
                    }}>
                      <StatusIcon size={11} className={ds.status === 'downloading' ? 'spin' : ''} />
                      {ds.status}
                    </span>
                  </div>

                  {/* Error message */}
                  {ds.status === 'error' && ds.errorMessage && (
                    <div style={{
                      fontSize: 11, color: 'var(--error)', marginBottom: 8,
                      padding: '6px 8px', borderRadius: 4,
                      background: 'rgba(220,38,38,0.06)', border: '1px solid rgba(220,38,38,0.15)',
                      lineHeight: 1.4,
                      display: '-webkit-box', WebkitLineClamp: 3,
                      WebkitBoxOrient: 'vertical', overflow: 'hidden',
                    }}>
                      {ds.errorMessage}
                    </div>
                  )}

                  {/* Stats grid */}
                  {(ds.numRows > 0 || (ds.columns && ds.columns.length > 0) || ds.sizeBytes > 0) && (
                    <div style={{ display: 'flex', gap: 10, fontSize: 11, color: 'var(--text-3)', marginBottom: 8 }}>
                      {ds.numRows > 0 && (
                        <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                          <Rows3 size={11} /> {fmtNum(ds.numRows)} rows
                        </span>
                      )}
                      {ds.columns && ds.columns.length > 0 && (
                        <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                          <Columns3 size={11} /> {ds.columns.length} cols
                        </span>
                      )}
                      {ds.sizeBytes > 0 && (
                        <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                          <HardDrive size={11} /> {fmtBytes(ds.sizeBytes)}
                        </span>
                      )}
                    </div>
                  )}

                  {/* Splits info */}
                  {ds.splits && Object.keys(ds.splits).length > 0 && (
                    <div style={{ display: 'flex', flexWrap: 'wrap', gap: 4, marginBottom: 8 }}>
                      {Object.entries(ds.splits).map(([split, info]: [string, any]) => (
                        <span key={split} className="badge badge-neutral" style={{ fontSize: 10 }}>
                          {split}: {fmtNum(typeof info === 'number' ? info : info?.numRows ?? info?.rows ?? 0)} rows
                        </span>
                      ))}
                    </div>
                  )}

                  {/* Cache path */}
                  {ds.cachePath && (
                    <div
                      title={ds.cachePath}
                      style={{
                        fontSize: 10, color: 'var(--text-3)', marginBottom: 10,
                        overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                        fontFamily: 'var(--font-mono, monospace)',
                      }}
                    >
                      {ds.cachePath}
                    </div>
                  )}

                  {/* Actions */}
                  <div style={{ display: 'flex', gap: 6 }}>
                    {ds.status === 'error' ? (
                      <>
                        <button
                          className="btn btn-sm"
                          style={{ fontSize: 11, background: '#059669', color: '#fff', border: 'none', flex: 1, justifyContent: 'center' }}
                          onClick={() => handleRetry(ds)}
                        >
                          <RefreshCcw size={11} /> Retry Download
                        </button>
                        <button
                          className="btn btn-ghost btn-sm"
                          style={{ fontSize: 11, color: 'var(--error)' }}
                          onClick={() => handleRemove(ds.id)}
                        >
                          <Trash2 size={11} />
                        </button>
                      </>
                    ) : (
                      <>
                        <button className="btn btn-ghost btn-sm" style={{ fontSize: 11 }} disabled>
                          <Eye size={11} /> Preview
                        </button>
                        <button
                          className="btn btn-ghost btn-sm"
                          style={{ fontSize: 11, color: 'var(--error)' }}
                          onClick={() => handleRemove(ds.id)}
                        >
                          <Trash2 size={11} /> Remove
                        </button>
                      </>
                    )}
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>
    </div>

    {/* Shimmer animation for downloading status */}
    <style>{`
      @keyframes shimmer {
        0% { transform: translateX(-100%); }
        100% { transform: translateX(100%); }
      }
    `}</style>
    </div>
  );
}

/* ── Model Search Dropdown ─────────────────────────────── */

function ModelSearchDropdown({
  query, source, genType, localModels, onSelect, onClose, onDelete,
}: {
  query: string;
  source: 'ollama' | 'llamacpp' | 'huggingface';
  genType: 'text' | 'image' | 'audio';
  localModels: string[];
  onSelect: (name: string, pipelineTag?: string) => void;
  onClose: () => void;
  onDelete?: (name: string) => void;
}) {
  const [remoteResults, setRemoteResults] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);

  // Debounced remote search — only when 2+ chars typed
  useEffect(() => {
    if (query.trim().length < 2) { setRemoteResults([]); setSearched(false); return; }
    setLoading(true);
    setSearched(false);
    const timer = setTimeout(async () => {
      try {
        const params = new URLSearchParams({ q: query.trim(), source, gen_type: genType, limit: '10' });
        const res = await fetch(`/api/synthesis/models/search?${params}`);
        const data = await res.json();
        setRemoteResults(data.models ?? []);
      } catch {
        setRemoteResults([]);
      } finally {
        setLoading(false);
        setSearched(true);
      }
    }, 300);
    return () => clearTimeout(timer);
  }, [query, source, genType]);

  // Filter local models by query
  const q = query.toLowerCase();
  const localMatches = q
    ? localModels.filter(m => m.toLowerCase().includes(q))
    : localModels; // show ALL local when no query

  // Deduplicate: local names that also appear in remote
  const remoteNames = new Set(remoteResults.map(r => r.name));
  const localItems = localMatches
    .filter(m => !remoteNames.has(m))
    .map(m => ({ name: m, description: 'Installed locally', local: true, remote: false }));
  const remoteItems = remoteResults.map(r => ({ ...r, remote: true }));

  const hasLocal = localItems.length > 0;
  const hasRemote = remoteItems.length > 0;
  const hasAnything = hasLocal || hasRemote || loading;

  if (!hasAnything && !searched) {
    // No local models and haven't searched yet — show hint
  }

  const sourceLabel = source === 'ollama' ? 'Ollama Library' : source === 'huggingface' ? 'HuggingFace Hub' : 'llama.cpp';

  return (
    <>
      <div style={{ position: 'fixed', inset: 0, zIndex: 40 }} onClick={onClose} />
      <div style={{
        position: 'absolute', top: '100%', left: 0, right: 0, zIndex: 50,
        marginTop: 4, maxHeight: 300, overflowY: 'auto',
        background: 'var(--bg-surface)', border: '1px solid var(--border)',
        borderRadius: 8, boxShadow: '0 8px 24px rgba(0,0,0,0.15)',
      }}>
        {/* Local models section */}
        {hasLocal && (
          <>
            <div style={{
              padding: '6px 12px', fontSize: 10, fontWeight: 700, color: '#10B981',
              textTransform: 'uppercase', letterSpacing: '0.06em',
              borderBottom: '1px solid var(--border)',
              background: 'rgba(16,185,129,0.04)',
            }}>
              Installed Locally ({localItems.length})
            </div>
            {localItems.map((m, i) => (
              <ModelOption key={`local-${m.name}-${i}`} m={m} onSelect={onSelect} onDelete={onDelete} />
            ))}
          </>
        )}

        {/* Remote search results section */}
        {(hasRemote || loading || (searched && query.trim().length >= 2)) && (
          <>
            <div style={{
              padding: '6px 12px', fontSize: 10, fontWeight: 700,
              color: source === 'ollama' ? '#10B981' : source === 'huggingface' ? '#F59E0B' : '#3B82F6',
              textTransform: 'uppercase', letterSpacing: '0.06em',
              borderBottom: '1px solid var(--border)',
              borderTop: hasLocal ? '1px solid var(--border)' : 'none',
              background: 'var(--bg-elevated)',
              display: 'flex', alignItems: 'center', gap: 6,
            }}>
              <Search size={10} />
              {loading ? `Searching ${sourceLabel}...` : `${sourceLabel} (${remoteItems.length})`}
            </div>
            {loading && (
              <div style={{ padding: '10px 12px', fontSize: 11, color: 'var(--text-4)', display: 'flex', alignItems: 'center', gap: 6 }}>
                <Loader2 size={12} className="spin" /> Searching...
              </div>
            )}
            {remoteItems.map((m, i) => (
              <ModelOption key={`remote-${m.name}-${i}`} m={m} onSelect={onSelect} />
            ))}
            {searched && !loading && remoteItems.length === 0 && query.trim().length >= 2 && (
              <div style={{ padding: '10px 12px', fontSize: 11, color: 'var(--text-4)', textAlign: 'center' }}>
                No remote models found for &ldquo;{query}&rdquo;
              </div>
            )}
          </>
        )}

        {/* Empty state — no local, no search yet */}
        {!hasLocal && !hasRemote && !loading && !searched && (
          <div style={{ padding: '14px 12px', fontSize: 11, color: 'var(--text-4)', textAlign: 'center' }}>
            {source === 'llamacpp'
              ? 'No models loaded in llama.cpp server — type a model path above'
              : `Type 2+ characters to search ${sourceLabel}`
            }
          </div>
        )}
      </div>
    </>
  );
}

function ModelOption({ m, onSelect, onDelete }: {
  m: any;
  onSelect: (name: string, pipelineTag?: string) => void;
  onDelete?: (name: string) => void;
}) {
  const [deleting, setDeleting] = useState(false);

  return (
    <div
      style={{
        display: 'flex', alignItems: 'center', gap: 8,
        width: '100%', padding: '8px 12px',
        background: 'transparent', cursor: 'pointer',
        textAlign: 'left', fontSize: 12,
        transition: 'background 0.1s',
      }}
      onMouseEnter={e => { e.currentTarget.style.background = 'var(--bg-elevated)'; }}
      onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
    >
      <div
        style={{ flex: 1, minWidth: 0, cursor: 'pointer' }}
        onClick={() => onSelect(m.name, m.pipeline_tag ?? m.pipelineTag)}
      >
        <div style={{ fontWeight: 500, color: 'var(--text-1)', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {m.name}
        </div>
        {m.description && (
          <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 1 }}>{m.description}</div>
        )}
      </div>
      {m.local ? (
        <span style={{
          fontSize: 9, fontWeight: 600, padding: '2px 5px', borderRadius: 3,
          background: 'rgba(16,185,129,0.1)', color: '#10B981',
          border: '1px solid rgba(16,185,129,0.2)', flexShrink: 0,
        }}>LOCAL</span>
      ) : (
        <span style={{
          fontSize: 9, fontWeight: 600, padding: '2px 5px', borderRadius: 3,
          background: 'rgba(107,114,128,0.08)', color: 'var(--text-4)',
          border: '1px solid var(--border)', flexShrink: 0,
        }}>REMOTE</span>
      )}
      {m.local && onDelete && (
        <button
          title={`Delete ${m.name}`}
          onClick={async (e) => {
            e.stopPropagation();
            if (deleting) return;
            setDeleting(true);
            try {
              await onDelete(m.name);
            } finally {
              setDeleting(false);
            }
          }}
          style={{
            background: 'none', border: 'none', cursor: deleting ? 'wait' : 'pointer',
            color: 'var(--text-4)', display: 'flex', padding: 3, borderRadius: 3,
            transition: 'color 0.15s, background 0.15s',
            flexShrink: 0, opacity: deleting ? 0.4 : 1,
          }}
          onMouseEnter={e => { e.currentTarget.style.color = '#DC2626'; e.currentTarget.style.background = 'rgba(220,38,38,0.08)'; }}
          onMouseLeave={e => { e.currentTarget.style.color = 'var(--text-4)'; e.currentTarget.style.background = 'none'; }}
        >
          {deleting ? <Loader2 size={12} className="spin" /> : <Trash2 size={12} />}
        </button>
      )}
    </div>
  );
}

/* ── Synthesis Edit Modal ──────────────────────────────── */

function SynthesisEditModal({
  job, onClose, onSave,
}: {
  job: any;
  onClose: () => void;
  onSave: (id: string, updates: Record<string, unknown>, restart: boolean) => Promise<void>;
}) {
  const [name, setName] = useState(job.name ?? '');
  const [description, setDescription] = useState(job.description ?? '');
  const [source, setSource] = useState(job.source ?? 'ollama');
  const [model, setModel] = useState(job.model ?? '');
  const [outputFormat, setOutputFormat] = useState(job.outputFormat ?? 'jsonl');
  const [targetSamples, setTargetSamples] = useState(job.targetSamples ?? 100);
  const [temperature, setTemperature] = useState(Number(job.temperature) || 0.8);
  const [batchSize, setBatchSize] = useState(job.batchSize ?? 5);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');

  const isRunning = job.status === 'running' || job.status === 'queued';
  const hasChanges = name !== job.name || description !== (job.description ?? '') ||
    source !== job.source || model !== job.model || outputFormat !== (job.outputFormat ?? 'jsonl') ||
    targetSamples !== (job.targetSamples ?? 100) || temperature !== (Number(job.temperature) || 0.8) ||
    batchSize !== (job.batchSize ?? 5);

  const handleSave = async (restart: boolean) => {
    if (!name.trim()) { setError('Name is required'); return; }
    if (!model.trim()) { setError('Model is required'); return; }
    setSaving(true);
    setError('');
    try {
      await onSave(job.id, { name, description, source, model, outputFormat, targetSamples, temperature, batchSize }, restart);
      onClose();
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Save failed');
    } finally {
      setSaving(false);
    }
  };

  const LABEL: React.CSSProperties = {
    fontSize: 11, fontWeight: 700, color: 'var(--text-3)',
    textTransform: 'uppercase', letterSpacing: '0.08em',
    display: 'block', marginBottom: 5,
  };

  return (
    <div style={{
      position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
      backdropFilter: 'blur(4px)', zIndex: 200,
      display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: 60,
    }} onClick={onClose}>
      <div
        onClick={e => e.stopPropagation()}
        style={{
          width: 540, maxWidth: '92vw', maxHeight: '80vh', overflowY: 'auto',
          background: 'var(--bg-surface)', border: '1px solid var(--border)',
          borderRadius: 12, boxShadow: '0 24px 64px rgba(0,0,0,0.3)',
        }}
      >
        {/* Header */}
        <div style={{
          padding: '16px 20px', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
            <Sparkles size={16} color="var(--teal)" />
            <span style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>Edit Synthesis Job</span>
          </div>
          <button onClick={onClose} style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-3)', display: 'flex', padding: 4 }}>
            <X size={16} />
          </button>
        </div>

        {/* Status bar */}
        <div style={{
          padding: '10px 20px', background: 'var(--bg-elevated)', borderBottom: '1px solid var(--border)',
          display: 'flex', alignItems: 'center', gap: 10, fontSize: 12,
        }}>
          <span style={{
            padding: '2px 8px', borderRadius: 4, fontSize: 10, fontWeight: 700,
            textTransform: 'uppercase',
            background: isRunning ? 'rgba(245,158,11,0.1)' : job.status === 'completed' ? 'rgba(16,185,129,0.1)' : 'var(--bg)',
            color: isRunning ? '#D97706' : job.status === 'completed' ? '#059669' : 'var(--text-3)',
            border: `1px solid ${isRunning ? '#D9770630' : job.status === 'completed' ? '#05966930' : 'var(--border)'}`,
          }}>
            {job.status}
          </span>
          {job.currentSamples > 0 && (
            <span style={{ color: 'var(--text-3)' }}>
              {fmtNum(job.currentSamples)} / {fmtNum(job.targetSamples)} samples
            </span>
          )}
          {job.tokensUsed > 0 && (
            <span style={{ color: 'var(--text-4)' }}>· {fmtNum(job.tokensUsed)} tokens</span>
          )}
        </div>

        {/* Form */}
        <div style={{ padding: '18px 20px', display: 'flex', flexDirection: 'column', gap: 14 }}>
          {/* Name */}
          <div>
            <label style={LABEL}>Name *</label>
            <input className="input" style={{ width: '100%', fontSize: 13 }} value={name} onChange={e => setName(e.target.value)} />
          </div>

          {/* Description */}
          <div>
            <label style={LABEL}>Description</label>
            <textarea className="textarea" style={{ width: '100%', minHeight: 70, fontSize: 12 }}
              value={description} onChange={e => setDescription(e.target.value)} />
          </div>

          {/* Source + Model */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 2fr', gap: 12 }}>
            <div>
              <label style={LABEL}>Source</label>
              <select className="input" style={{ width: '100%', fontSize: 12 }} value={source} onChange={e => setSource(e.target.value)}>
                <option value="ollama">Ollama</option>
                <option value="llamacpp">llama.cpp</option>
                <option value="huggingface">HuggingFace</option>
              </select>
            </div>
            <div>
              <label style={LABEL}>Model *</label>
              <input className="input" style={{ width: '100%', fontSize: 12, fontFamily: 'var(--font-mono, monospace)' }}
                value={model} onChange={e => setModel(e.target.value)} />
            </div>
          </div>

          {/* Format + Samples */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <div>
              <label style={LABEL}>Output Format</label>
              <select className="input" style={{ width: '100%', fontSize: 12 }} value={outputFormat} onChange={e => setOutputFormat(e.target.value)}>
                <option value="jsonl">JSONL</option>
                <option value="csv">CSV</option>
                <option value="alpaca">Alpaca</option>
                <option value="chatml">ChatML</option>
                <option value="sharegpt">ShareGPT</option>
                <option value="delta">Delta</option>
              </select>
            </div>
            <div>
              <label style={LABEL}>Target Samples</label>
              <input type="number" className="input" style={{ width: '100%', fontSize: 12 }}
                value={targetSamples} onChange={e => setTargetSamples(Number(e.target.value) || 100)} min={10} max={100000} />
            </div>
          </div>

          {/* Temp + Batch */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
            <div>
              <label style={LABEL}>Temperature ({temperature.toFixed(1)})</label>
              <input type="range" min={0} max={2} step={0.1} style={{ width: '100%' }}
                value={temperature} onChange={e => setTemperature(Number(e.target.value))} />
            </div>
            <div>
              <label style={LABEL}>Batch Size</label>
              <input type="number" className="input" style={{ width: '100%', fontSize: 12 }}
                value={batchSize} onChange={e => setBatchSize(Math.max(1, Math.min(20, Number(e.target.value) || 1)))} min={1} max={20} />
            </div>
          </div>

          {error && (
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 12, color: '#ef4444' }}>
              <AlertTriangle size={13} /> {error}
            </div>
          )}
        </div>

        {/* Footer */}
        <div style={{
          padding: '14px 20px', borderTop: '1px solid var(--border)',
          display: 'flex', gap: 8, justifyContent: 'flex-end',
        }}>
          <button onClick={onClose} style={{
            padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 500,
            border: '1px solid var(--border-md)', background: 'transparent',
            color: 'var(--text-3)', cursor: 'pointer',
          }}>Cancel</button>
          {/* Save only (name/description change, no restart) */}
          <button onClick={() => handleSave(false)} disabled={saving || !name.trim()} style={{
            padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 600,
            border: '1px solid var(--border-md)', background: 'var(--bg-elevated)',
            color: 'var(--text-1)', cursor: saving ? 'wait' : 'pointer',
            opacity: saving || !name.trim() ? 0.5 : 1,
          }}>
            {saving ? 'Saving...' : 'Save'}
          </button>
          {/* Save & restart (if configs changed) */}
          {hasChanges && (
            <button onClick={() => handleSave(true)} disabled={saving || !model.trim()} style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: 700,
              border: '1.5px solid var(--teal)',
              background: 'rgba(5,150,105,0.08)', color: 'var(--teal)',
              cursor: saving ? 'wait' : 'pointer',
              opacity: saving || !model.trim() ? 0.5 : 1,
            }}>
              <RefreshCcw size={12} />
              Save &amp; Restart
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

/* ── Main Page ─────────────────────────────────────────── */

export default function DataSynthesisPage() {
  const [tab, setTab] = useState<'datasets' | 'huggingface' | 'generate'>('datasets');

  // Database-backed datasets
  const { data: datasetsData, isLoading: datasetsLoading } = useSWR<{ datasets: Dataset[] }>('/api/data/datasets', fetcher);
  const myDatasets = datasetsData?.datasets ?? [];

  // Synthesis form state
  const [synthName, setSynthName] = useState('');
  const [synthDescription, setSynthDescription] = useState('');
  const [synthGenType, setSynthGenType] = useState<'text' | 'image' | 'audio'>('text');
  const [synthSource, setSynthSource] = useState<'ollama' | 'llamacpp' | 'huggingface'>('ollama');
  const [synthModel, setSynthModel] = useState('');
  const [synthModelPipeline, setSynthModelPipeline] = useState<string | null>(null); // pipeline_tag of selected model
  const [synthFormat, setSynthFormat] = useState('jsonl');
  const [synthTarget, setSynthTarget] = useState(1000);
  const [synthTemp, setSynthTemp] = useState(0.8);
  const [synthBatch, setSynthBatch] = useState(5);
  const [synthSystemPrompt, setSynthSystemPrompt] = useState('');
  const [synthShowSystem, setSynthShowSystem] = useState(false);
  const [synthSaveQdrant, setSynthSaveQdrant] = useState(false);
  const [synthSubmitting, setSynthSubmitting] = useState(false);
  const [synthModelSearchOpen, setSynthModelSearchOpen] = useState(false);

  // Models fetched from engine — extract name strings from model objects
  const [availableModels, setAvailableModels] = useState<{ ollama: string[]; llamacpp: string[]; huggingface: string[] }>({ ollama: [], llamacpp: [], huggingface: [] });
  const refreshModels = useCallback(() => {
    fetch('/api/synthesis/models')
      .then(r => r.json())
      .then(d => {
        const extract = (arr: any[]) => (arr ?? []).map((m: any) => typeof m === 'string' ? m : m.name ?? '').filter(Boolean);
        setAvailableModels({
          ollama: extract(d.ollama),
          llamacpp: extract(d.llamacpp),
          huggingface: extract(d.huggingface),
        });
      })
      .catch(() => {});
  }, []);
  useEffect(() => { refreshModels(); }, [refreshModels]);

  const handleDeleteModel = useCallback(async (modelName: string) => {
    try {
      const res = await fetch('/api/synthesis/models/delete', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ source: synthSource, model: modelName }),
      });
      if (!res.ok) throw new Error('Delete failed');
      // Clear selection if deleted model was selected
      if (synthModel === modelName) setSynthModel('');
      // Refresh models list
      refreshModels();
    } catch (err) {
      console.error('Model delete failed:', err);
    }
  }, [synthSource, synthModel, refreshModels]);

  // Synthesis jobs list
  const { data: synthJobsData, mutate: mutateSynthJobs } = useSWR('/api/synthesis', fetcher, { refreshInterval: 5000 });
  const synthJobs: any[] = synthJobsData?.jobs ?? [];
  const hasActiveJobs = synthJobs.some((j: any) => j.status === 'running' || j.status === 'queued');

  // System stats — only poll when jobs are running
  const { data: sysStats } = useSWR(
    hasActiveJobs ? '/api/system/stats' : null,
    fetcher,
    { refreshInterval: 3000 },
  );

  const handleStartSynthesis = async () => {
    if (!synthName.trim() || !synthModel.trim()) return;
    setSynthSubmitting(true);
    try {
      const res = await fetch('/api/synthesis', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          name: synthName,
          description: synthDescription,
          source: synthSource,
          model: synthModel,
          systemPrompt: synthSystemPrompt || undefined,
          targetSamples: synthTarget,
          outputFormat: synthFormat,
          temperature: synthTemp,
          maxTokens: 1024,
          batchSize: synthBatch,
          saveToQdrant: synthSaveQdrant,
        }),
      });
      if (res.ok) {
        mutateSynthJobs();
        setSynthName('');
        setSynthDescription('');
        setSynthModel('');
        setSynthSystemPrompt('');
      }
    } catch (err) {
      console.error('Synthesis start failed:', err);
    } finally {
      setSynthSubmitting(false);
    }
  };

  const [editJob, setEditJob] = useState<any>(null);

  const handleSaveSynthJob = async (id: string, updates: Record<string, unknown>, restart: boolean) => {
    const res = await fetch('/api/synthesis', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ id, ...updates, restart }),
    });
    if (!res.ok) throw new Error('Save failed');
    mutateSynthJobs();
  };

  const handleCancelSynthJob = async (id: string) => {
    try {
      await fetch('/api/synthesis', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ id, status: 'cancelled' }),
      });
      mutateSynthJobs();
    } catch (err) {
      console.error('Cancel failed:', err);
    }
  };

  const handleRemoveSynthJob = async (id: string) => {
    try {
      await fetch(`/api/synthesis?id=${id}`, { method: 'DELETE' });
      mutateSynthJobs();
    } catch (err) {
      console.error('Remove job failed:', err);
    }
  };

  // Model compatibility check
  const GEN_TYPE_PIPELINES: Record<string, string[]> = {
    text: ['text-generation', 'text2text-generation', 'summarization', 'translation', 'fill-mask', 'question-answering'],
    image: ['text-to-image', 'image-to-image', 'image-classification', 'unconditional-image-generation'],
    audio: ['text-to-speech', 'text-to-audio', 'automatic-speech-recognition', 'audio-classification'],
  };
  const modelMismatch = synthModel && synthModelPipeline && !GEN_TYPE_PIPELINES[synthGenType]?.includes(synthModelPipeline);

  const readyCount     = myDatasets.filter(d => d.status === 'ready').length;
  const generatingCount = myDatasets.filter(d => d.status === 'generating').length;
  const totalSamples   = myDatasets.reduce((s, d) => s + d.sampleCount, 0);

  return (
    <div style={{ padding: 24, maxWidth: 1200, margin: '0 auto' }}>

      {/* Header */}
      <div style={{
        display: 'flex', alignItems: 'center', justifyContent: 'space-between',
        marginBottom: 24,
      }}>
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Data Synthesis
          </h1>
          <p style={{ fontSize: 13, color: 'var(--text-3)', margin: '4px 0 0' }}>
            Generate, manage, and export high-quality training datasets
          </p>
        </div>
        <button className="btn btn-primary btn-sm" onClick={() => setTab('generate')}>
          <Sparkles size={13} /> Synthesize Data
        </button>
      </div>

      {/* Stats */}
      <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4,1fr)', gap: 12, marginBottom: 20 }}>
        {[
          { label: 'TOTAL DATASETS',   value: datasetsLoading ? '...' : String(myDatasets.length), color: 'var(--teal)',    icon: Database },
          { label: 'READY',            value: String(readyCount),            color: 'var(--success)', icon: CheckCircle2 },
          { label: 'GENERATING',       value: String(generatingCount),       color: 'var(--amber)',   icon: RefreshCcw },
          { label: 'TOTAL SAMPLES',    value: fmt(totalSamples),             color: 'var(--primary)', icon: Zap },
        ].map(stat => (
          <div key={stat.label} className="metric-card" style={{ position: 'relative', overflow: 'hidden' }}>
            <div style={{ position: 'absolute', top: 0, left: 0, right: 0, height: 2, background: stat.color, opacity: 0.7 }} />
            <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start' }}>
              <div>
                <div className="metric-value">{stat.value}</div>
                <div className="metric-label" style={{ marginTop: 6 }}>{stat.label}</div>
              </div>
              <stat.icon size={16} color={stat.color} />
            </div>
          </div>
        ))}
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 20 }}>
        {([
          { key: 'datasets', label: 'My Datasets' },
          { key: 'huggingface', label: 'HuggingFace Hub' },
          { key: 'generate', label: 'Generate New' },
        ] as const).map(t => (
          <button
            key={t.key}
            onClick={() => setTab(t.key)}
            style={{
              padding: '10px 18px',
              background: 'none', border: 'none',
              borderBottom: `2px solid ${tab === t.key ? 'var(--teal)' : 'transparent'}`,
              cursor: 'pointer', fontSize: 13,
              fontWeight: tab === t.key ? 600 : 400,
              color: tab === t.key ? 'var(--text-1)' : 'var(--text-3)',
              marginBottom: -1,
            }}
          >
            {t.label}
          </button>
        ))}
      </div>

      {/* My Datasets */}
      {tab === 'datasets' && (
        datasetsLoading ? (
          <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13, display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 8 }}>
            <Loader2 size={16} className="animate-spin" /> Loading datasets...
          </div>
        ) : myDatasets.length === 0 ? (
          <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)', fontSize: 13 }}>
            No datasets yet. Switch to the &quot;Generate New&quot; tab or import from HuggingFace Hub.
          </div>
        ) : (
          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(320px,1fr))', gap: 12 }}>
            {myDatasets.map(ds => <DatasetCard key={ds.id} ds={ds} />)}
          </div>
        )
      )}

      {/* HuggingFace Hub */}
      {tab === 'huggingface' && <HuggingFaceHubTab />}

      {/* Generate */}
      {tab === 'generate' && (
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 20, alignItems: 'start' }}>
          {/* ── Left: Synthesis Form ────────────────── */}
          <div className="card" style={{ padding: 24 }}>
            <h2 style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)', margin: '0 0 20px' }}>
              Synthesize New Dataset
            </h2>
            <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
              {/* Name */}
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Dataset Name *
                </label>
                <input
                  className="input"
                  placeholder="e.g. Legal Contract Analysis v2"
                  value={synthName}
                  onChange={e => setSynthName(e.target.value)}
                />
              </div>

              {/* Description */}
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Description *
                </label>
                <textarea
                  className="textarea"
                  style={{ minHeight: 100 }}
                  placeholder="Describe what kind of training data to generate. Be specific about the domain, task type, complexity, and quality requirements..."
                  value={synthDescription}
                  onChange={e => setSynthDescription(e.target.value)}
                />
              </div>

              {/* Generation Type */}
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Generation Type
                </label>
                <select
                  className="input"
                  value={synthGenType}
                  onChange={e => { setSynthGenType(e.target.value as 'text' | 'image' | 'audio'); setSynthModelPipeline(null); }}
                >
                  <option value="text">📝 Text — code, structured data, Q&amp;A</option>
                  <option value="image">🖼️ Image — generation &amp; editing</option>
                  <option value="audio">🔊 Audio — speech, music, sound</option>
                </select>
              </div>

              {/* Model Source */}
              <div>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 8 }}>
                  Model Source
                </label>
                <div style={{ display: 'flex', gap: 8 }}>
                  {([
                    { key: 'ollama' as const, label: 'Ollama', color: '#10B981', icon: Server },
                    { key: 'llamacpp' as const, label: 'llama.cpp', color: '#3B82F6', icon: Cpu },
                    { key: 'huggingface' as const, label: 'HuggingFace', color: '#F59E0B', icon: Sparkle },
                  ]).map(s => (
                    <button
                      key={s.key}
                      onClick={() => { setSynthSource(s.key); setSynthModel(''); }}
                      style={{
                        flex: 1, padding: '10px 12px', borderRadius: 8,
                        border: `1.5px solid ${synthSource === s.key ? s.color : 'var(--border)'}`,
                        background: synthSource === s.key ? `${s.color}10` : 'var(--bg)',
                        cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
                        fontSize: 12, fontWeight: synthSource === s.key ? 700 : 500,
                        color: synthSource === s.key ? s.color : 'var(--text-3)',
                        transition: 'all 0.15s',
                      }}
                    >
                      <s.icon size={14} />
                      {s.label}
                    </button>
                  ))}
                </div>
              </div>

              {/* Model Selector — shows local models on focus, searches remote on typing */}
              <div style={{ position: 'relative' }}>
                <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                  Model *
                </label>
                <div style={{ position: 'relative' }}>
                  <Search size={14} color="var(--text-4)" style={{ position: 'absolute', left: 10, top: '50%', transform: 'translateY(-50%)' }} />
                  <input
                    className="input"
                    style={{ paddingLeft: 32 }}
                    placeholder={
                      synthSource === 'ollama' ? 'Select installed model or search Ollama library...'
                        : synthSource === 'llamacpp' ? 'Select loaded model or type model path...'
                        : 'Select downloaded model or search HuggingFace Hub...'
                    }
                    value={synthModel}
                    onChange={e => {
                      setSynthModel(e.target.value);
                      setSynthModelSearchOpen(true);
                    }}
                    onFocus={() => setSynthModelSearchOpen(true)}
                  />
                  {synthModel && (
                    <button
                      onClick={() => { setSynthModel(''); setSynthModelSearchOpen(true); }}
                      style={{
                        position: 'absolute', right: 8, top: '50%', transform: 'translateY(-50%)',
                        background: 'none', border: 'none', cursor: 'pointer',
                        color: 'var(--text-4)', display: 'flex', padding: 2,
                      }}
                    >
                      <X size={14} />
                    </button>
                  )}
                </div>

                {/* Dropdown — shows on focus, local models always visible, remote on search */}
                {synthModelSearchOpen && (
                  <ModelSearchDropdown
                    query={synthModel}
                    source={synthSource}
                    genType={synthGenType}
                    localModels={availableModels[synthSource]}
                    onSelect={(name, pipelineTag) => { setSynthModel(name); setSynthModelPipeline(pipelineTag ?? null); setSynthModelSearchOpen(false); }}
                    onClose={() => setSynthModelSearchOpen(false)}
                    onDelete={synthSource === 'ollama' ? handleDeleteModel : undefined}
                  />
                )}

                {synthSource === 'huggingface' && !modelMismatch && (
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 4 }}>
                    Requires HuggingFace API key configured in Providers
                  </div>
                )}

                {/* Model compatibility warning */}
                {modelMismatch && (
                  <div style={{
                    marginTop: 6, padding: '8px 10px', borderRadius: 6,
                    background: 'rgba(245,158,11,0.08)', border: '1px solid rgba(245,158,11,0.25)',
                    display: 'flex', alignItems: 'flex-start', gap: 8, fontSize: 11,
                  }}>
                    <AlertTriangle size={14} color="#F59E0B" style={{ flexShrink: 0, marginTop: 1 }} />
                    <div>
                      <div style={{ fontWeight: 600, color: '#B45309' }}>Model may not be compatible</div>
                      <div style={{ color: 'var(--text-3)', marginTop: 2 }}>
                        <strong>{synthModel}</strong> is a <em>{synthModelPipeline}</em> model, but you selected <em>{synthGenType}</em> generation.
                        This may produce unexpected results. Consider selecting a different model suited for {synthGenType} generation.
                      </div>
                    </div>
                  </div>
                )}
              </div>

              {/* Output Format + Target Samples */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div>
                  <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                    Output Format
                  </label>
                  <select className="input" value={synthFormat} onChange={e => setSynthFormat(e.target.value)}>
                    <option value="jsonl">JSONL</option>
                    <option value="csv">CSV</option>
                    <option value="alpaca">Alpaca</option>
                    <option value="chatml">ChatML</option>
                    <option value="sharegpt">ShareGPT</option>
                    <option value="delta">Delta</option>
                  </select>
                </div>
                <div>
                  <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                    Target Samples
                  </label>
                  <input
                    type="number"
                    className="input"
                    value={synthTarget}
                    onChange={e => setSynthTarget(Math.max(10, Math.min(100000, Number(e.target.value) || 10)))}
                    min={10} max={100000}
                  />
                </div>
              </div>

              {/* Temperature + Batch Size */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
                <div>
                  <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                    Temperature
                    <span className="mono" style={{ marginLeft: 8, color: 'var(--text-1)', fontWeight: 600 }}>{synthTemp.toFixed(1)}</span>
                  </label>
                  <input
                    type="range"
                    min={0} max={2} step={0.1}
                    value={synthTemp}
                    onChange={e => setSynthTemp(Number(e.target.value))}
                    style={{ width: '100%', accentColor: 'var(--teal)' }}
                  />
                  <div style={{ display: 'flex', justifyContent: 'space-between', fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>
                    <span>Precise</span>
                    <span>Creative</span>
                  </div>
                </div>
                <div>
                  <label style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-3)', textTransform: 'uppercase', letterSpacing: '0.08em', display: 'block', marginBottom: 6 }}>
                    Batch Size
                  </label>
                  <input
                    type="number"
                    className="input"
                    value={synthBatch}
                    onChange={e => setSynthBatch(Math.max(1, Math.min(20, Number(e.target.value) || 1)))}
                    min={1} max={20}
                  />
                  <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 2 }}>
                    Parallel generations (1-20)
                  </div>
                </div>
              </div>

              {/* System Prompt (collapsible) */}
              <div>
                <button
                  onClick={() => setSynthShowSystem(!synthShowSystem)}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 6,
                    background: 'none', border: 'none', cursor: 'pointer',
                    fontSize: 11, fontWeight: 700, color: 'var(--text-3)',
                    textTransform: 'uppercase', letterSpacing: '0.08em', padding: 0,
                  }}
                >
                  {synthShowSystem ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                  System Prompt (optional)
                </button>
                {synthShowSystem && (
                  <textarea
                    className="textarea"
                    style={{ minHeight: 80, marginTop: 8 }}
                    placeholder="Custom system prompt for the generation model..."
                    value={synthSystemPrompt}
                    onChange={e => setSynthSystemPrompt(e.target.value)}
                  />
                )}
              </div>

              {/* Store in Qdrant */}
              <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
                <input
                  type="checkbox"
                  checked={synthSaveQdrant}
                  onChange={e => setSynthSaveQdrant(e.target.checked)}
                  style={{ accentColor: 'var(--teal)' }}
                />
                <span style={{ fontSize: 12, color: 'var(--text-2)' }}>Store generated data in Qdrant vector database</span>
              </label>

              {/* Submit */}
              <div style={{ display: 'flex', gap: 8, paddingTop: 4 }}>
                <button
                  className="btn btn-primary"
                  onClick={handleStartSynthesis}
                  disabled={!synthName.trim() || !synthModel.trim() || synthSubmitting}
                >
                  {synthSubmitting
                    ? <><Loader2 size={14} className="spin" /> Starting...</>
                    : <><Sparkles size={14} /> Start Synthesis</>
                  }
                </button>
              </div>
            </div>
          </div>

          {/* ── Right: Active Jobs ──────────────────── */}
          <div>
            <div style={{
              fontSize: 14, fontWeight: 700, color: 'var(--text-1)', marginBottom: 14,
              display: 'flex', alignItems: 'center', gap: 8,
            }}>
              <Zap size={15} color="var(--teal)" />
              Synthesis Jobs
              {synthJobs.length > 0 && (
                <span className="badge badge-teal" style={{ fontSize: 10, marginLeft: 4 }}>
                  {synthJobs.length}
                </span>
              )}
            </div>

            {synthJobs.length === 0 ? (
              <div className="card" style={{
                padding: '48px 20px', textAlign: 'center',
                color: 'var(--text-3)', fontSize: 13,
              }}>
                <Sparkles size={28} style={{ opacity: 0.3, marginBottom: 10 }} />
                <div>No synthesis jobs yet</div>
                <div style={{ fontSize: 11, marginTop: 4 }}>Configure and start a synthesis to see progress here</div>
              </div>
            ) : (
              <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
                {synthJobs.map((job: any) => {
                  const isActive = job.status === 'running' || job.status === 'queued';
                  const isCompleted = job.status === 'completed';
                  const isFailed = job.status === 'failed';
                  const isCancelled = job.status === 'cancelled';

                  const statusColor = isCompleted ? 'var(--success)'
                    : isActive ? 'var(--amber)'
                    : isFailed ? 'var(--error)'
                    : 'var(--text-3)';

                  const StatusIcon = isCompleted ? CheckCircle2
                    : job.status === 'running' ? Loader2
                    : job.status === 'queued' ? Clock
                    : isFailed ? AlertTriangle
                    : Clock;

                  const progress = job.targetSamples > 0
                    ? Math.round((job.currentSamples / job.targetSamples) * 100)
                    : job.progress ?? 0;

                  const sourceColor = job.source === 'ollama' ? '#10B981'
                    : job.source === 'llamacpp' ? '#3B82F6'
                    : '#F59E0B';

                  const SourceIcon = job.source === 'ollama' ? Server
                    : job.source === 'llamacpp' ? Cpu
                    : Sparkle;

                  // Duration calc
                  let duration = '';
                  if (job.startedAt) {
                    const end = job.completedAt ? new Date(job.completedAt) : new Date();
                    const secs = Math.round((end.getTime() - new Date(job.startedAt).getTime()) / 1000);
                    if (secs < 60) duration = `${secs}s`;
                    else if (secs < 3600) duration = `${Math.floor(secs / 60)}m ${secs % 60}s`;
                    else duration = `${(secs / 3600).toFixed(1)}h`;
                  }

                  return (
                    <div key={job.id} className="card" style={{ padding: 14, position: 'relative', overflow: 'hidden' }}>
                      {job.status === 'running' && (
                        <div style={{
                          position: 'absolute', top: 0, left: 0, right: 0, height: 2,
                          background: 'linear-gradient(90deg, transparent, var(--amber), transparent)',
                          animation: 'shimmer 1.5s infinite',
                          borderRadius: '8px 8px 0 0',
                        }} />
                      )}

                      {/* Header — clickable name opens edit modal */}
                      <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'flex-start', marginBottom: 8 }}>
                        <div style={{ minWidth: 0, flex: 1 }}>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 3 }}>
                            <span
                              onClick={() => setEditJob(job)}
                              style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-1)', cursor: 'pointer', textDecoration: 'none' }}
                              onMouseEnter={e => { e.currentTarget.style.textDecoration = 'underline'; }}
                              onMouseLeave={e => { e.currentTarget.style.textDecoration = 'none'; }}
                              title="Click to edit"
                            >{job.name}</span>
                          </div>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 6, fontSize: 11, color: 'var(--text-3)' }}>
                            <span style={{
                              display: 'inline-flex', alignItems: 'center', gap: 3,
                              padding: '1px 6px', borderRadius: 4,
                              background: `${sourceColor}15`, color: sourceColor,
                              fontSize: 10, fontWeight: 600,
                            }}>
                              <SourceIcon size={10} />
                              {job.source}
                            </span>
                            <span className="mono" style={{ fontSize: 11 }}>{job.model}</span>
                          </div>
                        </div>
                        <span style={{
                          display: 'inline-flex', alignItems: 'center', gap: 4,
                          fontSize: 10, fontWeight: 600, color: statusColor,
                          textTransform: 'uppercase', letterSpacing: '0.05em',
                          flexShrink: 0,
                        }}>
                          <StatusIcon size={11} className={job.status === 'running' ? 'spin' : ''} />
                          {job.status}
                        </span>
                      </div>

                      {/* Progress bar */}
                      {(isActive || isCompleted) && (
                        <div style={{ marginBottom: 8 }}>
                          <div style={{
                            display: 'flex', justifyContent: 'space-between',
                            fontSize: 10, color: 'var(--text-3)', marginBottom: 4,
                          }}>
                            <span>{fmtNum(job.currentSamples ?? 0)} / {fmtNum(job.targetSamples ?? 0)} samples</span>
                            <span className="mono">{progress}%</span>
                          </div>
                          <div className="progress-track">
                            <div
                              className="progress-fill"
                              style={{
                                width: `${Math.min(100, progress)}%`,
                                background: isCompleted ? 'var(--success)' : 'var(--amber)',
                                transition: 'width 0.5s ease',
                              }}
                            />
                          </div>
                        </div>
                      )}

                      {/* Error */}
                      {isFailed && job.error && (
                        <div style={{
                          fontSize: 11, color: 'var(--error)', marginBottom: 8,
                          padding: '6px 8px', borderRadius: 4,
                          background: 'rgba(220,38,38,0.06)', border: '1px solid rgba(220,38,38,0.15)',
                          lineHeight: 1.4,
                          display: '-webkit-box', WebkitLineClamp: 3,
                          WebkitBoxOrient: 'vertical', overflow: 'hidden',
                        }}>
                          {job.error}
                        </div>
                      )}

                      {/* Stats row */}
                      <div style={{ display: 'flex', gap: 12, fontSize: 11, color: 'var(--text-3)', marginBottom: 8, flexWrap: 'wrap' }}>
                        {job.tokensUsed > 0 && (
                          <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                            <Zap size={10} /> {fmtNum(job.tokensUsed)} tokens
                          </span>
                        )}
                        {duration && (
                          <span style={{ display: 'flex', alignItems: 'center', gap: 3 }}>
                            <Clock size={10} /> {duration}
                          </span>
                        )}
                        <span className="mono" style={{ fontSize: 10 }}>
                          {job.outputFormat?.toUpperCase()}
                        </span>
                        {/* CPU/GPU usage — shown for active jobs */}
                        {isActive && sysStats && (
                          <>
                            <span style={{
                              display: 'flex', alignItems: 'center', gap: 3,
                              color: (sysStats.cpu_percent ?? 0) > 80 ? '#DC2626' : (sysStats.cpu_percent ?? 0) > 50 ? '#D97706' : '#10b981',
                              fontWeight: 600,
                            }}>
                              <Cpu size={10} /> {(sysStats.cpu_percent ?? 0).toFixed(0)}% CPU
                            </span>
                            <span style={{
                              display: 'flex', alignItems: 'center', gap: 3,
                              color: (sysStats.memory_percent ?? 0) > 85 ? '#DC2626' : 'var(--text-4)',
                            }}>
                              {(sysStats.memory_percent ?? 0).toFixed(0)}% RAM
                            </span>
                            {sysStats.gpu?.devices?.[0]?.gpu_percent != null && (
                              <span style={{
                                display: 'flex', alignItems: 'center', gap: 3,
                                color: sysStats.gpu.devices[0].gpu_percent > 80 ? '#DC2626' : '#7C3AED',
                                fontWeight: 600,
                              }}>
                                GPU {sysStats.gpu.devices[0].gpu_percent}%
                              </span>
                            )}
                          </>
                        )}
                      </div>

                      {/* Output path */}
                      {isCompleted && job.outputPath && (
                        <div
                          title={job.outputPath}
                          style={{
                            fontSize: 10, color: 'var(--text-3)', marginBottom: 8,
                            overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                            fontFamily: 'var(--font-mono, monospace)',
                            padding: '4px 6px', borderRadius: 4,
                            background: 'var(--bg)', border: '1px solid var(--border)',
                          }}
                        >
                          {job.outputPath}
                        </div>
                      )}

                      {/* Actions */}
                      <div style={{ display: 'flex', gap: 6 }}>
                        {/* Running: Stop + Edit */}
                        {isActive && (
                          <>
                            <button
                              className="btn btn-sm"
                              style={{ fontSize: 11, background: '#DC2626', color: '#fff', border: 'none' }}
                              onClick={() => handleCancelSynthJob(job.id)}
                            >
                              <X size={11} /> Stop
                            </button>
                            <button
                              className="btn btn-ghost btn-sm"
                              style={{ fontSize: 11 }}
                              onClick={() => setEditJob(job)}
                            >
                              <Sparkles size={11} /> Edit
                            </button>
                          </>
                        )}
                        {/* Completed/Failed/Cancelled: Edit (restart) + Remove */}
                        {(isCompleted || isFailed || isCancelled) && (
                          <>
                            <button
                              className="btn btn-ghost btn-sm"
                              style={{ fontSize: 11 }}
                              onClick={() => setEditJob(job)}
                            >
                              <Sparkles size={11} /> Edit &amp; Restart
                            </button>
                            <button
                              className="btn btn-ghost btn-sm"
                              style={{ fontSize: 11, color: 'var(--error)' }}
                              onClick={() => handleRemoveSynthJob(job.id)}
                            >
                              <Trash2 size={11} /> Remove
                            </button>
                          </>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>
      )}

      {/* Synthesis Edit Modal */}
      {editJob && (
        <SynthesisEditModal
          job={editJob}
          onClose={() => setEditJob(null)}
          onSave={handleSaveSynthJob}
        />
      )}
    </div>
  );
}
