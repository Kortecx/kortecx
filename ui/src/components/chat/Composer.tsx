import { useRef, useState } from "react";
import { IMAGE_ACCEPT } from "../../lib/content-resolver";
import { MonacoMount } from "../editor/MonacoMount";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";

/** Attach categories not yet wired — shown in the menu as honest-disabled rows so
 *  the surface is complete but never fakes a capability that does not exist yet.
 *  Context and Tools are LIVE and rendered separately. */
const SOON_CATEGORIES: ReadonlyArray<{ label: string; testId: string }> = [
  { label: "Blueprint", testId: "attach-blueprint" },
  { label: "Dataset", testId: "attach-dataset" },
];

/** PR-7b: the context-bundle picker state the parent (ChatPanel) owns + passes in.
 *  `bundles` are the party's authored handles; toggling attaches/detaches a handle
 *  to the NEXT turn (multi-select — the menu stays open). Absent ⇒ no picker. */
export interface ContextPickerProps {
  readonly bundles: readonly string[];
  readonly attached: readonly string[];
  readonly notWired: boolean;
  readonly onToggle: (handle: string) => void;
}

/** The tool picker state the parent owns + passes in. `options` are the fireable
 *  registry tools keyed `${toolName}@${toolVersion}`; toggling attaches/detaches
 *  one to the NEXT turn (multi-select — the menu stays open). A picked tool is a
 *  request, never a fire guarantee — authority is re-checked per turn. Absent ⇒
 *  no picker. */
export interface ToolPickerProps {
  readonly options: readonly string[];
  readonly attached: readonly string[];
  readonly notWired: boolean;
  readonly onToggle: (id: string) => void;
}

/**
 * The message input — a Monaco surface with MARKDOWN highlighting (D141.2's
 * last exception closed; jsdom renders the textarea fallback with the same
 * test handles). Plain Enter sends; Shift+Enter inserts a newline. The action
 * column sits on the right: send on top, attach BELOW it (user-directed
 * 2026-06-12 review feedback); the parent owns uploads + the pending strip.
 */
