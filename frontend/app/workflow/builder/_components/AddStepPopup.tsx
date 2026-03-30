'use client';

import { motion, AnimatePresence } from 'framer-motion';
import type { StepNodeType } from './nodes/BaseStepNode';

const STEP_OPTIONS: Array<{ type: StepNodeType; label: string; icon: string; color: string; note: string }> = [
  { type: 'agent',        label: 'Agent',        icon: '🤖', color: '#D97706', note: 'LLM agent with task & prompt' },
  { type: 'mcp-server',   label: 'MCP Server',   icon: '🔌', color: '#2563eb', note: 'Runs in Docker py_env / ts_env' },
  { type: 'executable',   label: 'Executable',   icon: '⚡', color: '#10b981', note: 'Python/TypeScript in Docker' },
  { type: 'action',       label: 'Action',       icon: '📄', color: '#8b5cf6', note: 'Generate Markdown or PDF' },
  { type: 'integration',  label: 'Integration',  icon: '🔗', color: '#06b6d4', note: 'Connect external service' },
  { type: 'cloud-model',  label: 'Cloud Model',  icon: '☁️', color: '#6366f1', note: 'Anthropic, OpenAI, Google' },
  { type: 'transformer',  label: 'Transformer',  icon: '🔄', color: '#f43f5e', note: 'HuggingFace NLP/vision/audio tasks' },
  { type: 'model',        label: 'Model',        icon: '🧠', color: '#8b5cf6', note: 'Direct inference local/cloud' },
  { type: 'plugin',       label: 'Plugin',       icon: '🧩', color: '#ec4899', note: 'Use installed plugin' },
];

interface AddStepPopupProps {
  open: boolean;
  onClose: () => void;
  onSelect: (type: StepNodeType) => void;
}

export default function AddStepPopup({ open, onClose, onSelect }: AddStepPopupProps) {
  if (!open) return null;

  return (
    <AnimatePresence>
      {open && (
        <motion.div
          initial={{ opacity: 0, y: -6, scale: 0.97 }}
          animate={{ opacity: 1, y: 0, scale: 1 }}
          exit={{ opacity: 0, y: -4, scale: 0.97 }}
          transition={{ duration: 0.14 }}
          style={{
            position: 'absolute', top: 44, right: 0, zIndex: 50,
            background: 'var(--bg-surface)', border: '1px solid var(--border)',
            borderRadius: 10, padding: 6, width: 240,
            boxShadow: '0 8px 24px rgba(0,0,0,0.12)',
          }}
        >
          {STEP_OPTIONS.map(opt => (
            <button
              key={opt.type}
              onClick={() => { onSelect(opt.type); onClose(); }}
              style={{
                display: 'flex', alignItems: 'center', gap: 10, width: '100%',
                padding: '8px 10px', borderRadius: 7, cursor: 'pointer',
                border: 'none', background: 'transparent',
                textAlign: 'left', transition: 'all 0.12s',
              }}
              onMouseEnter={e => { e.currentTarget.style.background = `${opt.color}0a`; }}
              onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; }}
            >
              <span style={{ fontSize: 16, lineHeight: 1, flexShrink: 0 }}>{opt.icon}</span>
              <div style={{ flex: 1, minWidth: 0 }}>
                <div style={{ fontSize: 12, fontWeight: 700, color: opt.color }}>{opt.label}</div>
                <div style={{ fontSize: 9, color: 'var(--text-4)', lineHeight: 1.3, marginTop: 1 }}>{opt.note}</div>
              </div>
            </button>
          ))}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

export { STEP_OPTIONS };
