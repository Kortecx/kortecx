'use client';

import { useState, useEffect, useCallback } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import dynamic from 'next/dynamic';
import {
  X, Save, Settings, FileText, BarChart2, Server,
  Loader2, History, Eye, EyeOff, Pencil,
} from 'lucide-react';
import { useExpertFiles } from '@/lib/hooks/useApi';
import { ROLE_META } from '@/lib/constants';
import type { Expert, ExpertRole } from '@/lib/types';
import VersionHistoryPanel from './VersionHistoryPanel';

const MonacoEditor = dynamic(() => import('@monaco-editor/react'), { ssr: false });

/* ─── Local Prompt Versioning ─────────────────────────── */

interface LocalPromptVersion {
  content: string;
  timestamp: string;
  label: string;
}

function getLocalVersions(expertId: string, type: 'system' | 'user'): LocalPromptVersion[] {
  try {
    const raw = localStorage.getItem(`kortecx:expert-prompts:${expertId}:${type}`);
    return raw ? JSON.parse(raw) : [];
  } catch { return []; }
}

function saveLocalVersion(expertId: string, type: 'system' | 'user', content: string, maxVersions: number) {
  const versions = getLocalVersions(expertId, type);
  const nextNum = versions.length > 0
    ? Math.max(...versions.map(v => parseInt(v.label.replace('v', ''), 10) || 0)) + 1
    : 1;
  const updated = [
    { content, timestamp: new Date().toISOString(), label: `v${nextNum}` },
    ...versions,
  ].slice(0, maxVersions);
  localStorage.setItem(`kortecx:expert-prompts:${expertId}:${type}`, JSON.stringify(updated));
  return updated;
}

/* ─── Component ───────────────────────────────────────── */

interface ExpertEditDialogProps {
  expert: Expert | null;
  open: boolean;
  onClose: () => void;
  onSaved: () => void;
}

type Tab = 'config' | 'prompts' | 'model' | 'files' | 'stats';

const TABS: { key: Tab; label: string; icon: typeof Settings }[] = [
  { key: 'config',  label: 'Config',  icon: Settings   },
  { key: 'prompts', label: 'Prompts', icon: FileText   },
  { key: 'model',   label: 'Model',   icon: Server     },
  { key: 'files',   label: 'Files',   icon: FileText   },
  { key: 'stats',   label: 'Stats',   icon: BarChart2  },
];

const ROLES: ExpertRole[] = [
  'researcher', 'analyst', 'writer', 'coder', 'reviewer', 'planner',
  'synthesizer', 'critic', 'legal', 'financial', 'medical', 'coordinator',
  'data-engineer', 'creative', 'translator', 'custom',
];

const MONACO_OPTIONS = {
  minimap: { enabled: false },
  wordWrap: 'on' as const,
  lineNumbers: 'off' as const,
  scrollBeyondLastLine: false,
  fontSize: 12,
  fontFamily: 'monospace',
  padding: { top: 12, bottom: 12 },
  scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
  overviewRulerLanes: 0,
  renderLineHighlight: 'none' as const,
  folding: false,
};

