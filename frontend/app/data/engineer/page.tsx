'use client';

import { Suspense, useState, useMemo, useCallback, useRef, useEffect } from 'react';
import { useSearchParams } from 'next/navigation';
import Link from 'next/link';
import {
  Zap, Database, ArrowLeft, Table2, BarChart3, Code2,
  Filter, Columns3, Rows3, Play, Download, Loader2,
  Save, Trash2, Plus, ArrowUpDown, ChevronDown, Palette,
  Type, PieChart, ScatterChart, TrendingUp, Search, FolderOpen, X, Check,
} from 'lucide-react';
import useSWR from 'swr';

const fetcher = (url: string) => fetch(url).then(r => { if (!r.ok) throw new Error(`HTTP ${r.status}`); return r.json(); });

// ─── Types ──────────────────────────────────────────────────────────────────────

interface ColumnInfo {
  name: string;
  type: string;
}

interface LoadedData {
  rows: any[];
  columns: ColumnInfo[];
  path: string;
  datasetId: string;
}

type FilterOp = '=' | '!=' | '>' | '<' | 'contains';
type AggFn = 'COUNT' | 'SUM' | 'AVG' | 'MIN' | 'MAX';
type SortDir = 'ASC' | 'DESC';
type ChartType = 'bar' | 'line' | 'pie' | 'scatter' | 'histogram' | 'area' | 'donut' | 'heatmap';
type ColorScheme = 'default' | 'cool' | 'warm' | 'pastel';

interface TransformOp {
  type: 'filter' | 'sort' | 'rename' | 'drop' | 'add' | 'aggregate';
  params: Record<string, any>;
}

// ─── Color Palettes ─────────────────────────────────────────────────────────────

const COLOR_PALETTES: Record<ColorScheme, string[]> = {
  default: ['#7C3AED', '#06B6D4', '#10B981', '#F59E0B', '#EF4444', '#EC4899', '#8B5CF6', '#14B8A6'],
  cool: ['#3B82F6', '#06B6D4', '#8B5CF6', '#6366F1', '#0EA5E9', '#7C3AED', '#2DD4BF', '#818CF8'],
  warm: ['#EF4444', '#F59E0B', '#F97316', '#EC4899', '#E11D48', '#D97706', '#FB923C', '#F472B6'],
  pastel: ['#93C5FD', '#A5F3FC', '#A7F3D0', '#FDE68A', '#FCA5A5', '#F9A8D4', '#C4B5FD', '#99F6E4'],
};

// ─── SVG Chart Renderers ────────────────────────────────────────────────────────

const SVG_W = 600;
const SVG_H = 400;
const PAD = { top: 40, right: 30, bottom: 60, left: 60 };
const PLOT_W = SVG_W - PAD.left - PAD.right;
const PLOT_H = SVG_H - PAD.top - PAD.bottom;

function numericValues(rows: any[], col: string): number[] {
  return rows.map(r => parseFloat(r[col])).filter(v => !isNaN(v));
}

function uniqueLabels(rows: any[], col: string): string[] {
  return [...new Set(rows.map(r => String(r[col] ?? '')))];
}

function aggregateByLabel(rows: any[], xCol: string, yCol: string): { label: string; value: number }[] {
  const map = new Map<string, { sum: number; count: number }>();
  rows.forEach(r => {
    const label = String(r[xCol] ?? '');
    const v = parseFloat(r[yCol]);
    if (isNaN(v)) return;
    const entry = map.get(label) ?? { sum: 0, count: 0 };
    entry.sum += v;
    entry.count += 1;
    map.set(label, entry);
  });
  return [...map.entries()].map(([label, { sum }]) => ({ label, value: sum }));
}

function renderBarChart(rows: any[], xCol: string, yCol: string, colors: string[], title: string) {
  const data = aggregateByLabel(rows, xCol, yCol);
  if (data.length === 0) return <text x={SVG_W / 2} y={SVG_H / 2} textAnchor="middle" fill="var(--text-3)" fontSize={13}>No data</text>;
  const maxVal = Math.max(...data.map(d => d.value), 1);
  const barW = Math.min(40, (PLOT_W / data.length) * 0.7);
  const gap = PLOT_W / data.length;
  return (
    <g>
      <text x={SVG_W / 2} y={20} textAnchor="middle" fill="var(--text-1)" fontSize={14} fontWeight={600}>{title}</text>
      {/* Y axis */}
      {[0, 0.25, 0.5, 0.75, 1].map(pct => {
        const y = PAD.top + PLOT_H * (1 - pct);
        return (
          <g key={pct}>
            <line x1={PAD.left} y1={y} x2={PAD.left + PLOT_W} y2={y} stroke="var(--border)" strokeDasharray="3,3" />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fill="var(--text-4)" fontSize={10}>
              {(maxVal * pct).toFixed(maxVal > 100 ? 0 : 1)}
            </text>
          </g>
        );
      })}
      {data.map((d, i) => {
        const h = (d.value / maxVal) * PLOT_H;
        const x = PAD.left + i * gap + (gap - barW) / 2;
        const y = PAD.top + PLOT_H - h;
        return (
          <g key={i}>
            <rect x={x} y={y} width={barW} height={h} fill={colors[i % colors.length]} rx={3} />
            <text x={x + barW / 2} y={PAD.top + PLOT_H + 14} textAnchor="middle" fill="var(--text-3)" fontSize={9}
              transform={`rotate(-30, ${x + barW / 2}, ${PAD.top + PLOT_H + 14})`}>
              {d.label.length > 12 ? d.label.slice(0, 11) + '...' : d.label}
            </text>
          </g>
        );
      })}
    </g>
  );
}

function renderLineChart(rows: any[], xCol: string, yCol: string, colors: string[], title: string) {
  const data = aggregateByLabel(rows, xCol, yCol);
  if (data.length === 0) return <text x={SVG_W / 2} y={SVG_H / 2} textAnchor="middle" fill="var(--text-3)" fontSize={13}>No data</text>;
  const maxVal = Math.max(...data.map(d => d.value), 1);
  const gap = PLOT_W / Math.max(data.length - 1, 1);
  const points = data.map((d, i) => ({
    x: PAD.left + i * gap,
    y: PAD.top + PLOT_H - (d.value / maxVal) * PLOT_H,
  }));
  const polyline = points.map(p => `${p.x},${p.y}`).join(' ');
  return (
    <g>
      <text x={SVG_W / 2} y={20} textAnchor="middle" fill="var(--text-1)" fontSize={14} fontWeight={600}>{title}</text>
      {[0, 0.25, 0.5, 0.75, 1].map(pct => {
        const y = PAD.top + PLOT_H * (1 - pct);
        return (
          <g key={pct}>
            <line x1={PAD.left} y1={y} x2={PAD.left + PLOT_W} y2={y} stroke="var(--border)" strokeDasharray="3,3" />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fill="var(--text-4)" fontSize={10}>
              {(maxVal * pct).toFixed(maxVal > 100 ? 0 : 1)}
            </text>
          </g>
        );
      })}
      <polyline points={polyline} fill="none" stroke={colors[0]} strokeWidth={2} />
      {points.map((p, i) => (
        <circle key={i} cx={p.x} cy={p.y} r={4} fill={colors[0]} stroke="var(--bg)" strokeWidth={2} />
      ))}
      {data.map((d, i) => (
        <text key={i} x={points[i].x} y={PAD.top + PLOT_H + 14} textAnchor="middle" fill="var(--text-3)" fontSize={9}
          transform={`rotate(-30, ${points[i].x}, ${PAD.top + PLOT_H + 14})`}>
          {d.label.length > 12 ? d.label.slice(0, 11) + '...' : d.label}
        </text>
      ))}
    </g>
  );
}

