import type { DecodedContent } from "../../lib/content-decode";
import { AssetViewer } from "../AssetViewer";

/** Render one decoded artifact blob. Delegates to the shared {@link AssetViewer}
 *  (image/video/audio/markdown/json/text/binary) and keeps the `artifact-view`
 *  wrapper + caption that the artifact e2e selects on. */
export function ArtifactView({ content, stem }: { content: DecodedContent; stem?: string }) {
  return (
    <div className="artifact" data-testid="artifact-view" data-kind={content.kind}>
      <p className="muted">
        {content.kind} · {content.byteLength} bytes
        {content.truncated ? " · preview truncated" : ""}
      </p>
      <AssetViewer content={content} stem={stem ?? "artifact"} bodyTestId="artifact-view-body" />
    </div>
  );
}
