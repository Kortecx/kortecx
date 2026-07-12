/**
 * PR-A: the grounded-answer SOURCES disclosure for a read-only RAG turn. Resolves
 * the exact chunk refs the runtime folded into the answer Mote (via
 * {@link useGroundingSources}) and renders them as a compact `<details>` — each a
 * grounding snippet + its content-address chip (the citation). Renders NOTHING when
 * the turn is unsettled, ungrounded, or degraded (empty dataset → no folded refs) —
 * a plain answer never grows a faked citation (don't-fake-gaps). No score is shown
 * (SN-8 — the wire carries none; we never fabricate one).
 */

import { useGroundingSources } from "../../kx/use-grounding-sources";
import { DigestChip } from "../DigestChip";

/** Collapse whitespace + clamp a chunk to a scannable preview (the chip copies the
 *  full ref; the store holds the full bytes). */
const SNIPPET_MAX = 240;
function preview(text: string): { text: string; clipped: boolean } {
  const flat = text.replace(/\s+/g, " ").trim();
  return flat.length > SNIPPET_MAX
    ? { text: flat.slice(0, SNIPPET_MAX), clipped: true }
    : { text: flat, clipped: false };
}

export function MessageSources({
  instanceId,
  moteId,
  active,
}: {
  instanceId: string | undefined;
  moteId: string | undefined;
  /** The turn has settled (`status === "done"`) — gate the RPC + the render. */
  active: boolean;
}) {
  const { sources, truncated } = useGroundingSources(instanceId, moteId, active);
  if (!active || sources.length === 0) {
    return null;
  }
  return (
    <details className="chat-sources" data-testid="chat-sources">
      <summary className="chat-sources__summary">
        Sources <span className="chat-sources__count">{sources.length}</span>
      </summary>
      <ol className="chat-sources__list">
        {sources.map((s, i) => {
          const p = preview(s.snippet);
          return (
            <li key={s.ref} className="chat-sources__item" data-testid="chat-source">
              <div className="chat-sources__head">
                <span className="chat-sources__label">{s.label || `Source ${i + 1}`}</span>
                <DigestChip hex={s.ref} label="source" />
              </div>
              {s.missing ? (
                <p className="chat-sources__snippet muted">(content unavailable)</p>
              ) : p.text ? (
                <p className="chat-sources__snippet" data-testid="chat-source-detail">
                  {p.text}
                  {p.clipped || s.truncated ? <span className="muted"> …</span> : null}
                </p>
              ) : null}
            </li>
          );
        })}
      </ol>
      {truncated ? (
        <p className="chat-sources__more muted" data-testid="chat-sources-truncated">
          Some sources omitted (too many to list).
        </p>
      ) : null}
    </details>
  );
}