function renderAreaChart(rows: any[], xCol: string, yCol: string, colors: string[], title: string) {
  const data = aggregateByLabel(rows, xCol, yCol);
  if (data.length === 0) return <text x={SVG_W / 2} y={SVG_H / 2} textAnchor="middle" fill="var(--text-3)" fontSize={13}>No data</text>;
  const maxVal = Math.max(...data.map(d => d.value), 1);
  const gap = PLOT_W / Math.max(data.length - 1, 1);
  const points = data.map((d, i) => ({
    x: PAD.left + i * gap,
    y: PAD.top + PLOT_H - (d.value / maxVal) * PLOT_H,
  }));
  const baseline = PAD.top + PLOT_H;
  const areaPath = `M${points[0].x},${baseline} ` + points.map(p => `L${p.x},${p.y}`).join(' ') + ` L${points[points.length - 1].x},${baseline} Z`;
  const linePath = points.map(p => `${p.x},${p.y}`).join(' ');
  return (
    <g>
      <text x={SVG_W / 2} y={20} textAnchor="middle" fill="var(--text-1)" fontSize={14} fontWeight={600}>{title}</text>
      {[0, 0.25, 0.5, 0.75, 1].map(pct => {
        const y = PAD.top + PLOT_H * (1 - pct);
        return (
          <g key={pct}>
            <line x1={PAD.left} y1={y} x2={PAD.left + PLOT_W} y2={y} stroke="var(--border)" strokeDasharray="3,3" />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fill="var(--text-4)" fontSize={10}>
              {(maxVal * pct).toFixed(maxVal > 100 ? 0 : 1)}
            </text>
          </g>
        );
      })}
      <path d={areaPath} fill={colors[0]} opacity={0.2} />
      <polyline points={linePath} fill="none" stroke={colors[0]} strokeWidth={2} />
      {points.map((p, i) => (
        <circle key={i} cx={p.x} cy={p.y} r={3} fill={colors[0]} />
      ))}
    </g>
  );
}

function renderPieOrDonut(rows: any[], xCol: string, yCol: string, colors: string[], title: string, donut: boolean) {
  const data = aggregateByLabel(rows, xCol, yCol);
  if (data.length === 0) return <text x={SVG_W / 2} y={SVG_H / 2} textAnchor="middle" fill="var(--text-3)" fontSize={13}>No data</text>;
  const total = data.reduce((s, d) => s + Math.abs(d.value), 0) || 1;
  const cx = SVG_W / 2;
  const cy = SVG_H / 2 + 10;
  const R = 140;
  const innerR = donut ? R * 0.55 : 0;
  let cumAngle = -Math.PI / 2;
  const slices = data.map((d, i) => {
    const angle = (Math.abs(d.value) / total) * 2 * Math.PI;
    const startAngle = cumAngle;
    cumAngle += angle;
    const endAngle = cumAngle;
    const largeArc = angle > Math.PI ? 1 : 0;
    const x1 = cx + R * Math.cos(startAngle);
    const y1 = cy + R * Math.sin(startAngle);
    const x2 = cx + R * Math.cos(endAngle);
    const y2 = cy + R * Math.sin(endAngle);
    const ix1 = cx + innerR * Math.cos(startAngle);
    const iy1 = cy + innerR * Math.sin(startAngle);
    const ix2 = cx + innerR * Math.cos(endAngle);
    const iy2 = cy + innerR * Math.sin(endAngle);
    const midAngle = startAngle + angle / 2;
    const labelR = R + 18;
    const lx = cx + labelR * Math.cos(midAngle);
    const ly = cy + labelR * Math.sin(midAngle);
    let path: string;
    if (donut) {
      path = `M${x1},${y1} A${R},${R} 0 ${largeArc} 1 ${x2},${y2} L${ix2},${iy2} A${innerR},${innerR} 0 ${largeArc} 0 ${ix1},${iy1} Z`;
    } else {
      path = `M${cx},${cy} L${x1},${y1} A${R},${R} 0 ${largeArc} 1 ${x2},${y2} Z`;
    }
    return (
      <g key={i}>
        <path d={path} fill={colors[i % colors.length]} stroke="var(--bg)" strokeWidth={2} />
        {angle > 0.2 && (
          <text x={lx} y={ly} textAnchor="middle" fill="var(--text-2)" fontSize={9}>
            {d.label.length > 10 ? d.label.slice(0, 9) + '..' : d.label}
          </text>
        )}
      </g>
    );
  });
  return (
    <g>
      <text x={SVG_W / 2} y={20} textAnchor="middle" fill="var(--text-1)" fontSize={14} fontWeight={600}>{title}</text>
      {slices}
    </g>
  );
}

function renderScatterChart(rows: any[], xCol: string, yCol: string, colors: string[], title: string) {
  const xVals = numericValues(rows, xCol);
  const yVals = numericValues(rows, yCol);
  if (xVals.length === 0 || yVals.length === 0) return <text x={SVG_W / 2} y={SVG_H / 2} textAnchor="middle" fill="var(--text-3)" fontSize={13}>Need numeric columns</text>;
  const xMin = Math.min(...xVals); const xMax = Math.max(...xVals) || 1;
  const yMin = Math.min(...yVals); const yMax = Math.max(...yVals) || 1;
  const xRange = xMax - xMin || 1;
  const yRange = yMax - yMin || 1;
  const points = rows.map(r => {
    const x = parseFloat(r[xCol]);
    const y = parseFloat(r[yCol]);
    if (isNaN(x) || isNaN(y)) return null;
    return {
      x: PAD.left + ((x - xMin) / xRange) * PLOT_W,
      y: PAD.top + PLOT_H - ((y - yMin) / yRange) * PLOT_H,
    };
  }).filter(Boolean) as { x: number; y: number }[];
  return (
    <g>
      <text x={SVG_W / 2} y={20} textAnchor="middle" fill="var(--text-1)" fontSize={14} fontWeight={600}>{title}</text>
      {[0, 0.5, 1].map(pct => {
        const y = PAD.top + PLOT_H * (1 - pct);
        return (
          <g key={pct}>
            <line x1={PAD.left} y1={y} x2={PAD.left + PLOT_W} y2={y} stroke="var(--border)" strokeDasharray="3,3" />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fill="var(--text-4)" fontSize={10}>
              {(yMin + yRange * pct).toFixed(1)}
            </text>
          </g>
        );
      })}
      {[0, 0.5, 1].map(pct => (
        <text key={pct} x={PAD.left + PLOT_W * pct} y={PAD.top + PLOT_H + 16} textAnchor="middle" fill="var(--text-4)" fontSize={10}>
          {(xMin + xRange * pct).toFixed(1)}
        </text>
      ))}
      {points.slice(0, 500).map((p, i) => (
        <circle key={i} cx={p.x} cy={p.y} r={4} fill={colors[i % colors.length]} opacity={0.7} />
      ))}
    </g>
  );
}

