'use client';

import { useState, useEffect } from 'react';
import {
  X, Sparkles, Loader2, Plus, ChevronDown, ChevronUp, Cpu, Clock, Code,
} from 'lucide-react';
import { motion } from 'framer-motion';
import { buttonHover } from '@/lib/motion';
import type { McpLanguage } from '@/lib/types';

type PromptType = 'general' | 'data_synthesis' | 'training' | 'finetuning' | 'mcp';

interface ProviderInfo {
  id: string;
  name: string;
  color: string;
  icon: string;
  models: Array<{ id: string; name: string }>;
}

interface ExecutableGenerateDialogProps {
  open: boolean;
  onClose: () => void;
  onGenerated: () => void;
  providers: ProviderInfo[];
  models: { ollama: string[]; llamacpp: string[] };
  streamGenerate: (opts: {
    prompt: string;
    description: string;
    language: McpLanguage;
    promptType: PromptType;
    systemPrompt: string;
    source: string;
    model: string;
    providerId: string;
    attachments: File[];
    onToken: (code: string) => void;
    onDone: (stats: { time_ms: number; cpu: number }) => void;
    onError: (msg: string) => void;
  }) => Promise<void>;
}

const SYSTEM_PROMPTS: Record<PromptType, (lang: McpLanguage) => string> = {
  general: (lang) => `You are a senior software engineer.\nGenerate a clean, production-ready ${lang} script as described.\nOnly output the code — no explanations, no markdown fences.`,
  data_synthesis: (lang) => `You are a data engineering expert.\nGenerate a ${lang} script that synthesizes or transforms data as described.\nThe script should handle input/output, validation, and produce clean structured data.\nOnly output the code — no explanations.`,
  training: (lang) => `You are an ML training pipeline expert.\nGenerate a ${lang} script for the described training workflow.\nInclude data loading, model setup, training loop, and evaluation.\nOnly output the code — no explanations.`,
  finetuning: (lang) => `You are an LLM fine-tuning expert.\nGenerate a ${lang} script for fine-tuning as described.\nInclude dataset preparation, LoRA/PEFT config, and training setup.\nOnly output the code — no explanations.`,
  mcp: (lang) => `You are an expert MCP (Model Context Protocol) server developer.\nGenerate a complete, working MCP server script in ${lang}.\nThe script must be self-contained and runnable.\nInclude proper imports, tool definitions, and a main entry point.\nOnly output the code — no explanations, no markdown fences.${lang === 'python' ? '\nUse the mcp SDK (from mcp.server import Server).' : '\nUse the @modelcontextprotocol/sdk package.'}`,
};

