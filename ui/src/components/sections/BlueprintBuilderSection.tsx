/**
 * The visual Blueprint builder (D141.3/.4/.5/.6) — author a Tier-1 DAG (a vetted
 * PURE / MODEL palette) on an interactive reactflow canvas, configure each step in
 * a Monaco drawer, then SUBMIT it through `SubmitWorkflow`. The client sends ONLY
 * topology + params; the SERVER compiles the DAG + builds every warrant (SN-8).
 *
 * Rich-graph counterpart of the read-only run viewer: nodes are draggable +
 * connectable. An edge carries an optional instruction-file (D141.5) folded into
 * the downstream agent's prompt at submit. "Clone to edit" (D141.4) seeds the
 * builder from a committed run's DAG; the submit is always a NEW workflow (new
 * identity by construction). The acyclicity precheck mirrors the server's compile.
 */

import { useNavigate } from "@tanstack/react-router";
import {
  Background,
  Controls,
  MiniMap,
  ReactFlow,
  ReactFlowProvider,
  addEdge,
  useEdgesState,
  useNodesState,
} from "@xyflow/react";
import type { Connection, Edge, EdgeMouseHandler, NodeMouseHandler } from "@xyflow/react";
import { useCallback, useMemo, useRef, useState } from "react";
import { toUiError } from "../../kx/errors";
import { useModels } from "../../kx/use-models";
import { useSubmitWorkflow } from "../../kx/use-submit-workflow";
import { ErrorNotice } from "../ErrorNotice";
import { BuilderNode } from "../builder/BuilderNode";
import type { BuilderFlowNode } from "../builder/BuilderNode";
import { EdgeInstructionDrawer } from "../builder/EdgeInstructionDrawer";
import { StepConfigDrawer } from "../builder/StepConfigDrawer";
import {
  type BuilderEdge,
  type BuilderGraph,
  type BuilderStep,
  type BuilderStepKind,
  type PatternKind,
  insertPattern,
  newStep,
  toRequest,
  validationError,
} from "../builder/builder-graph";
import { NODE_H, NODE_W, layoutGraph } from "../dag/layout";

// Module-level stable reference (an inline object re-registers node types every
// render — a reactflow perf footgun).
const nodeTypes = { builder: BuilderNode };

/** Edge data the builder carries (the D141.5 instruction + the edge kind). */
interface BuilderEdgeData extends Record<string, unknown> {
  instruction: string;
  edge: "data" | "control";
}

/** Seed reactflow nodes/edges from an optional clone-to-edit graph (laid out via
 *  the shared dagre layout), else a single starter agent node. */
function seed(initial: BuilderGraph | undefined): {
  nodes: BuilderFlowNode[];
  edges: Edge[];
  nextId: number;
} {
  if (!initial || initial.steps.length === 0) {
    const step = newStep("model", "s0");
    return {
      nodes: [{ id: "s0", type: "builder", position: { x: 80, y: 40 }, data: { step } }],
      edges: [],
      nextId: 1,
    };
  }
  const positions = layoutGraph(
    initial.steps.map((s) => s.id),
    initial.edges.map((e) => ({
      id: e.id,
      source: e.source,
      target: e.target,
      edgeKind: e.edge === "control" ? "control" : "data",
      nonCascade: false,
    })),
  );
  const nodes: BuilderFlowNode[] = initial.steps.map((step) => ({
    id: step.id,
    type: "builder",
    position: positions.get(step.id) ?? { x: 80, y: 40 },
    data: { step },
  }));
  const edges: Edge[] = initial.edges.map((e) => toRfEdge(e));
  // The next free numeric id past any `s<n>` already used.
  const used = initial.steps
    .map((s) => Number.parseInt(s.id.replace(/^s/, ""), 10))
    .filter((n) => Number.isFinite(n));
  return { nodes, edges, nextId: (used.length ? Math.max(...used) : -1) + 1 };
}

function toRfEdge(e: BuilderEdge): Edge {
  const data: BuilderEdgeData = { instruction: e.instruction, edge: e.edge };
  return {
    id: e.id,
    source: e.source,
    target: e.target,
    data,
    label: e.instruction.trim() ? "📎 instruction" : undefined,
    animated: false,
  };
}

