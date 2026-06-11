/**
 * An editable JSON control backed by Monaco (offline, lazy). A drop-in for the raw
 * `<textarea>` JSON-args input — it owns ONLY the editing surface; validation stays
 * in the parent's submit handler (the existing `JSON.parse` + `field-error` UX is
 * unchanged). In jsdom it renders a `<textarea id data-testid>` so the manual-invoke
 * tests keep driving it by the same handles.
 */

import { MonacoMount } from "./MonacoMount";

export function JsonEditor({
  id,
  value,
  onChange,
  testId = "args",
  ariaLabel = "Args (JSON object)",
  height = 180,
}: {
  id?: string;
  value: string;
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
        language="json"
        onChange={onChange}
        testId={testId}
        ariaLabel={ariaLabel}
        height={height}
      />
    </div>
  );
}
