/**
 * The inspector's Tool-contract pane (PR-2): the closed set of tools this
 * Mote may call (each at its pinned version — exact (id,version) equality is
 * the broker's gate, SN-8: this display never authorizes), plus the def-level
 * identity facts (logic ref, nd-class, effect pattern, shaper/critic flags).
 */

import type { MoteDetailVM } from "../../../kx/use-mote-detail";
import { DigestChip } from "../../DigestChip";

export function ToolContractPane({ detail }: { detail: MoteDetailVM }) {
  const tools = Object.entries(detail.toolContract).sort(([a], [b]) => a.localeCompare(b));
  return (
    <div data-testid="inspector-tools">
      <dl className="node-drawer__meta">
        <div>
          <dt>Step kind</dt>
          <dd>
            <span className="badge">{detail.stepKind}</span>
          </dd>
        </div>
        <div>
          <dt>ND class</dt>
          <dd className="mono">{detail.ndClassName}</dd>
        </div>
        <div>
          <dt>Effect pattern</dt>
          <dd className="mono">{detail.effectPatternName}</dd>
        </div>
        {detail.isTopologyShaper ? (
          <div>
            <dt>Topology shaper</dt>
            <dd>yes — commits a TopologyDecision</dd>
          </div>
        ) : null}
        {detail.criticFor ? (
          <div>
            <dt>Critic for</dt>
            <dd>
              <DigestChip hex={detail.criticFor} label="producer" />
            </dd>
          </div>
        ) : null}
        <div>
          <dt>Logic ref</dt>
          <dd>
            <DigestChip hex={detail.logicRef} label="logic" />
          </dd>
        </div>
        <div>
          <dt>Def hash</dt>
          <dd>
            <DigestChip hex={detail.moteDefHash} label="def" />
          </dd>
        </div>
        <div>
          <dt>Def schema</dt>
          <dd className="mono">v{detail.schemaVersion}</dd>
        </div>
      </dl>
      <span className="section-label">Tool contract</span>
      {tools.length === 0 ? (
        <p className="muted">No tools — this Mote may call nothing.</p>
      ) : (
        <ul data-testid="inspector-tool-list">
          {tools.map(([name, version]) => (
            <li key={name} className="mono">
              {name}@{version}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
