/**
 * The REAL Monaco wrapper — the single module that imports the heavy editor graph
 * (`@monaco-editor/react` + the offline `setup`). It is reached ONLY via
 * `lazy(() => import("./MonacoEditorImpl"))` from {@link MonacoMount}, so Rollup
 * emits it (and the worker chunks) as a LAZY chunk that never enters the eager
 * bundle the size gate measures. Never import this module statically.
 */

import Editor, { type OnMount } from "@monaco-editor/react";
import { useRef } from "react";
import { useTheme } from "../../app/use-theme";
import { KX_DARK, KX_LIGHT, configureMonacoOnce } from "../../lib/monaco/setup";
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
  onSubmit,
  placeholder,
}: EditorSurfaceProps) {
  const { resolved } = useTheme();
  // The Enter command is registered ONCE on mount; route it through a ref so
  // the latest submit callback fires without re-registering per render.
  const submitRef = useRef(onSubmit);
  submitRef.current = onSubmit;

  const handleMount: OnMount = (editor, monaco) => {
    if (onSubmit !== undefined) {
      // Plain Enter submits (the chat-composer contract); Shift+Enter keeps the
      // DEFAULT newline binding (a distinct keybinding, untouched).
      editor.addCommand(monaco.KeyCode.Enter, () => submitRef.current?.());
    }
  };

  return (
    <div className="monaco-host" data-testid={testId} aria-label={ariaLabel}>
      <Editor
        value={value}
        language={language}
        theme={resolved === "dark" ? KX_DARK : KX_LIGHT}
        height={height}
        onChange={(v) => onChange?.(v ?? "")}
        onMount={handleMount}
        options={{
          ...FIXED_OPTIONS,
          readOnly,
          domReadOnly: readOnly,
          lineNumbers: readOnly ? "on" : "on",
          placeholder,
        }}
      />
    </div>
  );
}
