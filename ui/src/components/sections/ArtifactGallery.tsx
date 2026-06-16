/**
 * Browse one run's committed artifacts (UI-2). The run's projection yields each
 * committed Mote's `result_ref`; we list them and fetch + decode ONE on demand
 * (lazy — bounded by clicks, and the content cache is immutable so re-opening is
 * free). Rendering is the fail-closed `ArtifactView` (text / JSON / bounded hex —
 * never innerHTML); download saves the rendered text.
 */

import { m } from "framer-motion";
import { useMemo, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useContent } from "../../kx/use-content";
import { useResultMap } from "../../kx/use-content-batch";
import { useRunArtifacts } from "../../kx/use-run-artifacts";
import { artifactKindVisual } from "../../lib/artifact-kind";
import type { DecodedContent } from "../../lib/content-decode";
import { shortHex } from "../../lib/format";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { ResultPreview } from "../ResultPreview";
import { ArtifactView } from "./ArtifactView";

export function ArtifactGallery({ instanceId }: { instanceId: string }) {
  const { artifacts, isLoading, error, refetch } = useRunArtifacts(instanceId);
  // Track the open row by its MOTE id, not its result_ref: distinct Motes can
  // legitimately commit IDENTICAL content (content-addressing dedups it to one
  // ref — e.g. a fan-out of honest PURE passthrough Motes), so a result_ref is
  // NOT a unique row key. Keying by the unique moteId keeps each committed output
  // its own independently-expandable row.
  const [openMote, setOpenMote] = useState<string | null>(null);
  // Batch-resolve every artifact's text for the list headline (one RPC, the N+1
  // collapse) — the full payload still opens lazily on click below.
  const refs = useMemo(() => artifacts.map((a) => a.resultRef), [artifacts]);
  const { byRef, isLoading: previewsLoading } = useResultMap(instanceId, refs);

  if (isLoading) {
    return <EmptyState title="Loading run…" />;
  }
  if (error) {
    return <ErrorNotice error={toUiError(error)} onRetry={refetch} />;
  }
  if (artifacts.length === 0) {
    return (
      <EmptyState
        title="No artifacts yet"
        detail="This run has no committed outputs yet — they appear here as its Motes commit."
      />
    );
  }

  return (
    <div data-testid="artifact-gallery">
      <p className="muted">
        {artifacts.length} committed {artifacts.length === 1 ? "output" : "outputs"} · select one to
        review
      </p>
      <m.ul className="artifact-list" variants={stagger()} initial="hidden" animate="show">
        {artifacts.map((a) => {
          const open = openMote === a.moteId;
          const vm = byRef.get(a.resultRef);
          return (
            <m.li
              className="artifact-list__item card-hover"
              key={a.moteId}
              variants={fadeUp}
              {...hoverLift}
            >
              <div className="artifact-list__row">
                <button
                  type="button"
                  className="artifact-list__toggle"
                  data-testid={`artifact-${a.moteId}`}
                  aria-expanded={open}
                  onClick={() => setOpenMote(open ? null : a.moteId)}
                >
                  <span className="artifact-list__mote mono">{shortHex(a.moteId)}</span>
                  <span className="muted" aria-hidden="true">
                    →
                  </span>
                  {/* Resolved text is the headline; the chip rides as a sibling
                      (a DigestChip button can't nest in this toggle button). */}
                  <ResultPreview
                    resultRef={a.resultRef}
                    content={vm?.content}
                    missing={vm?.missing ?? false}
                    loading={previewsLoading}
                    max={120}
                    chip={false}
                  />
                </button>
                <DigestChip hex={a.resultRef} label="result" />
              </div>
              {open ? <ArtifactCard instanceId={instanceId} contentRef={a.resultRef} /> : null}
            </m.li>
          );
        })}
      </m.ul>
    </div>
  );
}

function ArtifactCard({ instanceId, contentRef }: { instanceId: string; contentRef: string }) {
  const content = useContent(instanceId, contentRef);
  if (content.isLoading) {
    return <EmptyState title="Loading artifact…" />;
  }
  if (content.error) {
    return <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />;
  }
  if (!content.data) {
    return null;
  }
  return <ArtifactCardBody data={content.data} contentRef={contentRef} />;
}

function ArtifactCardBody({
  data,
  contentRef,
}: {
  data: DecodedContent;
  contentRef: string;
}) {
  const kind = artifactKindVisual(data.kind);
  return (
    <div className="artifact-card">
      <div className="artifact-card__head">
        <span className="artifact-card__kind" aria-hidden="true">
          {kind.glyph}
        </span>
        <span>{kind.label}</span>
      </div>
      {/* The AssetViewer owns the per-asset download (bytes for media, text for
          text/json) — so a media artifact downloads its real bytes, not a hex preview. */}
      <ArtifactView content={data} stem={contentRef.slice(0, 12)} />
    </div>
  );
}
