import { useRef, useState } from "react";
import { IMAGE_ACCEPT } from "../../lib/content-resolver";
import { MonacoMount } from "../editor/MonacoMount";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";

/** Attach categories that need Context bundles (PR-7) to ride a message — shown
 *  in the menu as honest-disabled rows so the surface is complete but never fakes
 *  a capability that does not exist yet (D142 don't-fake-gaps). */
const SOON_CATEGORIES: ReadonlyArray<{ label: string; testId: string }> = [
  { label: "Blueprint", testId: "attach-blueprint" },
  { label: "Dataset", testId: "attach-dataset" },
  { label: "Tool", testId: "attach-tool" },
  { label: "Context", testId: "attach-context" },
];

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
}: {
  disabled: boolean;
  /** Extra send-only block (e.g. an attachment upload still in flight). */
  sendBlocked?: boolean;
  onSend: (text: string) => void;
  onPickFiles?: (files: ArrayLike<File>) => void;
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
                  {/* Attaching a Blueprint/Dataset/Tool/Context as message context
                      rides the Context-bundle work (PR-7) — shown but honest-
                      disabled so the menu is complete without faking the gap. */}
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
