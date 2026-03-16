'use client';

import { Suspense, useState } from 'react';
import { useSearchParams } from 'next/navigation';
import Link from 'next/link';
import {
  Zap, Database, ArrowLeft, Table2, BarChart3, Code2,
  Filter, Columns3, Rows3, Play, Download, Loader2,
} from 'lucide-react';
import useSWR from 'swr';

const fetcher = (url: string) => fetch(url).then(r => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.json(); });

function DataEngineerPage() {
  const searchParams = useSearchParams();
  const datasetId = searchParams.get('dataset');
  const filePath = searchParams.get('path') ?? '';

  const [activeTab, setActiveTab] = useState<'query' | 'transform' | 'visualize'>('query');
  const [sql, setSql] = useState('SELECT * FROM data_view LIMIT 100');
  const [queryResult, setQueryResult] = useState<any>(null);
  const [querying, setQuerying] = useState(false);

  // Load dataset info
  const { data: dsData } = useSWR(
    datasetId ? `/api/data/datasets` : null,
    fetcher,
  );
  const dataset = (dsData?.datasets ?? []).find((d: any) => d.id === datasetId);

  const handleRunQuery = async () => {
    if (!filePath) return;
    setQuerying(true);
    try {
      const res = await fetch('/api/data/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: filePath, sql }),
      });
      setQueryResult(await res.json());
    } catch {
      setQueryResult({ error: 'Query failed' });
    } finally {
      setQuerying(false);
    }
  };

  return (
    <div style={{ padding: 24, maxWidth: 1400, margin: '0 auto' }}>
      {/* Header */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, marginBottom: 20 }}>
        <Link href="/data" style={{ color: 'var(--text-3)', display: 'flex' }}>
          <ArrowLeft size={18} />
        </Link>
        <Zap size={20} color="#7C3AED" />
        <div>
          <h1 style={{ fontSize: 20, fontWeight: 700, color: 'var(--text-1)', margin: 0 }}>
            Data Lab
          </h1>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '2px 0 0' }}>
            {dataset?.name ?? 'Dataset'} — query, transform, and visualize data
          </p>
        </div>
      </div>

      {/* Tabs */}
      <div style={{ display: 'flex', gap: 0, borderBottom: '1px solid var(--border)', marginBottom: 20 }}>
        {([
          { key: 'query' as const, label: 'SQL Query', icon: Code2 },
          { key: 'transform' as const, label: 'Transforms', icon: Filter },
          { key: 'visualize' as const, label: 'Visualize', icon: BarChart3 },
        ]).map(t => (
          <button
            key={t.key}
            onClick={() => setActiveTab(t.key)}
            style={{
              padding: '10px 18px', background: 'none', border: 'none',
              borderBottom: `2px solid ${activeTab === t.key ? '#7C3AED' : 'transparent'}`,
              cursor: 'pointer', fontSize: 13,
              fontWeight: activeTab === t.key ? 600 : 400,
              color: activeTab === t.key ? 'var(--text-1)' : 'var(--text-3)',
              marginBottom: -1, display: 'flex', alignItems: 'center', gap: 6,
            }}
          >
            <t.icon size={14} /> {t.label}
          </button>
        ))}
      </div>

      {/* SQL Query tab */}
      {activeTab === 'query' && (
        <div>
          <div style={{ display: 'flex', gap: 12, marginBottom: 16 }}>
            <textarea
              className="textarea"
              style={{ flex: 1, minHeight: 100, fontFamily: 'var(--font-mono, monospace)', fontSize: 13 }}
              value={sql}
              onChange={e => setSql(e.target.value)}
              placeholder="SELECT * FROM data_view WHERE ..."
            />
          </div>
          <div style={{ display: 'flex', gap: 8, marginBottom: 20 }}>
            <button
              className="btn btn-primary btn-sm"
              onClick={handleRunQuery}
              disabled={querying || !filePath}
            >
              {querying ? <><Loader2 size={12} className="spin" /> Running...</> : <><Play size={12} /> Run Query</>}
            </button>
            <div style={{ fontSize: 11, color: 'var(--text-4)', display: 'flex', alignItems: 'center', gap: 4 }}>
              <Database size={11} /> Powered by DuckDB — use data_view as your table
            </div>
          </div>

          {/* Results */}
          {queryResult && (
            <div style={{ border: '1px solid var(--border)', borderRadius: 8, overflow: 'auto', maxHeight: 500 }}>
              {queryResult.error ? (
                <div style={{ padding: 20, color: 'var(--error)', fontSize: 13 }}>{queryResult.error}</div>
              ) : (
                <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
                  <thead>
                    <tr style={{ background: 'var(--bg-elevated)', position: 'sticky', top: 0 }}>
                      {(queryResult.columns ?? []).map((c: any) => (
                        <th key={c.name} style={{
                          padding: '8px 12px', textAlign: 'left', fontSize: 10, fontWeight: 700,
                          color: 'var(--text-3)', borderBottom: '2px solid var(--border)',
                          textTransform: 'uppercase', whiteSpace: 'nowrap',
                        }}>
                          {c.name} <span style={{ fontWeight: 400, color: 'var(--text-4)' }}>{c.type}</span>
                        </th>
                      ))}
                    </tr>
                  </thead>
                  <tbody>
                    {(queryResult.rows ?? []).map((row: any, i: number) => (
                      <tr key={i} style={{ borderBottom: '1px solid var(--border)' }}>
                        {(queryResult.columns ?? []).map((c: any) => (
                          <td key={c.name} style={{
                            padding: '6px 12px', color: 'var(--text-2)',
                            maxWidth: 300, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                          }}>
                            {row[c.name] === null ? '—' : typeof row[c.name] === 'object' ? JSON.stringify(row[c.name]) : String(row[c.name])}
                          </td>
                        ))}
                      </tr>
                    ))}
                  </tbody>
                </table>
              )}
              {queryResult.rows && (
                <div style={{ padding: '8px 12px', borderTop: '1px solid var(--border)', fontSize: 11, color: 'var(--text-4)' }}>
                  {queryResult.rows.length} rows returned · {queryResult.totalRows ?? '?'} total
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {/* Transform tab placeholder */}
      {activeTab === 'transform' && (
        <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)' }}>
          <Filter size={32} style={{ opacity: 0.3, margin: '0 auto 12px' }} />
          <div style={{ fontSize: 15, fontWeight: 600 }}>Data Transforms</div>
          <div style={{ fontSize: 12, marginTop: 4 }}>
            Column operations, filtering, aggregation, joins — coming soon
          </div>
        </div>
      )}

      {/* Visualize tab placeholder */}
      {activeTab === 'visualize' && (
        <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)' }}>
          <BarChart3 size={32} style={{ opacity: 0.3, margin: '0 auto 12px' }} />
          <div style={{ fontSize: 15, fontWeight: 600 }}>Data Visualization</div>
          <div style={{ fontSize: 12, marginTop: 4 }}>
            Charts, histograms, scatter plots, distributions — coming soon
          </div>
        </div>
      )}
    </div>
  );
}

export default function DataEngineerPageWrapper() {
  return (
    <Suspense fallback={<div style={{ padding: 28 }}><Loader2 size={24} style={{ animation: 'spin 1s linear infinite' }} /></div>}>
      <DataEngineerPage />
    </Suspense>
  );
}
