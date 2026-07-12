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

import { Popover } from "../shell/Popover";
import { DatasetPicker } from "./DatasetPicker";

export interface GroundingBarProps {
  readonly dataset: string | undefined;
  readonly onDataset: (dataset: string | undefined) => void;
  /** The party's authored context-bundle handles (the pick list). */
  readonly bundles: readonly string[];
  /** The handles attached to every turn (the removable chips). */
  readonly attached: readonly string[];
  /** The gateway lacks the context-bundle seam (honest-disabled row). */
  readonly notWired: boolean;
  readonly onToggleContext: (handle: string) => void;
}

export function GroundingBar({
  dataset,
  onDataset,
  bundles,
  attached,
  notWired,
  onToggleContext,
}: GroundingBarProps) {
  const n = attached.length;
  return (
    <div className="chat-grounding" data-testid="chat-grounding">
      <div className="chat-grounding__controls">
        <DatasetPicker value={dataset} onChange={onDataset} />
        <Popover
          trigger={<span>+ Context</span>}
          triggerClassName="chip chip--toggle chat-grounding__add"
          triggerLabel="Attach context files"
          triggerTestId="chat-grounding-add"
          align="left"
          direction="down"
          menuTestId="chat-grounding-menu"
        >
          {() =>
            notWired ? (
              <button
                type="button"
                role="menuitem"
                className="popover__item popover__item--disabled"
                data-testid="chat-grounding-not-wired"
                disabled
                aria-disabled="true"
                title="Context bundles need a newer gateway"
              >
                <span>Needs a newer gateway</span>
              </button>
            ) : bundles.length === 0 ? (
              <button
                type="button"
                role="menuitem"
                className="popover__item popover__item--disabled"
                data-testid="chat-grounding-empty"
                disabled
                aria-disabled="true"
                title="Author context files in the Context section"
              >
                <span>No context files — author in Context</span>
              </button>
            ) : (
              bundles.map((handle) => {
                const on = attached.includes(handle);
                return (
                  <button
                    key={handle}
                    type="button"
                    role="menuitemcheckbox"
                    aria-checked={on}
                    className={`popover__item${on ? " popover__item--active" : ""}`}
                    data-testid={`chat-grounding-option-${handle}`}
                    onClick={() => onToggleContext(handle)}
                  >
                    <span className="mono">{handle}</span>
                    {on ? (
                      <span className="popover__check" aria-hidden="true">
                        ✓
                      </span>
                    ) : null}
                  </button>
                );
              })
            )
          }
        </Popover>
      </div>
      <p className="chat-grounding__summary muted" data-testid="chat-grounding-summary">
        {dataset ? (
          <>
            Grounded on dataset <strong>{dataset}</strong>
          </>
        ) : (
          <>Not grounded — pick a dataset to answer from your own data</>
        )}
        {n > 0 ? (
          <>
            {" · "}
            {n} context file{n === 1 ? "" : "s"}
          </>
        ) : null}
      </p>
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
