import { useRef, useState } from "react";
import { IMAGE_ACCEPT } from "../../lib/content-resolver";
import { MonacoMount } from "../editor/MonacoMount";
import { Icon } from "../shell/Icon";

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
            <button
              type="button"
              className="iconbtn composer__attach"
              disabled={disabled}
              onClick={() => fileRef.current?.click()}
              aria-label="Attach an image"
              title="Attach an image"
              data-testid="attach-btn"
            >
              <Icon name="attach" />
            </button>
          </>
        ) : null}
      </div>
    </div>
  );
}
