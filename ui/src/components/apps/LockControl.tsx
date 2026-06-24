/**
 * POC-5b: the per-App lock control — CHIP/BUTTON controls (NEVER a `<select>`;
 * Playwright can't drive a controlled React select). A locked App shows a
 * "Locked" chip + an "Unlock" button; an unlocked App shows a "Lock" button.
 * Locking REFUSES agentic in-CAS edits at the runtime advance() chokepoint.
 */

import { toUiError } from "../../kx/errors";
import { useLockApp, useUnlockApp } from "../../kx/use-app-lock";

export function LockControl({ handle, locked }: { handle: string; locked: boolean }) {
  const lock = useLockApp();
  const unlock = useUnlockApp();
  const pending =
    (lock.isPending && lock.variables?.handle === handle) ||
    (unlock.isPending && unlock.variables?.handle === handle);
  const error = lock.error ?? unlock.error;

  return (
    <span className="lock-control">
      <span
        className={
          locked
            ? "chip chip--tag lock-control__state--locked"
            : "chip lock-control__state--unlocked"
        }
        data-testid={`app-lock-state-${handle}`}
        data-locked={locked}
      >
        {locked ? "🔒 Locked" : "Unlocked"}
      </span>
      {locked ? (
        <button
          type="button"
          className="btn-ghost"
          data-testid={`app-unlock-${handle}`}
          disabled={pending}
          title="Re-enable agentic in-CAS edits for this App"
          onClick={() => unlock.mutate({ handle })}
        >
          {pending ? "…" : "Unlock"}
        </button>
      ) : (
        <button
          type="button"
          className="btn-ghost"
          data-testid={`app-lock-${handle}`}
          disabled={pending}
          title="Refuse agentic in-CAS edits for this App"
          onClick={() => lock.mutate({ handle })}
        >
          {pending ? "…" : "Lock"}
        </button>
      )}
      {error ? (
        <span className="field-error" data-testid={`app-lock-error-${handle}`} role="alert">
          {toUiError(error).message}
        </span>
      ) : null}
    </span>
  );
}
