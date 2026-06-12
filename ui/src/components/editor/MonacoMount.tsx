/**
 * The lazy boundary for the Monaco editor + a headless fallback.
 *
 * Monaco needs Web Workers + a real layout, neither of which jsdom provides, and it
 * is a multi-MB graph we must not pull into a test or the eager bundle. So:
 *  - the real editor ({@link MonacoEditorImpl}) is imported only via `lazy()`, so it
 *    is a lazy chunk;
 *  - in a headless environment (jsdom — no `Worker`) we render a plain
 *    `<textarea>`/`<pre>` directly and NEVER trigger the lazy import.
 * The fallback keeps the same `value`/`onChange`/test-ids, so component tests drive a
 * real, assertable control and the browser E2E exercises the real Monaco.
 */

import { Suspense, lazy } from "react";
import type { EditorSurfaceProps } from "./editor-surface";

const RealEditor = lazy(() => import("./MonacoEditorImpl"));

/** No Web Worker ⇒ jsdom/SSR ⇒ render the plain fallback, never load Monaco. */
function isHeadless(): boolean {
  return typeof window === "undefined" || typeof Worker === "undefined";
}

function Fallback({
  value,
  readOnly,
  onChange,
  testId,
  ariaLabel,
  id,
  height,
  onSubmit,
  placeholder,
}: EditorSurfaceProps) {
  const style = { height: typeof height === "number" ? `${height}px` : height };
  if (readOnly) {
    return (
      <pre className="editor-surface__fallback mono" data-testid={testId} aria-label={ariaLabel}>
        {value}
      </pre>
    );
  }
  return (
    <textarea
      className="editor-surface__fallback mono"
      id={id}
      data-testid={testId}
      aria-label={ariaLabel}
      value={value}
      style={style}
      spellCheck={false}
      autoComplete="off"
      placeholder={placeholder}
      onChange={(e) => onChange?.(e.target.value)}
      onKeyDown={
        onSubmit === undefined
          ? undefined
          : (e) => {
              // The same contract as the real editor's Enter command: plain
              // Enter submits, Shift+Enter inserts the newline.
              if (e.key === "Enter" && !e.shiftKey) {
                e.preventDefault();
                onSubmit();
              }
            }
      }
    />
  );
}

export function MonacoMount(props: EditorSurfaceProps) {
  if (isHeadless()) {
    return <Fallback {...props} />;
  }
  return (
    <Suspense fallback={<Fallback {...props} />}>
      <RealEditor {...props} />
    </Suspense>
  );
}
