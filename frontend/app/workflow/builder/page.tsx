'use client';

import { useState, useCallback, Suspense, useRef, useEffect } from 'react';
import { useSearchParams, useRouter } from 'next/navigation';
import { motion, AnimatePresence } from 'framer-motion';
import dynamic from 'next/dynamic';
import {
  Loader2, Save, Paperclip, X, Mic, Eye, EyeOff,
  Workflow as WorkflowIcon, ArrowLeft, FileText,
} from 'lucide-react';
import {
  useNodesState,
  useEdgesState,
  addEdge,
  type Connection,
  type Edge,
  type Node,
  ReactFlowProvider,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';

import StepFlowEditor, { createStepNode, createStartNode, STEP_DEFAULTS } from './_components/StepFlowEditor';
import MasterAgentPanel, { type MasterAgent } from './_components/MasterAgentPanel';
import StepConfigDrawer, { type StepConfig } from './_components/StepConfigDrawer';
import type { StepNodeType, StepNodeData } from './_components/nodes/BaseStepNode';

const MonacoEditor = dynamic(() => import('@monaco-editor/react'), { ssr: false });

const ACCENT = '#D97706';

const MONO_OPTIONS = {
  minimap: { enabled: false },
  wordWrap: 'on' as const,
  lineNumbers: 'off' as const,
  scrollBeyondLastLine: false,
  fontSize: 13,
  fontFamily: 'monospace',
  padding: { top: 14, bottom: 14 },
  scrollbar: { verticalScrollbarSize: 6, horizontalScrollbarSize: 6 },
  renderLineHighlight: 'none' as const,
  folding: false,
  wordBasedSuggestions: 'off' as const,
  quickSuggestions: false,
  suggestOnTriggerCharacters: false,
  acceptSuggestionOnCommitCharacter: false,
};

/* ── Expert Picker Modal ──────────────────────────── */
function ExpertPickerModal({ open, onClose, onSelect }: {
  open: boolean;
  onClose: () => void;
  onSelect: (expert: { id: string; name: string; role: string; description?: string; modelName?: string }) => void;
}) {
  const [experts, setExperts] = useState<Array<Record<string, unknown>>>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    fetch('/api/experts')
      .then(r => r.json())
      .then(d => setExperts(d.experts ?? []))
      .catch(() => {})
      .finally(() => setLoading(false));
  }, [open]);

  if (!open) return null;

  const filtered = search
    ? experts.filter(e => ((e.name as string) ?? '').toLowerCase().includes(search.toLowerCase()))
    : experts;

  return (
    <motion.div
      initial={{ opacity: 0 }} animate={{ opacity: 1 }}
      style={{
        position: 'fixed', inset: 0, zIndex: 1000,
        display: 'flex', alignItems: 'center', justifyContent: 'center',
        background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
      }}
      onClick={onClose}
    >
      <motion.div
        initial={{ opacity: 0, scale: 0.96, y: 20 }}
        animate={{ opacity: 1, scale: 1, y: 0 }}
        onClick={e => e.stopPropagation()}
        style={{
          background: 'var(--bg-surface)', border: '1px solid var(--border)',
          borderRadius: 14, width: 440, maxHeight: '65vh',
          display: 'flex', flexDirection: 'column', overflow: 'hidden',
        }}
      >
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)' }}>Select Agent</span>
          <button onClick={onClose} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--text-3)' }}><X size={14} /></button>
        </div>
        <div style={{ padding: '8px 18px', borderBottom: '1px solid var(--border)' }}>
          <input value={search} onChange={e => setSearch(e.target.value)} placeholder="Search agents..."
            style={{ width: '100%', padding: '7px 10px', borderRadius: 7, border: '1px solid var(--border)', background: 'var(--bg-elevated)', fontSize: 11, color: 'var(--text-1)', outline: 'none' }}
          />
        </div>
        <div style={{ flex: 1, overflow: 'auto', padding: '8px 18px' }}>
          {loading && <div style={{ textAlign: 'center', padding: 16, color: 'var(--text-4)' }}><Loader2 size={14} className="spin" /></div>}
          {filtered.map(e => (
            <button key={e.id as string} onClick={() => { onSelect({ id: e.id as string, name: e.name as string, role: e.role as string, description: e.description as string, modelName: e.modelName as string }); onClose(); }}
              style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%', padding: '8px 10px', borderRadius: 7, cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--bg-elevated)', marginBottom: 4, textAlign: 'left', transition: 'all 0.12s' }}
              onMouseEnter={e => { e.currentTarget.style.borderColor = `${ACCENT}60`; }}
              onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--border)'; }}
            >
              <div style={{ width: 28, height: 28, borderRadius: 6, background: `${ACCENT}12`, display: 'flex', alignItems: 'center', justifyContent: 'center', fontSize: 14 }}>🤖</div>
              <div>
                <div style={{ fontSize: 12, fontWeight: 600, color: 'var(--text-1)' }}>{e.name as string}</div>
                <div style={{ fontSize: 10, color: 'var(--text-3)' }}>{e.role as string}</div>
              </div>
            </button>
          ))}
        </div>
      </motion.div>
    </motion.div>
  );
}

