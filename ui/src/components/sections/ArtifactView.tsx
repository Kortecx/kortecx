import type { DecodedContent } from "../../lib/content-decode";
import { EmptyState } from "../EmptyState";
import { CodeViewer } from "../editor/CodeViewer";

/** Render one decoded artifact blob (pretty JSON / text / bounded hex preview). A
 *  syntax-highlighted, read-only Monaco viewer for non-empty payloads (offline +
 *  lazy; jsdom degrades to a `<pre>` carrying the same test handle). */
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
        <CodeViewer
          value={content.text}
          language={content.kind === "json" ? "json" : "plaintext"}
          testId="artifact-view-body"
          ariaLabel="Artifact content"
          height={Math.min(420, Math.max(120, content.text.split("\n").length * 19 + 24))}
        />
      )}
    </div>
  );
}
