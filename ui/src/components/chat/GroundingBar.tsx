/**
 * PR-A: the read-only RAG chat's GROUNDING BAR — makes selecting the grounding
 * DATASET + CONTEXT FILES the headline affordance (vs a control buried in the
 * composer's attach menu). Picking a dataset routes each turn to
 * `kx/recipes/chat-rag`; attached context bundles fold into every turn. Honest-
 * degrade: no `hnsw` / no datasets ⇒ the {@link DatasetPicker} renders nothing; the
 * summary states plainly whether the chat is grounded (and the composer copy already
 * notes a plain answer when the picked dataset is empty). Read-only: it SELECTS what
 * to retrieve over; it never mutates the corpora.
 */

import { DatasetPicker } from "./DatasetPicker";

export interface GroundingBarProps {
  readonly dataset: string | undefined;
  readonly onDataset: (dataset: string | undefined) => void;
  /** The context files attached to every turn (the removable chips). */
  readonly attached: readonly string[];
  readonly onToggleContext: (handle: string) => void;
}

export function GroundingBar({ dataset, onDataset, attached, onToggleContext }: GroundingBarProps) {
  const n = attached.length;
  return (
    <div className="chat-grounding" data-testid="chat-grounding">
      <div className="chat-grounding__controls">
        <DatasetPicker value={dataset} onChange={onDataset} />
      </div>
      {dataset || n > 0 ? (
        <p className="chat-grounding__summary muted" data-testid="chat-grounding-summary">
          {dataset ? (
            <>
              Grounded on dataset <strong data-testid="chat-grounded-on">{dataset}</strong>
            </>
          ) : null}
          {dataset && n > 0 ? " · " : null}
          {n > 0 ? (
            <>
              {n} context file{n === 1 ? "" : "s"}
            </>
          ) : null}
        </p>
      ) : null}
      {n > 0 ? (
        <div className="context-strip" data-testid="chat-grounding-context-strip">
          {attached.map((handle) => (
            <span
              key={handle}
              className="context-strip__chip"
              data-testid={`chat-grounding-context-${handle}`}
            >
              <span className="mono">{handle}</span>
              <button
                type="button"
                className="context-strip__remove"
                aria-label={`Detach ${handle}`}
                data-testid={`chat-grounding-context-remove-${handle}`}
                onClick={() => onToggleContext(handle)}
              >
                ✕
              </button>
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
}
