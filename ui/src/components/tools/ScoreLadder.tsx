import type { BundleScore } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { progressBar } from "../../app/motion";
import { Badge } from "../ds/Badge";

/**
 * Rung label from the advisory basis points. Only the 10000 ceiling proves an
 * EXACT keyword/phrase hit; below it the wire does not say WHICH rung produced
 * the number (Jaro-Winkler tops out at 9000, embedding cosine at 8000 — the
 * bands overlap), so anything else is honestly just "Similar".
 */
export function rungLabel(scoreBp: number): string {
  if (scoreBp === 10000) {
    return "Exact";
  }
  if (scoreBp > 0) {
    return "Similar";
  }
  return "—";
}

const VERDICTS: Record<string, { label: string; color: string }> = {
  "would-lower": { label: "Would lower", color: "var(--success)" },
  unavailable: { label: "No live model", color: "var(--warning)" },
  refused: { label: "Refused", color: "var(--error)" },
};

/**
 * The advisory ranking ladder for a scored bundle: every registered manifest
 * best-first with its bp score, the bundle's content fingerprint, and the
 * lowering-gate DRY-RUN verdict. ALL display-only (SN-8) — a score can surface
 * a tool, never grant one; the broker re-gates any real dispatch.
 */
export function ScoreLadder({ score }: { score: BundleScore }) {
  const verdict = VERDICTS[score.verdict] ?? { label: "Unknown", color: "var(--text-3)" };
  const showDetail = score.verdict !== "would-lower" && score.verdictDetail.length > 0;
  return (
    <div className="score-ladder" data-testid="score-ladder">
      <h2>Advisory ranking</h2>
      {score.ranked.map((row) => (
        <div
          key={`${row.toolId}@${row.toolVersion}`}
          className="score-row"
          data-testid={`score-row-${row.toolId}`}
        >
          <span className="score-row__name mono">{row.toolId}</span>
          <div className="progress score-row__bar">
            <m.div className="progress-fill" {...progressBar(row.scoreBp / 100)} />
          </div>
          <span className="score-row__bp mono">{row.scoreBp}</span>
          <span className="score-row__rung section-label">{rungLabel(row.scoreBp)}</span>
        </div>
      ))}
      <dl className="facts">
        <dt>Bundle fingerprint</dt>
        <dd className="mono bundle-fingerprint" data-testid="bundle-fingerprint">
          {score.bundleFingerprint}
        </dd>
        <dt>Lowering dry-run</dt>
        <dd>
          <span data-testid="verdict-badge">
            <Badge label={verdict.label} color={verdict.color} dot />
          </span>
          {showDetail ? <p className="muted score-ladder__detail">{score.verdictDetail}</p> : null}
        </dd>
      </dl>
      <p className="muted">Advisory only — scores never authorize.</p>
    </div>
  );
}