function renderHistogram(rows: any[], xCol: string, colors: string[], title: string) {
  const vals = numericValues(rows, xCol);
  if (vals.length === 0) return <text x={SVG_W / 2} y={SVG_H / 2} textAnchor="middle" fill="var(--text-3)" fontSize={13}>Need numeric column</text>;
  const min = Math.min(...vals);
  const max = Math.max(...vals);
  const range = max - min || 1;
  const binCount = Math.min(20, Math.max(5, Math.ceil(Math.sqrt(vals.length))));
  const binSize = range / binCount;
  const bins = new Array(binCount).fill(0);
  vals.forEach(v => {
    const idx = Math.min(Math.floor((v - min) / binSize), binCount - 1);
    bins[idx]++;
  });
  const maxBin = Math.max(...bins, 1);
  const barW = PLOT_W / binCount - 2;
  return (
    <g>
      <text x={SVG_W / 2} y={20} textAnchor="middle" fill="var(--text-1)" fontSize={14} fontWeight={600}>{title}</text>
      {[0, 0.5, 1].map(pct => {
        const y = PAD.top + PLOT_H * (1 - pct);
        return (
          <g key={pct}>
            <line x1={PAD.left} y1={y} x2={PAD.left + PLOT_W} y2={y} stroke="var(--border)" strokeDasharray="3,3" />
            <text x={PAD.left - 6} y={y + 4} textAnchor="end" fill="var(--text-4)" fontSize={10}>
              {Math.round(maxBin * pct)}
            </text>
          </g>
        );
      })}
      {bins.map((count, i) => {
        const h = (count / maxBin) * PLOT_H;
        const x = PAD.left + i * (PLOT_W / binCount) + 1;
        const y = PAD.top + PLOT_H - h;
        return (
          <g key={i}>
            <rect x={x} y={y} width={barW} height={h} fill={colors[0]} rx={2} opacity={0.85} />
            {binCount <= 15 && (
              <text x={x + barW / 2} y={PAD.top + PLOT_H + 14} textAnchor="middle" fill="var(--text-4)" fontSize={8}>
                {(min + i * binSize).toFixed(1)}
              </text>
            )}
          </g>
        );
      })}
    </g>
  );
}

// ─── Data Table Component ───────────────────────────────────────────────────────

