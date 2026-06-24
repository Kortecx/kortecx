/**
 * POC-5b: the Policies section — the OSS per-App agent-write policy gate. Lists
 * the caller's Apps with each App's lock state + Lock/Unlock controls
 * ({@link LockControl}, CHIP/BUTTON — never a controlled select). A LOCKED App
 * refuses agentic in-CAS edits at the runtime advance() chokepoint (the only
 * place a branch advance can be vetoed), so a lock is an honest, enforced policy
 * — not a UI affordance. Mirrors AppsSection's honest empty / not-wired states.
 *
 * Cross-party RBAC + richer policy is Cloud (D129) — this OSS surface is the
 * single-party per-App lock only, stated plainly (GR15 don't-fake-gaps).
 */

import { m } from "framer-motion";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useApps } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { LockControl } from "../apps/LockControl";

export function PoliciesSection() {
  const { apps, notWired, isLoading, isError, error, refetch } = useApps();

  return (
    <section className="screen" data-testid="policies-section">
      <div className="section-head">
        <div>
          <h1>Policies</h1>
          <p className="muted">
            Per-App locks — the agent-write policy gate. Locking an App makes the runtime REFUSE
            agentic in-CAS edits to its project files (enforced at the advance() chokepoint, not
            just hidden in the UI). Cross-party roles & richer policy are a Cloud capability.
          </p>
        </div>
      </div>

      {isLoading ? <EmptyState title="Loading apps…" /> : null}

      {notWired ? (
        <EmptyState
          title="Policies not available"
          detail="This gateway does not expose the App catalog (an older build, or the apps.db sidecar is absent)."
        />
      ) : isError ? (
        <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />
      ) : !isLoading && apps.length === 0 ? (
        <EmptyState
          title="No apps to govern yet"
          detail="Create an App (New App) or author one with the SDK/CLI, then lock or unlock it here."
        />
      ) : null}

      {apps.length > 0 ? (
        <m.ul
          className="registry-list"
          data-testid="policies-list"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {apps.map((a) => (
            <m.li
              key={a.handle}
              className="registry-row glow-card glow-card--hover"
              data-testid={`policy-${a.handle}`}
              variants={fadeUp}
              {...hoverLift}
            >
              <div className="registry-row__main">
                <div className="registry-row__head">
                  <span className="registry-row__name">{a.name}</span>
                  <code className="mono registry-row__sub" title={a.handle}>
                    {a.handle}
                  </code>
                </div>
                {a.locked ? (
                  <p className="muted registry-row__desc">
                    Locked — the agent cannot rewrite this App's project files in-CAS.
                  </p>
                ) : (
                  <p className="muted registry-row__desc">
                    Unlocked — agentic in-CAS edits to this App's project files are allowed.
                  </p>
                )}
              </div>
              <LockControl handle={a.handle} locked={a.locked} />
            </m.li>
          ))}
        </m.ul>
      ) : null}
    </section>
  );
}
