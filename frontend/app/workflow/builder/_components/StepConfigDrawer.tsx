'use client';

import { useState, useEffect } from 'react';
import { motion, AnimatePresence } from 'framer-motion';
import { X, Save, Sparkles, Loader2 } from 'lucide-react';
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
  wordBasedSuggestions: 'off' as const,
  quickSuggestions: false,
  suggestOnTriggerCharacters: false,
  acceptSuggestionOnCommitCharacter: false,
};

export default function StepConfigDrawer({ open, nodeId, config, onClose, onSave }: StepConfigDrawerProps) {
  const [form, setForm] = useState<StepConfig | null>(null);

  /* eslint-disable react-hooks/set-state-in-effect */
  useEffect(() => {
    if (config) setForm({ ...config });
    else setForm(null);
  }, [config, nodeId]);
  /* eslint-enable react-hooks/set-state-in-effect */

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

            {/* Advanced Inference (placeholder) */}
            {(form.stepType === 'agent' || form.stepType === 'cloud-model') && (
              <div style={{ padding: '8px 10px', borderRadius: 7, border: '1px solid var(--border)', background: 'var(--bg-elevated)', opacity: 0.5 }}>
                <div style={{ fontSize: 9, fontWeight: 700, color: 'var(--text-4)', textTransform: 'uppercase', marginBottom: 6, display: 'flex', alignItems: 'center', gap: 4 }}>
                  Advanced Inference
                  <span style={{ fontSize: 7, padding: '1px 4px', borderRadius: 3, background: '#f59e0b18', color: '#f59e0b', fontWeight: 700, marginLeft: 'auto' }}>COMING SOON</span>
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6 }}>
                  {[
                    { label: 'KV Cache', options: ['Auto', 'Aggressive', 'Conservative'] },
                    { label: 'Memory', options: ['Standard', '100x Boost'] },
                    { label: 'Quantization', options: ['None', 'INT8', 'INT4'] },
                    { label: 'SLM Mode', options: ['Standard', 'Enhanced'] },
                  ].map(cfg => (
                    <div key={cfg.label}>
                      <div style={{ fontSize: 8, color: 'var(--text-4)', marginBottom: 2 }}>{cfg.label}</div>
                      <select disabled style={{ ...inputStyle, fontSize: 10, padding: '4px 6px', cursor: 'not-allowed', opacity: 0.6 }}>
                        {cfg.options.map(o => <option key={o}>{o}</option>)}
                      </select>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {/* Executable: Runtime + Existing selector + Script + Generate */}
            {form.stepType === 'executable' && (
              <ExecutableSection form={form} update={update} inputStyle={inputStyle} labelStyle={labelStyle} />
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

            {/* Transformer: HuggingFace task config */}
            {form.stepType === 'transformer' && (
              <>
                <div>
                  <div style={labelStyle}>Task Type</div>
                  <select value={form.provider ?? 'text-generation'} onChange={e => update('provider', e.target.value)} style={{ ...inputStyle, cursor: 'pointer' }}>
                    <option value="text-generation">Text Generation</option>
                    <option value="summarization">Summarization</option>
                    <option value="translation">Translation</option>
                    <option value="text-classification">Text Classification</option>
                    <option value="fill-mask">Fill Mask</option>
                    <option value="question-answering">Question Answering</option>
                    <option value="feature-extraction">Feature Extraction</option>
                  </select>
                </div>
                <div>
                  <div style={labelStyle}>HuggingFace Model</div>
                  <input value={form.model} onChange={e => update('model', e.target.value)} placeholder="e.g. facebook/bart-large-cnn" style={inputStyle} />
                </div>
                <div>
                  <div style={labelStyle}>Max Length</div>
                  <input type="number" value={form.maxTokens} onChange={e => update('maxTokens', parseInt(e.target.value) || 512)} style={inputStyle} />
                </div>
              </>
            )}

            {/* Model: Direct inference */}
            {form.stepType === 'model' && (
              <>
                <div>
                  <div style={labelStyle}>Task Description</div>
                  <textarea value={form.taskDescription} onChange={e => update('taskDescription', e.target.value)} rows={3} style={{ ...inputStyle, resize: 'vertical' }} />
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                  <div>
                    <div style={labelStyle}>Engine</div>
                    <select value={form.engine} onChange={e => { update('engine', e.target.value); if (e.target.value === 'anthropic') update('model', 'claude-sonnet-4-6'); else if (e.target.value === 'openai') update('model', 'gpt-4o'); else update('model', 'llama3.2:3b'); }} style={{ ...inputStyle, cursor: 'pointer' }}>
                      <option value="ollama">Ollama</option>
                      <option value="llamacpp">llama.cpp</option>
                      <option value="anthropic">Anthropic</option>
                      <option value="openai">OpenAI</option>
                      <option value="google">Google</option>
                    </select>
                  </div>
                  <div>
                    <div style={labelStyle}>Model</div>
                    <input value={form.model} onChange={e => update('model', e.target.value)} style={inputStyle} />
                  </div>
                </div>
                <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
                  <div>
                    <div style={labelStyle}>Temperature ({form.temperature.toFixed(1)})</div>
                    <input type="range" min={0} max={2} step={0.1} value={form.temperature} onChange={e => update('temperature', parseFloat(e.target.value))} style={{ width: '100%' }} />
                  </div>
                  <div>
                    <div style={labelStyle}>Max Tokens</div>
                    <input type="number" value={form.maxTokens} onChange={e => update('maxTokens', parseInt(e.target.value) || 4096)} style={inputStyle} />
                  </div>
                </div>
              </>
            )}

            {/* Plugin: Select from installed */}
            {form.stepType === 'plugin' && (
              <div>
                <div style={labelStyle}>Plugin</div>
                <select value={form.mcpServerId ?? ''} onChange={e => update('mcpServerId', e.target.value)} style={{ ...inputStyle, cursor: 'pointer' }}>
                  <option value="">Select installed plugin...</option>
                  <option value="web-scraper">Web Scraper</option>
                  <option value="pdf-parser">PDF Parser</option>
                  <option value="code-executor">Code Executor</option>
                  <option value="image-generator">Image Generator</option>
                  <option value="multi-translator">Multi-Translator</option>
                  <option value="chart-builder">Chart Builder</option>
                  <option value="email-composer">Email Composer</option>
                  <option value="vector-search">Vector Search</option>
                </select>
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

/* ── Executable Section with dropdown + generate ──── */
function ExecutableSection({ form, update, inputStyle, labelStyle }: {
  form: StepConfig;
  update: (key: keyof StepConfig, value: unknown) => void;
  inputStyle: React.CSSProperties;
  labelStyle: React.CSSProperties;
}) {
  const [executables, setExecutables] = useState<Array<{ name: string; language: string }>>([]);
  const [generating, setGenerating] = useState(false);

  useEffect(() => {
    fetch('/api/mcp/servers')
      .then(r => r.ok ? r.json() : { artifacts: [] })
      .then(d => {
        const servers = [...(d.prebuilt ?? []), ...(d.persisted ?? []), ...(d.cached ?? [])];
        setExecutables(servers.filter((s: Record<string, unknown>) =>
          typeof s.name === 'string'
        ).map((s: Record<string, unknown>) => ({
          name: (s.name as string) || (s.id as string) || 'unnamed',
          language: ((s.language as string) || 'python'),
        })));
      })
      .catch(() => {});
  }, []);

  const handleGenerate = async () => {
    if (!form.taskDescription?.trim()) return;
    setGenerating(true);
    try {
      const lang = form.runtime === 'typescript' ? 'TypeScript' : 'Python';
      const res = await fetch('/api/mcp/generate', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          prompt: `Write a ${lang} script that: ${form.taskDescription}. Output only the code, no explanations.`,
          language: form.runtime || 'python',
        }),
      });
      if (res.ok) {
        const data = await res.json();
        if (data.code) update('scriptContent', data.code);
      }
    } catch { /* ignore */ }
    setGenerating(false);
  };

  return (
    <>
      <div>
        <div style={labelStyle}>Runtime</div>
        <div style={{ display: 'flex', gap: 6 }}>
          {(['python', 'typescript'] as const).map(rt => (
            <button key={rt} onClick={() => update('runtime', rt)} style={{
              flex: 1, padding: '8px 0', borderRadius: 7, fontSize: 12, fontWeight: 600,
              border: form.runtime === rt ? `1.5px solid ${SECTION_COLOR}` : '1px solid var(--border)',
              background: form.runtime === rt ? `${SECTION_COLOR}12` : 'var(--bg-elevated)',
              color: form.runtime === rt ? SECTION_COLOR : 'var(--text-3)', cursor: 'pointer',
            }}>
              {rt === 'python' ? 'Python' : 'TypeScript'}
            </button>
          ))}
        </div>
      </div>

      {/* Existing executables dropdown */}
      {executables.length > 0 && (
        <div>
          <div style={labelStyle}>Use Existing</div>
          <select
            value=""
            onChange={e => {
              const sel = executables.find(ex => ex.name === e.target.value);
              if (sel) {
                update('runtime', sel.language === 'python' ? 'python' : 'typescript');
                update('label', sel.name.replace(/\.[^.]+$/, ''));
              }
            }}
            style={{ ...inputStyle, cursor: 'pointer' }}
          >
            <option value="">Select existing executable...</option>
            {executables.map(ex => (
              <option key={ex.name} value={ex.name}>{ex.name} ({ex.language})</option>
            ))}
          </select>
        </div>
      )}

      <div>
        <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <div style={labelStyle}>Script</div>
          <button onClick={handleGenerate} disabled={generating || !form.taskDescription?.trim()} style={{
            display: 'flex', alignItems: 'center', gap: 4, padding: '3px 8px', borderRadius: 5,
            fontSize: 10, fontWeight: 600, cursor: generating ? 'wait' : 'pointer',
            border: `1px solid ${SECTION_COLOR}40`, background: `${SECTION_COLOR}08`, color: SECTION_COLOR,
            opacity: (generating || !form.taskDescription?.trim()) ? 0.5 : 1,
          }}>
            {generating ? <Loader2 size={10} className="spin" /> : <Sparkles size={10} />}
            {generating ? 'Generating...' : 'Generate'}
          </button>
        </div>
        <div style={{ border: '1px solid var(--border)', borderRadius: 8, overflow: 'hidden', marginTop: 4 }}>
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
  );
}

export type { StepConfig };