function DataTable({ rows, columns }: { rows: any[]; columns: ColumnInfo[] }) {
  return (
    <div style={{ border: '1px solid var(--border)', borderRadius: 8, overflow: 'auto', maxHeight: 400 }}>
      <table style={{ width: '100%', borderCollapse: 'collapse', fontSize: 12 }}>
        <thead>
          <tr style={{ background: 'var(--bg-elevated)', position: 'sticky', top: 0 }}>
            {columns.map(c => (
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
          {rows.map((row, i) => (
            <tr key={i} style={{ borderBottom: '1px solid var(--border)' }}>
              {columns.map(c => (
                <td key={c.name} style={{
                  padding: '6px 12px', color: 'var(--text-2)',
                  maxWidth: 300, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap',
                }}>
                  {row[c.name] === null ? '\u2014' : typeof row[c.name] === 'object' ? JSON.stringify(row[c.name]) : String(row[c.name])}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      <div style={{ padding: '8px 12px', borderTop: '1px solid var(--border)', fontSize: 11, color: 'var(--text-4)' }}>
        {rows.length} rows displayed
      </div>
    </div>
  );
}

// ─── Dataset Search Dropdown ─────────────────────────────────────────────────────

function DatasetSearchDropdown({
  datasets,
  value,
  onChange,
}: {
  datasets: any[];
  value: string;
  onChange: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const containerRef = useRef<HTMLDivElement>(null);
  const inputRef = useRef<HTMLInputElement>(null);

  // Close on outside click
  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (containerRef.current && !containerRef.current.contains(e.target as Node)) {
        setOpen(false);
      }
    };
    document.addEventListener('mousedown', handler);
    return () => document.removeEventListener('mousedown', handler);
  }, []);

  // Focus search input when opened
  useEffect(() => {
    if (open && inputRef.current) inputRef.current.focus();
  }, [open]);

  const selected = datasets.find((d: any) => d.id === value);
  const lowerQuery = query.toLowerCase();
  const filtered = datasets.filter((d: any) => {
    if (!query) return true;
    const name = (d.name ?? '').toLowerCase();
    const format = (d.format ?? '').toLowerCase();
    const path = (d.outputPath ?? '').toLowerCase();
    const tags = (d.tags ?? []).join(' ').toLowerCase();
    return name.includes(lowerQuery) || format.includes(lowerQuery) || path.includes(lowerQuery) || tags.includes(lowerQuery);
  });

  // Group datasets that have paths vs those without
  const withPath = filtered.filter((d: any) => d.outputPath);


  return (
    <div ref={containerRef} style={{ position: 'relative', minWidth: 320 }}>
      {/* Trigger Button */}
      <button
        type="button"
        className="input"
        onClick={() => { setOpen(!open); setQuery(''); }}
        style={{
          width: '100%', fontSize: 12, display: 'flex', alignItems: 'center',
          justifyContent: 'space-between', gap: 8, cursor: 'pointer',
          padding: '6px 10px', textAlign: 'left', minHeight: 34,
          background: 'var(--bg-input, var(--bg-elevated))',
        }}
      >
        <span style={{ flex: 1, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>
          {selected
            ? <>{selected.name} <span style={{ color: 'var(--text-4)' }}>({selected.format ?? 'unknown'}{selected.sampleCount ? ` · ${selected.sampleCount} samples` : ''})</span></>
            : <span style={{ color: 'var(--text-4)' }}>Select a dataset...</span>}
        </span>
        <ChevronDown size={13} color="var(--text-4)" style={{ flexShrink: 0, transform: open ? 'rotate(180deg)' : 'none', transition: 'transform 0.15s' }} />
      </button>

      {/* Dropdown Panel */}
      {open && (
        <div style={{
          position: 'absolute', top: '100%', left: 0, right: 0, zIndex: 50,
          marginTop: 4, borderRadius: 8, border: '1px solid var(--border)',
          background: 'var(--bg-surface, var(--bg))', boxShadow: '0 8px 24px rgba(0,0,0,0.15)',
          maxHeight: 400, display: 'flex', flexDirection: 'column',
        }}>
          {/* Search Input */}
          <div style={{ padding: '8px 10px', borderBottom: '1px solid var(--border)' }}>
            <div style={{ display: 'flex', alignItems: 'center', gap: 6, background: 'var(--bg-elevated)', borderRadius: 6, padding: '5px 8px' }}>
              <Search size={13} color="var(--text-4)" style={{ flexShrink: 0 }} />
              <input
                ref={inputRef}
                type="text"
                value={query}
                onChange={e => setQuery(e.target.value)}
                placeholder="Search datasets by name, format, or path..."
                style={{
                  flex: 1, border: 'none', outline: 'none', background: 'transparent',
                  fontSize: 12, color: 'var(--text-1)', padding: 0,
                }}
              />
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery('')}
                  style={{ background: 'none', border: 'none', cursor: 'pointer', padding: 0, display: 'flex' }}
                >
                  <X size={12} color="var(--text-4)" />
                </button>
              )}
            </div>
          </div>

          {/* Options List */}
          <div style={{ overflowY: 'auto', flex: 1 }}>
            {filtered.length === 0 ? (
              <div style={{ padding: '20px 14px', textAlign: 'center', fontSize: 12, color: 'var(--text-4)' }}>
                No datasets match &ldquo;{query}&rdquo;
              </div>
            ) : (
              <>
                {/* Dataset items */}
                {filtered.map((ds: any) => {
                  const isSelected = ds.id === value;
                  return (
                    <button
                      key={ds.id}
                      type="button"
                      onClick={() => { onChange(ds.id); setOpen(false); setQuery(''); }}
                      style={{
                        width: '100%', textAlign: 'left', border: 'none', cursor: 'pointer',
                        padding: '8px 12px', display: 'flex', flexDirection: 'column', gap: 2,
                        background: isSelected ? 'var(--bg-elevated)' : 'transparent',
                        borderLeft: isSelected ? '2px solid #7C3AED' : '2px solid transparent',
                      }}
                      onMouseEnter={e => { if (!isSelected) (e.currentTarget.style.background = 'var(--bg-elevated)'); }}
                      onMouseLeave={e => { if (!isSelected) (e.currentTarget.style.background = 'transparent'); }}
                    >
                      <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                        <Database size={12} color={isSelected ? '#7C3AED' : 'var(--text-4)'} style={{ flexShrink: 0 }} />
                        <span style={{ fontSize: 12, fontWeight: 500, color: 'var(--text-1)', flex: 1 }}>{ds.name}</span>
                        {isSelected && <Check size={13} color="#7C3AED" style={{ flexShrink: 0 }} />}
                        <span style={{
                          fontSize: 10, padding: '1px 5px', borderRadius: 4,
                          background: ds.status === 'ready' ? 'rgba(16,185,129,0.1)' : 'var(--bg-elevated)',
                          color: ds.status === 'ready' ? 'var(--success)' : 'var(--text-4)',
                          fontWeight: 500,
                        }}>
                          {ds.format ?? 'unknown'}
                        </span>
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8, paddingLeft: 18 }}>
                        {ds.sampleCount != null && (
                          <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                            {ds.sampleCount.toLocaleString()} samples
                          </span>
                        )}
                        {ds.sizeBytes != null && ds.sizeBytes > 0 && (
                          <span style={{ fontSize: 10, color: 'var(--text-4)' }}>
                            {(ds.sizeBytes / 1024 / 1024).toFixed(1)} MB
                          </span>
                        )}
                      </div>
                    </button>
                  );
                })}

                {/* Paths Section — shown at the end of the dropdown */}
                {withPath.length > 0 && (
                  <div style={{
                    borderTop: '1px solid var(--border)', marginTop: 4, padding: '8px 0',
                  }}>
                    <div style={{
                      padding: '2px 12px 6px', fontSize: 10, fontWeight: 600,
                      color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.5px',
                      display: 'flex', alignItems: 'center', gap: 4,
                    }}>
                      <FolderOpen size={11} /> Storage Paths
                    </div>
                    {withPath.map((ds: any) => (
                      <button
                        key={`path-${ds.id}`}
                        type="button"
                        onClick={() => { onChange(ds.id); setOpen(false); setQuery(''); }}
                        style={{
                          width: '100%', textAlign: 'left', border: 'none', cursor: 'pointer',
                          padding: '4px 12px', background: 'transparent', display: 'flex',
                          alignItems: 'center', gap: 6,
                        }}
                        onMouseEnter={e => (e.currentTarget.style.background = 'var(--bg-elevated)')}
                        onMouseLeave={e => (e.currentTarget.style.background = 'transparent')}
                      >
                        <span style={{
                          fontSize: 10, color: ds.id === value ? '#7C3AED' : 'var(--text-3)',
                          fontWeight: ds.id === value ? 600 : 400,
                        }}>
                          {ds.name}
                        </span>
                        <span style={{
                          fontSize: 10, color: 'var(--text-4)',
                          fontFamily: 'var(--font-mono, monospace)',
                          overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', flex: 1,
                        }}>
                          {ds.outputPath}
                        </span>
                      </button>
                    ))}
                  </div>
                )}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ─── Main Page Component ────────────────────────────────────────────────────────

function DataEngineerPage() {
  const searchParams = useSearchParams();
  const datasetId = searchParams.get('dataset');
  const filePath = searchParams.get('path') ?? '';

  // Tab state
  const [activeTab, setActiveTab] = useState<'query' | 'transform' | 'visualize'>('query');

  // Query tab state
  const [sql, setSql] = useState('SELECT * FROM data_view LIMIT 100');
  const [queryResult, setQueryResult] = useState<any>(null);
  const [querying, setQuerying] = useState(false);

  // Shared loaded data state
  const [loadedData, setLoadedData] = useState<LoadedData | null>(null);
  const [originalColumns, setOriginalColumns] = useState<ColumnInfo[]>([]);
  const [selectedDatasetId, setSelectedDatasetId] = useState<string>(datasetId ?? '');
  const [loadingData, setLoadingData] = useState(false);
  const [savingData, setSavingData] = useState(false);
  const [transformOps, setTransformOps] = useState<TransformOp[]>([]);

  // Transform operation form state
  const [filterCol, setFilterCol] = useState('');
  const [filterOp, setFilterOp] = useState<FilterOp>('=');
  const [filterVal, setFilterVal] = useState('');
  const [sortCol, setSortCol] = useState('');
  const [sortDir, setSortDir] = useState<SortDir>('ASC');
  const [renameOld, setRenameOld] = useState('');
  const [renameNew, setRenameNew] = useState('');
  const [dropCol, setDropCol] = useState('');
  const [addColName, setAddColName] = useState('');
  const [addColType, setAddColType] = useState('VARCHAR');
  const [addColDefault, setAddColDefault] = useState('');
  const [aggGroupBy, setAggGroupBy] = useState('');
  const [aggFn, setAggFn] = useState<AggFn>('COUNT');
  const [aggValueCol, setAggValueCol] = useState('');

  // Visualize state
  const [chartType, setChartType] = useState<ChartType>('bar');
  const [xAxisCol, setXAxisCol] = useState('');
  const [yAxisCol, setYAxisCol] = useState('');
  const [groupByCol, setGroupByCol] = useState('');
  const [colorScheme, setColorScheme] = useState<ColorScheme>('default');
  const [chartTitle, setChartTitle] = useState('');

  // Fetch datasets list
  const { data: dsData } = useSWR('/api/data/datasets', fetcher);
  const datasetsList: any[] = dsData?.datasets ?? [];
  const dataset = datasetsList.find((d: any) => d.id === datasetId);

  // Resolve active file path — from URL param or selected dataset
  const activeFilePath = filePath || datasetsList.find((d: any) => d.id === selectedDatasetId)?.outputPath || '';

  // ─── Query Tab Handler ──────────────────────────────────────────────────────

  const handleRunQuery = async () => {
    if (!activeFilePath) return;
    setQuerying(true);
    try {
      const res = await fetch('/api/data/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: activeFilePath, sql }),
      });
      setQueryResult(await res.json());
    } catch {
      setQueryResult({ error: 'Query failed' });
    } finally {
      setQuerying(false);
    }
  };

  // ─── Load Dataset ───────────────────────────────────────────────────────────

  const handleLoadDataset = async () => {
    const ds = datasetsList.find((d: any) => d.id === selectedDatasetId);
    if (!ds) return;
    setLoadingData(true);
    try {
      const res = await fetch('/api/data/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: ds.outputPath, sql: 'SELECT * FROM data_view' }),
      });
      const result = await res.json();
      if (result.error) throw new Error(result.error);
      setLoadedData({
        rows: result.rows ?? [],
        columns: result.columns ?? [],
        path: ds.outputPath,
        datasetId: ds.id,
      });
      setOriginalColumns(result.columns ?? []);
      setTransformOps([]);
    } catch (e: any) {
      console.error('Load failed:', e);
    } finally {
      setLoadingData(false);
    }
  };

  // ─── Build SQL from Transform Ops ──────────────────────────────────────────

  const buildTransformSQL = useCallback((ops: TransformOp[]): string => {
    let select = '*';
    const wheres: string[] = [];
    let orderBy = '';
    const renames: { old: string; new_: string }[] = [];
    const drops: string[] = [];
    const adds: { name: string; type: string; default_: string }[] = [];

    for (const op of ops) {
      switch (op.type) {
        case 'filter': {
          const { column, operator, value } = op.params;
          // Resolve actual column name (may have been renamed)
          let actualCol = column;
          const renameMatch = renames.find(r => r.new_ === column);
          if (renameMatch) actualCol = renameMatch.old;
          if (operator === 'contains') {
            wheres.push(`"${actualCol}" LIKE '%${value.replace(/'/g, "''")}%'`);
          } else {
            const isNum = !isNaN(Number(value));
            const quotedVal = isNum ? value : `'${value.replace(/'/g, "''")}'`;
            wheres.push(`"${actualCol}" ${operator} ${quotedVal}`);
          }
          break;
        }
        case 'sort': {
          let actualCol = op.params.column;
          const renameMatch = renames.find(r => r.new_ === op.params.column);
          if (renameMatch) actualCol = renameMatch.old;
          orderBy = `ORDER BY "${actualCol}" ${op.params.direction}`;
          break;
        }
        case 'rename':
          renames.push({ old: op.params.oldName, new_: op.params.newName });
          break;
        case 'drop':
          drops.push(op.params.column);
          break;
        case 'add':
          adds.push({ name: op.params.name, type: op.params.type, default_: op.params.defaultValue });
          break;
        case 'aggregate': {
          let groupCol = op.params.groupBy;
          let valCol = op.params.valueColumn;
          const gr = renames.find(r => r.new_ === groupCol);
          if (gr) groupCol = gr.old;
          const vr = renames.find(r => r.new_ === valCol);
          if (vr) valCol = vr.old;
          let aggQuery = `SELECT "${groupCol}"${gr ? ` AS "${op.params.groupBy}"` : ''}, ${op.params.fn}("${valCol}") as "${op.params.fn.toLowerCase()}_${op.params.valueColumn}" FROM data_view`;
          if (wheres.length > 0) aggQuery += ` WHERE ${wheres.join(' AND ')}`;
          aggQuery += ` GROUP BY "${groupCol}"`;
          if (orderBy) aggQuery += ` ${orderBy}`;
          return aggQuery;
        }
      }
    }

    // Build column list using original columns as baseline
    if (renames.length > 0 || drops.length > 0 || adds.length > 0) {
      const baseColNames = originalColumns.map(c => c.name);
      const colNames = baseColNames.filter(n => {
        // Drop by original name or renamed name
        if (drops.includes(n)) return false;
        const rename = renames.find(r => r.old === n);
        const displayName = rename ? rename.new_ : n;
        return !drops.includes(displayName);
      });

      const selectParts = colNames.map(name => {
        const rename = renames.find(r => r.old === name);
        return rename ? `"${name}" AS "${rename.new_}"` : `"${name}"`;
      });
      // Also drop columns that were renamed then dropped
      const finalParts = selectParts.filter(p => {
        for (const d of drops) {
          if (p === `"${d}"`) return false;
          // Check if this is a rename whose new name was dropped
          const rename = renames.find(r => r.new_ === d);
          if (rename && p.includes(`"${rename.old}"`)) return false;
        }
        return true;
      });
      for (const add of adds) {
        if (!drops.includes(add.name)) {
          const def = add.default_ || 'NULL';
          finalParts.push(`${isNaN(Number(def)) ? `'${def}'` : def} AS "${add.name}"`);
        }
      }
      select = finalParts.length > 0 ? finalParts.join(', ') : '*';
    }

    let query = `SELECT ${select} FROM data_view`;
    if (wheres.length > 0) query += ` WHERE ${wheres.join(' AND ')}`;
    if (orderBy) query += ` ${orderBy}`;
    return query;
  }, [originalColumns]);

  // ─── Apply Transform Op ────────────────────────────────────────────────────

  const applyTransformOp = async (op: TransformOp) => {
    if (!loadedData) return;
    const newOps = [...transformOps, op];
    setTransformOps(newOps);
    setLoadingData(true);
    try {
      const sql = buildTransformSQL(newOps);
      const res = await fetch('/api/data/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: loadedData.path, sql }),
      });
      const result = await res.json();
      if (result.error) throw new Error(result.error);
      setLoadedData({
        ...loadedData,
        rows: result.rows ?? [],
        columns: result.columns ?? [],
      });
    } catch (e: any) {
      console.error('Transform failed:', e);
      setTransformOps(transformOps); // revert
    } finally {
      setLoadingData(false);
    }
  };

  // ─── Save Transformed Data ─────────────────────────────────────────────────

  const handleSave = async () => {
    if (!loadedData) return;
    setSavingData(true);
    try {
      // Get the full transformed dataset via DuckDB query
      const sql = transformOps.length > 0 ? buildTransformSQL(transformOps) : 'SELECT * FROM data_view';
      const queryRes = await fetch('/api/data/query', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ path: loadedData.path, sql }),
      });
      const queryData = await queryRes.json();
      if (queryData.error) throw new Error(queryData.error);

      const allRows = queryData.rows ?? loadedData.rows;
      const colNames = (queryData.columns ?? loadedData.columns).map((c: any) => c.name);

      // Write the complete transformed data back to the file
      const res = await fetch('/api/data/rewrite', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          path: loadedData.path,
          rows: allRows,
          columns: colNames,
          create_version: true,
          datasetId: loadedData.datasetId,
        }),
      });
      const saveResult = await res.json();
      if (saveResult.error) throw new Error(saveResult.error);

      // Reset — the file now matches the transformed state, reload as new baseline
      const newCols = queryData.columns ?? loadedData.columns;
      setLoadedData({ ...loadedData, rows: allRows, columns: newCols });
      setOriginalColumns(newCols);
      setTransformOps([]);
    } catch (e: any) {
      console.error('Save failed:', e);
    } finally {
      setSavingData(false);
    }
  };

  // ─── Chart Rendering ───────────────────────────────────────────────────────

  const colors = COLOR_PALETTES[colorScheme];
  const renderedChart = useMemo(() => {
    if (!loadedData || !xAxisCol) return null;
    const rows = loadedData.rows.slice(0, 500);
    const title = chartTitle || `${chartType.charAt(0).toUpperCase() + chartType.slice(1)} Chart`;
    switch (chartType) {
      case 'bar': return yAxisCol ? renderBarChart(rows, xAxisCol, yAxisCol, colors, title) : null;
      case 'line': return yAxisCol ? renderLineChart(rows, xAxisCol, yAxisCol, colors, title) : null;
      case 'area': return yAxisCol ? renderAreaChart(rows, xAxisCol, yAxisCol, colors, title) : null;
      case 'pie': return yAxisCol ? renderPieOrDonut(rows, xAxisCol, yAxisCol, colors, title, false) : null;
      case 'donut': return yAxisCol ? renderPieOrDonut(rows, xAxisCol, yAxisCol, colors, title, true) : null;
      case 'scatter': return yAxisCol ? renderScatterChart(rows, xAxisCol, yAxisCol, colors, title) : null;
      case 'histogram': return renderHistogram(rows, xAxisCol, colors, title);
      case 'heatmap': return yAxisCol ? renderScatterChart(rows, xAxisCol, yAxisCol, colors, title) : null; // fallback to scatter for heatmap
      default: return null;
    }
  }, [loadedData, xAxisCol, yAxisCol, chartType, colors, chartTitle]);

  // ─── Dataset Selector Component ─────────────────────────────────────────────

  const DatasetSelector = ({ showLoad = true }: { showLoad?: boolean }) => (
    <div className="card" style={{ padding: 16, marginBottom: 16 }}>
      <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
        <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          <Database size={14} color="var(--text-3)" />
          <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>Dataset</span>
        </div>
        <DatasetSearchDropdown
          datasets={datasetsList}
          value={selectedDatasetId}
          onChange={setSelectedDatasetId}
        />
        {showLoad && (
          <button
            className="btn btn-primary btn-sm"
            onClick={handleLoadDataset}
            disabled={!selectedDatasetId || loadingData}
          >
            {loadingData ? <><Loader2 size={12} className="spin" /> Loading...</> : <><Download size={12} /> Load Data</>}
          </button>
        )}
        {loadedData && (
          <span style={{ fontSize: 11, color: 'var(--success)', fontWeight: 500 }}>
            {loadedData.rows.length} rows, {loadedData.columns.length} columns loaded
          </span>
        )}
      </div>
    </div>
  );

  // ─── Render ─────────────────────────────────────────────────────────────────

  const inputStyle = { fontSize: 12, minWidth: 120 } as const;
  const panelCardStyle = { padding: 14, marginBottom: 12 } as const;
  const sectionLabel = { fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 8, textTransform: 'uppercase' as const, letterSpacing: '0.5px' };
  const opRow = { display: 'flex', alignItems: 'center', gap: 8, flexWrap: 'wrap' as const, marginBottom: 10 };

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

      {/* ═══════════════════ SQL Query Tab ═══════════════════ */}
      {activeTab === 'query' && (
        <div>
          {/* Dataset selector for query */}
          {!filePath && (
            <div className="card" style={{ padding: 16, marginBottom: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12, flexWrap: 'wrap' }}>
                <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Database size={14} color="var(--text-3)" />
                  <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>Dataset</span>
                </div>
                <DatasetSearchDropdown
                  datasets={datasetsList}
                  value={selectedDatasetId}
                  onChange={setSelectedDatasetId}
                />
                {activeFilePath && (
                  <span style={{ fontSize: 10, color: 'var(--text-4)', fontFamily: 'var(--font-mono, monospace)' }}>
                    {activeFilePath}
                  </span>
                )}
              </div>
            </div>
          )}
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
              disabled={querying || !activeFilePath}
            >
              {querying ? <><Loader2 size={12} className="spin" /> Running...</> : <><Play size={12} /> Run Query</>}
            </button>
            <div style={{ fontSize: 11, color: 'var(--text-4)', display: 'flex', alignItems: 'center', gap: 4 }}>
              <Database size={11} /> Powered by DuckDB — use <code style={{ background: 'var(--bg-elevated)', padding: '1px 4px', borderRadius: 3 }}>data_view</code> as your table
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
                            {row[c.name] === null ? '\u2014' : typeof row[c.name] === 'object' ? JSON.stringify(row[c.name]) : String(row[c.name])}
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

      {/* ═══════════════════ Transform Tab ═══════════════════ */}
      {activeTab === 'transform' && (
        <div>
          <DatasetSelector />

          {loadedData && (
            <>
              {/* Applied Operations Badge Strip */}
              {transformOps.length > 0 && (
                <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap', marginBottom: 12 }}>
                  {transformOps.map((op, i) => (
                    <span key={i} style={{
                      padding: '3px 10px', borderRadius: 12, fontSize: 10, fontWeight: 600,
                      background: 'var(--bg-elevated)', color: 'var(--text-2)',
                      border: '1px solid var(--border)',
                    }}>
                      {op.type}: {Object.values(op.params).join(', ')}
                      <button
                        style={{ marginLeft: 6, background: 'none', border: 'none', cursor: 'pointer', color: 'var(--error)', fontSize: 10, padding: 0 }}
                        onClick={async () => {
                          const newOps = transformOps.filter((_, j) => j !== i);
                          setTransformOps(newOps);
                          setLoadingData(true);
                          try {
                            const sql = newOps.length > 0 ? buildTransformSQL(newOps) : 'SELECT * FROM data_view';
                            const res = await fetch('/api/data/query', {
                              method: 'POST',
                              headers: { 'Content-Type': 'application/json' },
                              body: JSON.stringify({ path: loadedData.path, sql }),
                            });
                            const result = await res.json();
                            if (!result.error) {
                              setLoadedData({ ...loadedData, rows: result.rows ?? [], columns: result.columns ?? [] });
                            }
                          } finally {
                            setLoadingData(false);
                          }
                        }}
                      >
                        x
                      </button>
                    </span>
                  ))}
                </div>
              )}

              {/* Transform Operations Panel */}
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12, marginBottom: 16 }}>
                {/* Filter Rows */}
                <div className="card" style={panelCardStyle}>
                  <div style={sectionLabel}><Filter size={11} style={{ marginRight: 4, verticalAlign: 'middle' }} />Filter Rows</div>
                  <div style={opRow}>
                    <select className="input" style={inputStyle} value={filterCol} onChange={e => setFilterCol(e.target.value)}>
                      <option value="">Column...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                    <select className="input" style={{ fontSize: 12, minWidth: 70 }} value={filterOp} onChange={e => setFilterOp(e.target.value as FilterOp)}>
                      <option value="=">=</option>
                      <option value="!=">!=</option>
                      <option value=">">&gt;</option>
                      <option value="<">&lt;</option>
                      <option value="contains">contains</option>
                    </select>
                    <input className="input" style={inputStyle} placeholder="Value..." value={filterVal} onChange={e => setFilterVal(e.target.value)} />
                    <button className="btn btn-sm" disabled={!filterCol || !filterVal}
                      onClick={() => {
                        applyTransformOp({ type: 'filter', params: { column: filterCol, operator: filterOp, value: filterVal } });
                        setFilterVal('');
                      }}
                    >
                      <Plus size={11} /> Add
                    </button>
                  </div>
                </div>

                {/* Sort */}
                <div className="card" style={panelCardStyle}>
                  <div style={sectionLabel}><ArrowUpDown size={11} style={{ marginRight: 4, verticalAlign: 'middle' }} />Sort</div>
                  <div style={opRow}>
                    <select className="input" style={inputStyle} value={sortCol} onChange={e => setSortCol(e.target.value)}>
                      <option value="">Column...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                    <select className="input" style={{ fontSize: 12, minWidth: 80 }} value={sortDir} onChange={e => setSortDir(e.target.value as SortDir)}>
                      <option value="ASC">ASC</option>
                      <option value="DESC">DESC</option>
                    </select>
                    <button className="btn btn-sm" disabled={!sortCol}
                      onClick={() => applyTransformOp({ type: 'sort', params: { column: sortCol, direction: sortDir } })}
                    >
                      <Play size={11} /> Apply
                    </button>
                  </div>
                </div>

                {/* Rename Column */}
                <div className="card" style={panelCardStyle}>
                  <div style={sectionLabel}><Type size={11} style={{ marginRight: 4, verticalAlign: 'middle' }} />Rename Column</div>
                  <div style={opRow}>
                    <select className="input" style={inputStyle} value={renameOld} onChange={e => setRenameOld(e.target.value)}>
                      <option value="">Old name...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                    <input className="input" style={inputStyle} placeholder="New name..." value={renameNew} onChange={e => setRenameNew(e.target.value)} />
                    <button className="btn btn-sm" disabled={!renameOld || !renameNew}
                      onClick={() => {
                        applyTransformOp({ type: 'rename', params: { oldName: renameOld, newName: renameNew } });
                        setRenameNew('');
                      }}
                    >
                      <Play size={11} /> Rename
                    </button>
                  </div>
                </div>

                {/* Drop Column */}
                <div className="card" style={panelCardStyle}>
                  <div style={sectionLabel}><Trash2 size={11} style={{ marginRight: 4, verticalAlign: 'middle' }} />Drop Column</div>
                  <div style={opRow}>
                    <select className="input" style={inputStyle} value={dropCol} onChange={e => setDropCol(e.target.value)}>
                      <option value="">Column...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                    <button className="btn btn-sm" style={{ color: 'var(--error)' }} disabled={!dropCol}
                      onClick={() => {
                        applyTransformOp({ type: 'drop', params: { column: dropCol } });
                        setDropCol('');
                      }}
                    >
                      <Trash2 size={11} /> Drop
                    </button>
                  </div>
                </div>

                {/* Add Column */}
                <div className="card" style={panelCardStyle}>
                  <div style={sectionLabel}><Plus size={11} style={{ marginRight: 4, verticalAlign: 'middle' }} />Add Column</div>
                  <div style={opRow}>
                    <input className="input" style={inputStyle} placeholder="Name..." value={addColName} onChange={e => setAddColName(e.target.value)} />
                    <select className="input" style={{ fontSize: 12, minWidth: 90 }} value={addColType} onChange={e => setAddColType(e.target.value)}>
                      <option value="VARCHAR">VARCHAR</option>
                      <option value="INTEGER">INTEGER</option>
                      <option value="FLOAT">FLOAT</option>
                      <option value="BOOLEAN">BOOLEAN</option>
                      <option value="DATE">DATE</option>
                    </select>
                    <input className="input" style={inputStyle} placeholder="Default..." value={addColDefault} onChange={e => setAddColDefault(e.target.value)} />
                    <button className="btn btn-sm" disabled={!addColName}
                      onClick={() => {
                        applyTransformOp({ type: 'add', params: { name: addColName, type: addColType, defaultValue: addColDefault } });
                        setAddColName('');
                        setAddColDefault('');
                      }}
                    >
                      <Plus size={11} /> Add
                    </button>
                  </div>
                </div>

                {/* Aggregate */}
                <div className="card" style={panelCardStyle}>
                  <div style={sectionLabel}><Rows3 size={11} style={{ marginRight: 4, verticalAlign: 'middle' }} />Aggregate</div>
                  <div style={opRow}>
                    <select className="input" style={inputStyle} value={aggGroupBy} onChange={e => setAggGroupBy(e.target.value)}>
                      <option value="">Group by...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                    <select className="input" style={{ fontSize: 12, minWidth: 80 }} value={aggFn} onChange={e => setAggFn(e.target.value as AggFn)}>
                      <option value="COUNT">COUNT</option>
                      <option value="SUM">SUM</option>
                      <option value="AVG">AVG</option>
                      <option value="MIN">MIN</option>
                      <option value="MAX">MAX</option>
                    </select>
                    <select className="input" style={inputStyle} value={aggValueCol} onChange={e => setAggValueCol(e.target.value)}>
                      <option value="">Value col...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                    <button className="btn btn-sm" disabled={!aggGroupBy || !aggValueCol}
                      onClick={() => applyTransformOp({ type: 'aggregate', params: { groupBy: aggGroupBy, fn: aggFn, valueColumn: aggValueCol } })}
                    >
                      <Play size={11} /> Run
                    </button>
                  </div>
                </div>
              </div>

              {/* Save Button */}
              <div style={{ display: 'flex', gap: 8, marginBottom: 16 }}>
                <button className="btn btn-primary btn-sm" onClick={handleSave} disabled={savingData || transformOps.length === 0}>
                  {savingData ? <><Loader2 size={12} className="spin" /> Saving...</> : <><Save size={12} /> Save Transformed Data</>}
                </button>
                {transformOps.length > 0 && (
                  <span style={{ fontSize: 11, color: 'var(--text-3)', display: 'flex', alignItems: 'center' }}>
                    {transformOps.length} operation{transformOps.length !== 1 ? 's' : ''} applied
                  </span>
                )}
              </div>

              {/* Preview Table */}
              <div style={{ marginBottom: 8 }}>
                <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)', marginBottom: 8 }}>
                  <Table2 size={13} style={{ verticalAlign: 'middle', marginRight: 4 }} />
                  Preview {loadedData.rows.length > 100 ? '(first 100 rows)' : `(${loadedData.rows.length} rows)`}
                </div>
                {loadingData ? (
                  <div style={{ textAlign: 'center', padding: 40, color: 'var(--text-3)' }}>
                    <Loader2 size={20} className="spin" style={{ margin: '0 auto 8px' }} />
                    <div style={{ fontSize: 12 }}>Applying transform...</div>
                  </div>
                ) : (
                  <DataTable rows={loadedData.rows.slice(0, 100)} columns={loadedData.columns} />
                )}
              </div>
            </>
          )}

          {!loadedData && !loadingData && (
            <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)' }}>
              <Filter size={32} style={{ opacity: 0.3, margin: '0 auto 12px' }} />
              <div style={{ fontSize: 15, fontWeight: 600 }}>Select and load a dataset to begin</div>
              <div style={{ fontSize: 12, marginTop: 4 }}>
                Filter, sort, rename, drop, add columns, and aggregate your data
              </div>
            </div>
          )}
        </div>
      )}

      {/* ═══════════════════ Visualize Tab ═══════════════════ */}
      {activeTab === 'visualize' && (
        <div>
          <DatasetSelector />

          {loadedData ? (
            <div style={{ display: 'grid', gridTemplateColumns: '300px 1fr', gap: 16 }}>
              {/* Chart Configuration Panel */}
              <div>
                <div className="card" style={{ padding: 16 }}>
                  <div style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)', marginBottom: 14 }}>
                    <Palette size={14} style={{ verticalAlign: 'middle', marginRight: 6 }} />
                    Chart Configuration
                  </div>

                  {/* Chart Type */}
                  <div style={{ marginBottom: 12 }}>
                    <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 }}>Chart Type</label>
                    <select className="input" style={{ width: '100%', fontSize: 12 }} value={chartType} onChange={e => setChartType(e.target.value as ChartType)}>
                      <option value="bar">Bar Chart</option>
                      <option value="line">Line Chart</option>
                      <option value="pie">Pie Chart</option>
                      <option value="scatter">Scatter Plot</option>
                      <option value="histogram">Histogram</option>
                      <option value="area">Area Chart</option>
                      <option value="donut">Donut Chart</option>
                      <option value="heatmap">Heatmap</option>
                    </select>
                  </div>

                  {/* X-Axis */}
                  <div style={{ marginBottom: 12 }}>
                    <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 }}>X-Axis Column</label>
                    <select className="input" style={{ width: '100%', fontSize: 12 }} value={xAxisCol} onChange={e => setXAxisCol(e.target.value)}>
                      <option value="">Select column...</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name} ({c.type})</option>)}
                    </select>
                  </div>

                  {/* Y-Axis (not needed for histogram) */}
                  {chartType !== 'histogram' && (
                    <div style={{ marginBottom: 12 }}>
                      <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 }}>Y-Axis Column</label>
                      <select className="input" style={{ width: '100%', fontSize: 12 }} value={yAxisCol} onChange={e => setYAxisCol(e.target.value)}>
                        <option value="">Select column...</option>
                        {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name} ({c.type})</option>)}
                      </select>
                    </div>
                  )}

                  {/* Group By */}
                  <div style={{ marginBottom: 12 }}>
                    <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 }}>Group By (optional)</label>
                    <select className="input" style={{ width: '100%', fontSize: 12 }} value={groupByCol} onChange={e => setGroupByCol(e.target.value)}>
                      <option value="">None</option>
                      {loadedData.columns.map(c => <option key={c.name} value={c.name}>{c.name}</option>)}
                    </select>
                  </div>

                  {/* Color Scheme */}
                  <div style={{ marginBottom: 12 }}>
                    <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 }}>Color Scheme</label>
                    <select className="input" style={{ width: '100%', fontSize: 12 }} value={colorScheme} onChange={e => setColorScheme(e.target.value as ColorScheme)}>
                      <option value="default">Default</option>
                      <option value="cool">Cool</option>
                      <option value="warm">Warm</option>
                      <option value="pastel">Pastel</option>
                    </select>
                    <div style={{ display: 'flex', gap: 3, marginTop: 6 }}>
                      {COLOR_PALETTES[colorScheme].slice(0, 6).map((c, i) => (
                        <div key={i} style={{ width: 18, height: 12, borderRadius: 3, background: c }} />
                      ))}
                    </div>
                  </div>

                  {/* Title */}
                  <div style={{ marginBottom: 8 }}>
                    <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', display: 'block', marginBottom: 4 }}>Chart Title</label>
                    <input className="input" style={{ width: '100%', fontSize: 12 }} value={chartTitle} onChange={e => setChartTitle(e.target.value)} placeholder="Untitled chart" />
                  </div>
                </div>
              </div>

              {/* Chart Display Area */}
              <div className="card" style={{ padding: 16 }}>
                {renderedChart ? (
                  <>
                    <div id="chart-container" style={{ display: 'flex', justifyContent: 'center', alignItems: 'center' }}>
                      <svg
                        viewBox={`0 0 ${SVG_W} ${SVG_H}`}
                        width="100%"
                        height="auto"
                        style={{ maxWidth: SVG_W, maxHeight: SVG_H, overflow: 'visible' }}
                      >
                        <rect width={SVG_W} height={SVG_H} fill="var(--bg-surface)" rx={8} />
                        {!['pie', 'donut'].includes(chartType) && (
                          <>
                            <line x1={PAD.left} y1={PAD.top} x2={PAD.left} y2={PAD.top + PLOT_H} stroke="var(--border)" />
                            <line x1={PAD.left} y1={PAD.top + PLOT_H} x2={PAD.left + PLOT_W} y2={PAD.top + PLOT_H} stroke="var(--border)" />
                          </>
                        )}
                        {renderedChart}
                      </svg>
                    </div>
                    <div style={{ display: 'flex', gap: 8, marginTop: 12, justifyContent: 'flex-end' }}>
                      <button
                        className="btn btn-secondary btn-sm"
                        onClick={() => {
                          const svgEl = document.querySelector('#chart-container svg');
                          if (!svgEl) return;
                          const serializer = new XMLSerializer();
                          const svgStr = serializer.serializeToString(svgEl);
                          const blob = new Blob([svgStr], { type: 'image/svg+xml' });
                          const url = URL.createObjectURL(blob);
                          const a = document.createElement('a');
                          a.href = url;
                          a.download = `${(chartTitle || chartType).replace(/\s+/g, '_')}.svg`;
                          a.click();
                          URL.revokeObjectURL(url);
                        }}
                      >
                        <Download size={12} /> Save SVG
                      </button>
                      <button
                        className="btn btn-primary btn-sm"
                        onClick={async () => {
                          const svgEl = document.querySelector('#chart-container svg');
                          if (!svgEl) return;
                          const serializer = new XMLSerializer();
                          const svgStr = serializer.serializeToString(svgEl);
                          const canvas = document.createElement('canvas');
                          canvas.width = SVG_W * 2;
                          canvas.height = SVG_H * 2;
                          const ctx = canvas.getContext('2d');
                          if (!ctx) return;
                          const img = new window.Image();
                          img.onload = () => {
                            ctx.fillStyle = '#ffffff';
                            ctx.fillRect(0, 0, canvas.width, canvas.height);
                            ctx.drawImage(img, 0, 0, canvas.width, canvas.height);
                            canvas.toBlob(blob => {
                              if (!blob) return;
                              const url = URL.createObjectURL(blob);
                              const a = document.createElement('a');
                              a.href = url;
                              a.download = `${(chartTitle || chartType).replace(/\s+/g, '_')}.png`;
                              a.click();
                              URL.revokeObjectURL(url);
                            }, 'image/png');
                          };
                          img.src = 'data:image/svg+xml;base64,' + btoa(unescape(encodeURIComponent(svgStr)));
                        }}
                      >
                        <Save size={12} /> Save PNG
                      </button>
                    </div>
                  </>
                ) : (
                  <div style={{ textAlign: 'center', padding: 80, color: 'var(--text-3)' }}>
                    <BarChart3 size={36} style={{ opacity: 0.2, margin: '0 auto 12px' }} />
                    <div style={{ fontSize: 14, fontWeight: 600 }}>Configure your chart</div>
                    <div style={{ fontSize: 12, marginTop: 4, color: 'var(--text-4)' }}>
                      Select X-axis{chartType !== 'histogram' ? ' and Y-axis' : ''} column{chartType !== 'histogram' ? 's' : ''} to render
                    </div>
                  </div>
                )}
              </div>
            </div>
          ) : (
            <div style={{ textAlign: 'center', padding: 60, color: 'var(--text-3)' }}>
              <BarChart3 size={32} style={{ opacity: 0.3, margin: '0 auto 12px' }} />
              <div style={{ fontSize: 15, fontWeight: 600 }}>Select and load a dataset to visualize</div>
              <div style={{ fontSize: 12, marginTop: 4 }}>
                Bar, line, pie, scatter, histogram, area, and donut charts with pure SVG
              </div>
            </div>
          )}
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
