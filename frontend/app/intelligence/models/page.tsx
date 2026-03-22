/* eslint-disable @typescript-eslint/no-explicit-any */
'use client';

import { useState, useEffect, useCallback, useRef } from 'react';
import { motion } from 'framer-motion';
import useSWR from 'swr';
import dynamic from 'next/dynamic';
import {
  Boxes, Server, Cloud, ExternalLink, Lock, Search,
  Download, Loader2, Trash2, X, HardDrive,
  Sparkles, RefreshCw, Scale, Play, Clock,
  Paperclip, Eye, Code2, FileText, Cpu,
} from 'lucide-react';

const MonacoEditor = dynamic(() => import('@monaco-editor/react').then(m => m.default), { ssr: false });

const ENGINE_URL = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';
const KORTECX_CLOUD_URL = 'https://www.kortecx.com';

/* ── Types ────────────────────────────────────────────── */
interface LocalModel {
  name: string;
  size: number;
  modified_at: string;
  digest?: string;
}

interface CompareModelResult {
  model: string;
  engine: string;
  response: string;
  tokens: number;
  duration_ms: number;
  tokens_per_sec: number;
  error: string | null;
}

interface CompareResult {
  model_a: CompareModelResult;
  model_b: CompareModelResult;
  temperature: number;
  prompt: string;
  mlflow_run_id: string | null;
}

/* ── Helpers ──────────────────────────────────────────── */
function formatSize(bytes: number) {
  if (!bytes) return '—';
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(1)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(0)} MB`;
  return `${(bytes / 1e3).toFixed(0)} KB`;
}

function timeAgo(iso: string) {
  if (!iso) return '—';
  const sec = Math.floor((Date.now() - new Date(iso).getTime()) / 1000);
  if (sec < 60) return 'just now';
  if (sec < 3600) return `${Math.floor(sec / 60)}m ago`;
  if (sec < 86400) return `${Math.floor(sec / 3600)}h ago`;
  return `${Math.floor(sec / 86400)}d ago`;
}

const fetcher = (url: string) => fetch(url).then(r => r.ok ? r.json() : Promise.reject(new Error(`HTTP ${r.status}`)));

/* ── Monaco Prompt Editor ────────────────────────────── */
function MonacoPromptEditor({
  value,
  onChange,
  height = 80,
  language = 'markdown',
  placeholder,
}: {
  value: string;
  onChange: (val: string) => void;
  height?: number;
  language?: string;
  placeholder?: string;
}) {
  return (
    <MonacoEditor
      height={height}
      language={language}
      theme="vs-dark"
      value={value}
      onChange={val => onChange(val ?? '')}
      options={{
        minimap: { enabled: false },
        lineNumbers: 'off',
        glyphMargin: false,
        folding: false,
        lineDecorationsWidth: 8,
        lineNumbersMinChars: 0,
        scrollBeyondLastLine: false,
        wordWrap: 'on',
        wrappingStrategy: 'advanced',
        fontSize: 12,
        fontFamily: 'var(--font-mono, monospace)',
        renderLineHighlight: 'none',
        overviewRulerBorder: false,
        hideCursorInOverviewRuler: true,
        scrollbar: { vertical: 'hidden', horizontal: 'hidden' },
        padding: { top: 8, bottom: 8 },
        placeholder,
      }}
    />
  );
}

/* ── Markdown Preview ────────────────────────────────── */
function formatInline(text: string): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  while (remaining) {
    const boldMatch = remaining.match(/\*\*(.+?)\*\*/);
    const codeMatch = remaining.match(/`(.+?)`/);
    const italicMatch = remaining.match(/(?<!\*)\*([^*]+?)\*(?!\*)/);

    const matches = [
      boldMatch ? { type: 'bold', match: boldMatch, index: boldMatch.index! } : null,
      codeMatch ? { type: 'code', match: codeMatch, index: codeMatch.index! } : null,
      italicMatch ? { type: 'italic', match: italicMatch, index: italicMatch.index! } : null,
    ].filter(Boolean).sort((a, b) => a!.index - b!.index);

    if (matches.length === 0) {
      parts.push(remaining);
      break;
    }

    const first = matches[0]!;
    if (first.index > 0) {
      parts.push(remaining.slice(0, first.index));
    }

    if (first.type === 'bold') {
      parts.push(<strong key={key++} style={{ fontWeight: 700 }}>{first.match![1]}</strong>);
      remaining = remaining.slice(first.index + first.match![0].length);
    } else if (first.type === 'code') {
      parts.push(
        <code key={key++} style={{
          background: 'var(--bg)', padding: '1px 5px', borderRadius: 3,
          fontSize: '0.9em', fontFamily: 'var(--font-mono, monospace)',
          border: '1px solid var(--border)',
        }}>{first.match![1]}</code>
      );
      remaining = remaining.slice(first.index + first.match![0].length);
    } else if (first.type === 'italic') {
      parts.push(<em key={key++}>{first.match![1]}</em>);
      remaining = remaining.slice(first.index + first.match![0].length);
    }
  }

  return parts.length === 1 && typeof parts[0] === 'string' ? parts[0] : <>{parts}</>;
}

function MarkdownPreview({ text }: { text: string }) {
  const lines = text.split('\n');
  const elements: React.ReactNode[] = [];
  let inCodeBlock = false;
  let codeContent = '';
  let codeLang = '';

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];

    if (line.startsWith('```')) {
      if (inCodeBlock) {
        elements.push(
          <pre key={`code-${i}`} style={{
            background: 'var(--bg)', border: '1px solid var(--border)',
            borderRadius: 4, padding: '10px 12px', fontSize: 11,
            fontFamily: 'var(--font-mono, monospace)', overflowX: 'auto',
            margin: '8px 0', lineHeight: 1.5,
          }}>
            {codeLang && <div style={{ fontSize: 9, color: 'var(--text-4)', marginBottom: 4, textTransform: 'uppercase' }}>{codeLang}</div>}
            <code>{codeContent}</code>
          </pre>
        );
        codeContent = '';
        codeLang = '';
        inCodeBlock = false;
      } else {
        inCodeBlock = true;
        codeLang = line.slice(3).trim();
      }
      continue;
    }

    if (inCodeBlock) {
      codeContent += (codeContent ? '\n' : '') + line;
      continue;
    }

    if (!line.trim()) {
      elements.push(<div key={`br-${i}`} style={{ height: 8 }} />);
      continue;
    }

    if (line.startsWith('### ')) {
      elements.push(<h3 key={i} style={{ fontSize: 14, fontWeight: 700, margin: '12px 0 4px', color: 'var(--text-1)' }}>{formatInline(line.slice(4))}</h3>);
    } else if (line.startsWith('## ')) {
      elements.push(<h2 key={i} style={{ fontSize: 15, fontWeight: 700, margin: '14px 0 6px', color: 'var(--text-1)' }}>{formatInline(line.slice(3))}</h2>);
    } else if (line.startsWith('# ')) {
      elements.push(<h1 key={i} style={{ fontSize: 17, fontWeight: 700, margin: '16px 0 8px', color: 'var(--text-1)' }}>{formatInline(line.slice(2))}</h1>);
    } else if (line.match(/^[-*]\s/)) {
      elements.push(
        <div key={i} style={{ display: 'flex', gap: 8, paddingLeft: 4 }}>
          <span style={{ color: 'var(--text-4)' }}>•</span>
          <span>{formatInline(line.slice(2))}</span>
        </div>
      );
    } else if (line.match(/^\d+\.\s/)) {
      const num = line.match(/^(\d+)\./)?.[1];
      elements.push(
        <div key={i} style={{ display: 'flex', gap: 8, paddingLeft: 4 }}>
          <span style={{ color: 'var(--text-4)', fontWeight: 600, minWidth: 16 }}>{num}.</span>
          <span>{formatInline(line.replace(/^\d+\.\s*/, ''))}</span>
        </div>
      );
    } else {
      elements.push(<p key={i} style={{ margin: '4px 0' }}>{formatInline(line)}</p>);
    }
  }

  return <>{elements}</>;
}

