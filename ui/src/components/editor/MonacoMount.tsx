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

import { Suspense, lazy, useEffect, useRef } from "react";
import type { EditorSurfaceProps } from "./editor-surface";

/**
 * Stick a scrollable element to its bottom as content grows, unless the reader has
 * scrolled away. Shared by the fallback here and mirrored by the real editor's
 * `revealLine`, so both surfaces follow a stream the same way.
 */
const NEAR_BOTTOM_PX = 24;

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
  followTail,
}: EditorSurfaceProps) {
  const style = { height: typeof height === "number" ? `${height}px` : height };
  const preRef = useRef<HTMLPreElement>(null);
  // Mirror the real editor's follow behaviour in the headless surface — that is what
  // makes it assertable in a component test instead of only in a browser.
  // biome-ignore lint/correctness/useExhaustiveDependencies: `value` is the TRIGGER, not a read — the effect must re-run each time the streamed text grows, or it follows once and then goes still.
  useEffect(() => {
    const el = preRef.current;
    if (!followTail || el === null) {
      return;
    }
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    if (distance <= NEAR_BOTTOM_PX || el.scrollTop === 0) {
      el.scrollTop = el.scrollHeight;
    }
  }, [value, followTail]);
  if (readOnly) {
    return (
      <pre
        ref={preRef}
        className="editor-surface__fallback mono"
        data-testid={testId}
        aria-label={ariaLabel}
        data-follow-tail={followTail ? "true" : undefined}
        style={style}
      >
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
