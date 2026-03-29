'use client';

import { useEffect, useRef, useCallback, useState } from 'react';
import { Maximize2, Minus, Plus, RotateCcw } from 'lucide-react';
import type cytoscape from 'cytoscape';

/* ── Vibrant role colors ── */
const ROLE_COLOR: Record<string, string> = {
  researcher: '#a78bfa', analyst: '#60a5fa', writer: '#fbbf24', coder: '#34d399',
  reviewer: '#22d3ee', planner: '#818cf8', legal: '#f87171', financial: '#fb923c',
  medical: '#f472b6', coordinator: '#c084fc', 'data-engineer': '#2dd4bf',
  creative: '#e879f9', translator: '#67e8f9', custom: '#94a3b8',
};

export interface SimilarityEdge {
  source: string;
  target: string;
  weight: number;
}

interface AgentGraphProps {
  agents: Array<Record<string, unknown>>;
  edges: SimilarityEdge[];
  onNodeClick: (agentId: string) => void;
  roleFilter?: string;
  statusFilter?: string;
  search?: string;
}

export default function AgentGraph({
  agents, edges, onNodeClick, roleFilter, statusFilter, search,
}: AgentGraphProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const cyRef = useRef<cytoscape.Core | null>(null);
  const [ready, setReady] = useState(false);

  const filteredAgents = agents.filter(p => {
    if (roleFilter && roleFilter !== 'all' && p.role !== roleFilter) return false;
    if (statusFilter && statusFilter !== 'all' && p.status !== statusFilter) return false;
    if (search) {
      const s = search.toLowerCase();
      const name = ((p.name as string) ?? '').toLowerCase();
      const desc = ((p.description as string) ?? '').toLowerCase();
      if (!name.includes(s) && !desc.includes(s)) return false;
    }
    return true;
  });

  const filteredIds = new Set(filteredAgents.map(p => p.id as string));
  const filteredEdges = edges.filter(e => filteredIds.has(e.source) && filteredIds.has(e.target));

  const initGraph = useCallback(async () => {
    if (!containerRef.current) return;

    const cy = (await import('cytoscape')).default;
    // @ts-expect-error -- cytoscape-fcose has no type declarations
    const fcose = (await import('cytoscape-fcose')).default;
    cy.use(fcose);

    const nodes = filteredAgents.map(p => {
      const id = p.id as string;
      const role = (p.role as string) ?? 'custom';
      const complexity = (p.complexityLevel as number) ?? 3;
      const totalRuns = (p.totalRuns as number) ?? 0;
      const size = 32 + complexity * 7 + Math.min(totalRuns, 50) * 0.3;
      const color = ROLE_COLOR[role] ?? '#94a3b8';
      return { data: { id, label: ((p.name as string) ?? '').slice(0, 18), role, color, size } };
    });

    const cyEdges = filteredEdges.map(e => ({
      data: {
        id: `${e.source}-${e.target}`,
        source: e.source,
        target: e.target,
        weight: e.weight,
        sourceColor: nodes.find(n => n.data.id === e.source)?.data.color ?? '#94a3b8',
      },
    }));

    if (cyRef.current) cyRef.current.destroy();

    // Adaptive layout
    const n = nodes.length;
    const lerp = (lo: number, hi: number) => {
      const t = Math.min(Math.max((n - 5) / 35, 0), 1);
      return lo + (hi - lo) * t;
    };
    const repulsion   = Math.round(lerp(12000, 4000));
    const edgeLenBase = Math.round(lerp(250, 80));
    const grav        = lerp(0.08, 0.5);
    const gravRange   = lerp(4.0, 1.5);
    const edgeWMin    = lerp(1.5, 0.5);
    const edgeWMax    = lerp(5, 3);
    const edgeOpMin   = lerp(0.25, 0.08);
    const edgeOpMax   = lerp(0.75, 0.45);

    const instance = cy({
      container: containerRef.current,
      elements: [...nodes, ...cyEdges],
      style: [
        {
          selector: 'node',
          style: {
            'width': 'data(size)',
            'height': 'data(size)',
            'background-color': 'data(color)',
            'background-opacity': 0.9,
            'label': 'data(label)',
            'font-size': '10px',
            'font-weight': 'bold',
            'color': '#1a1a1a',
            'text-valign': 'bottom',
            'text-margin-y': 8,
            'text-outline-width': 2,
            'text-outline-color': '#ffffff',
            'text-outline-opacity': 0.8,
            'border-width': 3,
            'border-color': 'data(color)',
            'border-opacity': 0.6,
            'border-style': 'solid',
            'overlay-opacity': 0,
          } as unknown as cytoscape.Css.Node,
        },
        {
          selector: 'node:active',
          style: {
            'border-width': 5,
            'border-opacity': 1,
            'background-opacity': 1,
          } as unknown as cytoscape.Css.Node,
        },
        {
          selector: 'edge',
          style: {
            'width': `mapData(weight, 0.2, 1, ${edgeWMin}, ${edgeWMax})`,
            'line-color': '#333333',
            'opacity': `mapData(weight, 0.2, 1, ${edgeOpMin}, ${edgeOpMax})`,
            'curve-style': 'bezier',
          } as unknown as cytoscape.Css.Edge,
        },
        {
          selector: 'edge:active',
          style: { 'opacity': 0.9, 'width': 4 } as unknown as cytoscape.Css.Edge,
        },
      ],
      layout: {
        name: 'fcose',
        animate: true,
        animationDuration: 600,
        quality: 'proof',
        randomize: true,
        nodeDimensionsIncludeLabels: true,
        idealEdgeLength: (edge: cytoscape.EdgeSingular) => {
          const w = edge.data('weight') ?? 0.5;
          return Math.round(edgeLenBase / Math.max(w, 0.1));
        },
        nodeRepulsion: () => repulsion,
        gravity: grav,
        gravityRange: gravRange,
      } as unknown as cytoscape.LayoutOptions,
      minZoom: 0.15,
      maxZoom: 5,
      wheelSensitivity: 0.25,
    });

    instance.on('tap', 'node', (evt) => onNodeClick(evt.target.id()));

    cyRef.current = instance;
    setReady(true);
  }, [filteredAgents, filteredEdges, onNodeClick]);

  useEffect(() => {
    initGraph();
    return () => { if (cyRef.current) { cyRef.current.destroy(); cyRef.current = null; } };
  }, [initGraph]);

  const handleZoomIn = () => cyRef.current?.zoom(cyRef.current.zoom() * 1.3);
  const handleZoomOut = () => cyRef.current?.zoom(cyRef.current.zoom() / 1.3);
  const handleFit = () => cyRef.current?.fit(undefined, 40);
  const handleReset = () => {
    cyRef.current?.layout({ name: 'fcose', animate: true, animationDuration: 600, randomize: true } as unknown as cytoscape.LayoutOptions).run();
  };

  return (
    <div style={{ position: 'relative', width: '100%', height: '100%', minHeight: 500 }}>
      <div
        ref={containerRef}
        style={{
          width: '100%', height: '100%', minHeight: 500,
          borderRadius: 12, border: '1px solid var(--border)',
          background: '#ffffff',
        }}
      />

      {/* Toolbar */}
      <div style={{
        position: 'absolute', top: 12, right: 12,
        display: 'flex', flexDirection: 'column', gap: 4,
        background: 'rgba(255,255,255,0.9)', backdropFilter: 'blur(8px)',
        borderRadius: 10, border: '1px solid #e5e7eb', padding: 4,
        boxShadow: '0 2px 8px rgba(0,0,0,0.08)',
      }}>
        {[
          { icon: Plus, onClick: handleZoomIn, title: 'Zoom in' },
          { icon: Minus, onClick: handleZoomOut, title: 'Zoom out' },
          { icon: Maximize2, onClick: handleFit, title: 'Fit to screen' },
          { icon: RotateCcw, onClick: handleReset, title: 'Reset layout' },
        ].map(({ icon: Icon, onClick, title }) => (
          <button
            key={title} onClick={onClick} title={title}
            style={{
              display: 'flex', alignItems: 'center', justifyContent: 'center',
              width: 32, height: 32, borderRadius: 7,
              border: 'none', background: 'transparent',
              color: '#6b7280', cursor: 'pointer', transition: 'all 0.15s',
            }}
            onMouseEnter={e => { e.currentTarget.style.background = '#f3f4f6'; e.currentTarget.style.color = '#111'; }}
            onMouseLeave={e => { e.currentTarget.style.background = 'transparent'; e.currentTarget.style.color = '#6b7280'; }}
          >
            <Icon size={14} />
          </button>
        ))}
      </div>

      {/* Empty state */}
      {ready && filteredAgents.length === 0 && (
        <div style={{
          position: 'absolute', top: '50%', left: '50%',
          transform: 'translate(-50%, -50%)',
          textAlign: 'center', color: '#9ca3af',
        }}>
          <div style={{ fontSize: 14, fontWeight: 600 }}>No Agents to display</div>
          <div style={{ fontSize: 12, marginTop: 4 }}>Bundle Agents to see them in the graph</div>
        </div>
      )}

      {/* Legend */}
      {filteredAgents.length > 0 && (
        <div style={{
          position: 'absolute', bottom: 12, left: 12,
          display: 'flex', flexWrap: 'wrap', gap: 8,
          background: 'rgba(255,255,255,0.9)', backdropFilter: 'blur(8px)',
          borderRadius: 10, border: '1px solid #e5e7eb',
          padding: '6px 12px', fontSize: 10, color: '#6b7280',
          boxShadow: '0 2px 8px rgba(0,0,0,0.06)',
        }}>
          {Object.entries(ROLE_COLOR).slice(0, 8).map(([role, color]) => (
            <div key={role} style={{ display: 'flex', alignItems: 'center', gap: 4 }}>
              <div style={{ width: 8, height: 8, borderRadius: '50%', background: color }} />
              {role}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
