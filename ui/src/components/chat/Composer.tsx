import { type FormEvent, type KeyboardEvent, useRef, useState } from "react";
import { IMAGE_ACCEPT } from "../../lib/content-resolver";
import { Icon } from "../shell/Icon";

/** The message input. Enter sends; Shift+Enter inserts a newline. The attach
 *  button (Batch A) picks image files; the parent owns the upload + strip. */
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

  function submit(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    doSend();
  }

  function onKeyDown(e: KeyboardEvent<HTMLTextAreaElement>): void {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      doSend();
    }
  }

  return (
    <form className="composer" onSubmit={submit} data-testid="composer">
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
      <textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={onKeyDown}
        rows={2}
        placeholder="Message the runtime…"
        aria-label="Message"
        spellCheck={false}
        disabled={disabled}
      />
      <button
        type="submit"
        className="iconbtn composer__send"
        disabled={disabled || sendBlocked || text.trim() === ""}
        aria-label="Send message"
        data-testid="send"
      >
        <Icon name="send" />
      </button>
    </form>
  );
}
