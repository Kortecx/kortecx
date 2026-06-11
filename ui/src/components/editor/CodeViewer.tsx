/**
 * A read-only, syntax-highlighted viewer backed by Monaco (offline, lazy). Used for
 * committed artifact payloads, run results, and the DAG node-detail. Never executes
 * or `innerHTML`s its input — Monaco renders the text as code. The language is
 * inferred (json vs plaintext) unless the caller pins it. In jsdom it renders a
 * `<pre data-testid>` so component tests assert the text content directly.
 */

import { type MonacoLanguage, inferLanguage } from "../../lib/monaco/infer-language";
import { MonacoMount } from "./MonacoMount";

export function CodeViewer({
  value,
  language,
  height = 240,
  testId,
  ariaLabel = "Code viewer",
}: {
  value: string;
  /** Pin the language; omit to infer json/plaintext from the value. */
  language?: MonacoLanguage;
  height?: number | string;
  testId?: string;
  ariaLabel?: string;
}) {
  const lang = language ?? inferLanguage(value);
  return (
    <div className="editor-surface editor-surface--view">
      <MonacoMount
        value={value}
        language={lang}
        readOnly
        height={height}
        testId={testId}
        ariaLabel={ariaLabel}
      />
    </div>
  );
}
