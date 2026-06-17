/**
 * Agent Outputs — review the data agents generate. Sourced from the Morphic
 * capture action-exhaust (`ListCaptureRecords`): every committed Mote's output,
 * newest-first, opened in the multi-modal {@link AssetViewer} (a tool's JSON, a
 * model's text/answer, an image, …). ReAct turns carry a `turn N · <branch>` badge
 * (the branch is joined onto the TURN mote — `tool` = a tool-call decision,
 * `answer` = a final answer; an Observation/tool RESULT is its own committed record
 * with no branch). The "ReAct turns" filter narrows to records that carry a branch.
 *
 * Read-only (OSS view, D157/GR19); rendering is from the content-addressed store
 * ONLY (blob URLs, never a remote `src`, never innerHTML — zero SSRF surface).
 */

import type { CaptureRecord } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { useMemo, useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useCaptureRecords } from "../../kx/use-capture-records";
import { useContent } from "../../kx/use-content";
import { shortHex } from "../../lib/format";
import { AssetViewer } from "../AssetViewer";
import { DigestChip } from "../DigestChip";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/** Branch badge color (mirrors the ReAct-turn vocabulary). */
function branchColor(branch: string): string {
  if (branch === "tool") return "var(--primary)";
  if (branch === "answer") return "var(--success)";
  if (branch === "dead_lettered") return "var(--error)";
  if (branch === "pending") return "var(--warning)";
  return "var(--text-2)";
}

export function AgentOutputsPanel() {
  const { records, notWired, isLoading } = useCaptureRecords({ limit: 100 });
  const [reactOnly, setReactOnly] = useState(false);
  const [openMote, setOpenMote] = useState<string | null>(null);

  const shown = useMemo(
    () => (reactOnly ? records.filter((r) => r.reactBranch !== "") : records),
    [records, reactOnly],
  );

  if (isLoading) {
    return <EmptyState title="Loading agent outputs…" />;
  }
  if (notWired) {
    return (
      <EmptyState
        title="Agent outputs need a newer gateway"
        detail="This gateway doesn't expose the capture action stream (an older build)."
      />
    );
  }

  return (
    <div data-testid="agent-outputs">
      <div className="chip-row agent-outputs__filter">
        <button
          type="button"
          className={`chip${reactOnly ? "" : " chip--active"}`}
          data-testid="agent-outputs-filter-all"
          aria-pressed={!reactOnly}
          onClick={() => setReactOnly(false)}
        >
          <span className="chip__label">All outputs</span>
        </button>
        <button
          type="button"
          className={`chip${reactOnly ? " chip--active" : ""}`}
          data-testid="agent-outputs-filter-react"
          aria-pressed={reactOnly}
          onClick={() => setReactOnly(true)}
        >
          <span className="chip__label">ReAct turns</span>
        </button>
      </div>
      {shown.length === 0 ? (
        <EmptyState
          title="No agent outputs yet"
          detail="Run an agentic workflow — every committed output appears here for review as its Motes commit."
        />
      ) : (
        <m.ul
          className="agent-outputs-list"
          data-testid="agent-outputs-panel"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {shown.map((record) => (
            <OutputRow
              key={record.moteId}
              record={record}
              open={openMote === record.moteId}
              onToggle={() => setOpenMote(openMote === record.moteId ? null : record.moteId)}
            />
          ))}
        </m.ul>
      )}
    </div>
  );
}

function OutputRow({
  record,
  open,
  onToggle,
}: {
  record: CaptureRecord;
  open: boolean;
  onToggle: () => void;
}) {
  const branchLabel =
    record.reactTurn !== null
      ? `turn ${record.reactTurn} · ${record.reactBranch}`
      : record.reactBranch;
  return (
    <GlowCard className="agent-output-row" variants={fadeUp} {...hoverLift}>
      <div className="agent-output-row__head">
        <button
          type="button"
          className="agent-output-row__toggle"
          data-testid={`agent-output-${record.moteId}`}
          aria-expanded={open}
          onClick={onToggle}
        >
          <span className="agent-output-row__mote mono">{shortHex(record.moteId)}</span>
          {record.reactBranch ? (
            <Badge label={branchLabel} color={branchColor(record.reactBranch)} />
          ) : null}
          <Badge label={record.ndClass} color="var(--text-2)" />
        </button>
        <DigestChip hex={record.resultRef} label="result" />
      </div>
      {open ? <OutputBody instanceId={record.instanceId} contentRef={record.resultRef} /> : null}
    </GlowCard>
  );
}

function OutputBody({ instanceId, contentRef }: { instanceId: string; contentRef: string }) {
  const content = useContent(instanceId, contentRef);
  if (content.isLoading) {
    return <EmptyState title="Loading output…" />;
  }
  if (content.error) {
    return <ErrorNotice error={toUiError(content.error)} onRetry={() => void content.refetch()} />;
  }
  if (!content.data) {
    return null;
  }
  return <AssetViewer content={content.data} stem={contentRef.slice(0, 12)} />;
}
