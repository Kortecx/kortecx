'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Save } from 'lucide-react';
import dynamic from 'next/dynamic';
import type { StepNodeType } from './nodes/BaseStepNode';

const MonacoEditor = dynamic(() => import('@monaco-editor/react'), { ssr: false });

const SECTION_COLOR = '#06b6d4';

interface StepConfig {
  label: string;
  stepType: StepNodeType;
  taskDescription: string;
  systemInstructions: string;
  model: string;
  engine: string;
  temperature: number;
  maxTokens: number;
  runtime?: 'python' | 'typescript';
  scriptContent?: string;
  mcpServerId?: string;
  outputFormat?: 'markdown' | 'pdf';
  outputFilename?: string;
  expertId?: string;
  expertName?: string;
  provider?: string;
}

interface StepConfigDrawerProps {
  open: boolean;
  nodeId: string | null;
  config: StepConfig | null;
  onClose: () => void;
  onSave: (nodeId: string, config: StepConfig) => void;
}

const MONO_OPTIONS = {
  minimap: { enabled: false },
  wordWrap: 'on' as const,
  lineNumbers: 'off' as const,
  scrollBeyondLastLine: false,
  fontSize: 12,
  padding: { top: 12, bottom: 12 },
  scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
  renderLineHighlight: 'none' as const,
  folding: false,
};

