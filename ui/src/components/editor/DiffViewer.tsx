/**
 * POC-5d: the agentic-edit REVIEW gate's diff surface — the user reviews the model's
 * proposed file body against the current body before approving (closes
 * `T-AGENTIC-EDIT-REVIEW-GATE`). Mirrors {@link MonacoMount}: the real Monaco
 * `DiffEditor` is a lazy chunk reached only in a browser; in a headless environment
 * (jsdom — no `Worker`) we render a deterministic LCS line-diff `<pre>` so component
 * tests can assert added/removed lines and the browser E2E exercises real Monaco.
 */

import { Suspense, lazy, useMemo } from "react";
import type { MonacoLanguage } from "../../lib/monaco/infer-language";

const RealDiff = lazy(() => import("./DiffViewerImpl"));

function isHeadless(): boolean {
  return typeof window === "undefined" || typeof Worker === "undefined";
}

/** One line of a unified diff. */
export interface DiffLine {
  readonly kind: "same" | "add" | "del";
  readonly text: string;
}

/**
 * A pure LCS line diff (original → modified). Deterministic + total; the headless
 * fallback + tests assert it directly. Classic dynamic-programming LCS over lines.
 */
export function lineDiff(original: string, modified: string): DiffLine[] {
  const a = original.split("\n");
  const b = modified.split("\n");
  const n = a.length;
  const m = b.length;
  const at = (xs: string[], k: number): string => xs[k] ?? "";
  // lcs[i][j] = LCS length of a[i:] and b[j:].
  const lcs: number[][] = Array.from({ length: n + 1 }, () => new Array<number>(m + 1).fill(0));
  const cell = (i: number, j: number): number => lcs[i]?.[j] ?? 0;
  for (let i = n - 1; i >= 0; i--) {
    const row = lcs[i] as number[];
    for (let j = m - 1; j >= 0; j--) {
      row[j] =
        at(a, i) === at(b, j) ? cell(i + 1, j + 1) + 1 : Math.max(cell(i + 1, j), cell(i, j + 1));
    }
  }
  const out: DiffLine[] = [];
  let i = 0;
  let j = 0;
  while (i < n && j < m) {
    if (at(a, i) === at(b, j)) {
      out.push({ kind: "same", text: at(a, i) });
      i++;
      j++;
    } else if (cell(i + 1, j) >= cell(i, j + 1)) {
      out.push({ kind: "del", text: at(a, i) });
      i++;
    } else {
      out.push({ kind: "add", text: at(b, j) });
      j++;
    }
  }
  while (i < n) {
    out.push({ kind: "del", text: at(a, i++) });
  }
  while (j < m) {
    out.push({ kind: "add", text: at(b, j++) });
  }
  return out;
}

/** `true` when the two texts are identical (a no-op edit — Approve is disabled). */
export function isNoOpDiff(original: string, modified: string): boolean {
  return original === modified;
}

export interface DiffViewerProps {
  readonly original: string;
  readonly modified: string;
  readonly language: MonacoLanguage;
  readonly height?: number | string;
  readonly testId?: string;
  readonly ariaLabel?: string;
}

function FallbackDiff({ original, modified, testId, ariaLabel }: DiffViewerProps) {
  const lines = useMemo(() => lineDiff(original, modified), [original, modified]);
  if (isNoOpDiff(original, modified)) {
    return (
      <p className="muted" data-testid="app-edit-noop" role="note">
        The proposed edit is identical to the current file.
      </p>
    );
  }
  return (
    <pre
      className="diff-fallback mono"
      data-testid={testId ?? "app-diff-fallback"}
      aria-label={ariaLabel}
    >
      {lines.map((l, idx) => (
        <span
          // biome-ignore lint/suspicious/noArrayIndexKey: line order is the identity
          key={idx}
          className={`diff-line diff-line--${l.kind}`}
          data-diff-kind={l.kind}
        >
          {l.kind === "add" ? "+ " : l.kind === "del" ? "- " : "  "}
          {l.text}
          {"\n"}
        </span>
      ))}
    </pre>
  );
}

export function DiffViewer(props: DiffViewerProps) {
  if (isHeadless()) {
    return <FallbackDiff {...props} />;
  }
  return (
    <Suspense fallback={<FallbackDiff {...props} />}>
      <RealDiff {...props} />
    </Suspense>
  );
}