/* ── Model Search Dropdown (shared) ──────────────────── */
function CompareModelOption({ m, onSelect, onDelete }: {
  m: { name: string; description?: string; local: boolean; remote?: boolean; pipeline_tag?: string; pipelineTag?: string };
  onSelect: (name: string) => void;
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
        onClick={() => onSelect(m.name)}
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
            try { await onDelete(m.name); } finally { setDeleting(false); }
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

function CompareModelSearchDropdown({
  query, source, localModels, onSelect, onClose, onDelete,
}: {
  query: string;
  source: 'ollama' | 'llamacpp';
  localModels: string[];
  onSelect: (name: string) => void;
  onClose: () => void;
  onDelete?: (name: string) => void;
}) {
  const [remoteResults, setRemoteResults] = useState<any[]>([]);
  const [loading, setLoading] = useState(false);
  const [searched, setSearched] = useState(false);

  useEffect(() => {
    if (query.trim().length < 2) { setRemoteResults([]); setSearched(false); return; }
    setLoading(true);
    setSearched(false);
    const timer = setTimeout(async () => {
      try {
        const params = new URLSearchParams({ q: query.trim(), source, gen_type: 'text', limit: '10' });
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
  }, [query, source]);

  const q = query.toLowerCase();
  const localMatches = q
    ? localModels.filter(m => m.toLowerCase().includes(q))
    : localModels;

  const remoteNames = new Set(remoteResults.map((r: any) => r.name));
  const localItems = localMatches
    .filter(m => !remoteNames.has(m))
    .map(m => ({ name: m, description: 'Installed locally', local: true, remote: false }));
  const remoteItems = remoteResults.map((r: any) => ({ ...r, remote: true, local: false }));

  const hasLocal = localItems.length > 0;
  const hasRemote = remoteItems.length > 0;
  const sourceLabel = source === 'ollama' ? 'Ollama Library' : 'llama.cpp';

  return (
    <>
      <div style={{ position: 'fixed', inset: 0, zIndex: 40 }} onClick={onClose} />
      <div style={{
        position: 'absolute', top: '100%', left: 0, right: 0, zIndex: 50,
        marginTop: 4, maxHeight: 300, overflowY: 'auto',
        background: 'var(--bg-surface)', border: '1px solid var(--border)',
        borderRadius: 8, boxShadow: '0 8px 24px rgba(0,0,0,0.15)',
      }}>
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
              <CompareModelOption key={`local-${m.name}-${i}`} m={m} onSelect={onSelect} onDelete={onDelete} />
            ))}
          </>
        )}

        {(hasRemote || loading || (searched && query.trim().length >= 2)) && (
          <>
            <div style={{
              padding: '6px 12px', fontSize: 10, fontWeight: 700,
              color: source === 'ollama' ? '#10B981' : '#3B82F6',
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
              <CompareModelOption key={`remote-${m.name}-${i}`} m={m} onSelect={onSelect} />
            ))}
            {searched && !loading && remoteItems.length === 0 && query.trim().length >= 2 && (
              <div style={{ padding: '10px 12px', fontSize: 11, color: 'var(--text-4)', textAlign: 'center' }}>
                No remote models found for &ldquo;{query}&rdquo;
              </div>
            )}
          </>
        )}

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

/* ── Compare Tab Component ───────────────────────────── */

const MAX_FILE_SIZE = 5 * 1024 * 1024; // 5MB
const MAX_TOTAL_SIZE = 20 * 1024 * 1024; // 20MB
const ACCEPTED_EXTENSIONS = '.txt,.md,.pdf,.csv,.json,.jsonl,.html,.xml,.yaml,.yml,.py,.ts,.js,.go';

function CompareTab() {
  const [engineA, setEngineA] = useState<'ollama' | 'llamacpp'>('ollama');
  const [engineB, setEngineB] = useState<'ollama' | 'llamacpp'>('ollama');
  const [modelA, setModelA] = useState('');
  const [modelB, setModelB] = useState('');
  const [prompt, setPrompt] = useState('');
  const [systemPrompt, setSystemPrompt] = useState('');
  const [temperature, setTemperature] = useState(0.7);
  const [maxTokens, setMaxTokens] = useState(4096);
  const [running, setRunning] = useState(false);
  const [result, setResult] = useState<CompareResult | null>(null);
  const [error, setError] = useState('');

  // Preview toggles
  const [promptPreview, setPromptPreview] = useState(false);
  const [sysPreview, setSysPreview] = useState(false);

  // Document attachments
  const [attachedFiles, setAttachedFiles] = useState<File[]>([]);
  const fileInputRef = useRef<HTMLInputElement>(null);

  // Model lists per engine
  const [modelsA, setModelsA] = useState<LocalModel[]>([]);
  const [modelsB, setModelsB] = useState<LocalModel[]>([]);

  // Dropdown open state
  const [modelASearchOpen, setModelASearchOpen] = useState(false);
  const [modelBSearchOpen, setModelBSearchOpen] = useState(false);

  // MLflow status
  const [mlflowEnabled, setMlflowEnabled] = useState(false);

  const fetchEngineModels = useCallback(async (engine: string) => {
    try {
      const resp = await fetch(`${ENGINE_URL}/api/orchestrator/models/${engine}`);
      if (resp.ok) {
        const data = await resp.json();
        return (data.models || []) as LocalModel[];
      }
    } catch { /* offline */ }
    return [];
  }, []);

  useEffect(() => { fetchEngineModels(engineA).then(setModelsA); }, [engineA, fetchEngineModels]);
  useEffect(() => { fetchEngineModels(engineB).then(setModelsB); }, [engineB, fetchEngineModels]);

  useEffect(() => {
    fetch(`${ENGINE_URL}/api/mlflow/status`)
      .then(r => r.ok ? r.json() : null)
      .then(d => { if (d) setMlflowEnabled(d.enabled); })
      .catch(() => {});
  }, []);

  // History
  const { data: historyData, mutate: mutateHistory } = useSWR(
    '/api/comparisons?limit=20',
    fetcher,
    { refreshInterval: 30_000 },
  );
  const history = historyData?.comparisons ?? [];

  const handleFileAttach = (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(e.target.files || []);
    const totalExisting = attachedFiles.reduce((s, f) => s + f.size, 0);
    const valid: File[] = [];
    let runningTotal = totalExisting;

    for (const f of files) {
      if (f.size > MAX_FILE_SIZE) {
        setError(`File "${f.name}" exceeds 5MB limit`);
        continue;
      }
      if (runningTotal + f.size > MAX_TOTAL_SIZE) {
        setError('Total attachment size exceeds 20MB limit');
        break;
      }
      runningTotal += f.size;
      valid.push(f);
    }

    setAttachedFiles(prev => [...prev, ...valid]);
    if (fileInputRef.current) fileInputRef.current.value = '';
  };

  const removeFile = (index: number) => {
    setAttachedFiles(prev => prev.filter((_, i) => i !== index));
  };

  const handleCompare = async () => {
    if (!prompt.trim() || !modelA.trim() || !modelB.trim()) return;
    setRunning(true);
    setError('');
    setResult(null);

    try {
      // Upload attached documents if any
      let documentUrls: string[] = [];
      let documentNames: string[] = [];
      if (attachedFiles.length > 0) {
        const formData = new FormData();
        attachedFiles.forEach(f => formData.append('files', f));
        const uploadResp = await fetch(`${ENGINE_URL}/api/orchestrator/upload`, {
          method: 'POST',
          body: formData,
        });
        if (!uploadResp.ok) throw new Error('Failed to upload documents');
        const uploadData = await uploadResp.json();
        documentUrls = (uploadData.files || []).map((f: any) => f.url);
        documentNames = (uploadData.files || []).map((f: any) => f.filename);
      }

      const resp = await fetch(`${ENGINE_URL}/api/models/compare`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          prompt: prompt.trim(),
          system_prompt: systemPrompt.trim(),
          model_a: modelA.trim(),
          engine_a: engineA,
          model_b: modelB.trim(),
          engine_b: engineB,
          temperature,
          max_tokens: maxTokens,
          document_urls: documentUrls,
        }),
      });
      if (!resp.ok) throw new Error(`Engine returned ${resp.status}`);
      const data: CompareResult = await resp.json();
      setResult(data);

      // Save to NeonDB with all new fields
      fetch('/api/comparisons', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          model_a: data.model_a,
          model_b: data.model_b,
          temperature: data.temperature,
          prompt: prompt.trim(),
          systemPrompt: systemPrompt.trim() || null,
          documentNames: documentNames.length > 0 ? documentNames : null,
        }),
      }).then(() => mutateHistory()).catch(() => {});
    } catch (e: any) {
      setError(e.message);
    } finally {
      setRunning(false);
    }
  };

  const canRun = prompt.trim() && modelA.trim() && modelB.trim() && !running;

  return (
    <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
      {/* Configuration */}
      <div className="card" style={{ padding: 20, marginBottom: 16 }}>
        <div style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.08em', marginBottom: 12 }}>
          Configuration
        </div>

        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16, marginBottom: 16 }}>
          {/* Model A */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', display: 'block', marginBottom: 6 }}>Engine A</label>
            <div style={{ display: 'flex', gap: 6, marginBottom: 10 }}>
              {([
                { key: 'ollama' as const, label: 'Ollama', icon: Server },
                { key: 'llamacpp' as const, label: 'llama.cpp', icon: Cpu },
              ]).map(s => (
                <button
                  key={s.key}
                  onClick={() => { setEngineA(s.key); setModelA(''); }}
                  style={{
                    flex: 1, padding: '8px 12px', borderRadius: 8,
                    border: `1.5px solid ${engineA === s.key ? '#0EA5E9' : 'var(--border)'}`,
                    background: engineA === s.key ? 'rgba(14,165,233,0.06)' : 'var(--bg)',
                    cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
                    fontSize: 12, fontWeight: engineA === s.key ? 700 : 500,
                    color: engineA === s.key ? '#0EA5E9' : 'var(--text-3)',
                    transition: 'all 0.15s',
                  }}
                >
                  <s.icon size={14} />
                  {s.label}
                </button>
              ))}
            </div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', display: 'block', marginBottom: 6 }}>Model A</label>
            <div style={{ position: 'relative' }}>
              <div style={{ position: 'relative' }}>
                <Search size={14} color="var(--text-4)" style={{ position: 'absolute', left: 10, top: '50%', transform: 'translateY(-50%)' }} />
                <input
                  className="input"
                  style={{ width: '100%', paddingLeft: 32 }}
                  placeholder={engineA === 'ollama' ? 'Select or search models...' : 'Select model or type path...'}
                  value={modelA}
                  onChange={e => { setModelA(e.target.value); setModelASearchOpen(true); }}
                  onFocus={() => setModelASearchOpen(true)}
                />
                {modelA && (
                  <button
                    onClick={() => { setModelA(''); setModelASearchOpen(true); }}
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
              {modelASearchOpen && (
                <CompareModelSearchDropdown
                  query={modelA}
                  source={engineA}
                  localModels={modelsA.map(m => m.name)}
                  onSelect={(name) => { setModelA(name); setModelASearchOpen(false); }}
                  onClose={() => setModelASearchOpen(false)}
                  onDelete={engineA === 'ollama' ? async (name) => {
                    if (!confirm(`Delete model "${name}"? This cannot be undone.`)) return;
                    try {
                      await fetch(`${ENGINE_URL}/api/orchestrator/models/delete`, {
                        method: 'POST', headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ engine: engineA, model: name }),
                      });
                      const updated = await fetchEngineModels(engineA);
                      setModelsA(updated);
                      if (modelA === name) setModelA('');
                    } catch { /* ignore */ }
                  } : undefined}
                />
              )}
            </div>
          </div>

          {/* Model B */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', display: 'block', marginBottom: 6 }}>Engine B</label>
            <div style={{ display: 'flex', gap: 6, marginBottom: 10 }}>
              {([
                { key: 'ollama' as const, label: 'Ollama', icon: Server },
                { key: 'llamacpp' as const, label: 'llama.cpp', icon: Cpu },
              ]).map(s => (
                <button
                  key={s.key}
                  onClick={() => { setEngineB(s.key); setModelB(''); }}
                  style={{
                    flex: 1, padding: '8px 12px', borderRadius: 8,
                    border: `1.5px solid ${engineB === s.key ? '#7C3AED' : 'var(--border)'}`,
                    background: engineB === s.key ? 'rgba(124,58,237,0.06)' : 'var(--bg)',
                    cursor: 'pointer', display: 'flex', alignItems: 'center', justifyContent: 'center', gap: 6,
                    fontSize: 12, fontWeight: engineB === s.key ? 700 : 500,
                    color: engineB === s.key ? '#7C3AED' : 'var(--text-3)',
                    transition: 'all 0.15s',
                  }}
                >
                  <s.icon size={14} />
                  {s.label}
                </button>
              ))}
            </div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', display: 'block', marginBottom: 6 }}>Model B</label>
            <div style={{ position: 'relative' }}>
              <div style={{ position: 'relative' }}>
                <Search size={14} color="var(--text-4)" style={{ position: 'absolute', left: 10, top: '50%', transform: 'translateY(-50%)' }} />
                <input
                  className="input"
                  style={{ width: '100%', paddingLeft: 32 }}
                  placeholder={engineB === 'ollama' ? 'Select or search models...' : 'Select model or type path...'}
                  value={modelB}
                  onChange={e => { setModelB(e.target.value); setModelBSearchOpen(true); }}
                  onFocus={() => setModelBSearchOpen(true)}
                />
                {modelB && (
                  <button
                    onClick={() => { setModelB(''); setModelBSearchOpen(true); }}
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
              {modelBSearchOpen && (
                <CompareModelSearchDropdown
                  query={modelB}
                  source={engineB}
                  localModels={modelsB.map(m => m.name)}
                  onSelect={(name) => { setModelB(name); setModelBSearchOpen(false); }}
                  onClose={() => setModelBSearchOpen(false)}
                  onDelete={engineB === 'ollama' ? async (name) => {
                    if (!confirm(`Delete model "${name}"? This cannot be undone.`)) return;
                    try {
                      await fetch(`${ENGINE_URL}/api/orchestrator/models/delete`, {
                        method: 'POST', headers: { 'Content-Type': 'application/json' },
                        body: JSON.stringify({ engine: engineB, model: name }),
                      });
                      const updated = await fetchEngineModels(engineB);
                      setModelsB(updated);
                      if (modelB === name) setModelB('');
                    } catch { /* ignore */ }
                  } : undefined}
                />
              )}
            </div>
          </div>
        </div>

        {/* Prompt with Monaco + preview toggle */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
          <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)' }}>Prompt</label>
          <button
            onClick={() => setPromptPreview(!promptPreview)}
            style={{
              background: 'none', border: 'none', cursor: 'pointer', padding: '2px 6px',
              color: promptPreview ? '#0EA5E9' : 'var(--text-4)', display: 'flex', alignItems: 'center', gap: 4,
              fontSize: 10, fontWeight: 600,
            }}
            title={promptPreview ? 'Switch to editor' : 'Preview markdown'}
          >
            {promptPreview ? <Code2 size={12} /> : <Eye size={12} />}
            {promptPreview ? 'Edit' : 'Preview'}
          </button>
        </div>
        {promptPreview ? (
          <div style={{
            border: '1px solid var(--border)', borderRadius: 6, padding: '10px 12px',
            minHeight: 120, maxHeight: 200, overflow: 'auto', marginBottom: 12,
            fontSize: 13, color: 'var(--text-2)', lineHeight: 1.65, background: 'var(--bg-surface)',
          }}>
            {prompt.trim() ? <MarkdownPreview text={prompt} /> : <span style={{ color: 'var(--text-4)', fontSize: 12 }}>Nothing to preview</span>}
          </div>
        ) : (
          <div style={{ border: '1px solid var(--border)', borderRadius: 6, overflow: 'hidden', marginBottom: 12 }}>
            <MonacoPromptEditor value={prompt} onChange={setPrompt} height={120} placeholder="Enter your prompt (supports markdown)..." />
          </div>
        )}

        {/* System Prompt with Monaco + preview toggle */}
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 4 }}>
          <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)' }}>System Prompt (optional)</label>
          <button
            onClick={() => setSysPreview(!sysPreview)}
            style={{
              background: 'none', border: 'none', cursor: 'pointer', padding: '2px 6px',
              color: sysPreview ? '#0EA5E9' : 'var(--text-4)', display: 'flex', alignItems: 'center', gap: 4,
              fontSize: 10, fontWeight: 600,
            }}
            title={sysPreview ? 'Switch to editor' : 'Preview markdown'}
          >
            {sysPreview ? <Code2 size={12} /> : <Eye size={12} />}
            {sysPreview ? 'Edit' : 'Preview'}
          </button>
        </div>
        {sysPreview ? (
          <div style={{
            border: '1px solid var(--border)', borderRadius: 6, padding: '10px 12px',
            minHeight: 80, maxHeight: 160, overflow: 'auto', marginBottom: 12,
            fontSize: 13, color: 'var(--text-2)', lineHeight: 1.65, background: 'var(--bg-surface)',
          }}>
            {systemPrompt.trim() ? <MarkdownPreview text={systemPrompt} /> : <span style={{ color: 'var(--text-4)', fontSize: 12 }}>Nothing to preview</span>}
          </div>
        ) : (
          <div style={{ border: '1px solid var(--border)', borderRadius: 6, overflow: 'hidden', marginBottom: 12 }}>
            <MonacoPromptEditor value={systemPrompt} onChange={setSystemPrompt} height={80} placeholder="Optional system instructions..." />
          </div>
        )}

        {/* Document Attachment */}
        <div style={{ marginBottom: 12 }}>
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept={ACCEPTED_EXTENSIONS}
            onChange={handleFileAttach}
            style={{ display: 'none' }}
          />
          <button
            onClick={() => fileInputRef.current?.click()}
            className="btn btn-secondary btn-sm"
            style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 8 }}
          >
            <Paperclip size={12} /> Attach Documents
          </button>
          {attachedFiles.length > 0 && (
            <div style={{ display: 'flex', flexWrap: 'wrap', gap: 6 }}>
              {attachedFiles.map((f, i) => (
                <div key={`${f.name}-${i}`} style={{
                  display: 'flex', alignItems: 'center', gap: 6, padding: '4px 10px',
                  borderRadius: 6, background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                  fontSize: 11, color: 'var(--text-2)',
                }}>
                  <FileText size={11} color="var(--text-4)" />
                  <span style={{ maxWidth: 160, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' }}>{f.name}</span>
                  <span className="mono" style={{ fontSize: 10, color: 'var(--text-4)' }}>{formatSize(f.size)}</span>
                  <button onClick={() => removeFile(i)} style={{ background: 'none', border: 'none', cursor: 'pointer', padding: 0, color: 'var(--text-4)', display: 'flex' }}>
                    <X size={11} />
                  </button>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Temperature + Max Tokens */}
        <div style={{ display: 'flex', gap: 16, alignItems: 'center', marginBottom: 16 }}>
          <div style={{ flex: 1 }}>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>
              Temperature: <span className="mono">{temperature.toFixed(2)}</span>
            </label>
            <input type="range" min={0} max={2} step={0.05} value={temperature}
              onChange={e => setTemperature(parseFloat(e.target.value))}
              style={{ width: '100%' }} />
          </div>
          <div style={{ width: 120 }}>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-2)', display: 'block', marginBottom: 4 }}>Max Tokens</label>
            <input className="input" type="number" value={maxTokens} min={128} max={32768}
              onChange={e => setMaxTokens(parseInt(e.target.value) || 4096)} style={{ width: '100%' }} />
          </div>
        </div>

        {/* Run Button */}
        <button className="btn btn-primary" onClick={handleCompare} disabled={!canRun}
          style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
          {running ? <Loader2 size={14} style={{ animation: 'spin 1s linear infinite' }} /> : <Play size={14} />}
          {running ? 'Running...' : 'Run Comparison'}
        </button>
      </div>

      {/* Error */}
      {error && (
        <div className="card" style={{ padding: '12px 16px', marginBottom: 16, borderColor: 'rgba(239,68,68,0.3)' }}>
          <span style={{ color: '#ef4444', fontSize: 13 }}>{error}</span>
        </div>
      )}

      {/* Metrics */}
      {result && (
        <div className="card" style={{ padding: 20, marginBottom: 16 }}>
          <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 12 }}>
            <span style={{ fontSize: 11, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.08em' }}>
              Metrics
            </span>
            {mlflowEnabled && result.mlflow_run_id && (
              <span style={{
                fontSize: 10, fontWeight: 600, padding: '2px 8px', borderRadius: 4,
                background: 'rgba(5,150,105,0.1)', color: '#059669', border: '1px solid rgba(5,150,105,0.2)',
              }}>
                MLflow tracked
              </span>
            )}
          </div>

          <div style={{ display: 'grid', gridTemplateColumns: 'repeat(4, 1fr)', gap: 12, marginBottom: 12 }}>
            {[
              { label: 'Tokens A', value: result.model_a.tokens, color: '#0EA5E9' },
              { label: 'Tokens B', value: result.model_b.tokens, color: '#7C3AED' },
              { label: 'Speed A', value: `${result.model_a.tokens_per_sec} t/s`, color: '#0EA5E9' },
              { label: 'Speed B', value: `${result.model_b.tokens_per_sec} t/s`, color: '#7C3AED' },
            ].map(m => (
              <div key={m.label} style={{
                padding: '10px 12px', borderRadius: 8,
                background: 'var(--bg-elevated)', textAlign: 'center',
              }}>
                <div style={{ fontSize: 10, color: 'var(--text-4)', marginBottom: 4 }}>{m.label}</div>
                <div className="mono" style={{ fontSize: 16, fontWeight: 700, color: m.color }}>{m.value}</div>
              </div>
            ))}
          </div>

          <div style={{ fontSize: 12, color: 'var(--text-3)' }}>
            <Clock size={12} style={{ display: 'inline', verticalAlign: -2, marginRight: 4 }} />
            Latency: <span className="mono">{result.model_a.duration_ms}ms</span> vs <span className="mono">{result.model_b.duration_ms}ms</span>
            {' — '}
            <span style={{ fontWeight: 600, color: result.model_a.tokens_per_sec >= result.model_b.tokens_per_sec ? '#0EA5E9' : '#7C3AED' }}>
              {result.model_a.tokens_per_sec >= result.model_b.tokens_per_sec ? 'Model A' : 'Model B'} faster
            </span>
          </div>
        </div>
      )}

      {/* Side-by-side Responses */}
      {result && (
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 16, marginBottom: 16 }}>
          {[
            { label: 'Model A', data: result.model_a, color: '#0EA5E9' },
            { label: 'Model B', data: result.model_b, color: '#7C3AED' },
          ].map(side => (
            <div key={side.label} className="card" style={{ padding: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 6, marginBottom: 10 }}>
                <div style={{ width: 8, height: 8, borderRadius: '50%', background: side.color }} />
                <span style={{ fontSize: 12, fontWeight: 700, color: 'var(--text-1)' }}>{side.label} Response</span>
              </div>
              <div className="mono" style={{ fontSize: 11, color: 'var(--text-3)', marginBottom: 8 }}>
                {side.data.model} ({side.data.engine})
              </div>
              {side.data.error ? (
                <div style={{ color: '#ef4444', fontSize: 13 }}>{side.data.error}</div>
              ) : (
                <div style={{
                  fontSize: 13, color: 'var(--text-2)', lineHeight: 1.65,
                  maxHeight: 400, overflow: 'auto',
                }}>
                  <MarkdownPreview text={side.data.response} />
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {/* History */}
      {history.length > 0 && (
        <div className="card" style={{ overflow: 'hidden' }}>
          <div style={{ padding: '12px 16px', fontSize: 11, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.08em', borderBottom: '1px solid var(--border)' }}>
            Comparison History
          </div>
          <table className="table-base">
            <thead>
              <tr>
                <th>Date</th>
                <th>Model A</th>
                <th>Model B</th>
                <th>Tokens</th>
                <th>Speed</th>
                <th>Latency</th>
              </tr>
            </thead>
            <tbody>
              {history.map((h: any) => (
                <tr key={h.id}>
                  <td style={{ fontSize: 11, color: 'var(--text-4)' }}>{h.createdAt ? timeAgo(h.createdAt) : '—'}</td>
                  <td><span className="mono" style={{ fontSize: 11 }}>{h.originalModel}</span></td>
                  <td><span className="mono" style={{ fontSize: 11 }}>{h.comparisonModel}</span></td>
                  <td className="mono" style={{ fontSize: 11 }}>{(h.originalTokens || 0) + (h.comparisonTokens || 0)}</td>
                  <td className="mono" style={{ fontSize: 11 }}>
                    {h.originalTokensPerSec ? `${h.originalTokensPerSec} / ${h.comparisonTokensPerSec || 0} t/s` : '—'}
                  </td>
                  <td className="mono" style={{ fontSize: 11 }}>{h.originalDurationMs || 0}ms / {h.comparisonDurationMs || 0}ms</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </motion.div>
  );
}

/* ── Tab Type ─────────────────────────────────────────── */
type ModelTab = 'local' | 'kortecx' | 'advanced' | 'compare';

/* ── Page ─────────────────────────────────────────────── */
export default function ModelsPage() {
  const [tab, setTab] = useState<ModelTab>('local');
  const [models, setModels] = useState<LocalModel[]>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');
  const [pullModel, setPullModel] = useState('');
  const [pulling, setPulling] = useState<string | null>(null);
  const [pullProgress, setPullProgress] = useState(0);
  const [pullStatus, setPullStatus] = useState('');
  const [engine, setEngine] = useState<'ollama' | 'llamacpp'>('ollama');

  const fetchModels = useCallback(async () => {
    setLoading(true);
    try {
      const resp = await fetch(`${ENGINE_URL}/api/orchestrator/models/${engine}`);
      if (resp.ok) {
        const data = await resp.json();
        setModels(data.models || []);
      }
    } catch { /* engine offline */ }
    setLoading(false);
  }, [engine]);

  // eslint-disable-next-line react-hooks/set-state-in-effect
  useEffect(() => { fetchModels(); }, [fetchModels]);

  const handlePull = async () => {
    if (!pullModel.trim() || pulling) return;
    const name = pullModel.trim();
    setPulling(name);
    setPullProgress(0);
    setPullStatus('Starting...');

    try {
      const resp = await fetch(`${ENGINE_URL}/api/orchestrator/models/pull/stream`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ engine, model: name }),
      });
      if (!resp.ok || !resp.body) { setPullStatus('Failed'); setTimeout(() => setPulling(null), 2000); return; }

      const reader = resp.body.getReader();
      const decoder = new TextDecoder();
      let buffer = '';
      while (true) {
        const { done, value } = await reader.read();
        if (done) break;
        buffer += decoder.decode(value, { stream: true });
        const lines = buffer.split('\n');
        buffer = lines.pop() || '';
        for (const line of lines) {
          if (!line.startsWith('data: ')) continue;
          try {
            const data = JSON.parse(line.slice(6));
            if (data.percent !== undefined) setPullProgress(data.percent);
            if (data.status) setPullStatus(data.status);
            if (data.status === 'success') {
              setPullProgress(100);
              await fetchModels();
              setTimeout(() => { setPulling(null); setPullModel(''); }, 1500);
              return;
            }
          } catch { /* skip */ }
        }
      }
      setPullProgress(100);
      await fetchModels();
      setTimeout(() => { setPulling(null); setPullModel(''); }, 1500);
    } catch (err) {
      setPullStatus(`Error: ${err instanceof Error ? err.message : 'Unknown'}`);
      setTimeout(() => setPulling(null), 3000);
    }

    // Log
    fetch('/api/logs', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ level: 'info', message: `Model pulled: ${name}`, source: 'models', metadata: { model: name, engine } }),
    }).catch(() => {});
  };

  const handleDelete = async (name: string) => {
    if (!confirm(`Delete model "${name}"? This cannot be undone.`)) return;
    try {
      await fetch(`${ENGINE_URL}/api/orchestrator/models/delete`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ engine, model: name }),
      });
      await fetchModels();
    } catch { /* ignore */ }
  };

  const filtered = models.filter(m => !search || m.name.toLowerCase().includes(search.toLowerCase()));

  const TABS: Array<{ id: ModelTab; label: string; icon: React.ElementType; color: string; enabled: boolean }> = [
    { id: 'local', label: 'Local Models', icon: HardDrive, color: '#059669', enabled: true },
    { id: 'kortecx', label: 'Kortecx Models', icon: Sparkles, color: '#7C3AED', enabled: false },
    { id: 'advanced', label: 'Advanced Models', icon: Cloud, color: '#2563EB', enabled: false },
    { id: 'compare', label: 'Compare Models', icon: Scale, color: '#F04500', enabled: true },
  ];

  return (
    <div style={{ padding: 20, maxWidth: '100%' }}>
      {/* Header */}
      <motion.div initial={{ opacity: 0, y: -8 }} animate={{ opacity: 1, y: 0 }}
        style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 20 }}>
        <div>
          <h1 style={{ fontSize: 18, fontWeight: 700, color: 'var(--text-1)', margin: 0, display: 'flex', alignItems: 'center', gap: 8 }}>
            <Boxes size={18} color="#7C3AED" /> Models
          </h1>
          <p style={{ fontSize: 12, color: 'var(--text-3)', margin: '3px 0 0' }}>
            Manage local and cloud model registries
          </p>
        </div>
        {tab !== 'compare' && (
          <button className="btn btn-secondary btn-sm" onClick={fetchModels} disabled={loading}>
            <RefreshCw size={12} style={loading ? { animation: 'spin 1s linear infinite' } : undefined} /> Refresh
          </button>
        )}
      </motion.div>

      {/* Tabs */}
      <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: 0.05 }}
        style={{ display: 'flex', gap: 6, marginBottom: 20 }}>
        {TABS.map(t => (
          <button key={t.id} onClick={() => t.enabled ? setTab(t.id) : window.open(KORTECX_CLOUD_URL, '_blank')}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              padding: '8px 16px', borderRadius: 7, fontSize: 12, fontWeight: tab === t.id ? 650 : 450,
              border: `1.5px solid ${tab === t.id ? t.color : 'var(--border-md)'}`,
              background: tab === t.id ? `${t.color}10` : 'transparent',
              color: tab === t.id ? t.color : 'var(--text-3)',
              cursor: 'pointer', transition: 'all 0.15s',
              opacity: t.enabled ? 1 : 0.6,
            }}>
            <t.icon size={13} />
            {t.label}
            {!t.enabled && <Lock size={10} style={{ marginLeft: 2, opacity: 0.6 }} />}
          </button>
        ))}
      </motion.div>

      {/* ── Local Models Tab ──────────────────────────── */}
      {tab === 'local' && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}>
          {/* Engine + Search + Pull */}
          <div style={{ display: 'flex', gap: 8, marginBottom: 16, flexWrap: 'wrap' }}>
            <select className="input" style={{ width: 120 }} value={engine} onChange={e => setEngine(e.target.value as 'ollama' | 'llamacpp')}>
              <option value="ollama">Ollama</option>
              <option value="llamacpp">llama.cpp</option>
            </select>
            <div style={{ flex: 1, display: 'flex', alignItems: 'center', gap: 6, border: '1px solid var(--border-md)', borderRadius: 4, padding: '0 10px', background: 'var(--bg-surface)' }}>
              <Search size={13} color="var(--text-4)" />
              <input className="input" style={{ border: 'none', padding: '7px 0' }} placeholder="Search models..." value={search} onChange={e => setSearch(e.target.value)} />
            </div>
            <div style={{ display: 'flex', gap: 4 }}>
              <input className="input" style={{ width: 200, fontFamily: 'var(--font-mono)' }} placeholder="Model to pull (e.g. mistral:7b)" value={pullModel} onChange={e => setPullModel(e.target.value)}
                onKeyDown={e => { if (e.key === 'Enter') handlePull(); }} />
              <button className="btn btn-primary btn-sm" onClick={handlePull} disabled={!pullModel.trim() || !!pulling}>
                <Download size={12} /> Pull
              </button>
            </div>
          </div>

          {/* Pull progress */}
          {pulling && (
            <div className="card" style={{ padding: '12px 16px', marginBottom: 16 }}>
              <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', marginBottom: 6 }}>
                <span style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)', display: 'flex', alignItems: 'center', gap: 6 }}>
                  <Loader2 size={13} style={{ animation: 'spin 1s linear infinite' }} color="#7C3AED" />
                  Pulling {pulling}
                </span>
                <span className="mono" style={{ fontSize: 11, color: pullProgress >= 100 ? '#059669' : 'var(--text-3)' }}>{pullProgress.toFixed(0)}%</span>
              </div>
              <div style={{ height: 4, background: 'var(--border)', borderRadius: 2, overflow: 'hidden' }}>
                <div style={{ height: '100%', width: `${pullProgress}%`, background: pullProgress >= 100 ? '#059669' : '#7C3AED', borderRadius: 2, transition: 'width 0.3s' }} />
              </div>
              <div style={{ fontSize: 10, color: 'var(--text-4)', marginTop: 4 }}>{pullStatus}</div>
            </div>
          )}

          {/* Model list */}
          <div className="card" style={{ overflow: 'hidden' }}>
            {loading ? (
              <div style={{ padding: 40, textAlign: 'center', color: 'var(--text-3)', fontSize: 12 }}>
                <Loader2 size={18} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite', display: 'block' }} />
                Loading models...
              </div>
            ) : filtered.length === 0 ? (
              <div style={{ padding: '40px 20px', textAlign: 'center' }}>
                <Server size={28} color="var(--text-4)" style={{ margin: '0 auto 10px' }} />
                <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-2)', marginBottom: 4 }}>
                  {search ? 'No models match your search' : `No models found on ${engine}`}
                </div>
                <div style={{ fontSize: 11, color: 'var(--text-4)' }}>
                  {search ? 'Try a different search term' : `Is ${engine} running? Pull a model to get started.`}
                </div>
              </div>
            ) : (
              <table className="table-base">
                <thead>
                  <tr>
                    <th>Model</th>
                    <th>Size</th>
                    <th>Modified</th>
                    <th></th>
                  </tr>
                </thead>
                <tbody>
                  {filtered.map((m, i) => (
                    <motion.tr key={m.name} initial={{ opacity: 0 }} animate={{ opacity: 1 }} transition={{ delay: i * 0.02 }}>
                      <td>
                        <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                          <Server size={13} color="#059669" />
                          <span className="mono" style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>{m.name}</span>
                        </div>
                      </td>
                      <td><span className="mono" style={{ fontSize: 11 }}>{formatSize(m.size)}</span></td>
                      <td style={{ fontSize: 11, color: 'var(--text-4)' }}>{timeAgo(m.modified_at)}</td>
                      <td>
                        <button onClick={() => handleDelete(m.name)}
                          style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 4 }}
                          title="Delete model">
                          <Trash2 size={12} />
                        </button>
                      </td>
                    </motion.tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
          <div style={{ marginTop: 8, fontSize: 10, color: 'var(--text-4)' }}>
            {filtered.length} model{filtered.length !== 1 ? 's' : ''} on {engine}
          </div>
        </motion.div>
      )}

      {/* ── Compare Tab ─────────────────────────────── */}
      {tab === 'compare' && <CompareTab />}

      {/* ── Kortecx Models Tab (Cloud) ───────────────── */}
      {tab === 'kortecx' && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
          className="card" style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{
            background: 'linear-gradient(135deg, rgba(124,58,237,0.06) 0%, rgba(236,72,153,0.06) 100%)',
            padding: '48px 32px', textAlign: 'center',
          }}>
            <div style={{ width: 52, height: 52, borderRadius: 12, margin: '0 auto 14px', background: 'rgba(124,58,237,0.1)', border: '1.5px solid rgba(124,58,237,0.2)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Sparkles size={24} color="#7C3AED" />
            </div>
            <h2 style={{ fontSize: 18, fontWeight: 800, color: 'var(--text-1)', margin: '0 0 8px' }}>Kortecx Model Registry</h2>
            <p style={{ fontSize: 13, color: 'var(--text-3)', lineHeight: 1.6, maxWidth: 480, margin: '0 auto 20px' }}>
              Access fine-tuned, optimized models built by the Kortecx team. Includes domain-specific models for coding, research, legal, finance, and more.
            </p>
            <a href={KORTECX_CLOUD_URL} target="_blank" rel="noopener noreferrer"
              className="btn btn-primary" style={{ textDecoration: 'none', display: 'inline-flex', padding: '10px 24px' }}>
              <ExternalLink size={14} /> Sign Up for Kortecx Cloud
            </a>
          </div>
        </motion.div>
      )}

      {/* ── Advanced Models Tab (Cloud) ───────────────── */}
      {tab === 'advanced' && (
        <motion.div initial={{ opacity: 0, y: 8 }} animate={{ opacity: 1, y: 0 }}
          className="card" style={{ padding: 0, overflow: 'hidden' }}>
          <div style={{
            background: 'linear-gradient(135deg, rgba(37,99,235,0.06) 0%, rgba(16,185,129,0.06) 100%)',
            padding: '48px 32px', textAlign: 'center',
          }}>
            <div style={{ width: 52, height: 52, borderRadius: 12, margin: '0 auto 14px', background: 'rgba(37,99,235,0.1)', border: '1.5px solid rgba(37,99,235,0.2)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
              <Cloud size={24} color="#2563EB" />
            </div>
            <h2 style={{ fontSize: 18, fontWeight: 800, color: 'var(--text-1)', margin: '0 0 8px' }}>Advanced Models</h2>
            <p style={{ fontSize: 13, color: 'var(--text-3)', lineHeight: 1.6, maxWidth: 480, margin: '0 auto 20px' }}>
              Enterprise-grade models with extended context windows, multi-modal capabilities, and custom training.
              Includes GPT-4o, Claude Opus, Gemini Ultra, and exclusive Kortecx Mixture-of-Experts models.
            </p>
            <a href={KORTECX_CLOUD_URL} target="_blank" rel="noopener noreferrer"
              className="btn btn-primary" style={{ textDecoration: 'none', display: 'inline-flex', padding: '10px 24px' }}>
              <ExternalLink size={14} /> Sign Up for Kortecx Cloud
            </a>
          </div>
        </motion.div>
      )}
    </div>
  );
}
