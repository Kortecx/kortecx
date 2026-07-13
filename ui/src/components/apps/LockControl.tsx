/**
 * POC-5b: the per-App lock control — a single REACTIVE icon toggle (a padlock
 * that flips lock↔unlock on click). Locking REFUSES agentic in-CAS edits at the runtime
 * advance() chokepoint; the icon reflects the current state (`data-locked`). NEVER a
 * `<select>` (Playwright can't drive a controlled React select).
 */

import { toUiError } from "../../kx/errors";
import { useLockApp, useUnlockApp } from "../../kx/use-app-lock";
import { Icon } from "../shell/Icon";

export function LockControl({ handle, locked }: { handle: string; locked: boolean }) {
  const lock = useLockApp();
  const unlock = useUnlockApp();
  const pending =
    (lock.isPending && lock.variables?.handle === handle) ||
    (unlock.isPending && unlock.variables?.handle === handle);
  const error = lock.error ?? unlock.error;

  return (
    <>
      <button
        type="button"
        className="iconbtn"
        data-testid={locked ? `app-unlock-${handle}` : `app-lock-${handle}`}
        data-locked={locked}
        aria-pressed={locked}
        disabled={pending}
        title={
          locked
            ? "Locked — click to unlock (re-enable agentic in-CAS edits)"
            : "Unlocked — click to lock (refuse agentic in-CAS edits)"
        }
        aria-label={locked ? "Unlock App" : "Lock App"}
        onClick={() => (locked ? unlock.mutate({ handle }) : lock.mutate({ handle }))}
      >
        <Icon name={locked ? "lock" : "unlock"} size={16} />
      </button>
      {error ? (
        <span className="field-error" data-testid={`app-lock-error-${handle}`} role="alert">
          {toUiError(error).message}
        </span>
      ) : null}
    </>
  );
}
