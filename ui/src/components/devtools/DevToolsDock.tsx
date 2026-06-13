import { useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { useEventStream } from "../../kx/use-event-stream";
import { useHealth } from "../../kx/use-health";
import { useRuns } from "../../kx/use-runs";
import { ActivityFeed } from "../activity/ActivityFeed";

const TAIL_MAX = 200;

/**
 * DevTools dock phase 1 (Monitoring/DevTools): a bottom dock with a live event
 * tail (the most recent run, over the WS bridge) and a gateway-health panel.
 * Pure UI over EXISTING surfaces (`wsEvents`, the health probe, the run history)
 * — zero new RPCs. The dock is lazy-loaded and mounted only while open, so its
 * chunk never enters the eager set and the WS tail never duplicates a closed
 * dock's connection.
 */
export function DevToolsDock({ onClose }: { onClose: () => void }) {
  const [tab, setTab] = useState<"events" | "health">("events");
  return (
    <section className="devtools" data-testid="devtools-dock" aria-label="DevTools">
      <header className="devtools__bar">
        <div className="devtools__tabs" role="tablist" aria-label="DevTools panels">
          <button
            type="button"
            role="tab"
            aria-selected={tab === "events"}
            className="devtools__tab"
            data-testid="devtools-tab-events"
            onClick={() => setTab("events")}
          >
            Events
          </button>
          <button
            type="button"
            role="tab"
            aria-selected={tab === "health"}
            className="devtools__tab"
            data-testid="devtools-tab-health"
            onClick={() => setTab("health")}
          >
            Health
          </button>
        </div>
        <button
          type="button"
          className="iconbtn devtools__close"
          onClick={onClose}
          aria-label="Close DevTools"
          data-testid="devtools-close"
        >
          ×
        </button>
      </header>
      <div className="devtools__body">{tab === "events" ? <EventsTab /> : <HealthTab />}</div>
    </section>
  );
}

/** Tail the MOST RECENT run's deltas (newest-first, bounded ring). */
function EventsTab() {
  const { runs } = useRuns();
  const latest = runs[0];
  const stream = useEventStream(latest?.instanceId, { max: TAIL_MAX });
  if (!latest) {
    return (
      <p className="muted" data-testid="devtools-no-run">
        No runs this session — run a blueprint and its events tail here.
      </p>
    );
  }
  return (
    <div data-testid="devtools-events">
      <p className="muted devtools__context mono">run {latest.instanceId}</p>
      <ActivityFeed
        events={stream.events}
        active={stream.active}
        dropped={stream.dropped}
        instanceId={latest.instanceId}
      />
    </div>
  );
}

/** Gateway liveness + the connection profile (the conn-status pill, expanded). */
function HealthTab() {
  const { endpoint, wsEndpoint } = useConnection();
  const health = useHealth();
  const h = health.data ?? "down";
  return (
    <dl className="facts" data-testid="devtools-health">
      <dt>Gateway</dt>
      <dd>
        <span
          className={`dot ${h === "live" ? "dot--ok" : h === "degraded" ? "dot--degraded" : "dot--off"}`}
        />{" "}
        {h}
      </dd>
      <dt>Endpoint</dt>
      <dd className="mono">{endpoint}</dd>
      <dt>WS bridge</dt>
      <dd className="mono">{wsEndpoint ?? "derived from the endpoint"}</dd>
    </dl>
  );
}