/* ── Prompt Dialog ────────────────────────────────── */
function PromptDialog({ open, onClose, prompt, onSave, attachments, onAttach, onRemoveAttachment }: {
  open: boolean;
  onClose: () => void;
  prompt: string;
  onSave: (value: string) => void;
  attachments: Array<{ name: string; url: string }>;
  onAttach: () => void;
  onRemoveAttachment: (index: number) => void;
}) {
  const [value, setValue] = useState(prompt);
  const [preview, setPreview] = useState(false);

  useEffect(() => { if (open) { setValue(prompt); setPreview(false); } }, [open, prompt]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
          style={{
            position: 'fixed', inset: 0, zIndex: 1000,
            display: 'flex', alignItems: 'center', justifyContent: 'center',
            background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)',
          }}
          onClick={onClose}
        >
          <motion.div
            initial={{ opacity: 0, scale: 0.96, y: 20 }}
            animate={{ opacity: 1, scale: 1, y: 0 }}
            exit={{ opacity: 0, scale: 0.96, y: 20 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={e => e.stopPropagation()}
            style={{
              background: 'var(--bg-surface)', border: '1px solid var(--border)',
              borderRadius: 14, width: '90vw', maxWidth: 700, maxHeight: '80vh',
              display: 'flex', flexDirection: 'column', overflow: 'hidden',
            }}
          >
            {/* Header */}
            <div style={{
              display: 'flex', alignItems: 'center', justifyContent: 'space-between',
              padding: '14px 20px', borderBottom: '1px solid var(--border)',
            }}>
              <span style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>Workflow Prompt</span>
              <div style={{ display: 'flex', gap: 6 }}>
                <button onClick={onAttach} style={{
                  display: 'flex', alignItems: 'center', gap: 4, padding: '5px 10px', borderRadius: 6,
                  fontSize: 11, fontWeight: 500, border: '1px solid var(--border)', background: 'transparent',
                  color: 'var(--text-3)', cursor: 'pointer',
                }}>
                  <Paperclip size={11} /> Attach
                </button>
                <button disabled title="Available in Kortecx Cloud" style={{
                  display: 'flex', alignItems: 'center', gap: 4, padding: '5px 10px', borderRadius: 6,
                  fontSize: 11, fontWeight: 500, border: '1px solid var(--border)', background: 'transparent',
                  color: 'var(--text-4)', cursor: 'not-allowed', opacity: 0.5,
                }}>
                  <Mic size={11} /> Voice
                </button>
                <button onClick={() => setPreview(p => !p)} style={{
                  display: 'flex', alignItems: 'center', gap: 4, padding: '5px 10px', borderRadius: 6,
                  fontSize: 11, fontWeight: 500,
                  border: preview ? `1px solid ${ACCENT}` : '1px solid var(--border)',
                  background: preview ? `${ACCENT}10` : 'transparent',
                  color: preview ? ACCENT : 'var(--text-3)', cursor: 'pointer',
                }}>
                  {preview ? <EyeOff size={11} /> : <Eye size={11} />}
                  {preview ? 'Edit' : 'Preview'}
                </button>
                <button onClick={onClose} style={{
                  width: 28, height: 28, borderRadius: 7, border: '1px solid var(--border)',
                  background: 'transparent', cursor: 'pointer', color: 'var(--text-3)',
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                }}>
                  <X size={13} />
                </button>
              </div>
            </div>

            {/* Attachments */}
            {attachments.length > 0 && (
              <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap', padding: '8px 20px', borderBottom: '1px solid var(--border)' }}>
                {attachments.map((a, i) => (
                  <div key={i} style={{
                    display: 'flex', alignItems: 'center', gap: 4, padding: '3px 8px', borderRadius: 5,
                    background: `${ACCENT}10`, border: `1px solid ${ACCENT}30`, fontSize: 10, color: ACCENT, fontWeight: 500,
                  }}>
                    {a.name}
                    <button onClick={() => onRemoveAttachment(i)} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: ACCENT, padding: 0 }}>
                      <X size={9} />
                    </button>
                  </div>
                ))}
              </div>
            )}

            {/* Monaco Editor or Preview */}
            {!preview ? (
              <div style={{ height: 400 }}>
                <MonacoEditor
                  height={400}
                  language="markdown"
                  value={value}
                  onChange={v => setValue(v ?? '')}
                  theme="vs-dark"
                  options={MONO_OPTIONS}
                />
              </div>
            ) : (
              <div style={{
                height: 400, overflow: 'auto', padding: 20,
                fontSize: 13, color: 'var(--text-2)', lineHeight: 1.7,
                whiteSpace: 'pre-wrap', wordBreak: 'break-word',
              }}>
                {value || <span style={{ color: 'var(--text-4)', fontStyle: 'italic' }}>No prompt written yet.</span>}
              </div>
            )}

            {/* Footer */}
            <div style={{
              display: 'flex', justifyContent: 'flex-end', gap: 8,
              padding: '10px 20px', borderTop: '1px solid var(--border)',
            }}>
              <button onClick={onClose} style={{
                padding: '6px 14px', borderRadius: 7, fontSize: 11, fontWeight: 500,
                border: '1px solid var(--border)', background: 'transparent', color: 'var(--text-3)', cursor: 'pointer',
              }}>Cancel</button>
              <button onClick={() => { onSave(value); onClose(); }} style={{
                padding: '6px 16px', borderRadius: 7, fontSize: 11, fontWeight: 700,
                border: `1.5px solid ${ACCENT}`, background: ACCENT, color: '#fff', cursor: 'pointer',
              }}>Save Prompt</button>
            </div>
          </motion.div>
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/* ── Main Builder ──────────────────────────────────── */
function WorkflowBuilderInner() {
  const router = useRouter();
  const searchParams = useSearchParams();
  const workflowId = searchParams.get('id');

  // Form state
  const [name, setName] = useState(() => `workflow-${Date.now()}`);
  const [description, setDescription] = useState('');
  const [prompt, setPrompt] = useState('');
  const [showPromptDialog, setShowPromptDialog] = useState(false);
  const [attachments, setAttachments] = useState<Array<{ name: string; url: string }>>([]);
  const [saving, setSaving] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  // Master agent
  const [masterAgent, setMasterAgent] = useState<MasterAgent | null>(null);
  const [showExpertPicker, setShowExpertPicker] = useState(false);

  // ReactFlow state — initialize with Start node
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([createStartNode()]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  // Step config drawer
  const [configNodeId, setConfigNodeId] = useState<string | null>(null);
  const [nodeConfigs, setNodeConfigs] = useState<Record<string, StepConfig>>({});
  const nodeCounter = useRef(0);

  // Connect handler
  const onConnect = useCallback((params: Connection) => {
    setEdges(eds => addEdge(params, eds));
  }, [setEdges]);

  // Add step node
  const handleAddStep = useCallback((type: StepNodeType) => {
    nodeCounter.current += 1;
    const id = `step-${nodeCounter.current}`;
    const xBase = 180 + (nodeCounter.current - 1) * 210;
    const yBase = 80 + ((nodeCounter.current - 1) % 2) * 100;

    const newNode = createStepNode(
      type, id,
      { x: xBase, y: yBase },
      (nid) => setConfigNodeId(nid),
      (nid) => {
        setNodes(ns => ns.filter(n => n.id !== nid));
        setEdges(es => es.filter(e => e.source !== nid && e.target !== nid));
        setNodeConfigs(prev => { const next = { ...prev }; delete next[nid]; return next; });
      },
    );

    const defaultConfig: StepConfig = {
      label: STEP_DEFAULTS[type].label,
      stepType: type,
      taskDescription: '',
      systemInstructions: '',
      model: type === 'cloud-model' ? 'claude-sonnet-4-6' : 'llama3.2:3b',
      engine: type === 'cloud-model' ? 'anthropic' : 'ollama',
      temperature: 0.7,
      maxTokens: 4096,
      runtime: type === 'executable' ? 'python' : type === 'mcp-server' ? 'python' : undefined,
      scriptContent: type === 'executable' ? '# Your script here\nprint("Hello from step")' : undefined,
      outputFormat: type === 'action' ? 'markdown' : undefined,
      outputFilename: type === 'action' ? 'output.md' : undefined,
    };

    setNodes(ns => [...ns, newNode]);
    setNodeConfigs(prev => ({ ...prev, [id]: defaultConfig }));
  }, [setNodes, setEdges]);

  // Save step config
  const handleSaveConfig = useCallback((nodeId: string, config: StepConfig) => {
    setNodeConfigs(prev => ({ ...prev, [nodeId]: config }));
    setNodes(ns => ns.map(n => n.id === nodeId ? {
      ...n,
      data: {
        ...n.data,
        label: config.label,
        envLabel: (config.stepType === 'executable' || config.stepType === 'mcp-server')
          ? (config.runtime === 'typescript' ? 'ts_env' : 'py_env')
          : undefined,
      },
    } : n));
  }, [setNodes]);

  // File upload
  const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;
    for (const file of Array.from(files)) {
      try {
        const fd = new FormData();
        fd.append('file', file);
        const res = await fetch('/api/orchestrator/upload', { method: 'POST', body: fd });
        if (res.ok) {
          const data = await res.json();
          setAttachments(prev => [...prev, { name: file.name, url: data.url || data.fileUrl || '' }]);
        }
      } catch { /* ignore */ }
    }
    e.target.value = '';
  };

  // Save workflow
  const handleSave = async () => {
    if (!name.trim()) return;
    setSaving(true);
    try {
      const stepNodes = nodes.filter(n => n.id !== 'start');
      const stepConfigs = stepNodes.map((n, i) => {
        const cfg = nodeConfigs[n.id];
        return {
          order: i + 1,
          name: cfg?.label || (n.data as unknown as StepNodeData).label,
          expertId: cfg?.expertId || null,
          taskDescription: cfg?.taskDescription || '',
          systemInstructions: cfg?.systemInstructions || '',
          modelSource: cfg?.stepType === 'cloud-model' ? 'provider' : 'local',
          localModelConfig: { engine: cfg?.engine || 'ollama', modelName: cfg?.model || 'llama3.2:3b' },
          connectionType: 'sequential',
          stepType: cfg?.stepType === 'agent' || cfg?.stepType === 'cloud-model' ? 'agent' : 'action',
          actionConfig: cfg?.stepType === 'executable' ? {
            transformerType: 'executable', executionRuntime: cfg.runtime || 'python', outputFormat: 'markdown',
          } : cfg?.stepType === 'mcp-server' ? {
            transformerType: 'mcp', mcpServerId: cfg.mcpServerId || '', outputFormat: 'markdown',
          } : cfg?.stepType === 'action' ? {
            transformerType: 'none', outputFormat: cfg.outputFormat || 'markdown', outputFilename: cfg.outputFilename || 'output.md',
          } : undefined,
          temperature: cfg?.temperature ?? 0.7,
          maxTokens: cfg?.maxTokens ?? 4096,
        };
      });

      const body = {
        ...(workflowId ? { id: workflowId } : {}),
        name: name.trim(),
        description: description.trim(),
        goalStatement: prompt.trim(),
        inputFileUrls: attachments.map(a => a.url),
        steps: stepConfigs,
        metadata: {
          masterAgent: masterAgent ? { expertId: masterAgent.expertId, name: masterAgent.name, role: masterAgent.role, model: masterAgent.model } : null,
          graphNodes: nodes.map(n => ({ id: n.id, position: n.position, data: { label: (n.data as unknown as StepNodeData).label, stepType: (n.data as unknown as StepNodeData).stepType } })),
          graphEdges: edges.map(e => ({ source: e.source, target: e.target })),
        },
      };

      const res = await fetch('/api/workflows', {
        method: workflowId ? 'PATCH' : 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify(body),
      });
      if (res.ok) {
        // Save config + plan to disk with versioning
        fetch('/api/workflows/save-config', {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ workflowName: name.trim(), config: body, maxVersions: 3 }),
        }).catch(() => {}); // non-blocking
        router.push('/workflow');
      }
    } catch (err) {
      console.error('Save failed:', err);
    } finally {
      setSaving(false);
    }
  };

  const configForDrawer = configNodeId ? nodeConfigs[configNodeId] ?? null : null;
  const promptPreview = prompt.trim()
    ? prompt.slice(0, 120).replace(/\n/g, ' ') + (prompt.length > 120 ? '...' : '')
    : 'Click to add workflow prompt, context, and attachments...';

  return (
    <div style={{ padding: '12px 20px', maxWidth: 1400, margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100vh' }}>
      {/* Row 1: Back + Editable name + Save button */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 10, marginBottom: 8, flexShrink: 0 }}>
        <button onClick={() => router.push('/workflow')} style={{
          display: 'flex', alignItems: 'center', justifyContent: 'center',
          width: 30, height: 30, borderRadius: 7, border: '1px solid var(--border)',
          background: 'transparent', cursor: 'pointer', color: 'var(--text-3)', flexShrink: 0,
        }}>
          <ArrowLeft size={14} />
        </button>
        <WorkflowIcon size={16} color={ACCENT} style={{ flexShrink: 0 }} />
        <input
          value={name}
          onChange={e => setName(e.target.value)}
          style={{
            width: 280, padding: '6px 10px', borderRadius: 7,
            border: '1px solid transparent', background: 'transparent',
            fontSize: 16, fontWeight: 700, color: 'var(--text-1)', outline: 'none',
          }}
          onFocus={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.background = 'var(--bg-surface)'; }}
          onBlur={e => { e.currentTarget.style.borderColor = 'transparent'; e.currentTarget.style.background = 'transparent'; }}
        />
        <button onClick={handleSave} disabled={saving || !name.trim()} style={{
          display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0, marginLeft: 'auto',
          padding: '6px 14px', borderRadius: 7, fontSize: 11, fontWeight: 700,
          border: `1.5px solid ${ACCENT}`, background: ACCENT,
          color: '#fff', cursor: saving ? 'wait' : 'pointer',
          opacity: (!name.trim() || saving) ? 0.5 : 1,
        }}>
          {saving ? <Loader2 size={11} className="spin" /> : <Save size={11} />}
          Save
        </button>
      </div>

      {/* Row 2: Description + Prompt (left) | Master Agent (right, spans both) */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 280px', gap: 8, marginBottom: 8, flexShrink: 0 }}>
        {/* Left: description + prompt stacked */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          <input
            value={description}
            onChange={e => setDescription(e.target.value)}
            placeholder="One-line description..."
            style={{
              padding: '5px 10px', borderRadius: 6,
              border: '1px solid var(--border)', background: 'var(--bg-surface)',
              fontSize: 10, color: 'var(--text-2)', outline: 'none',
            }}
          />
          <button
            onClick={() => setShowPromptDialog(true)}
            style={{
              display: 'flex', alignItems: 'center', gap: 8,
              padding: '6px 10px', borderRadius: 6,
              border: '1px solid #333', background: '#1e1e1e',
              cursor: 'pointer', textAlign: 'left', transition: 'all 0.12s',
            }}
            onMouseEnter={e => { e.currentTarget.style.borderColor = `${ACCENT}60`; }}
            onMouseLeave={e => { e.currentTarget.style.borderColor = '#333'; }}
          >
            <FileText size={11} color="#808080" style={{ flexShrink: 0 }} />
            <div style={{
              flex: 1, fontSize: 10, color: prompt.trim() ? '#d4d4d4' : '#606060',
              overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis',
              fontFamily: 'monospace', fontStyle: prompt.trim() ? 'normal' : 'italic',
            }}>
              {promptPreview}
            </div>
            {attachments.length > 0 && (
              <span style={{
                padding: '1px 6px', borderRadius: 3, fontSize: 8, fontWeight: 700,
                background: `${ACCENT}20`, color: ACCENT, flexShrink: 0,
              }}>
                {attachments.length} file{attachments.length > 1 ? 's' : ''}
              </span>
            )}
          </button>
        </div>

        {/* Right: Master Agent (spans full height) */}
        <MasterAgentPanel
          masterAgent={masterAgent}
          onAttach={() => setShowExpertPicker(true)}
          onDetach={() => setMasterAgent(null)}
        />
      </div>

      {/* ReactFlow — fits window */}
      <div style={{ height: 'calc(100vh - 200px)', maxHeight: 700 }}>
        <StepFlowEditor
          onAddStep={handleAddStep}
          onConfigureNode={setConfigNodeId}
          onDeleteNode={(nid) => {
            setNodes(ns => ns.filter(n => n.id !== nid));
            setEdges(es => es.filter(e => e.source !== nid && e.target !== nid));
            setNodeConfigs(prev => { const next = { ...prev }; delete next[nid]; return next; });
          }}
          nodes={nodes}
          edges={edges}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
        />
      </div>

      {/* Hidden file input */}
      <input ref={fileRef} type="file" multiple style={{ display: 'none' }} onChange={handleFileUpload} />

      {/* Prompt Dialog */}
      <PromptDialog
        open={showPromptDialog}
        onClose={() => setShowPromptDialog(false)}
        prompt={prompt}
        onSave={setPrompt}
        attachments={attachments}
        onAttach={() => fileRef.current?.click()}
        onRemoveAttachment={(i) => setAttachments(prev => prev.filter((_, j) => j !== i))}
      />

      {/* Step Config Drawer + Backdrop */}
      {configNodeId && (
        <div
          onClick={() => setConfigNodeId(null)}
          style={{ position: 'fixed', inset: 0, zIndex: 790, background: 'rgba(0,0,0,0.15)' }}
        />
      )}
      <StepConfigDrawer
        open={!!configNodeId}
        nodeId={configNodeId}
        config={configForDrawer}
        onClose={() => setConfigNodeId(null)}
        onSave={handleSaveConfig}
      />

      {/* Expert Picker */}
      {showExpertPicker && (
        <ExpertPickerModal
          open={showExpertPicker}
          onClose={() => setShowExpertPicker(false)}
          onSelect={(expert) => {
            setMasterAgent({
              expertId: expert.id,
              name: expert.name,
              role: expert.role,
              description: expert.description,
              model: expert.modelName,
            });
          }}
        />
      )}
    </div>
  );
}

export default function WorkflowBuilderPage() {
  return (
    <Suspense fallback={
      <div style={{ padding: 120, textAlign: 'center' }}>
        <Loader2 size={20} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite' }} color="var(--text-3)" />
        <div style={{ fontSize: 13, color: 'var(--text-3)' }}>Loading...</div>
      </div>
    }>
      <ReactFlowProvider>
        <WorkflowBuilderInner />
      </ReactFlowProvider>
    </Suspense>
  );
}
