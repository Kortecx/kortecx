/**
 * The single-App LINEAGE pane — a clean, READ-ONLY diagram of the App's portable
 * blueprint (a `DagSpec`): step order, parallel-vs-sequential shape, one node per step.
 * Structure AUTHORING (add / remove / reorder / config + save-to-App) lives in the
 * Workflows builder; this pane VIEWS the structure and offers Run + an "Edit structure"
 * entry that opens the builder seeded from this App (POC-5d), unless the blueprint is
 * un-round-trippable (exec/binary) or the App is locked.
 *
 * The diagram is a STATIC dagre layout rendered as plain node cards + SVG connectors —
 * no editor canvas (no grid, minimap, zoom, drag, or connect handles), so it reads as
 * documentation, not a workspace. Reuses the pure `appBlueprintToBuilderGraph` parse
 * and the shared `layoutGraph` (dagre), so it stays byte-faithful to the DAG the run
 * executes without pulling reactflow into the read path.
 */

import { useNavigate } from "@tanstack/react-router";
import { useCallback, useMemo } from "react";
import { toUiError } from "../../kx/errors";
import { useApp, useRunApp } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { appBlueprintToBuilderGraph } from "../builder/app-blueprint";
import type { BuilderGraph } from "../builder/builder-graph";
import { NODE_H, NODE_W, layoutGraph } from "../dag/layout";

/** A clean static diagram: dagre-positioned node cards + SVG connectors, read-only. */
function LineageDiagram({ graph }: { graph: BuilderGraph }) {
  const positions = useMemo(
    () =>
      layoutGraph(
        graph.steps.map((s) => s.id),
        graph.edges.map((e) => ({
          id: e.id,
          source: e.source,
          target: e.target,
          edgeKind: e.edge === "control" ? "control" : "data",
          nonCascade: false,
        })),
      ),
    [graph],
  );
  const nodes = graph.steps.map((step) => ({
    step,
    pos: positions.get(step.id) ?? { x: 16, y: 16 },
  }));
  const width = Math.ceil(Math.max(NODE_W, ...nodes.map((n) => n.pos.x + NODE_W)) + 16);
  const height = Math.ceil(Math.max(NODE_H, ...nodes.map((n) => n.pos.y + NODE_H)) + 16);

  return (
    <div className="lineage-scroll" data-testid="app-lineage-canvas">
      <div
        className="lineage-diagram"
        data-testid="app-lineage-diagram"
        data-steps={nodes.length}
        style={{ width, height }}
        role="img"
        aria-label="App structure diagram"
      >
        <svg className="lineage-diagram__edges" width={width} height={height} aria-hidden="true">
          <title>Step connections</title>
          <defs>
            <marker
              id="lineage-arrow"
              viewBox="0 0 10 10"
              refX="9"
              refY="5"
              markerWidth="7"
              markerHeight="7"
              orient="auto-start-reverse"
            >
              <path d="M 0 0 L 10 5 L 0 10 z" className="lineage-diagram__arrow" />
            </marker>
          </defs>
          {graph.edges.map((e) => {
            const s = positions.get(e.source);
            const t = positions.get(e.target);
            if (!s || !t) {
              return null;
            }
            const x1 = s.x + NODE_W / 2;
            const y1 = s.y + NODE_H;
            const x2 = t.x + NODE_W / 2;
            const y2 = t.y;
            const my = (y1 + y2) / 2;
            return (
              <path
                key={e.id}
                d={`M ${x1} ${y1} C ${x1} ${my}, ${x2} ${my}, ${x2} ${y2}`}
                className={`lineage-diagram__edge lineage-diagram__edge--${e.edge}`}
                markerEnd="url(#lineage-arrow)"
                fill="none"
              />
            );
          })}
        </svg>
        {nodes.map(({ step, pos }) => (
          <div
            key={step.id}
            className="lineage-node"
            data-kind={step.kind}
            data-testid={`lineage-node-${step.id}`}
            style={{ left: pos.x, top: pos.y, width: NODE_W, height: NODE_H }}
          >
            <span className="lineage-node__kind">{step.kind}</span>
            <span className="lineage-node__label" title={step.label || step.id}>
              {step.label || step.id}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}

function LineageView({
  handle,
  envelope,
  locked,
}: {
  handle: string;
  envelope: Record<string, unknown>;
  locked: boolean;
}) {
  const navigate = useNavigate();
  const runApp = useRunApp();

  const parsed = useMemo(
    () => appBlueprintToBuilderGraph((envelope.blueprint ?? { seed: 0, steps: [] }) as never),
    [envelope.blueprint],
  );

  // Structure is editable in the builder unless the blueprint has an un-round-trippable
  // (exec/binary) step, or the App is locked (a locked App refuses a structure re-save).
  const canEdit = !parsed.unmodeled.refuseEdit && !locked;
  const notice = parsed.unmodeled.refuseEdit
    ? "A read-only view — this App's blueprint has a step the visual editor can't edit; change it via the SDK/CLI."
    : locked
      ? "A read-only view — this App is locked. Unlock it to edit the structure."
      : "This App's structure (order, parallel vs. sequential, one node per step). Edit it in the builder.";

  const onRun = useCallback(() => {
    runApp.mutate(
      { handle },
      {
        onSuccess: ({ instanceId }) =>
          void navigate({ to: "/workflows/$instanceId", params: { instanceId } }),
      },
    );
  }, [runApp, handle, navigate]);

  const onEditStructure = useCallback(() => {
    void navigate({ to: "/blueprints/new", search: { app: handle } });
  }, [navigate, handle]);

  return (
    <div className="app-lineage" data-testid="app-lineage">
      <div className="app-lineage__toolbar">
        <span className="muted" data-testid="lineage-readonly-notice" role="note">
          {notice}
        </span>
        {canEdit ? (
          <button
            type="button"
            className="btn-ghost"
            data-testid="lineage-edit-structure"
            title="Open this App's structure in the visual builder"
            onClick={onEditStructure}
          >
            Edit structure
          </button>
        ) : null}
        <button
          type="button"
          className="btn-ghost"
          data-testid="app-lineage-run"
          disabled={runApp.isPending}
          onClick={onRun}
        >
          {runApp.isPending ? "Running…" : "Run"}
        </button>
      </div>

      {runApp.isError ? (
        <ErrorNotice error={toUiError(runApp.error)} onRetry={() => runApp.reset()} />
      ) : null}

      {parsed.graph.steps.length === 0 ? (
        <EmptyState title="No steps" detail="This App's blueprint has no steps to show." />
      ) : (
        <LineageDiagram graph={parsed.graph} />
      )}
    </div>
  );
}

/** The Lineage pane: fetch the App's full envelope, then render the read-only diagram. */
export function AppLineageSection({ handle }: { handle: string }) {
  const app = useApp(handle);

  if (app.isLoading) {
    return <EmptyState title="Loading structure…" />;
  }
  if (app.isError) {
    return <ErrorNotice error={toUiError(app.error)} onRetry={() => void app.refetch()} />;
  }
  if (!app.data) {
    return (
      <EmptyState title="App not found" detail="This App is not in your catalog (or not owned)." />
    );
  }
  return (
    <LineageView handle={handle} envelope={app.data.envelope} locked={app.data.summary.locked} />
  );
}
