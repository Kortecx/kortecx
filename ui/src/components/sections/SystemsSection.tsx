import { m } from "framer-motion";
import { fadeUp } from "../../app/motion";
import { useApps } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { AppManifestPanel } from "../apps/AppManifestPanel";

/**
 * Security: the single-user capability-manifest surface. Pick an App to resolve its
 * warrant — reach, capability ceiling, and model route (`GetAppManifest`) — with each
 * requested tool / connection diffed against your live policy. Per-App locks live on
 * the App page (the lock control in the App header).
 *
 * Pure renderer: the selected handle rides the route's validated search.
 */
export function SystemsSection({
  handle,
  onHandle,
}: {
  handle?: string;
  onHandle?: (handle: string) => void;
} = {}) {
  const { apps, notWired, isLoading } = useApps();
  const selected = handle ?? apps[0]?.handle;

  return (
    <m.section
      className="screen"
      data-testid="systems-section"
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      <div className="section-head">
        <div>
          <h1>Security</h1>
          <p className="muted">
            Resolve an App's warrant — its reach, capability ceiling, and model route — against your
            live policy.
          </p>
        </div>
      </div>

      {notWired ? (
        <EmptyState title="Capability manifests need a newer gateway" />
      ) : isLoading ? (
        <EmptyState title="Loading apps…" />
      ) : apps.length === 0 ? (
        <EmptyState
          title="No apps yet"
          detail="Author an App to inspect its resolved capability manifest."
        />
      ) : (
        <>
          <div className="chip-row" data-testid="security-app-picker">
            {apps.map((a) => (
              <button
                key={a.handle}
                type="button"
                className={`chip${a.handle === selected ? " chip--active" : ""}`}
                aria-pressed={a.handle === selected}
                data-testid={`security-app-${a.handle}`}
                onClick={() => onHandle?.(a.handle)}
                title={a.handle}
              >
                {a.name}
              </button>
            ))}
          </div>
          {selected ? <AppManifestPanel handle={selected} /> : null}
        </>
      )}
    </m.section>
  );
}