export default function ExecutableGenerateDialog({
  open, onClose, onGenerated, providers, models, streamGenerate,
}: ExecutableGenerateDialogProps) {
  const [prompt, setPrompt] = useState('');
  const [description, setDescription] = useState('');
  const [promptType, setPromptType] = useState<PromptType>('general');
  const [language, setLanguage] = useState<McpLanguage>('python');
  const [source, setSource] = useState<'ollama' | 'llamacpp' | 'provider'>('ollama');
  const [providerId, setProviderId] = useState('');
  const [model, setModel] = useState('');
  const [systemPrompt, setSystemPrompt] = useState(() => SYSTEM_PROMPTS.general('python'));
  const [showSystemPrompt, setShowSystemPrompt] = useState(false);
  const [attachments, setAttachments] = useState<File[]>([]);
  const [generating, setGenerating] = useState(false);
  const [generatedCode, setGeneratedCode] = useState('');
  const [genStats, setGenStats] = useState<{ time_ms: number; cpu: number } | null>(null);
  const [error, setError] = useState<string | null>(null);

  // Update system prompt when type/language changes
  useEffect(() => {
    setSystemPrompt(SYSTEM_PROMPTS[promptType](language));
  }, [promptType, language]);

  // Set default model when models load
  useEffect(() => {
    if (!model && models.ollama.length > 0) setModel(models.ollama[0]);
  }, [models, model]);

  if (!open) return null;

  const handleGenerate = async () => {
    if (!prompt.trim() || generating) return;
    setGenerating(true);
    setGeneratedCode('');
    setGenStats(null);
    setError(null);

    await streamGenerate({
      prompt,
      description: description || prompt,
      language,
      promptType,
      systemPrompt,
      source: source === 'provider' ? 'provider' : source,
      model: model || '',
      providerId: source === 'provider' ? providerId : '',
      attachments,
      onToken: (code) => setGeneratedCode(code),
      onDone: (stats) => {
        setGenStats(stats);
        setGenerating(false);
        onGenerated();
      },
      onError: (msg) => {
        setError(msg);
        setGenerating(false);
      },
    });
  };

  const handleClose = () => {
    if (!generating) {
      onClose();
      setPrompt('');
      setDescription('');
      setGeneratedCode('');
      setGenStats(null);
      setError(null);
      setAttachments([]);
    }
  };

  return (
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,0.6)',
        backdropFilter: 'blur(4px)', zIndex: 200,
        display: 'flex', alignItems: 'flex-start', justifyContent: 'center', paddingTop: 80,
      }}
      onClick={(e) => { if (e.target === e.currentTarget) handleClose(); }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 560, maxWidth: '92vw', background: 'var(--bg-surface)',
          border: '1px solid var(--border)', borderRadius: 12, overflow: 'hidden',
          boxShadow: '0 24px 64px rgba(0,0,0,0.3)',
        }}
      >
        {/* Header */}
        <div style={{ padding: '18px 22px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', gap: 10 }}>
          <div style={{
            width: 36, height: 36, borderRadius: 8,
            background: 'rgba(124,58,237,0.1)', border: '1px solid rgba(124,58,237,0.25)',
            display: 'flex', alignItems: 'center', justifyContent: 'center',
          }}>
            <Code size={18} color="#7C3AED" />
          </div>
          <div style={{ flex: 1 }}>
            <div style={{ fontSize: 15, fontWeight: 700, color: 'var(--text-1)' }}>
              Generate Executable
            </div>
            <div style={{ fontSize: 12, color: 'var(--text-3)' }}>
              Describe what the script should do
            </div>
          </div>
          <button
            onClick={handleClose}
            style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-3)', display: 'flex', padding: 4 }}
          >
            <X size={16} />
          </button>
        </div>

        {/* Body */}
        <div style={{ padding: '20px 22px', display: 'flex', flexDirection: 'column', gap: 14, maxHeight: '60vh', overflow: 'auto' }}>
          {/* Prompt type */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Type</label>
            <div style={{ display: 'flex', gap: 6, flexWrap: 'wrap' }}>
              {([
                { id: 'general' as const, label: 'General' },
                { id: 'data_synthesis' as const, label: 'Data Synthesis' },
                { id: 'training' as const, label: 'Training' },
                { id: 'finetuning' as const, label: 'Fine-tuning' },
                { id: 'mcp' as const, label: 'MCP Server' },
              ]).map(t => (
                <button key={t.id} onClick={() => setPromptType(t.id)} style={{
                  padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: promptType === t.id ? 600 : 400,
                  border: `1px solid ${promptType === t.id ? '#7C3AED' : 'var(--border)'}`,
                  background: promptType === t.id ? 'rgba(124,58,237,0.08)' : 'transparent',
                  color: promptType === t.id ? '#7C3AED' : 'var(--text-3)',
                  cursor: 'pointer',
                }}>{t.label}</button>
              ))}
            </div>
          </div>

          {/* Prompt */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Prompt</label>
            <textarea
              rows={4}
              className="input"
              style={{ width: '100%', resize: 'vertical', fontSize: 13 }}
              placeholder="e.g., Create a Python script that reads a CSV file, cleans missing values, and outputs summary statistics..."
              value={prompt}
              onChange={(e) => setPrompt(e.target.value)}
            />
          </div>

          {/* Description */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Description</label>
            <input
              className="input"
              style={{ width: '100%', fontSize: 12 }}
              placeholder="Short description for this executable (optional — defaults to prompt)"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
            />
          </div>

          {/* System prompt — collapsible */}
          <div>
            <button onClick={() => setShowSystemPrompt(p => !p)} style={{
              fontSize: 11, fontWeight: 600, color: 'var(--text-3)', background: 'none', border: 'none',
              cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 4, padding: 0, marginBottom: 4,
            }}>
              {showSystemPrompt ? <ChevronUp size={10} /> : <ChevronDown size={10} />}
              System Prompt
              <span style={{ fontWeight: 400, color: 'var(--text-4)', marginLeft: 4 }}>
                (auto-configured for {promptType} + {language})
              </span>
            </button>
            {showSystemPrompt && (
              <textarea
                rows={4}
                className="input"
                style={{ width: '100%', resize: 'vertical', fontSize: 11, fontFamily: 'var(--font-mono, monospace)', color: 'var(--text-2)' }}
                value={systemPrompt}
                onChange={(e) => setSystemPrompt(e.target.value)}
              />
            )}
          </div>

          {/* Attachments */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'flex', alignItems: 'center', gap: 4 }}>
              Attachments
              <span style={{ fontWeight: 400, color: 'var(--text-4)' }}>(optional)</span>
            </label>
            <div style={{ display: 'flex', gap: 6, alignItems: 'center', flexWrap: 'wrap' }}>
              <label style={{
                padding: '5px 12px', borderRadius: 6, fontSize: 11, fontWeight: 500,
                border: '1px dashed var(--border-md)', background: 'var(--bg)',
                color: 'var(--text-3)', cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 4,
              }}>
                <Plus size={11} /> Add file
                <input type="file" multiple style={{ display: 'none' }}
                  onChange={(e) => { if (e.target.files) setAttachments(prev => [...prev, ...Array.from(e.target.files!)]); }} />
              </label>
              {attachments.map((f, i) => (
                <span key={i} style={{
                  fontSize: 11, padding: '3px 8px', borderRadius: 4,
                  background: 'var(--bg-elevated)', color: 'var(--text-2)',
                  display: 'flex', alignItems: 'center', gap: 4,
                }}>
                  {f.name}
                  <button onClick={() => setAttachments(prev => prev.filter((_, j) => j !== i))}
                    style={{ background: 'none', border: 'none', cursor: 'pointer', color: 'var(--text-4)', display: 'flex', padding: 0 }}>
                    <X size={10} />
                  </button>
                </span>
              ))}
            </div>
          </div>

          {/* Language */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Language</label>
            <div style={{ display: 'flex', gap: 4 }}>
              {(['python', 'typescript', 'javascript'] as const).map(lang => (
                <button key={lang} onClick={() => setLanguage(lang)} style={{
                  padding: '4px 10px', borderRadius: 5, fontSize: 11, fontWeight: language === lang ? 600 : 400,
                  border: `1px solid ${language === lang ? '#7C3AED' : 'var(--border)'}`,
                  background: language === lang ? 'rgba(124,58,237,0.08)' : 'transparent',
                  color: language === lang ? '#7C3AED' : 'var(--text-3)',
                  cursor: 'pointer', textTransform: 'capitalize',
                }}>{lang === 'javascript' ? 'JS' : lang === 'typescript' ? 'TS' : 'Python'}</button>
              ))}
            </div>
          </div>

          {/* Inference source */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Inference Source</label>
            <div style={{ display: 'flex', gap: 4, flexWrap: 'wrap' }}>
              {(['ollama', 'llamacpp'] as const).map(src => (
                <button key={src} onClick={() => {
                  setSource(src);
                  const m = models[src];
                  if (m.length && !m.includes(model)) setModel(m[0]);
                }} style={{
                  padding: '4px 10px', borderRadius: 5, fontSize: 11, fontWeight: source === src ? 600 : 400,
                  border: `1px solid ${source === src ? '#7C3AED' : 'var(--border)'}`,
                  background: source === src ? 'rgba(124,58,237,0.08)' : 'transparent',
                  color: source === src ? '#7C3AED' : 'var(--text-3)',
                  cursor: 'pointer',
                }}>{src === 'ollama' ? 'Ollama' : 'LlamaCpp'}</button>
              ))}
              {providers.map(prov => (
                <button key={prov.id} onClick={() => {
                  setSource('provider');
                  setProviderId(prov.id);
                  if (prov.models.length) setModel(prov.models[0].id);
                }} style={{
                  padding: '4px 10px', borderRadius: 5, fontSize: 11,
                  fontWeight: source === 'provider' && providerId === prov.id ? 600 : 400,
                  border: `1px solid ${source === 'provider' && providerId === prov.id ? prov.color : 'var(--border)'}`,
                  background: source === 'provider' && providerId === prov.id ? `${prov.color}14` : 'transparent',
                  color: source === 'provider' && providerId === prov.id ? prov.color : 'var(--text-3)',
                  cursor: 'pointer', display: 'flex', alignItems: 'center', gap: 4,
                }}>
                  <span style={{ width: 6, height: 6, borderRadius: '50%', background: prov.color, flexShrink: 0 }} />
                  {prov.name}
                </button>
              ))}
            </div>
          </div>

          {/* Model selector */}
          <div>
            <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Model</label>
            {source === 'provider' ? (
              (() => {
                const prov = providers.find(p => p.id === providerId);
                if (!prov || !prov.models.length) return (
                  <div style={{ padding: '6px 10px', borderRadius: 5, fontSize: 11, color: 'var(--text-4)', background: 'var(--bg)', border: '1px solid var(--border)' }}>
                    No models available for this provider
                  </div>
                );
                return (
                  <select className="input" style={{ width: '100%', fontSize: 12 }} value={model} onChange={(e) => setModel(e.target.value)}>
                    {prov.models.map(m => <option key={m.id} value={m.id}>{m.name}</option>)}
                  </select>
                );
              })()
            ) : models[source as 'ollama' | 'llamacpp']?.length > 0 ? (
              <select className="input" style={{ width: '100%', fontSize: 12 }} value={model} onChange={(e) => setModel(e.target.value)}>
                {models[source as 'ollama' | 'llamacpp'].map(m => <option key={m} value={m}>{m}</option>)}
              </select>
            ) : (
              <div style={{ padding: '6px 10px', borderRadius: 5, fontSize: 11, color: 'var(--text-4)', background: 'var(--bg)', border: '1px solid var(--border)' }}>
                No models available — ensure {source === 'ollama' ? 'Ollama' : 'LlamaCpp'} is running
              </div>
            )}
          </div>

          {/* Generated code preview */}
          {generatedCode && (
            <div>
              <label style={{ fontSize: 11, fontWeight: 600, color: 'var(--text-3)', marginBottom: 6, display: 'block' }}>Generated Code</label>
              <pre style={{
                padding: 12, borderRadius: 8, fontSize: 11,
                fontFamily: 'var(--font-mono, monospace)',
                background: 'var(--bg)', border: '1px solid var(--border)',
                maxHeight: 200, overflow: 'auto', whiteSpace: 'pre-wrap',
                color: 'var(--text-2)',
              }}>
                {generatedCode}
              </pre>
            </div>
          )}

          {/* Error */}
          {error && (
            <div style={{
              padding: '8px 12px', borderRadius: 6, fontSize: 11,
              background: 'rgba(220,38,38,0.08)', border: '1px solid rgba(220,38,38,0.2)',
              color: '#DC2626',
            }}>
              {error}
            </div>
          )}
        </div>

        {/* Generation stats */}
        {generating && (
          <div style={{
            padding: '10px 22px', background: 'rgba(124,58,237,0.04)',
            borderTop: '1px solid var(--border)',
            display: 'flex', alignItems: 'center', gap: 12, fontSize: 11, color: 'var(--text-3)',
          }}>
            <Loader2 size={12} color="#7C3AED" style={{ animation: 'spin 1s linear infinite' }} />
            Generating — this may take a moment depending on model size...
          </div>
        )}

        {/* Footer */}
        <div style={{ padding: '14px 22px', borderTop: '1px solid var(--border)', display: 'flex', gap: 8, alignItems: 'center' }}>
          {genStats && !generating && (
            <motion.div initial={{ opacity: 0, x: -8 }} animate={{ opacity: 1, x: 0 }} transition={{ duration: 0.3 }}
              style={{ display: 'flex', gap: 8, flex: 1 }}>
              <span style={{
                display: 'flex', alignItems: 'center', gap: 4, fontSize: 10, fontWeight: 700,
                padding: '2px 8px', borderRadius: 10,
                background: genStats.time_ms < 5000 ? 'rgba(5,150,105,0.1)' : genStats.time_ms < 15000 ? 'rgba(217,119,6,0.1)' : 'rgba(220,38,38,0.1)',
                color: genStats.time_ms < 5000 ? '#059669' : genStats.time_ms < 15000 ? '#D97706' : '#DC2626',
              }}>
                <Clock size={10} /> {(genStats.time_ms / 1000).toFixed(1)}s
              </span>
              <span style={{
                display: 'flex', alignItems: 'center', gap: 4, fontSize: 10, fontWeight: 700,
                padding: '2px 8px', borderRadius: 10,
                background: genStats.cpu < 30 ? 'rgba(5,150,105,0.1)' : genStats.cpu < 70 ? 'rgba(217,119,6,0.1)' : 'rgba(220,38,38,0.1)',
                color: genStats.cpu < 30 ? '#059669' : genStats.cpu < 70 ? '#D97706' : '#DC2626',
              }}>
                <Cpu size={10} /> {genStats.cpu}%
              </span>
            </motion.div>
          )}
          {!genStats && <div style={{ flex: 1 }} />}
          <button className="btn btn-secondary btn-sm" onClick={handleClose} disabled={generating}>Cancel</button>
          <motion.button {...buttonHover} className="btn btn-primary btn-sm" onClick={handleGenerate}
            disabled={generating || !prompt.trim()}
            style={{
              display: 'flex', alignItems: 'center', gap: 6,
              opacity: generating || !prompt.trim() ? 0.5 : 1,
            }}
          >
            {generating ? <Loader2 size={12} style={{ animation: 'spin 1s linear infinite' }} /> : <Sparkles size={12} />}
            {generating ? 'Generating...' : 'Generate'}
          </motion.button>
        </div>
      </div>
    </div>
  );
}