export default function StepConfigDrawer({ open, nodeId, config, onClose, onSave }: StepConfigDrawerProps) {
  const [form, setForm] = useState<StepConfig | null>(null);

  useEffect(() => {
    if (config) setForm({ ...config });
    else setForm(null);
  }, [config, nodeId]);

  if (!open || !form || !nodeId) return null;

  const update = (key: keyof StepConfig, value: unknown) => {
    setForm(prev => prev ? { ...prev, [key]: value } : prev);
  };

  const inputStyle: React.CSSProperties = {
    width: '100%', padding: '8px 12px', borderRadius: 8,
    border: '1px solid var(--border)', background: 'var(--bg-elevated)',
    fontSize: 12, color: 'var(--text-1)', outline: 'none',
  };

  const labelStyle: React.CSSProperties = {
    fontSize: 11, fontWeight: 700, color: 'var(--text-3)',
    textTransform: 'uppercase', letterSpacing: '0.04em', marginBottom: 6,
  };

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ x: 400, opacity: 0 }}
          animate={{ x: 0, opacity: 1 }}
          exit={{ x: 400, opacity: 0 }}
          transition={{ type: 'spring', stiffness: 400, damping: 35 }}
          style={{
            position: 'fixed', top: 0, right: 0, bottom: 0, width: 420,
            zIndex: 800, background: 'var(--bg-surface)',
            borderLeft: '1px solid var(--border)',
            display: 'flex', flexDirection: 'column',
            boxShadow: '-8px 0 24px rgba(0,0,0,0.08)',
          }}
        >
          {/* Header */}
          <div style={{
            display: 'flex', alignItems: 'center', justifyContent: 'space-between',
            padding: '16px 20px', borderBottom: '1px solid var(--border)',
          }}>
            <div style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>
              Configure: {form.label}
            </div>
            <button onClick={onClose} style={{
              width: 28, height: 28, borderRadius: 7, border: '1px solid var(--border)',
              background: 'transparent', cursor: 'pointer', color: 'var(--text-3)',
              display: 'flex', alignItems: 'center', justifyContent: 'center',
            }}>
              <X size={13} />
            </button>
          </div>

          {/* Content */}
          <div style={{ flex: 1, overflow: 'auto', padding: 20, display: 'flex', flexDirection: 'column', gap: 16 }}>
            {/* Name */}
            <div>
              <div style={labelStyle}>Step Name</div>
              <input value={form.label} onChange={e => update('label', e.target.value)} style={inputStyle} />
            </div>

            {/* Task Description (Agent, Cloud Model) */}
            {(form.stepType === 'agent' || form.stepType === 'cloud-model') && (
              <div>
                <div style={labelStyle}>Task Description</div>
                <textarea
                  value={form.taskDescription}
                  onChange={e => update('taskDescription', e.target.value)}
                  rows={3}
                  style={{ ...inputStyle, resize: 'vertical' }}
                />
              </div>
            )}

            {/* System Instructions (Agent, Cloud Model) */}
            {(form.stepType === 'agent' || form.stepType === 'cloud-model') && (
              <div>
                <div style={labelStyle}>System Instructions</div>
                <div style={{ border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
                  <MonacoEditor
                    height={150}
                    language="markdown"
                    value={form.systemInstructions}
                    onChange={v => update('systemInstructions', v ?? '')}
                    theme="vs-dark"
                    options={MONO_OPTIONS}
                  />
                </div>
              </div>
            )}

            {/* Engine/Provider + Model dropdowns */}
            {(form.stepType === 'agent' || form.stepType === 'cloud-model') && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <div>
                  <div style={labelStyle}>Provider / Engine</div>
                  <select
                    value={form.engine}
                    onChange={e => {
                      const eng = e.target.value;
                      update('engine', eng);
                      // Set sensible default model when engine changes
                      if (eng === 'anthropic') update('model', 'claude-sonnet-4-6');
                      else if (eng === 'openai') update('model', 'gpt-4o');
                      else if (eng === 'google') update('model', 'gemini-2.0-flash');
                      else if (eng === 'ollama') update('model', 'llama3.2:3b');
                      else if (eng === 'llamacpp') update('model', 'default');
                    }}
                    style={{ ...inputStyle, cursor: 'pointer' }}
                  >
                    <option value="ollama">Ollama (Local)</option>
                    <option value="llamacpp">llama.cpp (Local)</option>
                    <option value="anthropic">Anthropic</option>
                    <option value="openai">OpenAI</option>
                    <option value="google">Google</option>
                  </select>
                </div>
                <div>
                  <div style={labelStyle}>Model</div>
                  {form.engine === 'anthropic' || form.engine === 'openai' || form.engine === 'google' ? (
                    <select value={form.model} onChange={e => update('model', e.target.value)} style={{ ...inputStyle, cursor: 'pointer' }}>
                      {form.engine === 'anthropic' && (
                        <>
                          <option value="claude-sonnet-4-6">Claude Sonnet 4.6</option>
                          <option value="claude-opus-4-6">Claude Opus 4.6</option>
                          <option value="claude-haiku-4-5-20251001">Claude Haiku 4.5</option>
                        </>
                      )}
                      {form.engine === 'openai' && (
                        <>
                          <option value="gpt-4o">GPT-4o</option>
                          <option value="gpt-4o-mini">GPT-4o Mini</option>
                          <option value="o3-mini">o3-mini</option>
                        </>
                      )}
                      {form.engine === 'google' && (
                        <>
                          <option value="gemini-2.0-flash">Gemini 2.0 Flash</option>
                          <option value="gemini-2.5-pro">Gemini 2.5 Pro</option>
                        </>
                      )}
                    </select>
                  ) : (
                    <input value={form.model} onChange={e => update('model', e.target.value)} placeholder="e.g. llama3.2:3b" style={inputStyle} />
                  )}
                </div>
              </div>
            )}

            {/* Temperature / MaxTokens */}
            {(form.stepType === 'agent' || form.stepType === 'cloud-model') && (
              <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                <div>
                  <div style={labelStyle}>Temperature ({form.temperature.toFixed(1)})</div>
                  <input
                    type="range" min={0} max={2} step={0.1}
                    value={form.temperature}
                    onChange={e => update('temperature', parseFloat(e.target.value))}
                    style={{ width: '100%' }}
                  />
                </div>
                <div>
                  <div style={labelStyle}>Max Tokens</div>
                  <input
                    type="number" value={form.maxTokens}
                    onChange={e => update('maxTokens', parseInt(e.target.value) || 4096)}
                    style={inputStyle}
                  />
                </div>
              </div>
            )}

            {/* Executable: Runtime + Script */}
            {form.stepType === 'executable' && (
              <>
                <div>
                  <div style={labelStyle}>Runtime</div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    {(['python', 'typescript'] as const).map(rt => (
                      <button
                        key={rt}
                        onClick={() => update('runtime', rt)}
                        style={{
                          flex: 1, padding: '8px 0', borderRadius: 7, fontSize: 12, fontWeight: 600,
                          border: form.runtime === rt ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
                          background: form.runtime === rt ? `${SECTION_COLOR}12` : 'var(--bg-elevated)',
                          color: form.runtime === rt ? SECTION_COLOR : 'var(--text-3)',
                          cursor: 'pointer',
                        }}
                      >
                        {rt === 'python' ? 'Python' : 'TypeScript'}
                      </button>
                    ))}
                  </div>
                </div>
                <div>
                  <div style={labelStyle}>Script</div>
                  <div style={{ border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden' }}>
                    <MonacoEditor
                      height={200}
                      language={form.runtime === 'typescript' ? 'typescript' : 'python'}
                      value={form.scriptContent ?? ''}
                      onChange={v => update('scriptContent', v ?? '')}
                      theme="vs-dark"
                      options={MONO_OPTIONS}
                    />
                  </div>
                </div>
              </>
            )}

            {/* MCP Server: Server selector */}
            {form.stepType === 'mcp-server' && (
              <div>
                <div style={labelStyle}>MCP Server ID</div>
                <input
                  value={form.mcpServerId ?? ''}
                  onChange={e => update('mcpServerId', e.target.value)}
                  placeholder="Enter MCP server ID or select from catalog"
                  style={inputStyle}
                />
              </div>
            )}

            {/* Action: Output config */}
            {form.stepType === 'action' && (
              <>
                <div>
                  <div style={labelStyle}>Output Format</div>
                  <div style={{ display: 'flex', gap: 6 }}>
                    {(['markdown', 'pdf'] as const).map(fmt => (
                      <button
                        key={fmt}
                        onClick={() => update('outputFormat', fmt)}
                        style={{
                          flex: 1, padding: '8px 0', borderRadius: 7, fontSize: 12, fontWeight: 600,
                          border: form.outputFormat === fmt ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
                          background: form.outputFormat === fmt ? `${SECTION_COLOR}12` : 'var(--bg-elevated)',
                          color: form.outputFormat === fmt ? SECTION_COLOR : 'var(--text-3)',
                          cursor: 'pointer', textTransform: 'uppercase',
                        }}
                      >
                        {fmt}
                      </button>
                    ))}
                  </div>
                </div>
                <div>
                  <div style={labelStyle}>Output Filename</div>
                  <input
                    value={form.outputFilename ?? ''}
                    onChange={e => update('outputFilename', e.target.value)}
                    placeholder="output.md"
                    style={inputStyle}
                  />
                </div>
              </>
            )}

            {/* Cloud Model: Provider */}
            {form.stepType === 'cloud-model' && (
              <div>
                <div style={labelStyle}>Provider</div>
                <div style={{ display: 'flex', gap: 6 }}>
                  {['anthropic', 'openai', 'google'].map(p => (
                    <button
                      key={p}
                      onClick={() => update('provider', p)}
                      style={{
                        flex: 1, padding: '8px 0', borderRadius: 7, fontSize: 11, fontWeight: 600,
                        border: form.provider === p ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
                        background: form.provider === p ? `${SECTION_COLOR}12` : 'var(--bg-elevated)',
                        color: form.provider === p ? SECTION_COLOR : 'var(--text-3)',
                        cursor: 'pointer', textTransform: 'capitalize',
                      }}
                    >
                      {p}
                    </button>
                  ))}
                </div>
              </div>
            )}
          </div>

          {/* Footer */}
          <div style={{
            padding: '14px 20px', borderTop: '1px solid var(--border)',
            display: 'flex', gap: 8, justifyContent: 'flex-end',
          }}>
            <button onClick={onClose} style={{
              padding: '8px 16px', borderRadius: 8, fontSize: 12, fontWeight: 500,
              border: '1px solid var(--border)', background: 'transparent',
              color: 'var(--text-3)', cursor: 'pointer',
            }}>
              Cancel
            </button>
            <button
              onClick={() => { if (form) onSave(nodeId, form); onClose(); }}
              style={{
                display: 'flex', alignItems: 'center', gap: 6,
                padding: '8px 18px', borderRadius: 8, fontSize: 12, fontWeight: 700,
                border: `1.5px solid ${SECTION_COLOR}`, background: SECTION_COLOR,
                color: '#fff', cursor: 'pointer',
              }}
            >
              <Save size={12} />
              Save
            </button>
          </div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

export type { StepConfig };
