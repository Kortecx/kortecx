/**
 * PR-B: the swarm RUN roll-up — a compact strip above the DAG that makes a
 * parallel-branches → gather run legible at a glance: a pattern badge, then one row
 * per branch (short id + live {@link StatePill} + a "won" marker on the branch whose
 * output the gather emitted). Derived purely from the projection via
 * {@link detectSwarm} — no new RPC, no score (SN-8). Renders NOTHING for a plain
 * linear run (no fan-in) so ordinary runs show no swarm chrome.
 */

import { useMemo } from "react";
import type { ProjectionVM } from "../../kx/use-projection";
import { stateVisual } from "../../lib/colors";
import { shortHex } from "../../lib/format";
import { type SwarmShape, detectSwarm } from "./swarm-shape";

function patternLabel(shape: SwarmShape): string {
  return shape.pattern === "consensus" ? "Consensus" : "Parallel branches";
}

export function SwarmOverview({ projection }: { projection: ProjectionVM }) {
  const shape = useMemo(() => detectSwarm(projection.motes), [projection.motes]);
  if (shape === null) {
    return null;
  }
  const n = shape.branches.length;
  return (
    <div className="swarm-overview" data-testid="swarm-overview">
      <div className="swarm-overview__head">
        <span className="chip chip--static" data-testid="swarm-pattern-badge">
          {patternLabel(shape)}
        </span>
        <span className="muted swarm-overview__summary">
          {n} branch{n === 1 ? "" : "es"} → gather
          {shape.agreementCount >= 2 ? ` · ${shape.agreementCount} agreed` : ""}
        </span>
      </div>
      <ul className="swarm-overview__branches">
        {shape.branches.map((b) => {
          // A branch-status pill reusing the state tone/label, but under its OWN
          // testid — never `state-pill`, so it does not inflate the DAG's node count.
          const { label, tone } = stateVisual(b.stateCode);
          return (
            <li
              key={b.moteId}
              className="swarm-branch-row"
              data-testid="swarm-branch-row"
              data-branch={b.moteId}
            >
              <span className="mono swarm-branch__id" title={b.moteId}>
                {shortHex(b.moteId)}
              </span>
              <span
                className={`pill pill--${tone}`}
                data-testid="swarm-branch-state"
                data-tone={tone}
              >
                {label}
              </span>
              {b.won ? (
                <span
                  className="chip chip--active swarm-branch__won"
                  data-testid="swarm-branch-won"
                >
                  won
                </span>
              ) : null}
            </li>
          );
        })}
      </ul>
    </div>
  );
}
