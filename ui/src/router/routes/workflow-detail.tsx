import { createRoute, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { ErrorNotice } from "../../components/ErrorNotice";
import { MoteTable } from "../../components/MoteTable";
import { ProjectionSummary } from "../../components/ProjectionSummary";
import { ActivityFeed } from "../../components/activity/ActivityFeed";
import { TimeTravelScrubber } from "../../components/activity/TimeTravelScrubber";
import { MetricsPanel } from "../../components/metrics/MetricsPanel";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import { useEventStream } from "../../kx/use-event-stream";
import { type ProjectionVM, runSettled, useProjection } from "../../kx/use-projection";
import { shortHex } from "../../lib/format";
import { rootRoute } from "./__root";

// Code-split: reactflow + dagre (~250 kB gzip) load only when a run's graph is
// actually viewed — the connect/list screens stay lightweight.
const MoteDag = lazy(() =>
  import("../../components/dag/MoteDag").then((m) => ({ default: m.MoteDag })),
);
const ArtifactGallery = lazy(() =>
  import("../../components/sections/ArtifactGallery").then((m) => ({
    default: m.ArtifactGallery,
  })),
);
const ArtifactView = lazy(() =>
  import("../../components/sections/ArtifactView").then((m) => ({ default: m.ArtifactView })),
);

const ROUTE_ID = "/workflows/$instanceId";

/** The run-detail tabs (PR-2 merge, D141.1): the DAG/table telemetry views plus
 *  the folded-in Artifacts gallery and the run-scoped Activity panel. */
const RUN_TABS = ["graph", "table", "artifacts", "activity"] as const;
type RunTab = (typeof RUN_TABS)[number];

interface RunSearch {
  atSeq?: number;
  /** The recipe's terminal (sink) Mote id (hex) — the authoritative poll-stop signal. */
  terminal?: string;
  /** The active detail tab; absent = "graph". */
  tab?: RunTab;
  /** Artifacts tab deep link: one committed artifact by content ref (hex). */
  ref?: string;
}

const TAB_LABEL: Record<RunTab, string> = {
  graph: "Graph",
  table: "Table",
  artifacts: "Artifacts",
  activity: "Activity",
};

function WorkflowDetailScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <WorkflowDetailContent />;
}

function WorkflowDetailContent() {
  const { instanceId } = useParams({ from: ROUTE_ID });
  const search = useSearch({ from: ROUTE_ID });
  const { atSeq, terminal } = search;
  const tab: RunTab = search.tab ?? "graph";
  const navigate = useNavigate({ from: ROUTE_ID });
  const projection = useProjection(instanceId, {
    ...(atSeq != null ? { atSeq } : {}),
    ...(terminal ? { terminalMoteId: terminal } : {}),
  });
  const data = projection.data;
  const polling = atSeq == null && data != null && !runSettled(data, terminal);

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
      <fieldset className="view-toggle" aria-label="Run view" data-testid="run-tabs">
        {RUN_TABS.map((t) => (
          <button
            key={t}
            type="button"
            aria-pressed={tab === t}
            data-testid={`run-tab-${t}`}
            onClick={() =>
              void navigate({
                // Leaving the artifacts tab drops its `ref` deep link.
                search: (prev) => ({
                  ...prev,
                  tab: t === "graph" ? undefined : t,
                  ref: t === "artifacts" ? prev.ref : undefined,
                }),
              })
            }
          >
            {TAB_LABEL[t]}
          </button>
        ))}
      </fieldset>
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
          {tab === "graph" || tab === "table" ? (
            <RunGraph projection={data} table={tab === "table"} />
          ) : null}
          {tab === "artifacts" ? (
            <ArtifactsTab instanceId={instanceId} contentRef={search.ref} />
          ) : null}
          {tab === "activity" ? (
            <ActivityTab
              instanceId={instanceId}
              atSeq={atSeq}
              headSeq={data.currentSeq}
              display={data}
              onAtSeq={(seq) => void navigate({ search: (prev) => ({ ...prev, atSeq: seq }) })}
            />
          ) : null}
        </>
      ) : null}
    </section>
  );
}

/** The Motes as a live DAG or a status table — both read the same VM (D141.3:
 *  strictly read-only telemetry; building happens in Blueprints). */
function RunGraph({ projection, table }: { projection: ProjectionVM; table: boolean }) {
  if (table) {
    return <MoteTable projection={projection} />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading graph…" />}>
      <MoteDag projection={projection} />
    </Suspense>
  );
}

/** The folded-in Artifacts gallery (PR-2; was the /artifacts route). With a
 *  `ref` deep link it focuses one committed artifact. */
function ArtifactsTab({ instanceId, contentRef }: { instanceId: string; contentRef?: string }) {
  return (
    <div data-testid="artifacts-tab">
      <Suspense fallback={<EmptyState title="Loading artifacts…" />}>
        {contentRef ? (
          <SingleArtifact instanceId={instanceId} contentRef={contentRef} />
        ) : (
          <ArtifactGallery instanceId={instanceId} />
        )}
      </Suspense>
    </div>
  );
}

/** One committed artifact by ref (the old /artifacts?instance&ref deep link). */
function SingleArtifact({ instanceId, contentRef }: { instanceId: string; contentRef: string }) {
  const content = useContent(instanceId, contentRef);
  return (
    <>
      <p className="muted">
        Artifact <code className="mono">{shortHex(contentRef)}</code>
      </p>
      {content.isLoading ? <EmptyState title="Loading artifact…" /> : null}
      {content.error ? (
        <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />
      ) : null}
      {content.data ? <ArtifactView content={content.data} /> : null}
    </>
  );
}

/** The run-scoped Activity tab (PR-2): metrics + time-travel + the live event
 *  feed for THIS run, URL-addressable. The navbar Activity drawer remains the
 *  global any-run surface (its run picker stays drawer-only — D141.1: shared
 *  leaf components, disjoint operation). */
function ActivityTab({
  instanceId,
  atSeq,
  headSeq,
  display,
  onAtSeq,
}: {
  instanceId: string;
  atSeq?: number;
  headSeq: number;
  display: ProjectionVM;
  onAtSeq: (seq: number | undefined) => void;
}) {
  const stream = useEventStream(instanceId);
  return (
    <div data-testid="run-activity-tab">
      <MetricsPanel projection={display} />
      {headSeq > 0 ? (
        <TimeTravelScrubber currentSeq={headSeq} atSeq={atSeq} onChange={onAtSeq} />
      ) : null}
      <h2>Live events</h2>
      <ActivityFeed events={stream.events} dropped={stream.dropped} active={stream.active} />
    </div>
  );
}

export const workflowDetailRoute = createRoute({
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
    if (typeof search.tab === "string" && (RUN_TABS as readonly string[]).includes(search.tab)) {
      out.tab = search.tab as RunTab;
    }
    // An artifact content ref is a 32-byte (64 hex char) server-derived id.
    if (typeof search.ref === "string" && /^[0-9a-f]{64}$/.test(search.ref)) {
      out.ref = search.ref;
    }
    return out;
  },
  component: WorkflowDetailScreen,
});