function BuilderInner({ initialGraph }: { initialGraph?: BuilderGraph }) {
  const navigate = useNavigate();
  const { models, unsupported } = useModels();
  const submit = useSubmitWorkflow();
  const seeded = useMemo(() => seed(initialGraph), [initialGraph]);
  const [nodes, setNodes, onNodesChange] = useNodesState<BuilderFlowNode>(seeded.nodes);
  const [edges, setEdges, onEdgesChange] = useEdgesState<Edge>(seeded.edges);
  const [selNode, setSelNode] = useState<string | null>(null);
  const [selEdge, setSelEdge] = useState<string | null>(null);
  const idc = useRef(seeded.nextId);

  const addStep = useCallback(
    (kind: BuilderStepKind) => {
      const id = `s${idc.current++}`;
      setNodes((ns) => {
        const y = 40 + ns.length * (NODE_H + 40);
        return [
          ...ns,
          {
            id,
            type: "builder",
            position: { x: 80 + (ns.length % 2) * (NODE_W + 60), y },
            data: { step: newStep(kind, id) },
          },
        ];
      });
      setSelNode(id);
    },
    [setNodes],
  );

  // Scaffold a multi-agent orchestration pattern — insert a pre-wired
  // cluster of the EXISTING model/pure vocabulary (the same nodes/edges/drawer as
  // `addStep`), laid out below the current canvas via the shared dagre layout. The
  // cluster lowers to the SAME DAG the SDK/CLI author (`insertPattern` mirrors
  // `flow().swarm/supervisor/consensus()`). The author then fills in each node's
  // model (validation gates submit until every agent has one).
  const insertPatternMacro = useCallback(
    (kind: PatternKind) => {
      const { steps, edges: patEdges, firstId, nextId } = insertPattern(kind, idc.current);
      idc.current = nextId;
      const layout = layoutGraph(
        steps.map((s) => s.id),
        patEdges.map((e) => ({
          id: e.id,
          source: e.source,
          target: e.target,
          edgeKind: "data" as const,
          nonCascade: false,
        })),
      );
      const minY = Math.min(...steps.map((s) => layout.get(s.id)?.y ?? 0));
      setNodes((ns) => {
        const yBase = ns.length ? Math.max(...ns.map((n) => n.position.y)) + NODE_H + 80 : 40;
        const added: BuilderFlowNode[] = steps.map((step) => {
          const p = layout.get(step.id) ?? { x: 80, y: 0 };
          return {
            id: step.id,
            type: "builder",
            position: { x: p.x, y: p.y - minY + yBase },
            data: { step },
          };
        });
        return [...ns, ...added];
      });
      setEdges((es) => [...es, ...patEdges.map(toRfEdge)]);
      setSelNode(firstId);
    },
    [setNodes, setEdges],
  );

  const updateStep = useCallback(
    (id: string, step: BuilderStep) => {
      setNodes((ns) => ns.map((n) => (n.id === id ? { ...n, data: { step } } : n)));
    },
    [setNodes],
  );

  const deleteStep = useCallback(
    (id: string) => {
      setNodes((ns) => ns.filter((n) => n.id !== id));
      setEdges((es) => es.filter((e) => e.source !== id && e.target !== id));
      setSelNode(null);
    },
    [setNodes, setEdges],
  );

  const onConnect = useCallback(
    (c: Connection) => {
      if (!c.source || !c.target || c.source === c.target) {
        return;
      }
      const id = `e-${c.source}-${c.target}`;
      setEdges((es) =>
        es.some((e) => e.id === id)
          ? es
          : addEdge(
              {
                ...c,
                id,
                data: { instruction: "", edge: "data" } satisfies BuilderEdgeData,
              },
              es,
            ),
      );
    },
    [setEdges],
  );

  const updateEdgeInstruction = useCallback(
    (id: string, instruction: string) => {
      setEdges((es) =>
        es.map((e) =>
          e.id === id
            ? {
                ...e,
                data: { ...(e.data as BuilderEdgeData), instruction },
                label: instruction.trim() ? "📎 instruction" : undefined,
              }
            : e,
        ),
      );
    },
    [setEdges],
  );

  const deleteEdge = useCallback(
    (id: string) => {
      setEdges((es) => es.filter((e) => e.id !== id));
      setSelEdge(null);
    },
    [setEdges],
  );

  // The logical graph derived from the canvas — the single source the submit maps.
  const graph: BuilderGraph = useMemo(
    () => ({
      steps: nodes.map((n) => n.data.step),
      edges: edges.map((e) => ({
        id: e.id,
        source: e.source,
        target: e.target,
        edge: ((e.data as BuilderEdgeData | undefined)?.edge ?? "data") as "data" | "control",
        instruction: (e.data as BuilderEdgeData | undefined)?.instruction ?? "",
      })),
    }),
    [nodes, edges],
  );
  const invalid = validationError(graph);

  const onNodeClick = useCallback<NodeMouseHandler>((_e, node) => {
    setSelEdge(null);
    setSelNode(node.id);
  }, []);
  const onEdgeClick = useCallback<EdgeMouseHandler>((_e, edge) => {
    setSelNode(null);
    setSelEdge(edge.id);
  }, []);

  const onSubmit = useCallback(() => {
    let req: ReturnType<typeof toRequest>;
    try {
      req = toRequest(graph, 0);
    } catch {
      return; // `invalid` already surfaces the reason in the bar
    }
    submit.mutate(req, {
      onSuccess: ({ instanceId }) => {
        void navigate({ to: "/workflows/$instanceId", params: { instanceId } });
      },
    });
  }, [graph, submit, navigate]);

  const selectedStep = selNode ? (nodes.find((n) => n.id === selNode)?.data.step ?? null) : null;
  const selectedEdge = selEdge ? (graph.edges.find((e) => e.id === selEdge) ?? null) : null;

  return (
    <section className="screen builder" data-testid="blueprint-builder">
      <header className="section-head">
        <div>
          <h2>New blueprint</h2>
          <p className="muted">
            Drag to arrange, drag handle-to-handle to connect. The server compiles the DAG and
            builds every warrant — you author topology + params only.
          </p>
        </div>
        <div className="builder-toolbar">
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-agent"
            onClick={() => addStep("model")}
          >
            + Agent
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-pure"
            onClick={() => addStep("pure")}
          >
            + Pure step
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-tool"
            onClick={() => addStep("tool")}
          >
            + Tool
          </button>
          <span className="builder-toolbar__divider" aria-hidden="true" />
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-swarm"
            title="Fan out to parallel agents, then gather"
            onClick={() => insertPatternMacro("swarm")}
          >
            + Swarm
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-supervisor"
            title="A planner decomposes the task; workers run in parallel; a lead integrates"
            onClick={() => insertPatternMacro("supervisor")}
          >
            + Supervisor
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-consensus-judge"
            title="N voters, then a judge selects the single best answer"
            onClick={() => insertPatternMacro("consensusJudge")}
          >
            + Consensus · judge
          </button>
          <button
            type="button"
            className="btn-ghost"
            data-testid="builder-add-consensus-majority"
            title="N voters, then an exact-equality majority vote (server-reduced)"
            onClick={() => insertPatternMacro("consensusMajority")}
          >
            + Consensus · majority
          </button>
          <button
            type="button"
            className="builder-submit"
            data-testid="builder-submit"
            disabled={invalid !== null || submit.isPending}
            title={invalid ?? "Compile + run on the server"}
            onClick={onSubmit}
          >
            {submit.isPending ? "Submitting…" : "Build & run"}
          </button>
        </div>
      </header>

      {invalid ? (
        <output className="builder-validation" data-testid="builder-validation">
          {invalid}
        </output>
      ) : null}
      {submit.isError ? <ErrorNotice error={toUiError(submit.error)} /> : null}

      <div
        className="dag-canvas builder-canvas"
        data-testid="builder-canvas"
        role="application"
        aria-label="Blueprint builder canvas"
      >
        <ReactFlow
          nodes={nodes}
          edges={edges}
          nodeTypes={nodeTypes}
          onNodesChange={onNodesChange}
          onEdgesChange={onEdgesChange}
          onConnect={onConnect}
          onNodeClick={onNodeClick}
          onEdgeClick={onEdgeClick}
          fitView
          proOptions={{ hideAttribution: true }}
          minZoom={0.1}
          maxZoom={1.5}
          deleteKeyCode={["Backspace", "Delete"]}
        >
          <Background gap={20} />
          <Controls showInteractive={false} />
          <MiniMap pannable zoomable nodeStrokeWidth={2} className="dag-minimap" />
        </ReactFlow>
      </div>

      {selectedStep ? (
        <StepConfigDrawer
          key={selectedStep.id}
          step={selectedStep}
          models={models}
          modelsUnsupported={unsupported}
          onChange={(s) => updateStep(selectedStep.id, s)}
          onDelete={() => deleteStep(selectedStep.id)}
          onClose={() => setSelNode(null)}
        />
      ) : null}
      {selectedEdge ? (
        <EdgeInstructionDrawer
          key={selectedEdge.id}
          edge={selectedEdge}
          steps={graph.steps}
          onChange={(text) => updateEdgeInstruction(selectedEdge.id, text)}
          onDelete={() => deleteEdge(selectedEdge.id)}
          onClose={() => setSelEdge(null)}
        />
      ) : null}
    </section>
  );
}

/**
 * The builder section. `initialGraph` (optional) seeds clone-to-edit (D141.4) from
 * a committed run's reconstructed DAG; absent ⇒ a fresh single-agent blueprint.
 */
export function BlueprintBuilderSection({ initialGraph }: { initialGraph?: BuilderGraph }) {
  return (
    <ReactFlowProvider>
      <BuilderInner initialGraph={initialGraph} />
    </ReactFlowProvider>
  );
}