export default function ExpertEditDialog({ expert, open, onClose, onSaved }: ExpertEditDialogProps) {
  const [tab, setTab] = useState<Tab>('config');
  const [saving, setSaving] = useState(false);
  const [selectedFile, setSelectedFile] = useState<string | null>(null);

  // Editable fields
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [role, setRole] = useState<ExpertRole>('custom');
  const [temperature, setTemperature] = useState(0.7);
  const [maxTokens, setMaxTokens] = useState(4096);
  const [tags, setTags] = useState('');
  const [isPublic, setIsPublic] = useState(false);
  const [systemPrompt, setSystemPrompt] = useState('');
  const [userPrompt, setUserPrompt] = useState('');
  const [maxVersions, setMaxVersions] = useState(50);
  const [maxLocalVersions, setMaxLocalVersions] = useState(3);

  // Monaco / prompt UI state
  const [systemEditable, setSystemEditable] = useState(false);
  const [showSystemPreview, setShowSystemPreview] = useState(false);
  const [showUserPreview, setShowUserPreview] = useState(false);
  const [systemVersions, setSystemVersions] = useState<LocalPromptVersion[]>([]);
  const [userVersions, setUserVersions] = useState<LocalPromptVersion[]>([]);

  const { files, mutate: mutateFiles } = useExpertFiles(open ? expert?.id ?? null : null);

  // Load expert data into form when expert changes
  useEffect(() => {
    if (expert) {
      setName(expert.name);
      setDescription(expert.description || '');
      setRole(expert.role);
      setTemperature(expert.temperature);
      setMaxTokens(expert.maxTokens);
      setTags(expert.tags?.join(', ') || '');
      setIsPublic(expert.isPublic);
      setSystemPrompt(expert.systemPrompt || '');
      setTab('config');
      setSelectedFile(null);
      setSystemEditable(false);
      setShowSystemPreview(false);
      setShowUserPreview(false);
      // Load local versions
      setSystemVersions(getLocalVersions(expert.id, 'system'));
      setUserVersions(getLocalVersions(expert.id, 'user'));
    }
  }, [expert]);

  // Load prompts from engine files if available
  useEffect(() => {
    if (!open || !expert?.id) return;
    const loadPrompts = async () => {
      try {
        const sysRes = await fetch(
          `${process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000'}/api/experts/engine/${expert.id}/prompt/system`,
        );
        if (sysRes.ok) {
          const sysData = await sysRes.json();
          if (sysData.content) setSystemPrompt(sysData.content);
        }
        const userRes = await fetch(
          `${process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000'}/api/experts/engine/${expert.id}/prompt/user`,
        );
        if (userRes.ok) {
          const userData = await userRes.json();
          if (userData.content) setUserPrompt(userData.content);
        }
      } catch {
        // Use DB values as fallback
      }
    };
    loadPrompts();
  }, [open, expert?.id]);

  const handleSave = useCallback(async () => {
    if (!expert) return;
    setSaving(true);
    try {
      // 1. Update DB record
      await fetch('/api/experts', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          id: expert.id,
          name,
          description,
          role,
          temperature,
          maxTokens,
          tags: tags.split(',').map(t => t.trim()).filter(Boolean),
          isPublic,
          systemPrompt,
        }),
      });

      // 2. Update engine files (auto-versions)
      const engineUrl = process.env.NEXT_PUBLIC_ENGINE_URL || 'http://localhost:8000';

      // Update expert.json
      await fetch('/api/experts/files', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          expertId: expert.id,
          filename: 'expert.json',
          content: JSON.stringify({
            id: expert.id,
            name,
            description,
            role,
            version: expert.version,
            modelSource: expert.modelSource,
            localModelConfig: expert.localModelConfig,
            temperature,
            maxTokens,
            tags: tags.split(',').map(t => t.trim()).filter(Boolean),
            capabilities: expert.capabilities,
            isPublic,
            maxVersions,
            category: 'custom',
            createdAt: expert.createdAt,
            updatedAt: new Date().toISOString(),
          }, null, 2),
        }),
      });

      // Update system.md if changed
      if (systemPrompt !== expert.systemPrompt) {
        await fetch('/api/experts/files', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            expertId: expert.id,
            filename: 'system.md',
            content: systemPrompt,
          }),
        });
      }

      // Update user prompt
      if (userPrompt) {
        await fetch('/api/experts/files', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            expertId: expert.id,
            filename: 'user.md',
            content: userPrompt,
          }),
        });
      }

      // Update maxVersions config in engine
      await fetch(`${engineUrl}/api/experts/engine/${expert.id}/versions/config`, {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ maxVersions }),
      }).catch(() => {});

      // Save local prompt versions
      setSystemVersions(saveLocalVersion(expert.id, 'system', systemPrompt, maxLocalVersions));
      setUserVersions(saveLocalVersion(expert.id, 'user', userPrompt, maxLocalVersions));

      mutateFiles();
      onSaved();
    } catch (err) {
      console.error('Failed to save expert:', err);
    } finally {
      setSaving(false);
    }
  }, [expert, name, description, role, temperature, maxTokens, tags, isPublic, systemPrompt, userPrompt, maxVersions, maxLocalVersions, mutateFiles, onSaved]);

  if (!open || !expert) return null;

  const roleMeta = ROLE_META[role] || ROLE_META.custom;

  return (
    <AnimatePresence>
      {open && (
          <motion.div
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            onClick={onClose}
            style={{
              position: 'fixed', inset: 0, zIndex: 1000,
              background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}
          >
          {/* Dialog */}
          <motion.div
            initial={{ opacity: 0, scale: 0.95, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.95, y: 20 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={e => e.stopPropagation()}
            style={{
              zIndex: 1001,
              width: 'min(92vw, 780px)',
              maxHeight: '85vh',
              background: 'var(--bg-surface)',
              border: '1px solid var(--border)',
              borderRadius: 16,
              display: 'flex', flexDirection: 'column',
              overflow: 'hidden',
              boxShadow: '0 24px 80px rgba(0,0,0,0.15)',
            }}
          >
            {/* Header */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '18px 24px', borderBottom: '1px solid var(--border)',
            }}>
              <div style={{ display: 'flex', alignItems: 'center', gap: 12 }}>
                <div style={{
                  width: 36, height: 36, borderRadius: 8,
                  background: `${roleMeta.color}14`,
                  border: `1.5px solid ${roleMeta.color}28`,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  fontSize: 18,
                }}>
                  {roleMeta.emoji}
                </div>
                <div>
                  <h2 style={{ fontSize: 16, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.2 }}>
                    {expert.name}
                  </h2>
                  <span style={{ fontSize: 11, color: 'var(--text-3)' }}>
                    {expert.id} · v{expert.version}
                  </span>
                </div>
              </div>
              <button
                onClick={onClose}
                style={{
                  width: 32, height: 32, borderRadius: 8, border: '1px solid var(--border-md)',
                  background: 'transparent', cursor: 'pointer',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}
              >
                <X size={16} color="var(--text-3)" />
              </button>
            </div>

            {/* Tab bar */}
            <div style={{
              display: 'flex', gap: 2, padding: '0 24px',
              borderBottom: '1px solid var(--border)',
            }}>
              {TABS.map(({ key, label, icon: Icon }) => (
                <button
                  key={key}
                  onClick={() => { setTab(key); setSelectedFile(null); }}
                  style={{
                    display: 'flex', alignItems: 'center', gap: 6,
                    padding: '12px 16px', fontSize: 13, cursor: 'pointer',
                    border: 'none', background: 'transparent',
                    color: tab === key ? 'var(--text-1)' : 'var(--text-3)',
                    fontWeight: tab === key ? 700 : 400,
                    borderBottom: tab === key ? '2px solid #D97706' : '2px solid transparent',
                    transition: 'all 0.15s',
                  }}
                >
                  <Icon size={14} />
                  {label}
                </button>
              ))}
            </div>

            {/* Content */}
            <div style={{ flex: 1, overflow: 'auto', padding: 24 }}>
              {/* Config Tab */}
              {tab === 'config' && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
                  <Field label="Name">
                    <input
                      value={name}
                      onChange={e => setName(e.target.value)}
                      style={inputStyle}
                    />
                  </Field>
                  <Field label="Description">
                    <textarea
                      value={description}
                      onChange={e => setDescription(e.target.value)}
                      rows={3}
                      style={{ ...inputStyle, resize: 'vertical' }}
                    />
                  </Field>
                  <Field label="Role">
                    <select
                      value={role}
                      onChange={e => setRole(e.target.value as ExpertRole)}
                      style={inputStyle}
                    >
                      {ROLES.map(r => (
                        <option key={r} value={r}>
                          {ROLE_META[r]?.emoji ?? ''} {ROLE_META[r]?.label ?? r}
                        </option>
                      ))}
                    </select>
                  </Field>
                  <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 16 }}>
                    <Field label="Temperature">
                      <input
                        type="number"
                        value={temperature}
                        onChange={e => setTemperature(Number(e.target.value))}
                        min={0} max={2} step={0.1}
                        style={inputStyle}
                      />
                    </Field>
                    <Field label="Max Tokens">
                      <input
                        type="number"
                        value={maxTokens}
                        onChange={e => setMaxTokens(Number(e.target.value))}
                        min={1} max={128000}
                        style={inputStyle}
                      />
                    </Field>
                    <Field label="Local Versions">
                      <input
                        type="number"
                        value={maxLocalVersions}
                        onChange={e => setMaxLocalVersions(Math.max(1, Math.min(20, Number(e.target.value))))}
                        min={1} max={20}
                        style={inputStyle}
                      />
                    </Field>
                  </div>
                  <Field label="Tags (comma-separated)">
                    <input
                      value={tags}
                      onChange={e => setTags(e.target.value)}
                      placeholder="e.g. research, analysis, coding"
                      style={inputStyle}
                    />
                  </Field>
                  <label style={{ display: 'flex', alignItems: 'center', gap: 8, cursor: 'pointer' }}>
                    <input
                      type="checkbox"
                      checked={isPublic}
                      onChange={e => setIsPublic(e.target.checked)}
                      style={{ accentColor: '#D97706' }}
                    />
                    <span style={{ fontSize: 13, color: 'var(--text-2)' }}>Public (visible in marketplace)</span>
                  </label>
                </div>
              )}

              {/* Prompts Tab */}
              {tab === 'prompts' && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 24 }}>
                  {/* System Prompt */}
                  <div>
                    <div style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      marginBottom: 8,
                    }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        <label style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>
                          System Prompt
                        </label>
                        <VersionDropdown
                          versions={systemVersions}
                          onSelect={v => setSystemPrompt(v.content)}
                        />
                      </div>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                        <button
                          onClick={() => setShowSystemPreview(!showSystemPreview)}
                          title={showSystemPreview ? 'Hide preview' : 'Show preview'}
                          style={tinyBtnStyle}
                        >
                          {showSystemPreview ? <EyeOff size={12} /> : <Eye size={12} />}
                        </button>
                        <button
                          onClick={() => setSystemEditable(!systemEditable)}
                          title={systemEditable ? 'Lock editing' : 'Enable editing'}
                          style={{
                            ...tinyBtnStyle,
                            background: systemEditable ? '#D9770618' : undefined,
                            borderColor: systemEditable ? '#D97706' : undefined,
                            color: systemEditable ? '#D97706' : undefined,
                          }}
                        >
                          <Pencil size={12} />
                        </button>
                      </div>
                    </div>
                    {showSystemPreview ? (
                      <div style={previewStyle}>
                        <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontFamily: 'monospace', fontSize: 12, color: 'var(--text-2)' }}>
                          {systemPrompt || 'No system prompt defined.'}
                        </pre>
                      </div>
                    ) : (
                      <div style={{
                        borderRadius: 8, overflow: 'hidden',
                        border: `1px solid ${systemEditable ? '#D97706' : 'var(--border-md)'}`,
                        opacity: systemEditable ? 1 : 0.75,
                        transition: 'all 0.15s',
                      }}>
                        <MonacoEditor
                          height="250px"
                          language="markdown"
                          theme="vs-dark"
                          value={systemPrompt}
                          onChange={v => { if (systemEditable) setSystemPrompt(v || ''); }}
                          options={{
                            ...MONACO_OPTIONS,
                            readOnly: !systemEditable,
                          }}
                        />
                      </div>
                    )}
                  </div>

                  {/* User Prompt */}
                  <div>
                    <div style={{
                      display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                      marginBottom: 8,
                    }}>
                      <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                        <label style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-2)' }}>
                          User Prompt Template
                        </label>
                        <VersionDropdown
                          versions={userVersions}
                          onSelect={v => setUserPrompt(v.content)}
                        />
                      </div>
                      <button
                        onClick={() => setShowUserPreview(!showUserPreview)}
                        title={showUserPreview ? 'Hide preview' : 'Show preview'}
                        style={tinyBtnStyle}
                      >
                        {showUserPreview ? <EyeOff size={12} /> : <Eye size={12} />}
                      </button>
                    </div>
                    {showUserPreview ? (
                      <div style={previewStyle}>
                        <pre style={{ margin: 0, whiteSpace: 'pre-wrap', wordBreak: 'break-word', fontFamily: 'monospace', fontSize: 12, color: 'var(--text-2)' }}>
                          {userPrompt || 'No user prompt template defined.'}
                        </pre>
                      </div>
                    ) : (
                      <div style={{
                        borderRadius: 8, overflow: 'hidden',
                        border: '1px solid var(--border-md)',
                      }}>
                        <MonacoEditor
                          height="200px"
                          language="markdown"
                          theme="vs-dark"
                          value={userPrompt}
                          onChange={v => setUserPrompt(v || '')}
                          options={MONACO_OPTIONS}
                        />
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* Model Tab */}
              {tab === 'model' && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 18 }}>
                  <Field label="Model Source">
                    <div style={{ display: 'flex', gap: 8 }}>
                      {['local', 'provider'].map(src => (
                        <div
                          key={src}
                          style={{
                            padding: '10px 20px', borderRadius: 8, cursor: 'default',
                            border: expert.modelSource === src
                              ? '1.5px solid #D97706'
                              : '1px solid var(--border-md)',
                            background: expert.modelSource === src ? '#D9770614' : 'var(--bg-elevated)',
                            color: expert.modelSource === src ? '#D97706' : 'var(--text-3)',
                            fontSize: 13, fontWeight: expert.modelSource === src ? 600 : 400,
                          }}
                        >
                          {src === 'local' ? 'Local Inference' : 'Cloud Provider'}
                        </div>
                      ))}
                    </div>
                  </Field>
                  <Field label="Provider">
                    <div style={{
                      padding: '10px 14px', borderRadius: 8,
                      background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                      fontSize: 13, color: 'var(--text-2)',
                    }}>
                      {expert.providerName || expert.providerId || 'N/A'}
                    </div>
                  </Field>
                  <Field label="Model">
                    <div style={{
                      padding: '10px 14px', borderRadius: 8,
                      background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                      fontSize: 13, color: 'var(--text-2)',
                    }}>
                      {expert.modelName || expert.modelId || 'N/A'}
                    </div>
                  </Field>
                  {expert.localModelConfig && (
                    <Field label="Local Config">
                      <pre style={{
                        padding: '12px 14px', borderRadius: 8,
                        background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                        fontSize: 11, color: 'var(--text-2)', overflow: 'auto',
                        fontFamily: 'monospace',
                      }}>
                        {JSON.stringify(expert.localModelConfig, null, 2)}
                      </pre>
                    </Field>
                  )}
                </div>
              )}

              {/* Files Tab */}
              {tab === 'files' && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
                    <div style={{ fontSize: 13, fontWeight: 600, color: 'var(--text-2)' }}>
                      Expert Files
                    </div>
                    <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                      <label style={{ fontSize: 12, color: 'var(--text-3)' }}>Max Versions:</label>
                      <input
                        type="number"
                        value={maxVersions}
                        onChange={e => setMaxVersions(Number(e.target.value))}
                        min={1} max={200}
                        style={{ ...inputStyle, width: 70, padding: '4px 8px' }}
                      />
                    </div>
                  </div>

                  {/* File list */}
                  {files.length === 0 ? (
                    <div style={{
                      padding: '40px 0', textAlign: 'center',
                      color: 'var(--text-4)', fontSize: 13,
                    }}>
                      No files found. Files are synced from the engine.
                    </div>
                  ) : (
                    <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
                      {files.map((f: { name: string; size: number; modified?: string }) => (
                        <button
                          key={f.name}
                          onClick={() => setSelectedFile(selectedFile === f.name ? null : f.name)}
                          style={{
                            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
                            padding: '10px 14px', borderRadius: 8, cursor: 'pointer',
                            border: selectedFile === f.name
                              ? '1.5px solid #D97706'
                              : '1px solid var(--border)',
                            background: selectedFile === f.name ? '#D9770608' : 'var(--bg-elevated)',
                            transition: 'all 0.15s',
                          }}
                        >
                          <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                            <FileText size={14} color="var(--text-3)" />
                            <span style={{ fontSize: 13, fontWeight: 500, color: 'var(--text-1)' }}>
                              {f.name}
                            </span>
                          </div>
                          <div style={{ display: 'flex', alignItems: 'center', gap: 10 }}>
                            <span style={{ fontSize: 11, color: 'var(--text-4)' }}>
                              {f.size > 1024 ? `${(f.size / 1024).toFixed(1)}KB` : `${f.size}B`}
                            </span>
                            <History size={12} color={selectedFile === f.name ? '#D97706' : 'var(--text-4)'} />
                          </div>
                        </button>
                      ))}
                    </div>
                  )}

                  {/* Version history panel for selected file */}
                  {selectedFile && expert && (
                    <VersionHistoryPanel
                      expertId={expert.id}
                      filename={selectedFile}
                      onRestored={() => mutateFiles()}
                    />
                  )}
                </div>
              )}

              {/* Stats Tab */}
              {tab === 'stats' && (
                <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
                  <div style={{
                    display: 'grid', gridTemplateColumns: 'repeat(3, 1fr)', gap: 12,
                  }}>
                    {[
                      { label: 'Total Runs',    value: String(expert.stats?.totalRuns ?? 0) },
                      { label: 'Success Rate',  value: `${((expert.stats?.successRate ?? 0) * 100).toFixed(1)}%` },
                      { label: 'Avg Latency',   value: `${expert.stats?.avgLatencyMs ?? 0}ms` },
                      { label: 'Avg Tokens',    value: String(expert.stats?.avgTokensPerRun ?? 0) },
                      { label: 'Avg Cost',      value: `$${(expert.stats?.avgCostPerRun ?? 0).toFixed(4)}` },
                      { label: 'Rating',        value: `${(expert.stats?.rating ?? 0).toFixed(1)}/5` },
                    ].map(({ label, value }) => (
                      <div key={label} style={{
                        padding: '14px 16px', borderRadius: 10,
                        background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                      }}>
                        <div style={{ fontSize: 20, fontWeight: 800, color: 'var(--text-1)', lineHeight: 1 }}>
                          {value}
                        </div>
                        <div style={{ fontSize: 11, color: 'var(--text-3)', marginTop: 4 }}>
                          {label}
                        </div>
                      </div>
                    ))}
                  </div>
                  <div style={{
                    padding: '12px 16px', borderRadius: 8,
                    background: 'var(--bg-elevated)', border: '1px solid var(--border)',
                  }}>
                    <div style={{ fontSize: 11, color: 'var(--text-4)', marginBottom: 4 }}>Last Run</div>
                    <div style={{ fontSize: 13, color: 'var(--text-2)' }}>
                      {expert.stats?.lastRunAt
                        ? new Date(expert.stats.lastRunAt).toLocaleString()
                        : 'Never'}
                    </div>
                  </div>
                </div>
              )}
            </div>

            {/* Footer with Save */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'flex-end', gap: 10,
              padding: '14px 24px', borderTop: '1px solid var(--border)',
            }}>
              <button
                onClick={onClose}
                style={{
                  padding: '8px 18px', borderRadius: 8, fontSize: 13,
                  border: '1px solid var(--border-md)',
                  background: 'transparent', color: 'var(--text-2)',
                  cursor: 'pointer',
                }}
              >
                Cancel
              </button>
              <button
                onClick={handleSave}
                disabled={saving}
                style={{
                  display: 'flex', alignItems: 'center', gap: 6,
                  padding: '8px 20px', borderRadius: 8, fontSize: 13, fontWeight: 600,
                  border: 'none',
                  background: '#D97706', color: '#fff',
                  cursor: saving ? 'wait' : 'pointer',
                  opacity: saving ? 0.7 : 1,
                }}
              >
                {saving ? <Loader2 size={14} className="spin" /> : <Save size={14} />}
                {saving ? 'Saving...' : 'Save Changes'}
              </button>
            </div>
          </motion.div>
          </motion.div>
      )}
    </AnimatePresence>
  );
}

