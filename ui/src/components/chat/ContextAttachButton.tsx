/**
 * PR-F: the "Context" attach control — a header button (next to New chat) that opens
 * a multi-select popover of the party's authored context files (bundles) to ground
 * the read-only chat. Extracted from the grounding bar so it can live in the header;
 * keeps the same testids (`chat-grounding-add` / `-menu` / `-option-<h>` / `-empty` /
 * `-not-wired`). Honest states throughout; read-only (it selects, never mutates).
 */

import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";

export interface ContextAttachButtonProps {
  readonly bundles: readonly string[];
  readonly attached: readonly string[];
  readonly notWired: boolean;
  readonly onToggle: (handle: string) => void;
}

export function ContextAttachButton({
  bundles,
  attached,
  notWired,
  onToggle,
}: ContextAttachButtonProps) {
  const n = attached.length;
  return (
    <Popover
      trigger={
        <>
          <Icon name="context" />
          {n > 0 ? <span className="chat__context-count">{n}</span> : null}
        </>
      }
      triggerClassName="iconbtn chat__context-btn"
      triggerLabel={`Attach context files${n > 0 ? ` (${n} attached)` : ""}`}
      triggerTestId="chat-grounding-add"
      align="right"
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
            title="Context files need a newer gateway"
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
                onClick={() => onToggle(handle)}
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
  );
}
