import { useNavigate } from "@tanstack/react-router";
import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import { useRuns } from "../../kx/use-runs";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { ArtifactGallery } from "./ArtifactGallery";
import { ArtifactView } from "./ArtifactView";

/**
 * Review run artifacts (UI-2). Two modes:
 *  - GALLERY (`?run=<instanceId>`): pick a run and browse all of its committed
 *    outputs (each Mote's `result_ref`, fetched lazily via `GetContent`).
 *  - DEEP-LINK (`?instance=&ref=`): one committed artifact (e.g. linked from a
 *    run's Mote table). Both rely only on the ownership-checked `GetContent`.
 */
export function ArtifactsSection({
  runId,
  instanceId,
  contentRef,
}: {
  runId?: string;
  instanceId?: string;
  contentRef?: string;
}) {
  if (instanceId && contentRef) {
    return <SingleArtifact instanceId={instanceId} contentRef={contentRef} />;
  }
  return <GalleryBrowser runId={runId} />;
}

/** The browse view: a run picker + the selected run's artifact gallery. */
function GalleryBrowser({ runId }: { runId?: string }) {
  const navigate = useNavigate();
  const { runs } = useRuns();
  const active = runId ?? runs[0]?.instanceId;

  return (
    <section className="screen" data-testid="artifacts-section">
      <h1>Artifacts</h1>
      <p className="muted">Browse a run's committed outputs — review or download each.</p>

      {runs.length === 0 ? (
        <EmptyState
          title="No runs to browse"
          detail="Submit a recipe from the Recipes section, then review its outputs here."
        />
      ) : (
        <>
          <label htmlFor="artifact-run">Run</label>
          <select
            id="artifact-run"
            data-testid="artifact-run-pick"
            value={active ?? ""}
            onChange={(e) => navigate({ to: "/artifacts", search: { run: e.target.value } })}
          >
            {runs.map((r) => (
              <option key={r.instanceId} value={r.instanceId}>
                {shortHex(r.instanceId)}
                {r.handle ? ` · ${r.handle}` : ""}
              </option>
            ))}
          </select>
          {active ? <ArtifactGallery instanceId={active} /> : null}
        </>
      )}
    </section>
  );
}

/** The focused deep-link view: one committed artifact by `?instance=&ref=`. */
function SingleArtifact({ instanceId, contentRef }: { instanceId: string; contentRef: string }) {
  const content = useContent(instanceId, contentRef);
  return (
    <section className="screen" data-testid="artifacts-section">
      <h1>Artifacts</h1>
      <p className="muted">
        Run <code className="mono">{shortHex(instanceId)}</code> · ref{" "}
        <code className="mono">{shortHex(contentRef)}</code>
      </p>
      {content.isLoading ? <EmptyState title="Loading artifact…" /> : null}
      {content.error ? (
        <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />
      ) : null}
      {content.data ? <ArtifactView content={content.data} /> : null}
    </section>
  );
}
