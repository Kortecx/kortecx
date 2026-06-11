/**
 * The REAL Monaco wrapper — the single module that imports the heavy editor graph
 * (`@monaco-editor/react` + the offline `setup`). It is reached ONLY via
 * `lazy(() => import("./MonacoEditorImpl"))` from {@link MonacoMount}, so Rollup
 * emits it (and the worker chunks) as a LAZY chunk that never enters the eager
 * bundle the size gate measures. Never import this module statically.
 */

import Editor from "@monaco-editor/react";
import { KX_LIGHT, configureMonacoOnce } from "../../lib/monaco/setup";
import type { EditorSurfaceProps } from "./editor-surface";

// Point @monaco-editor/react at the bundled (offline) instance + theme before any
// editor mounts. Module-top so it runs once when this lazy chunk first loads.
configureMonacoOnce();

const FIXED_OPTIONS = {
  minimap: { enabled: false },
  scrollBeyondLastLine: false,
  automaticLayout: true,
  fontFamily: "var(--font-mono)",
  fontSize: 13,
  lineNumbersMinChars: 3,
  folding: false,
  renderLineHighlight: "line",
  scrollbar: { verticalScrollbarSize: 8, horizontalScrollbarSize: 8 },
  overviewRulerLanes: 0,
  wordWrap: "on",
  tabSize: 2,
} as const;

export default function MonacoEditorImpl({
  value,
  language,
  readOnly = false,
  onChange,
  height = 220,
  testId,
  ariaLabel,
}: EditorSurfaceProps) {
  return (
    <div className="monaco-host" data-testid={testId} aria-label={ariaLabel}>
      <Editor
        value={value}
        language={language}
        theme={KX_LIGHT}
        height={height}
        onChange={(v) => onChange?.(v ?? "")}
        options={{
          ...FIXED_OPTIONS,
          readOnly,
          domReadOnly: readOnly,
          lineNumbers: readOnly ? "on" : "on",
        }}
      />
    </div>
  );
}
