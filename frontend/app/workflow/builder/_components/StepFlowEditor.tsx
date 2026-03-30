'use client';

import { useMemo, useState, useRef, useEffect } from 'react';
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  type Connection,
  type Edge,
  type Node,
  BackgroundVariant,
  MarkerType,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import { Plus } from 'lucide-react';
import BaseStepNode, { type StepNodeData, type StepNodeType } from './nodes/BaseStepNode';
import AddStepPopup from './AddStepPopup';

const NODE_TYPES = { stepNode: BaseStepNode };

const STEP_DEFAULTS: Record<StepNodeType, { icon: string; color: string; label: string }> = {
  'start':        { icon: '▶️', color: '#10b981', label: 'Start' },
  'agent':        { icon: '🤖', color: '#D97706', label: 'Agent Step' },
  'mcp-server':   { icon: '🔌', color: '#2563eb', label: 'MCP Server' },
  'executable':   { icon: '⚡', color: '#10b981', label: 'Executable' },
  'action':       { icon: '📄', color: '#8b5cf6', label: 'Action' },
  'integration':  { icon: '🔗', color: '#06b6d4', label: 'Integration' },
  'cloud-model':  { icon: '☁️', color: '#6366f1', label: 'Cloud Model' },
  'master-agent': { icon: '🛡️', color: '#06b6d4', label: 'Master Agent' },
};

interface StepFlowEditorProps {
  onAddStep: (type: StepNodeType) => void;
  onConfigureNode: (nodeId: string) => void;
  onDeleteNode: (nodeId: string) => void;
  nodes: Node[];
  edges: Edge[];
  onNodesChange: (changes: any) => void;
  onEdgesChange: (changes: any) => void;
  onConnect: (connection: Connection) => void;
}

export default function StepFlowEditor({
  onAddStep,
  onConfigureNode,
  onDeleteNode,
  nodes,
  edges,
  onNodesChange,
  onEdgesChange,
  onConnect,
}: StepFlowEditorProps) {
  const [showAddMenu, setShowAddMenu] = useState(false);

  const defaultEdgeOptions = useMemo(() => ({
    animated: true,
    style: { stroke: 'var(--text-4)', strokeWidth: 2 },
    markerEnd: { type: MarkerType.ArrowClosed, color: 'var(--text-4)' },
  }), []);

  return (
    <div style={{
      width: '100%', height: '100%', minHeight: 400, borderRadius: 10,
      border: '1px solid var(--border)', overflow: 'hidden',
      background: 'var(--bg-elevated)', position: 'relative',
    }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={onNodesChange}
        onEdgesChange={onEdgesChange}
        onConnect={onConnect}
        nodeTypes={NODE_TYPES}
        defaultEdgeOptions={defaultEdgeOptions}
        fitView
        proOptions={{ hideAttribution: true }}
        style={{ background: 'transparent' }}
      >
        <Background variant={BackgroundVariant.Lines} gap={20} size={1} color="var(--text-4)" style={{ opacity: 0.08 }} />
        <Controls
          showInteractive={false}
          position="bottom-left"
          style={{
            borderRadius: 8, border: '1px solid var(--border)',
            background: 'var(--bg-surface)', boxShadow: 'none',
          }}
        />
        <MiniMap
          position="bottom-right"
          style={{ background: 'var(--bg-surface)', border: '1px solid var(--border)', borderRadius: 8 }}
          maskColor="rgba(0,0,0,0.08)"
          nodeColor="#06b6d430"
        />
      </ReactFlow>

      {/* Top-right Add Step button + dropdown */}
      <div style={{ position: 'absolute', top: 10, right: 10, zIndex: 10 }}>
        <button
          onClick={() => setShowAddMenu(o => !o)}
          style={{
            display: 'flex', alignItems: 'center', gap: 5,
            padding: '7px 14px', borderRadius: 8, cursor: 'pointer',
            border: '1.5px solid #D97706', background: '#D97706',
            color: '#fff', fontSize: 11, fontWeight: 700,
            boxShadow: '0 2px 8px rgba(217,119,6,0.25)',
          }}
        >
          <Plus size={13} />
          Add Step
        </button>
        <AddStepPopup
          open={showAddMenu}
          onClose={() => setShowAddMenu(false)}
          onSelect={(type) => { onAddStep(type); setShowAddMenu(false); }}
        />
      </div>
    </div>
  );
}

export function createStartNode(): Node {
  return {
    id: 'start',
    type: 'stepNode',
    position: { x: 20, y: 100 },
    draggable: false,
    deletable: false,
    data: {
      label: 'Start',
      stepType: 'start',
      icon: '▶️',
      color: '#10b981',
      status: 'idle',
    } satisfies StepNodeData,
  };
}

export function createStepNode(
  type: StepNodeType,
  id: string,
  position: { x: number; y: number },
  onConfigure: (id: string) => void,
  onDelete: (id: string) => void,
  label?: string,
): Node {
  const defaults = STEP_DEFAULTS[type];
  return {
    id,
    type: 'stepNode',
    position,
    data: {
      label: label || defaults.label,
      stepType: type,
      icon: defaults.icon,
      color: defaults.color,
      status: 'idle',
      envLabel: type === 'mcp-server' || type === 'executable'
        ? (type === 'executable' ? 'py_env' : 'py_env')
        : undefined,
      config: {},
      onConfigure,
      onDelete,
    } satisfies StepNodeData,
  };
}

export { STEP_DEFAULTS };
