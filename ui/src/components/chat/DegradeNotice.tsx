import type { UiError } from "../../kx/errors";
import { ErrorNotice } from "../ErrorNotice";

/**
 * Shown when the configured chat recipe/model isn't provisioned on this gateway.
 * `error` is optional: a turn failure passes the real `UiError`; the PROACTIVE
 * no-model state (a no-model serve, before any send) renders the guidance alone.
 */
export function DegradeNotice({ error }: { error?: UiError }) {
  return (
    <div data-testid="degrade-notice">
      {error ? <ErrorNotice error={error} /> : null}
      <p className="muted">
        No chat model is provisioned on this gateway. Start one with{" "}
        <code>kx serve --features inference</code>, or switch chat to the model-free{" "}
        <code>kx/recipes/echo</code> recipe in Settings.
      </p>
    </div>
  );
}
