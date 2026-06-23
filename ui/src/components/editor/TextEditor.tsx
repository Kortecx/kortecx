/**
 * An editable text/markdown/JSON control backed by Monaco (offline, lazy) — the
 * editable sibling of the read-only {@link CodeViewer}, used by the POC-2 context
 * item editor. It owns ONLY the editing surface; the parent owns Save/Cancel +
 * any validation. In jsdom it renders a `<textarea id data-testid>` so component
 * tests drive it by the same handles (Playwright drives it by keyboard, not
 * `fill()` — Monaco ignores programmatic value sets).
 */

import type { MonacoLanguage } from "../../lib/monaco/infer-language";
import { MonacoMount } from "./MonacoMount";

export function TextEditor({
  id,
  value,
  language = "plaintext",
  onChange,
  testId = "text-editor",
  ariaLabel = "Editable content",
  height = 280,
}: {
  id?: string;
  value: string;
  language?: MonacoLanguage;
  onChange: (value: string) => void;
  testId?: string;
  ariaLabel?: string;
  height?: number;
}) {
  return (
    <div className="editor-surface editor-surface--edit">
      <MonacoMount
        id={id}
        value={value}
        language={language}
        onChange={onChange}
        testId={testId}
        ariaLabel={ariaLabel}
        height={height}
      />
    </div>
  );
}
