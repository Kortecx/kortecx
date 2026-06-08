import { Suspense, lazy } from "react";
import type { ProjectionVM } from "../../kx/use-projection";
import { EmptyState } from "../EmptyState";

// Reuse the run-detail DAG viewer (code-split: reactflow + dagre load on demand).
const MoteDag = lazy(() => import("../dag/MoteDag").then((m) => ({ default: m.MoteDag })));

/** The assistant's DAG-of-thought: the in-flight run's Motes as a live graph. */
export function ThinkingTrace({ projection }: { projection: ProjectionVM }) {
  return (
    <div className="thinking-trace" data-testid="thinking-trace">
      <Suspense fallback={<EmptyState title="Loading graph…" />}>
        <MoteDag projection={projection} />
      </Suspense>
    </div>
  );
}
