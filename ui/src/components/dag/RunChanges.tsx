/**
 * WAVE-3 (PR-1): the App Run interface's REVIEW surface — the run's committed OUTPUTS
 * as a review list. Honest by construction: OSS keeps no run-file PRE-image, so this is
 * the post-run HEAD of each step's committed result, NOT a fabricated before/after diff
 * (a true run-file diff is a schema / Cloud capability). Selecting an output renders its
 * committed body via the lazy, sandboxed {@link CodeViewer} (text / JSON — never
 * `innerHTML`); a media payload (which carries no decoded text) falls back to a note
 * pointing at the Artifacts tab, which owns the blob-URL media viewer. Read-only /
 * display-only (SN-8), NO new RPC — reuses `GetProjection` (`resultRef`) + the shipped
 * {@link useContent}.
 */

import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import type { ProjectionVM } from "../../kx/use-projection";
import { shortHex } from "../../lib/format";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { CodeViewer } from "../editor/CodeViewer";

function OutputBody({ instanceId, contentRef }: { instanceId: string; contentRef: string }) {
  const content = useContent(instanceId, contentRef);
  if (content.isLoading) {
    return <p className="muted">Loading output…</p>;
  }
  if (content.error) {
    return <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />;
  }
  const d = content.data;
  if (!d) {
    return null;
  }
  // Media kinds (image/video/audio) carry no decoded `.text` — the Artifacts tab owns
  // the blob-URL media viewer, so point there rather than dump raw bytes here.
  if (d.text === "") {
    return (
      <p className="muted" data-testid="run-changes-media">
        {d.kind} output ({d.byteLength} bytes) — open it in the Artifacts tab.
      </p>
    );
  }
  return <CodeViewer value={d.text} testId="run-changes-body" ariaLabel="Run output" />;
}

export function RunChanges({
  instanceId,
  projection,
}: {
  instanceId: string;
  projection: ProjectionVM;
}) {
  // Every committed step carries its result as a content ref — the run's outputs.
  const outputs = projection.motes.filter((m) => m.resultRef !== null);
  const [selected, setSelected] = useState<string | null>(null);
  const activeRef = selected ?? outputs[0]?.resultRef ?? null;

  return (
    <section className="run-changes" data-testid="run-changes">
      <div className="run-changes__head">
        <h2>Run outputs</h2>
        <p className="muted">
          Post-run head — the committed output of each step. OSS keeps no pre-image, so this is the
          current output, not a before/after diff (a true run-file diff is a schema / Cloud
          capability).
        </p>
      </div>
      {outputs.length === 0 ? (
        <EmptyState
          title="No committed outputs"
          detail="This run has not committed any step results yet."
        />
      ) : (
        <div className="run-changes__body">
          <ul className="run-changes__list" data-testid="run-changes-list">
            {outputs.map((m) => {
              // biome-ignore lint/style/noNonNullAssertion: the filter above guarantees resultRef.
              const ref = m.resultRef!;
              const active = ref === activeRef;
              return (
                <li key={m.moteId}>
                  <button
                    type="button"
                    className={`run-changes__item${active ? " is-active" : ""}`}
                    data-testid={`run-output-${m.moteId}`}
                    aria-pressed={active}
                    onClick={() => setSelected(ref)}
                  >
                    <code className="mono">{shortHex(m.moteId)}</code>
                    <span className="muted">{shortHex(ref)}</span>
                  </button>
                </li>
              );
            })}
          </ul>
          <div className="run-changes__view">
            {activeRef ? <OutputBody instanceId={instanceId} contentRef={activeRef} /> : null}
          </div>
        </div>
      )}
    </section>
  );
}
