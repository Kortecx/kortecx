import { Link } from "@tanstack/react-router";
import { toUiError } from "../../kx/errors";
import { useEventStream } from "../../kx/use-event-stream";
import { useProjection } from "../../kx/use-projection";
import { ErrorNotice } from "../ErrorNotice";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { MetricsPanel } from "../metrics/MetricsPanel";
import { ActivityFeed } from "./ActivityFeed";
import { GlobalFeed } from "./GlobalFeed";
import { RunPicker } from "./RunPicker";
import { TimeTravelScrubber } from "./TimeTravelScrubber";

/**
 * The dashboard for one run: pick a run → its metrics + a live event feed + a
 * time-travel scrubber. A "head" projection (live, no atSeq) provides the scrubber
 * max + the feed/metrics when live; a second pinned projection drives the metrics
 * when time-travelling (so you can always scrub forward again to "live").
 *
 * Before a run is selected the panel lands on the GLOBAL cross-run feed (Batch C)
 * with the quick actions — the node-wide pulse, one click from anywhere; picking
 * a run (from the picker or a feed row) drills into the run-scoped view.
 */
export function ActivityPanel({
  instance,
  atSeq,
  onSelectInstance,
  onAtSeq,
  onNavigate,
}: {
  instance?: string;
  atSeq?: number;
  onSelectInstance: (id: string) => void;
  onAtSeq: (seq: number | undefined) => void;
  /** Called when a quick action navigates away (the drawer closes itself). */
  onNavigate?: () => void;
}) {
  const head = useProjection(instance);
  const pinned = useProjection(
    atSeq != null ? instance : undefined,
    atSeq != null ? { atSeq } : {},
  );
  const stream = useEventStream(instance);

  const display = atSeq != null ? pinned.data : head.data;
  const headSeq = head.data?.currentSeq ?? 0;
  const err = head.error ?? pinned.error;

  return (
    <section className="screen" data-testid="activity-panel">
      <div className="dashboard-hero">
        <div className="dashboard-hero__text">
          <h1>Activity</h1>
          <p className="muted">
            Live run telemetry — pick a run to watch its Motes commit, scrub its history, and tail
            its events.
          </p>
        </div>
        <div className="dashboard-hero__aside">
          <span className="dashboard-hero__label">Gateway</span>
          <HealthIndicator />
        </div>
      </div>
      <RunPicker selected={instance} onSelect={onSelectInstance} />
      {instance == null ? (
        <>
          <div className="quick-actions" data-testid="quick-actions">
            <Link to="/chat" className="linkbtn" data-testid="quick-new-chat" onClick={onNavigate}>
              + New Chat
            </Link>
            <Link
              to="/workflows"
              className="linkbtn"
              data-testid="quick-new-workflow"
              onClick={onNavigate}
            >
              + New Workflow
            </Link>
          </div>
          <h2>Across all runs</h2>
          <GlobalFeed onSelectRun={onSelectInstance} />
        </>
      ) : (
        <>
          {err ? <ErrorNotice error={toUiError(err)} /> : null}
          <MetricsPanel projection={display} />
          {headSeq > 0 ? (
            <TimeTravelScrubber currentSeq={headSeq} atSeq={atSeq} onChange={onAtSeq} />
          ) : null}
          <h2>Live events</h2>
          <ActivityFeed events={stream.events} dropped={stream.dropped} active={stream.active} />
        </>
      )}
    </section>
  );
}
