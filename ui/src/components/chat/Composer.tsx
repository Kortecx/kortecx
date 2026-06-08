import { type FormEvent, type KeyboardEvent, useState } from "react";
import { Icon } from "../shell/Icon";

/** The message input. Enter sends; Shift+Enter inserts a newline. */
export function Composer({
  disabled,
  onSend,
}: {
  disabled: boolean;
  onSend: (text: string) => void;
}) {
  const [text, setText] = useState("");

  function doSend(): void {
    const t = text.trim();
    if (t !== "" && !disabled) {
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
        disabled={disabled || text.trim() === ""}
        aria-label="Send message"
        data-testid="send"
      >
        <Icon name="send" />
      </button>
    </form>
  );
}
