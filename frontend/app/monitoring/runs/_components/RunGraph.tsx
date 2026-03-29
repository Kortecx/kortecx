'use client';

import { useCallback, useMemo, useState } from 'react';
import {
  ReactFlow,
  Background,
  Controls,
  MiniMap,
  useNodesState,
  useEdgesState,
  addEdge,
  type Node,
  type Edge,
  type Connection,
  MarkerType,
  BackgroundVariant,
} from '@xyflow/react';
import '@xyflow/react/dist/style.css';
import RunGraphNode from './RunGraphNode';
import type { PlanNode, PlanEdge } from '@/lib/types';

const nodeTypes = { agentNode: RunGraphNode };

interface RunGraphProps {
  planNodes: PlanNode[];
  planEdges: PlanEdge[];
  editable?: boolean;
  onSave?: (nodes: PlanNode[], edges: PlanEdge[]) => void;
}

export default function RunGraph({ planNodes, planEdges, editable = false, onSave }: RunGraphProps) {
  const [editMode, setEditMode] = useState(false);

  const initialNodes: Node[] = useMemo(() => planNodes.map(n => ({
    id: n.id,
    type: 'agentNode',
    position: n.position ?? { x: 0, y: 0 },
    data: {
      label: n.label,
      role: n.agentId?.includes('researcher') ? 'researcher' : 'custom',
      status: n.status ?? 'pending',
      tokensUsed: n.tokensUsed ?? 0,
      durationMs: n.durationMs ?? 0,
      agentId: n.agentId,
    },
    draggable: editMode,
  })), [planNodes, editMode]);

  const initialEdges: Edge[] = useMemo(() => planEdges.map(e => ({
    id: e.id,
    source: e.source,
    target: e.target,
    animated: e.animated ?? false,
    style: { stroke: '#333', strokeWidth: 2 },
    markerEnd: { type: MarkerType.ArrowClosed, color: '#333' },
  })), [planEdges]);

  const [nodes, setNodes, onNodesChange] = useNodesState(initialNodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState(initialEdges);

  const onConnect = useCallback((params: Connection) => {
    if (!editMode) return;
    setEdges(eds => addEdge({ ...params, style: { stroke: '#333', strokeWidth: 2 }, markerEnd: { type: MarkerType.ArrowClosed, color: '#333' } }, eds));
  }, [editMode, setEdges]);

  const handleSave = () => {
    if (!onSave) return;
    const savedNodes: PlanNode[] = nodes.map(n => ({
      id: n.id,
      agentId: (n.data as Record<string, unknown>).agentId as string ?? '',
      label: (n.data as Record<string, unknown>).label as string ?? '',
      position: n.position,
      status: (n.data as Record<string, unknown>).status as PlanNode['status'],
      tokensUsed: (n.data as Record<string, unknown>).tokensUsed as number,
      durationMs: (n.data as Record<string, unknown>).durationMs as number,
    }));
    const savedEdges: PlanEdge[] = edges.map(e => ({
      id: e.id,
      source: e.source,
      target: e.target,
      animated: e.animated,
    }));
    onSave(savedNodes, savedEdges);
  };

  return (
    <div style={{ width: '100%', height: 450, borderRadius: 12, border: '1px solid var(--border)', overflow: 'hidden', position: 'relative' }}>
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodesChange={editMode ? onNodesChange : undefined}
        onEdgesChange={editMode ? onEdgesChange : undefined}
        onConnect={onConnect}
        nodeTypes={nodeTypes}
        fitView
        fitViewOptions={{ padding: 0.3 }}
        nodesDraggable={editMode}
        nodesConnectable={editMode}
        elementsSelectable={editMode}
        style={{ background: '#ffffff' }}
      >
        <Background variant={BackgroundVariant.Dots} gap={20} size={1} color="#e5e7eb" />
        <Controls position="top-left" style={{ background: '#fff', borderRadius: 8, border: '1px solid #e5e7eb' }} />
        <MiniMap
          nodeColor={(n) => {
            const status = (n.data as Record<string, unknown>)?.status as string;
            if (status === 'completed') return '#22c55e';
            if (status === 'running') return '#3b82f6';
            if (status === 'failed') return '#ef4444';
            return '#d1d5db';
          }}
          style={{ borderRadius: 8, border: '1px solid #e5e7eb' }}
        />
      </ReactFlow>

      {/* Edit / Save toolbar */}
      {editable && (
        <div style={{
          position: 'absolute', top: 12, right: 12,
          display: 'flex', gap: 6, zIndex: 10,
        }}>
          <button
            onClick={() => setEditMode(!editMode)}
            style={{
              padding: '6px 14px', borderRadius: 7, fontSize: 11, fontWeight: 600,
              border: editMode ? '1.5px solid #D97706' : '1px solid #e5e7eb',
              background: editMode ? 'rgba(217,119,6,0.1)' : '#fff',
              color: editMode ? '#D97706' : '#6b7280',
              cursor: 'pointer',
            }}
          >
            {editMode ? 'Editing...' : 'Edit'}
          </button>
          {editMode && onSave && (
            <button
              onClick={handleSave}
              style={{
                padding: '6px 14px', borderRadius: 7, fontSize: 11, fontWeight: 600,
                border: '1.5px solid #22c55e', background: 'rgba(34,197,94,0.1)',
                color: '#15803d', cursor: 'pointer',
              }}
            >
              Save
            </button>
          )}
        </div>
      )}
    </div>
  );
}
