import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { ArtifactView } from "./ArtifactView";

/**
 * Review a committed artifact by `?instance=&ref=` (a run's `result_ref`, fetched
 * via the ownership-checked `GetContent`). Browse-all-artifacts arrives in UI-2.
 */
export function ArtifactsSection({
  instanceId,
  contentRef,
}: {
  instanceId?: string;
  contentRef?: string;
}) {
  const has = Boolean(instanceId) && Boolean(contentRef);
  const content = useContent(instanceId, contentRef);

  return (
    <section className="screen" data-testid="artifacts-section">
      <h1>Artifacts</h1>
      {has ? (
        <p className="muted">
          Run <code className="mono">{shortHex(instanceId ?? "")}</code> · ref{" "}
          <code className="mono">{shortHex(contentRef ?? "")}</code>
        </p>
      ) : (
        <EmptyState
          title="No artifact selected"
          detail="Open a committed Mote's result from a run's table, or full browsing arrives in UI-2."
        />
      )}
      {has && content.isLoading ? <EmptyState title="Loading artifact…" /> : null}
      {has && content.error ? (
        <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />
      ) : null}
      {has && content.data ? <ArtifactView content={content.data} /> : null}
    </section>
  );
}
