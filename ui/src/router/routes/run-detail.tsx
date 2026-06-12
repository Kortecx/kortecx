import { Link, createRoute, useParams, useSearch } from "@tanstack/react-router";
import { Suspense, lazy, useState } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { ErrorNotice } from "../../components/ErrorNotice";
import { MoteTable } from "../../components/MoteTable";
import { ProjectionSummary } from "../../components/ProjectionSummary";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { type ProjectionVM, runSettled, useProjection } from "../../kx/use-projection";
import { shortHex } from "../../lib/format";
import { rootRoute } from "./__root";

// Code-split: reactflow + dagre (~250 kB gzip) load only when a run's graph is
// actually viewed — the connect/runs screens stay lightweight.
const MoteDag = lazy(() =>
  import("../../components/dag/MoteDag").then((m) => ({ default: m.MoteDag })),
);

const ROUTE_ID = "/runs/$instanceId";

interface RunSearch {
  atSeq?: number;
  /** The recipe's terminal (sink) Mote id (hex) — the authoritative poll-stop signal. */
  terminal?: string;
}

type RunView = "dag" | "table";

/** The Motes as a live DAG (default) or a status table — both read the same VM. */
function RunBody({
  projection,
  view,
  onView,
}: {
  projection: ProjectionVM;
  view: RunView;
  onView: (v: RunView) => void;
}) {
  return (
    <>
      <fieldset className="view-toggle" aria-label="Run view">
        <button type="button" aria-pressed={view === "dag"} onClick={() => onView("dag")}>
          Graph
        </button>
        <button type="button" aria-pressed={view === "table"} onClick={() => onView("table")}>
          Table
        </button>
      </fieldset>
      {view === "dag" ? (
        <Suspense fallback={<EmptyState title="Loading graph…" />}>
          <MoteDag projection={projection} />
        </Suspense>
      ) : (
        <MoteTable projection={projection} />
      )}
    </>
  );
}

function RunDetailScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <RunDetailContent />;
}

function RunDetailContent() {
  const { instanceId } = useParams({ from: ROUTE_ID });
  const { atSeq, terminal } = useSearch({ from: ROUTE_ID });
  const projection = useProjection(instanceId, {
    ...(atSeq != null ? { atSeq } : {}),
    ...(terminal ? { terminalMoteId: terminal } : {}),
  });
  const data = projection.data;
  const polling = atSeq == null && data != null && !runSettled(data, terminal);
  const [view, setView] = useState<RunView>("dag");

  return (
    <section className="screen">
      <div className="screen__head">
        <h1>
          Run{" "}
          <code className="mono" title={instanceId}>
            {shortHex(instanceId)}
          </code>
        </h1>
        <button
          type="button"
          className="linkbtn"
          onClick={() => void projection.refetch()}
          disabled={projection.isFetching}
        >
          Refresh
        </button>
        {/* Artifacts left the sidebar (8-section IA); a run's outputs stay one
            click away until the PR-2 Workflows merge folds the gallery in here. */}
        <Link
          className="btnlink"
          to="/artifacts"
          search={{ run: instanceId }}
          data-testid="run-artifacts-link"
        >
          Artifacts →
        </Link>
      </div>
      {atSeq != null ? (
        <p className="muted">Pinned snapshot at seq #{atSeq} (live polling paused).</p>
      ) : null}
      {projection.isLoading ? <EmptyState title="Loading projection…" /> : null}
      {projection.error ? (
        <ErrorNotice
          error={toUiError(projection.error)}
          onRetry={() => void projection.refetch()}
        />
      ) : null}
      {data ? (
        <>
          <ProjectionSummary projection={data} polling={polling} />
          <RunBody projection={data} view={view} onView={setView} />
        </>
      ) : null}
    </section>
  );
}

export const runDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  validateSearch: (search: Record<string, unknown>): RunSearch => {
    const out: RunSearch = {};
    const raw = search.atSeq;
    const n = typeof raw === "string" ? Number(raw) : typeof raw === "number" ? raw : Number.NaN;
    if (Number.isFinite(n) && n >= 0) {
      out.atSeq = Math.floor(n);
    }
    // The terminal Mote id is a 32-byte (64 hex char) server-derived id.
    if (typeof search.terminal === "string" && /^[0-9a-f]{64}$/.test(search.terminal)) {
      out.terminal = search.terminal;
    }
    return out;
  },
  component: RunDetailScreen,
});