export function Composer({
  disabled,
  sendBlocked,
  onSend,
  onPickFiles,
  context,
  tools,
}: {
  disabled: boolean;
  /** Extra send-only block (e.g. an attachment upload still in flight). */
  sendBlocked?: boolean;
  onSend: (text: string) => void;
  onPickFiles?: (files: ArrayLike<File>) => void;
  /** PR-7b: the context-bundle picker (the attach-menu "Context" category). */
  context?: ContextPickerProps;
  /** The tool picker (the attach-menu "Tools" category). */
  tools?: ToolPickerProps;
}) {
  const [text, setText] = useState("");
  const fileRef = useRef<HTMLInputElement>(null);

  function doSend(): void {
    const t = text.trim();
    if (t !== "" && !disabled && !sendBlocked) {
      onSend(t);
      setText("");
    }
  }

  return (
    <div className="composer" data-testid="composer">
      <div className="composer__editor">
        <MonacoMount
          value={text}
          language="markdown"
          onChange={setText}
          onSubmit={doSend}
          readOnly={disabled}
          height={96}
          testId="composer-input"
          ariaLabel="Message"
          placeholder="Message the runtime… (markdown; Shift+Enter for a newline)"
        />
      </div>
      <div className="composer__actions">
        <button
          type="button"
          className="iconbtn composer__send"
          disabled={disabled || sendBlocked || text.trim() === ""}
          onClick={doSend}
          aria-label="Send message"
          data-testid="send"
        >
          <Icon name="send" />
        </button>
        {onPickFiles ? (
          <>
            <input
              ref={fileRef}
              type="file"
              accept={IMAGE_ACCEPT}
              multiple
              hidden
              data-testid="attach-input"
              onChange={(e) => {
                if (e.target.files && e.target.files.length > 0) {
                  onPickFiles(e.target.files);
                  e.target.value = ""; // re-picking the same file must re-fire
                }
              }}
            />
            <Popover
              trigger={<Icon name="attach" />}
              triggerClassName="iconbtn composer__attach"
              triggerLabel="Attach"
              triggerTestId="attach-btn"
              triggerDisabled={disabled}
              align="right"
              menuTestId="attach-menu"
            >
              {(close) => (
                <>
                  <button
                    type="button"
                    role="menuitem"
                    className="popover__item"
                    data-testid="attach-upload"
                    onClick={() => {
                      close();
                      fileRef.current?.click();
                    }}
                  >
                    <Icon name="attach" size={15} />
                    <span>Upload a file</span>
                  </button>
                  {/* PR-7b: the LIVE Context category — pick authored bundles to
                      ground the next turn (multi-select; the menu stays open). */}
                  {context ? (
                    <div className="popover__group" data-testid="attach-context-group">
                      <div className="popover__group-label">Context</div>
                      {context.notWired ? (
                        <button
                          type="button"
                          role="menuitem"
                          className="popover__item popover__item--disabled"
                          data-testid="attach-context-not-wired"
                          disabled
                          aria-disabled="true"
                          title="Context bundles need a newer gateway"
                        >
                          <span>Needs a newer gateway</span>
                        </button>
                      ) : context.bundles.length === 0 ? (
                        <button
                          type="button"
                          role="menuitem"
                          className="popover__item popover__item--disabled"
                          data-testid="attach-context-empty"
                          disabled
                          aria-disabled="true"
                          title="Author bundles in the Context section"
                        >
                          <span>No bundles — author in Context</span>
                        </button>
                      ) : (
                        context.bundles.map((handle) => {
                          const on = context.attached.includes(handle);
                          return (
                            <button
                              key={handle}
                              type="button"
                              role="menuitemcheckbox"
                              aria-checked={on}
                              className={`popover__item${on ? " popover__item--active" : ""}`}
                              data-testid={`attach-context-option-${handle}`}
                              onClick={() => context.onToggle(handle)}
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
                      )}
                    </div>
                  ) : null}
                  {/* The LIVE Tools category — attach fireable tools to the next
                      turn (multi-select; the menu stays open). A picked tool is a
                      request, not a fire guarantee. */}
                  {tools ? (
                    <div className="popover__group" data-testid="attach-tool-group">
                      <div className="popover__group-label">Tools</div>
                      {tools.notWired ? (
                        <button
                          type="button"
                          role="menuitem"
                          className="popover__item popover__item--disabled"
                          data-testid="attach-tool-not-wired"
                          disabled
                          aria-disabled="true"
                          title="Tool discovery needs a newer gateway"
                        >
                          <span>Needs a newer gateway</span>
                        </button>
                      ) : tools.options.length === 0 ? (
                        <button
                          type="button"
                          role="menuitem"
                          className="popover__item popover__item--disabled"
                          data-testid="attach-tool-empty"
                          disabled
                          aria-disabled="true"
                          title="Register tools in the Integrations section"
                        >
                          <span>No tools — add in Integrations</span>
                        </button>
                      ) : (
                        tools.options.map((id) => {
                          const on = tools.attached.includes(id);
                          return (
                            <button
                              key={id}
                              type="button"
                              role="menuitemcheckbox"
                              aria-checked={on}
                              className={`popover__item${on ? " popover__item--active" : ""}`}
                              data-testid={`attach-tool-option-${id}`}
                              onClick={() => tools.onToggle(id)}
                            >
                              <span className="mono">{id}</span>
                              {on ? (
                                <span className="popover__check" aria-hidden="true">
                                  ✓
                                </span>
                              ) : null}
                            </button>
                          );
                        })
                      )}
                    </div>
                  ) : null}
                  {/* Attaching a Blueprint/Dataset as message context is not yet
                      wired — shown but honest-disabled so the menu is complete
                      without faking the gap. */}
                  {SOON_CATEGORIES.map((c) => (
                    <button
                      key={c.testId}
                      type="button"
                      role="menuitem"
                      className="popover__item popover__item--disabled"
                      data-testid={c.testId}
                      disabled
                      aria-disabled="true"
                      title="Attach as context — available in a future release"
                    >
                      <span>{c.label}</span>
                      <span className="chip chip--soon">Soon</span>
                    </button>
                  ))}
                </>
              )}
            </Popover>
          </>
        ) : null}
      </div>
    </div>
  );
}
