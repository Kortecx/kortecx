import type { DecodedContent } from "../../lib/content-decode";
import { EmptyState } from "../EmptyState";

/** Render one decoded artifact blob (pretty JSON / text / bounded hex preview). */
export function ArtifactView({ content }: { content: DecodedContent }) {
  return (
    <div className="artifact" data-testid="artifact-view" data-kind={content.kind}>
      <p className="muted">
        {content.kind} · {content.byteLength} bytes
        {content.truncated ? " · preview truncated" : ""}
      </p>
      {content.kind === "empty" ? (
        <EmptyState title="Empty artifact" detail="This committed Mote produced no output." />
      ) : (
        <pre className="artifact__body">{content.text}</pre>
      )}
    </div>
  );
}
