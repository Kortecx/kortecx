import { createRoute, useParams, useSearch } from "@tanstack/react-router";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { ErrorNotice } from "../../components/ErrorNotice";
import { MoteTable } from "../../components/MoteTable";
import { ProjectionSummary } from "../../components/ProjectionSummary";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { allTerminal, useProjection } from "../../kx/use-projection";
import { shortHex } from "../../lib/format";
import { rootRoute } from "./__root";

const ROUTE_ID = "/runs/$instanceId";

interface RunSearch {
  atSeq?: number;
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
  const { atSeq } = useSearch({ from: ROUTE_ID });
  const projection = useProjection(instanceId, atSeq != null ? { atSeq } : {});
  const data = projection.data;
  const polling = atSeq == null && data != null && !allTerminal(data);

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
          <MoteTable projection={data} />
        </>
      ) : null}
    </section>
  );
}

export const runDetailRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: ROUTE_ID,
  validateSearch: (search: Record<string, unknown>): RunSearch => {
    const raw = search.atSeq;
    const n = typeof raw === "string" ? Number(raw) : typeof raw === "number" ? raw : Number.NaN;
    return Number.isFinite(n) && n >= 0 ? { atSeq: Math.floor(n) } : {};
  },
  component: RunDetailScreen,
});