/* ═══════════════════════════════════════════════════════
   Version Dropdown
   ═══════════════════════════════════════════════════════ */

function VersionDropdown({
  versions,
  onSelect,
}: {
  versions: LocalPromptVersion[];
  onSelect: (v: LocalPromptVersion) => void;
}) {
  if (versions.length === 0) return null;

  return (
    <select
      onChange={e => {
        const idx = parseInt(e.target.value, 10);
        if (idx >= 0 && versions[idx]) onSelect(versions[idx]);
        e.target.value = '';
      }}
      defaultValue=""
      style={{
        padding: '2px 6px', borderRadius: 6, fontSize: 10, fontWeight: 600,
        border: '1px solid var(--border)', background: 'var(--bg-elevated)',
        color: 'var(--text-3)', cursor: 'pointer', opacity: 0.7,
        transition: 'opacity 0.15s',
      }}
      onMouseEnter={e => { e.currentTarget.style.opacity = '1'; }}
      onMouseLeave={e => { e.currentTarget.style.opacity = '0.7'; }}
    >
      <option value="" disabled>
        {versions[0]?.label ?? 'Latest'} ({versions.length})
      </option>
      {versions.map((v, i) => (
        <option key={v.timestamp} value={i}>
          {v.label} — {new Date(v.timestamp).toLocaleString()}
        </option>
      ))}
    </select>
  );
}

/* ═══════════════════════════════════════════════════════
   Helpers
   ═══════════════════════════════════════════════════════ */

const inputStyle: React.CSSProperties = {
  width: '100%',
  padding: '9px 12px',
  borderRadius: 8,
  border: '1px solid var(--border-md)',
  background: 'var(--bg-elevated)',
  color: 'var(--text-1)',
  fontSize: 13,
  outline: 'none',
  transition: 'border-color 0.15s',
};

const tinyBtnStyle: React.CSSProperties = {
  display: 'flex', alignItems: 'center', justifyContent: 'center',
  width: 26, height: 26, borderRadius: 6,
  border: '1px solid var(--border-md)', background: 'transparent',
  color: 'var(--text-3)', cursor: 'pointer', transition: 'all 0.15s',
};

const previewStyle: React.CSSProperties = {
  padding: '16px 14px', borderRadius: 8,
  background: 'var(--bg-elevated)', border: '1px solid var(--border)',
  maxHeight: 280, overflow: 'auto',
};

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label style={{
        display: 'block', fontSize: 12, fontWeight: 600,
        color: 'var(--text-2)', marginBottom: 6,
      }}>
        {label}
      </label>
      {children}
    </div>
  );
}
