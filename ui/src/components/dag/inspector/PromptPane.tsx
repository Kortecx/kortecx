/**
 * The inspector's Prompt pane (PR-2): the Mote's admitted instruction text
 * (`config_subset["prompt"]` via `GetMoteDetail`), rendered READ-ONLY in the
 * Monaco viewer (D141.2). Truncation is honest (the server caps the field).
 */

import type { MoteDetailVM } from "../../../kx/use-mote-detail";
import { EmptyState } from "../../EmptyState";
import { CodeViewer } from "../../editor/CodeViewer";

export function PromptPane({ detail }: { detail: MoteDetailVM }) {
  if (!detail.prompt) {
    return (
      <EmptyState
        title="No prompt"
        detail="This Mote's definition carries no instruction text (not a model step)."
      />
    );
  }
  return (
    <div data-testid="inspector-prompt">
      {detail.modelId ? (
        <p className="muted">
          Model <code className="mono">{detail.modelId}</code>
        </p>
      ) : null}
      {detail.promptTruncated ? (
        <p className="muted">
          Truncated server-side (display cap) — the admitted prompt is longer.
        </p>
      ) : null}
      <CodeViewer
        value={detail.prompt}
        language="plaintext"
        testId="inspector-prompt-text"
        ariaLabel="Admitted prompt"
        height={Math.min(320, Math.max(96, detail.prompt.split("\n").length * 19 + 24))}
      />
    </div>
  );
}
