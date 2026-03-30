'use client';

import { useState, useCallback, Suspense, useRef, useEffect } from 'react';
import { useSearchParams, useRouter } from 'next/navigation';
import { motion, AnimatePresence } from 'framer-motion';
import dynamic from 'next/dynamic';
import {
  Loader2, Save, Paperclip, X, Mic, Eye, EyeOff, Calendar, Tag,
  Workflow as WorkflowIcon, ArrowLeft, FileText, ChevronDown, ChevronUp,
  Zap, Cpu, HardDrive, Brain, Lock,
} from 'lucide-react';
import {
  useNodesState, useEdgesState, addEdge,
  type Connection, type Edge, type Node, ReactFlowProvider,
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
  open: boolean; onClose: () => void;
  onSelect: (expert: { id: string; name: string; role: string; description?: string; modelName?: string }) => void;
}) {
  const [experts, setExperts] = useState<Array<Record<string, unknown>>>([]);
  const [loading, setLoading] = useState(false);
  const [search, setSearch] = useState('');

  useEffect(() => {
    if (!open) return;
    setLoading(true);
    fetch('/api/experts').then(r => r.json()).then(d => setExperts(d.experts ?? [])).catch(() => {}).finally(() => setLoading(false));
  }, [open]);

  if (!open) return null;
  const filtered = search ? experts.filter(e => ((e.name as string) ?? '').toLowerCase().includes(search.toLowerCase())) : experts;

  return (
    <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }}
      style={{ position: 'fixed', inset: 0, zIndex: 1000, display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)' }}
      onClick={onClose}>
      <motion.div initial={{ opacity: 0, scale: 0.96, y: 20 }} animate={{ opacity: 1, scale: 1, y: 0 }}
        onClick={e => e.stopPropagation()}
        style={{ background: 'var(--bg-surface)', border: '1px solid var(--border)', borderRadius: 14, width: 440, maxHeight: '65vh', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
        <div style={{ padding: '14px 18px', borderBottom: '1px solid var(--border)', display: 'flex', alignItems: 'center', justifyContent: 'space-between' }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: 'var(--text-1)' }}>Select Agent</span>
          <button onClick={onClose} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: 'var(--text-3)' }}><X size={14} /></button>
        </div>
        <div style={{ padding: '8px 18px', borderBottom: '1px solid var(--border)' }}>
          <input value={search} onChange={e => setSearch(e.target.value)} placeholder="Search agents..."
            style={{ width: '100%', padding: '7px 10px', borderRadius: 7, border: '1px solid var(--border)', background: 'var(--bg-elevated)', fontSize: 11, color: 'var(--text-1)', outline: 'none' }} />
        </div>
        <div style={{ flex: 1, overflow: 'auto', padding: '8px 18px' }}>
          {loading && <div style={{ textAlign: 'center', padding: 16, color: 'var(--text-4)' }}><Loader2 size={14} className="spin" /></div>}
          {filtered.map(e => (
            <button key={e.id as string}
              onClick={() => { onSelect({ id: e.id as string, name: e.name as string, role: e.role as string, description: e.description as string, modelName: e.modelName as string }); onClose(); }}
              style={{ display: 'flex', alignItems: 'center', gap: 8, width: '100%', padding: '8px 10px', borderRadius: 7, cursor: 'pointer', border: '1px solid var(--border)', background: 'var(--bg-elevated)', marginBottom: 4, textAlign: 'left', transition: 'all 0.12s' }}
              onMouseEnter={e => { e.currentTarget.style.borderColor = `${ACCENT}60`; }}
              onMouseLeave={e => { e.currentTarget.style.borderColor = 'var(--border)'; }}>
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

/* ── Prompt Dialog (with description) ─────────────── */
function PromptDialog({ open, onClose, prompt, description, onSave, attachments, onAttach, onRemoveAttachment }: {
  open: boolean; onClose: () => void;
  prompt: string; description: string;
  onSave: (prompt: string, description: string) => void;
  attachments: Array<{ name: string; url: string }>; onAttach: () => void; onRemoveAttachment: (i: number) => void;
}) {
  const [value, setValue] = useState(prompt);
  const [desc, setDesc] = useState(description);
  const [preview, setPreview] = useState(false);

  useEffect(() => { if (open) { setValue(prompt); setDesc(description); setPreview(false); } }, [open, prompt, description]);

  return (
    <AnimatePresence>
      {open && (
        <motion.div initial={{ opacity: 0 }} animate={{ opacity: 1 }} exit={{ opacity: 0 }}
          style={{ position: 'fixed', inset: 0, zIndex: 1000, display: 'flex', alignItems: 'center', justifyContent: 'center', background: 'rgba(0,0,0,0.5)', backdropFilter: 'blur(4px)' }}
          onClick={onClose}>
          <motion.div initial={{ opacity: 0, scale: 0.96, y: 20 }} animate={{ opacity: 1, scale: 1, y: 0 }} exit={{ opacity: 0, scale: 0.96, y: 20 }}
            transition={{ type: 'spring', stiffness: 400, damping: 30 }}
            onClick={e => e.stopPropagation()}
            style={{ background: 'var(--bg-surface)', border: '1px solid var(--border)', borderRadius: 14, width: '90vw', maxWidth: 700, maxHeight: '80vh', display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            {/* Header */}
            <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: '14px 20px', borderBottom: '1px solid var(--border)' }}>
              <span style={{ fontSize: 14, fontWeight: 700, color: 'var(--text-1)' }}>Workflow Prompt & Description</span>
              <div style={{ display: 'flex', gap: 6 }}>
                <button onClick={onAttach} style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '5px 10px', borderRadius: 6, fontSize: 11, fontWeight: 500, border: '1px solid var(--border)', background: 'transparent', color: 'var(--text-3)', cursor: 'pointer' }}>
                  <Paperclip size={11} /> Attach
                </button>
                <button disabled title="Available in Kortecx Cloud" style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '5px 10px', borderRadius: 6, fontSize: 11, fontWeight: 500, border: '1px solid var(--border)', background: 'transparent', color: 'var(--text-4)', cursor: 'not-allowed', opacity: 0.5 }}>
                  <Mic size={11} /> Voice
                </button>
                <button onClick={() => setPreview(p => !p)} style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '5px 10px', borderRadius: 6, fontSize: 11, fontWeight: 500, border: preview ? `1px solid ${ACCENT}` : '1px solid var(--border)', background: preview ? `${ACCENT}10` : 'transparent', color: preview ? ACCENT : 'var(--text-3)', cursor: 'pointer' }}>
                  {preview ? <EyeOff size={11} /> : <Eye size={11} />} {preview ? 'Edit' : 'Preview'}
                </button>
                <button onClick={onClose} style={{ width: 28, height: 28, borderRadius: 7, border: '1px solid var(--border)', background: 'transparent', cursor: 'pointer', color: 'var(--text-3)', display: 'flex', alignItems: 'center', justifyContent: 'center' }}>
                  <X size={13} />
                </button>
              </div>
            </div>

            {/* Description field */}
            <div style={{ padding: '10px 20px', borderBottom: '1px solid var(--border)' }}>
              <input value={desc} onChange={e => setDesc(e.target.value)} placeholder="One-line workflow description..."
                style={{ width: '100%', padding: '7px 10px', borderRadius: 7, border: '1px solid var(--border)', background: 'var(--bg-elevated)', fontSize: 12, color: 'var(--text-1)', outline: 'none' }} />
            </div>

            {/* Attachments */}
            {attachments.length > 0 && (
              <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap', padding: '8px 20px', borderBottom: '1px solid var(--border)' }}>
                {attachments.map((a, i) => (
                  <div key={i} style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '3px 8px', borderRadius: 5, background: `${ACCENT}10`, border: `1px solid ${ACCENT}30`, fontSize: 10, color: ACCENT, fontWeight: 500 }}>
                    {a.name}
                    <button onClick={() => onRemoveAttachment(i)} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: ACCENT, padding: 0 }}><X size={9} /></button>
                  </div>
                ))}
              </div>
            )}

            {/* Monaco or Preview */}
            {!preview ? (
              <div style={{ height: 400 }}>
                <MonacoEditor height={400} language="markdown" value={value} onChange={v => setValue(v ?? '')} theme="vs-dark" options={MONO_OPTIONS} />
              </div>
            ) : (
              <div style={{ height: 400, overflow: 'auto', padding: 20, fontSize: 13, color: 'var(--text-2)', lineHeight: 1.7, whiteSpace: 'pre-wrap', wordBreak: 'break-word' }}>
                {value || <span style={{ color: 'var(--text-4)', fontStyle: 'italic' }}>No prompt written yet.</span>}
              </div>
            )}

            {/* Footer */}
            <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8, padding: '10px 20px', borderTop: '1px solid var(--border)' }}>
              <button onClick={onClose} style={{ padding: '6px 14px', borderRadius: 7, fontSize: 11, fontWeight: 500, border: '1px solid var(--border)', background: 'transparent', color: 'var(--text-3)', cursor: 'pointer' }}>Cancel</button>
              <button onClick={() => { onSave(value, desc); onClose(); }} style={{ padding: '6px 16px', borderRadius: 7, fontSize: 11, fontWeight: 700, border: `1.5px solid ${ACCENT}`, background: ACCENT, color: '#fff', cursor: 'pointer' }}>Save</button>
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
  const [tags, setTags] = useState<string[]>([]);
  const [tagInput, setTagInput] = useState('');
  const [showPromptDialog, setShowPromptDialog] = useState(false);
  const [showAdvanced, setShowAdvanced] = useState(false);
  const [attachments, setAttachments] = useState<Array<{ name: string; url: string }>>([]);
  const [saving, setSaving] = useState(false);
  const fileRef = useRef<HTMLInputElement>(null);

  // Master agent + connected agents
  const [masterAgent, setMasterAgent] = useState<MasterAgent | null>(null);
  const [connectedAgents, setConnectedAgents] = useState<MasterAgent[]>([]);
  const [showExpertPicker, setShowExpertPicker] = useState(false);
  const [expertPickerTarget, setExpertPickerTarget] = useState<'master' | 'connected'>('master');

  // ReactFlow state
  const [nodes, setNodes, onNodesChange] = useNodesState<Node>([createStartNode()]);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>([]);

  // Load existing workflow when editing
  useEffect(() => {
    if (!workflowId) return;
    (async () => {
      try {
        const res = await fetch(`/api/workflows?id=${workflowId}`);
        if (!res.ok) return;
        const data = await res.json();
        const wf = data.workflow ?? data;
        if (!wf) return;
        setName(wf.name ?? '');
        setDescription(wf.description ?? '');
        setPrompt(wf.goalStatement ?? '');
        setTags(wf.tags ?? []);
        if (wf.inputFileUrls?.length) {
          setAttachments(wf.inputFileUrls.map((u: string) => ({ name: u.split('/').pop() || 'file', url: u })));
        }
        // Restore master agent + connected agents from metadata
        const meta = wf.metadata ?? {};
        if (meta.masterAgent) {
          setMasterAgent(meta.masterAgent);
        }
        if (meta.connectedAgents?.length) {
          setConnectedAgents(meta.connectedAgents);
        }
        // Restore graph nodes + edges from metadata
        if (meta.graphNodes?.length) {
          const restored: Node[] = [createStartNode()];
          let maxCounter = 0;
          for (const gn of meta.graphNodes) {
            if (gn.id === 'start') continue;
            const stepType = gn.data?.stepType || 'agent';
            const defaults = STEP_DEFAULTS[stepType as StepNodeType] || STEP_DEFAULTS.agent;
            restored.push({
              id: gn.id,
              type: 'stepNode',
              position: gn.position ?? { x: 200, y: 100 },
              data: {
                label: gn.data?.label || defaults.label,
                stepType: stepType,
                icon: defaults.icon,
                color: defaults.color,
                status: 'idle',
                config: {},
                onConfigure: (nid: string) => setConfigNodeId(nid),
                onDelete: (nid: string) => {
                  setNodes(ns => ns.filter(n => n.id !== nid));
                  setEdges(es => es.filter(e => e.source !== nid && e.target !== nid));
                  setNodeConfigs(prev => { const next = { ...prev }; delete next[nid]; return next; });
                },
              },
            });
            const num = parseInt(gn.id.replace('step-', ''), 10);
            if (!isNaN(num) && num > maxCounter) maxCounter = num;
          }
          nodeCounter.current = maxCounter;
          setNodes(restored);
        }
        if (meta.graphEdges?.length) {
          setEdges(meta.graphEdges.map((ge: { source: string; target: string }) => ({
            id: `e-${ge.source}-${ge.target}`,
            source: ge.source,
            target: ge.target,
          })));
        }
      } catch { /* ignore */ }
    })();
  }, [workflowId]); // eslint-disable-line react-hooks/exhaustive-deps

  // Step config drawer
  const [configNodeId, setConfigNodeId] = useState<string | null>(null);
  const [nodeConfigs, setNodeConfigs] = useState<Record<string, StepConfig>>({});
  const nodeCounter = useRef(0);

  const onConnect = useCallback((params: Connection) => { setEdges(eds => addEdge(params, eds)); }, [setEdges]);

  const handleAddStep = useCallback((type: StepNodeType) => {
    nodeCounter.current += 1;
    const id = `step-${nodeCounter.current}`;
    const xBase = 180 + (nodeCounter.current - 1) * 210;
    const yBase = 80 + ((nodeCounter.current - 1) % 2) * 100;
    const newNode = createStepNode(type, id, { x: xBase, y: yBase }, (nid) => setConfigNodeId(nid), (nid) => {
      setNodes(ns => ns.filter(n => n.id !== nid));
      setEdges(es => es.filter(e => e.source !== nid && e.target !== nid));
      setNodeConfigs(prev => { const next = { ...prev }; delete next[nid]; return next; });
    });
    const defaultConfig: StepConfig = {
      label: STEP_DEFAULTS[type].label, stepType: type, taskDescription: '', systemInstructions: '',
      model: type === 'cloud-model' ? 'claude-sonnet-4-6' : 'llama3.2:3b',
      engine: type === 'cloud-model' ? 'anthropic' : 'ollama', temperature: 0.7, maxTokens: 4096,
      runtime: type === 'executable' ? 'python' : type === 'mcp-server' ? 'python' : undefined,
      scriptContent: type === 'executable' ? '# Your script here\nprint("Hello from step")' : undefined,
      outputFormat: type === 'action' ? 'markdown' : undefined,
      outputFilename: type === 'action' ? 'output.md' : undefined,
    };
    setNodes(ns => [...ns, newNode]);
    setNodeConfigs(prev => ({ ...prev, [id]: defaultConfig }));
  }, [setNodes, setEdges]);

  const handleSaveConfig = useCallback((nodeId: string, config: StepConfig) => {
    setNodeConfigs(prev => ({ ...prev, [nodeId]: config }));
    setNodes(ns => ns.map(n => n.id === nodeId ? { ...n, data: { ...n.data, label: config.label, envLabel: (config.stepType === 'executable' || config.stepType === 'mcp-server') ? (config.runtime === 'typescript' ? 'ts_env' : 'py_env') : undefined } } : n));
  }, [setNodes]);

  const handleFileUpload = async (e: React.ChangeEvent<HTMLInputElement>) => {
    const files = e.target.files;
    if (!files || files.length === 0) return;
    for (const file of Array.from(files)) {
      try {
        const fd = new FormData(); fd.append('file', file);
        const res = await fetch('/api/orchestrator/upload', { method: 'POST', body: fd });
        if (res.ok) { const data = await res.json(); setAttachments(prev => [...prev, { name: file.name, url: data.url || data.fileUrl || '' }]); }
      } catch { /* ignore */ }
    }
    e.target.value = '';
  };

  const handleSave = async () => {
    if (!name.trim()) return;
    setSaving(true);
    try {
      const stepNodes = nodes.filter(n => n.id !== 'start');
      const stepConfigs = stepNodes.map((n, i) => {
        const cfg = nodeConfigs[n.id];
        return {
          order: i + 1, name: cfg?.label || (n.data as unknown as StepNodeData).label,
          expertId: cfg?.expertId || null, taskDescription: cfg?.taskDescription || '',
          systemInstructions: cfg?.systemInstructions || '',
          modelSource: cfg?.stepType === 'cloud-model' ? 'provider' : 'local',
          localModelConfig: { engine: cfg?.engine || 'ollama', modelName: cfg?.model || 'llama3.2:3b' },
          connectionType: 'sequential',
          stepType: cfg?.stepType === 'agent' || cfg?.stepType === 'cloud-model' ? 'agent' : 'action',
          actionConfig: cfg?.stepType === 'executable' ? { transformerType: 'executable', executionRuntime: cfg.runtime || 'python', outputFormat: 'markdown' }
            : cfg?.stepType === 'mcp-server' ? { transformerType: 'mcp', mcpServerId: cfg.mcpServerId || '', outputFormat: 'markdown' }
            : cfg?.stepType === 'action' ? { transformerType: 'none', outputFormat: cfg.outputFormat || 'markdown', outputFilename: cfg.outputFilename || 'output.md' }
            : undefined,
          temperature: cfg?.temperature ?? 0.7, maxTokens: cfg?.maxTokens ?? 4096,
        };
      });
      const body = {
        ...(workflowId ? { id: workflowId } : {}),
        name: name.trim(), description: description.trim(), goalStatement: prompt.trim(),
        inputFileUrls: attachments.map(a => a.url), tags,
        steps: stepConfigs,
        metadata: {
          masterAgent: masterAgent ? { expertId: masterAgent.expertId, name: masterAgent.name, role: masterAgent.role, model: masterAgent.model } : null,
          connectedAgents: connectedAgents.map(a => ({ expertId: a.expertId, name: a.name, role: a.role, model: a.model })),
          graphNodes: nodes.map(n => ({ id: n.id, position: n.position, data: { label: (n.data as unknown as StepNodeData).label, stepType: (n.data as unknown as StepNodeData).stepType } })),
          graphEdges: edges.map(e => ({ source: e.source, target: e.target })),
        },
      };
      const res = await fetch('/api/workflows', { method: workflowId ? 'PATCH' : 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
      if (res.ok) {
        fetch('/api/workflows/save-config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify({ workflowName: name.trim(), config: body, maxVersions: 3 }) }).catch(() => {});
        router.push('/workflow');
      }
    } catch (err) { console.error('Save failed:', err); } finally { setSaving(false); }
  };

  const configForDrawer = configNodeId ? nodeConfigs[configNodeId] ?? null : null;
  const promptPreview = prompt.trim() ? prompt.slice(0, 100).replace(/\n/g, ' ') + (prompt.length > 100 ? '...' : '') : 'Click to add prompt & description...';
  const stepCount = nodes.filter(n => n.id !== 'start').length;

  return (
    <div style={{ padding: '10px 20px', maxWidth: 1400, margin: '0 auto', display: 'flex', flexDirection: 'column', height: '100vh' }}>
      {/* Row 1: Back + Name + Schedule + Save */}
      <div style={{ display: 'flex', alignItems: 'center', gap: 8, marginBottom: 6, flexShrink: 0 }}>
        <button onClick={() => router.push('/workflow')} style={{ display: 'flex', alignItems: 'center', justifyContent: 'center', width: 28, height: 28, borderRadius: 6, border: '1px solid var(--border)', background: 'transparent', cursor: 'pointer', color: 'var(--text-3)', flexShrink: 0 }}>
          <ArrowLeft size={13} />
        </button>
        <WorkflowIcon size={14} color={ACCENT} style={{ flexShrink: 0 }} />
        <input value={name} onChange={e => setName(e.target.value)}
          style={{ width: 260, padding: '5px 8px', borderRadius: 6, border: '1px solid transparent', background: 'transparent', fontSize: 15, fontWeight: 700, color: 'var(--text-1)', outline: 'none' }}
          onFocus={e => { e.currentTarget.style.borderColor = 'var(--border)'; e.currentTarget.style.background = 'var(--bg-surface)'; }}
          onBlur={e => { e.currentTarget.style.borderColor = 'transparent'; e.currentTarget.style.background = 'transparent'; }}
        />
        <div style={{ marginLeft: 'auto', display: 'flex', gap: 6 }}>
          <button disabled title="Coming soon" style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '5px 12px', borderRadius: 6, fontSize: 10, fontWeight: 600, border: '1px solid var(--border)', background: 'transparent', color: 'var(--text-4)', cursor: 'not-allowed', opacity: 0.5 }}>
            <Calendar size={10} /> Schedule
          </button>
          <button onClick={handleSave} disabled={saving || !name.trim()} style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '5px 12px', borderRadius: 6, fontSize: 10, fontWeight: 700, border: `1.5px solid ${ACCENT}`, background: ACCENT, color: '#fff', cursor: saving ? 'wait' : 'pointer', opacity: (!name.trim() || saving) ? 0.5 : 1 }}>
            {saving ? <Loader2 size={10} className="spin" /> : <Save size={10} />} Save
          </button>
        </div>
      </div>

      {/* Row 2: Left (prompt + tags + advanced) | Right (master agent + metrics + inference) */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 6, marginBottom: 4, flexShrink: 0 }}>
        {/* Left column */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>
          {/* Prompt preview */}
          <button onClick={() => setShowPromptDialog(true)} style={{ display: 'flex', alignItems: 'center', gap: 8, padding: '8px 14px', borderRadius: 8, border: '1px solid #333', background: '#1e1e1e', cursor: 'pointer', textAlign: 'left', transition: 'all 0.12s' }}
            onMouseEnter={e => { e.currentTarget.style.borderColor = `${ACCENT}60`; }}
            onMouseLeave={e => { e.currentTarget.style.borderColor = '#333'; }}>
            <FileText size={13} color="#808080" style={{ flexShrink: 0 }} />
            <div style={{ flex: 1, fontSize: 12, color: prompt.trim() ? '#d4d4d4' : '#606060', overflow: 'hidden', whiteSpace: 'nowrap', textOverflow: 'ellipsis', fontFamily: 'monospace', fontStyle: prompt.trim() ? 'normal' : 'italic' }}>
              {promptPreview}
            </div>
            {attachments.length > 0 && <span style={{ padding: '2px 7px', borderRadius: 4, fontSize: 10, fontWeight: 700, background: `${ACCENT}20`, color: ACCENT, flexShrink: 0 }}>{attachments.length} file{attachments.length > 1 ? 's' : ''}</span>}
          </button>

          {/* Tags */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 5, flexWrap: 'wrap', padding: '6px 12px', borderRadius: 8, border: '1px solid var(--border)', background: 'var(--bg-surface)', minHeight: 30 }}>
            <Tag size={12} color="var(--text-4)" />
            {tags.map((t, i) => (
              <span key={t} style={{ display: 'flex', alignItems: 'center', gap: 4, padding: '2px 8px', borderRadius: 5, fontSize: 11, fontWeight: 500, background: `${ACCENT}12`, color: ACCENT, border: `1px solid ${ACCENT}25` }}>
                {t}
                <button onClick={() => setTags(prev => prev.filter((_, j) => j !== i))} style={{ background: 'transparent', border: 'none', cursor: 'pointer', color: ACCENT, padding: 0, lineHeight: 1 }}><X size={9} /></button>
              </span>
            ))}
            <input value={tagInput} onChange={e => setTagInput(e.target.value)} placeholder={tags.length === 0 ? 'Add tags...' : ''}
              onKeyDown={e => { if ((e.key === 'Enter' || e.key === ' ') && tagInput.trim()) { e.preventDefault(); setTags(prev => [...prev, tagInput.trim()]); setTagInput(''); } if (e.key === 'Backspace' && !tagInput && tags.length > 0) setTags(prev => prev.slice(0, -1)); }}
              style={{ border: 'none', outline: 'none', background: 'transparent', fontSize: 12, color: 'var(--text-2)', flex: 1, minWidth: 60, padding: 0 }} />
          </div>

          {/* Reactive step type counters */}
          <div style={{ display: 'flex', gap: 5, flexWrap: 'wrap' }}>
            {[
              { label: 'Agents', type: 'agent', color: '#D97706' },
              { label: 'MCP', type: 'mcp-server', color: '#2563eb' },
              { label: 'Executables', type: 'executable', color: '#10b981' },
              { label: 'Cloud', type: 'cloud-model', color: '#6366f1' },
              { label: 'Actions', type: 'action', color: '#8b5cf6' },
              { label: 'Integrations', type: 'integration', color: '#06b6d4' },
            ].map(m => {
              const count = Object.values(nodeConfigs).filter(c => c.stepType === m.type).length;
              return (
                <div key={m.type} style={{
                  display: 'flex', alignItems: 'center', gap: 5,
                  padding: '4px 10px', borderRadius: 6,
                  background: count > 0 ? `${m.color}10` : 'var(--bg-surface)',
                  border: `1px solid ${count > 0 ? `${m.color}30` : 'var(--border)'}`,
                }}>
                  <span style={{ fontSize: 12, fontWeight: 700, color: count > 0 ? m.color : 'var(--text-4)' }}>{count}</span>
                  <span style={{ fontSize: 10, color: count > 0 ? m.color : 'var(--text-4)', fontWeight: 500 }}>{m.label}</span>
                </div>
              );
            })}
          </div>

          {/* Advanced Configs */}
          <button onClick={() => setShowAdvanced(a => !a)} style={{ display: 'flex', alignItems: 'center', gap: 6, padding: '6px 12px', borderRadius: 7, border: '1px solid var(--border)', background: 'var(--bg-surface)', cursor: 'pointer', fontSize: 12, fontWeight: 600, color: 'var(--text-3)' }}>
            <Brain size={12} /> Advanced Configs
            {showAdvanced ? <ChevronUp size={12} style={{ marginLeft: 'auto' }} /> : <ChevronDown size={12} style={{ marginLeft: 'auto' }} />}
            <span style={{ fontSize: 9, padding: '2px 6px', borderRadius: 4, background: '#f59e0b18', color: '#f59e0b', fontWeight: 700 }}>SOON</span>
          </button>
          {showAdvanced && (
            <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr 1fr', gap: 6 }}>
              {[
                { icon: HardDrive, label: 'KV Cache', opts: ['Auto', 'Aggr.'] },
                { icon: Zap, label: 'Memory', opts: ['Std', '100x'] },
                { icon: Cpu, label: 'Quant', opts: ['None', 'INT8'] },
                { icon: Brain, label: 'SLM', opts: ['Std', 'Enh.'] },
              ].map(c => (
                <div key={c.label} style={{ padding: '6px 8px', borderRadius: 6, border: '1px solid var(--border)', background: 'var(--bg-elevated)', opacity: 0.5 }}>
                  <div style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
                    <c.icon size={11} color="var(--text-4)" />
                    <span style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-4)' }}>{c.label}</span>
                  </div>
                  <div style={{ display: 'flex', gap: 4, marginTop: 3 }}>
                    {c.opts.map(o => <span key={o} style={{ padding: '2px 6px', borderRadius: 4, fontSize: 9, background: 'var(--bg-surface)', color: 'var(--text-4)', border: '1px solid var(--border)' }}>{o}</span>)}
                  </div>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Right column: Master Agent + Metrics (expanded) + Inference */}
        <div style={{ display: 'flex', flexDirection: 'column', gap: 3 }}>
          <MasterAgentPanel
            masterAgent={masterAgent}
            connectedAgents={connectedAgents}
            onAttach={() => { setExpertPickerTarget('master'); setShowExpertPicker(true); }}
            onDetach={() => { setMasterAgent(null); setConnectedAgents([]); }}
            onAttachConnected={() => { setExpertPickerTarget('connected'); setShowExpertPicker(true); }}
            onDetachConnected={(i) => setConnectedAgents(prev => prev.filter((_, j) => j !== i))}
          />

          {/* Metrics — 2 rows of 3 */}
          <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr 1fr', gap: 3 }}>
            {[
              { label: 'Steps', value: String(stepCount) },
              { label: 'Est. Tokens', value: stepCount > 0 ? `~${stepCount * 4}k` : '—' },
              { label: 'Est. Cost', value: stepCount > 0 ? `$${(stepCount * 0.008).toFixed(3)}` : '—' },
              { label: 'Parallel', value: '—' },
              { label: 'Avg Latency', value: '—' },
              { label: 'Agents', value: String(Object.values(nodeConfigs).filter(c => c.stepType === 'agent').length) },
            ].map(m => (
              <div key={m.label} style={{ padding: '2px 4px', borderRadius: 4, background: 'var(--bg-surface)', border: '1px solid var(--border)', textAlign: 'center' }}>
                <div style={{ fontSize: 10, fontWeight: 700, color: 'var(--text-1)', lineHeight: 1.1 }}>{m.value}</div>
                <div style={{ fontSize: 6, color: 'var(--text-4)', textTransform: 'uppercase', letterSpacing: '0.04em' }}>{m.label}</div>
              </div>
            ))}
          </div>

          {/* Inference — single compact row */}
          <div style={{ display: 'flex', alignItems: 'center', gap: 3, padding: '2px 6px', borderRadius: 4, background: 'var(--bg-surface)', border: '1px solid var(--border)', opacity: 0.5 }}>
            <Cpu size={7} color="var(--text-4)" />
            {['KV:Auto', 'Mem:Std', 'Q:None', 'SLM:Std'].map(s => (
              <span key={s} style={{ padding: '0px 3px', borderRadius: 2, fontSize: 6, background: 'var(--bg-elevated)', color: 'var(--text-4)', border: '1px solid var(--border)' }}>{s}</span>
            ))}
            <Lock size={6} color="var(--text-4)" style={{ marginLeft: 'auto' }} />
          </div>
        </div>
      </div>

      {/* ReactFlow — fills remaining space */}
      <div style={{ flex: 1, minHeight: 300 }}>
        <StepFlowEditor onAddStep={handleAddStep} onConfigureNode={setConfigNodeId}
          onDeleteNode={(nid) => { setNodes(ns => ns.filter(n => n.id !== nid)); setEdges(es => es.filter(e => e.source !== nid && e.target !== nid)); setNodeConfigs(prev => { const next = { ...prev }; delete next[nid]; return next; }); }}
          nodes={nodes} edges={edges} onNodesChange={onNodesChange} onEdgesChange={onEdgesChange} onConnect={onConnect} />
      </div>

      {/* Hidden file input */}
      <input ref={fileRef} type="file" multiple style={{ display: 'none' }} onChange={handleFileUpload} />

      {/* Prompt Dialog */}
      <PromptDialog open={showPromptDialog} onClose={() => setShowPromptDialog(false)}
        prompt={prompt} description={description}
        onSave={(p, d) => { setPrompt(p); setDescription(d); }}
        attachments={attachments} onAttach={() => fileRef.current?.click()}
        onRemoveAttachment={(i) => setAttachments(prev => prev.filter((_, j) => j !== i))} />

      {/* Step Config Drawer + Backdrop */}
      {configNodeId && <div onClick={() => setConfigNodeId(null)} style={{ position: 'fixed', inset: 0, zIndex: 790, background: 'rgba(0,0,0,0.15)' }} />}
      <StepConfigDrawer open={!!configNodeId} nodeId={configNodeId} config={configForDrawer} onClose={() => setConfigNodeId(null)} onSave={handleSaveConfig} />

      {/* Expert Picker */}
      {showExpertPicker && (
        <ExpertPickerModal open={showExpertPicker} onClose={() => setShowExpertPicker(false)}
          onSelect={(expert) => {
            const agentData = { expertId: expert.id, name: expert.name, role: expert.role, description: expert.description, model: expert.modelName };
            if (expertPickerTarget === 'master') setMasterAgent(agentData);
            else setConnectedAgents(prev => prev.length < 4 ? [...prev, agentData] : prev);
          }} />
      )}
    </div>
  );
}

export default function WorkflowBuilderPage() {
  return (
    <Suspense fallback={<div style={{ padding: 120, textAlign: 'center' }}><Loader2 size={20} style={{ margin: '0 auto 8px', animation: 'spin 1s linear infinite' }} color="var(--text-3)" /><div style={{ fontSize: 13, color: 'var(--text-3)' }}>Loading...</div></div>}>
      <ReactFlowProvider><WorkflowBuilderInner /></ReactFlowProvider>
    </Suspense>
  );
}
