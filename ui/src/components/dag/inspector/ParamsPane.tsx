/**
 * The inspector's Params pane (PR-2): the Mote's curated `config_subset`
 * (behavior-affecting keys only; the prompt has its own pane). Values are
 * OPAQUE display bytes — rendered as UTF-8 when they decode, hex otherwise;
 * multi-line values get the Monaco viewer (D141.2). Server-side truncation
 * stays honest per entry.
 */

import type { MoteDetailVM } from "../../../kx/use-mote-detail";
import { EmptyState } from "../../EmptyState";
import { CodeViewer } from "../../editor/CodeViewer";

function decodeValue(value: Uint8Array): { text: string; hex: boolean } {
  try {
    return { text: new TextDecoder("utf-8", { fatal: true }).decode(value), hex: false };
  } catch {
    return {
      text: Array.from(value, (b) => b.toString(16).padStart(2, "0")).join(""),
      hex: true,
    };
  }
}

export function ParamsPane({ detail }: { detail: MoteDetailVM }) {
  if (detail.configSubset.length === 0) {
    return (
      <EmptyState
        title="No params"
        detail="This Mote's definition declares no behavior-affecting config."
      />
    );
  }
  return (
    <dl className="node-drawer__meta" data-testid="inspector-params">
      {detail.configSubset.map((entry) => {
        const { text, hex } = decodeValue(entry.value);
        const multiline = text.includes("\n") || text.length > 120;
        return (
          <div key={entry.key}>
            <dt>
              {entry.key}
              {entry.truncated ? (
                <span className="muted">
                  {" "}
                  (+{entry.fullLen - entry.value.length} bytes truncated)
                </span>
              ) : null}
            </dt>
            <dd className="mono">
              {multiline ? (
                <CodeViewer
                  value={text}
                  language="plaintext"
                  testId={`inspector-param-${entry.key}`}
                  ariaLabel={`Param ${entry.key}`}
                  height={Math.min(200, Math.max(60, text.split("\n").length * 19 + 24))}
                />
              ) : (
                <code title={hex ? "binary value (hex)" : undefined}>
                  {hex ? `0x${text}` : text}
                </code>
              )}
            </dd>
          </div>
        );
      })}
    </dl>
  );
}
