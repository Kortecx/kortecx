import { toUiError } from "../../kx/errors";
import { useEventStream } from "../../kx/use-event-stream";
import { useProjection } from "../../kx/use-projection";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { HealthIndicator } from "../metrics/HealthIndicator";
import { MetricsPanel } from "../metrics/MetricsPanel";
import { ActivityFeed } from "./ActivityFeed";
import { RunPicker } from "./RunPicker";
import { TimeTravelScrubber } from "./TimeTravelScrubber";

/**
 * The dashboard for one run: pick a run → its metrics + a live event feed + a
 * time-travel scrubber. A "head" projection (live, no atSeq) provides the scrubber
 * max + the feed/metrics when live; a second pinned projection drives the metrics
 * when time-travelling (so you can always scrub forward again to "live").
 */
export function ActivityPanel({
  instance,
  atSeq,
  onSelectInstance,
  onAtSeq,
}: {
  instance?: string;
  atSeq?: number;
  onSelectInstance: (id: string) => void;
  onAtSeq: (seq: number | undefined) => void;
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
        <EmptyState
          title="Select a run"
          detail="Pick a run (or paste an instance id) to see live metrics, events, and time-travel."
        />
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
