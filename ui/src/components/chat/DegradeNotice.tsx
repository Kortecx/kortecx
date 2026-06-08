import type { UiError } from "../../kx/errors";
import { ErrorNotice } from "../ErrorNotice";

/** Shown when the configured chat recipe/model isn't provisioned on this gateway. */
export function DegradeNotice({ error }: { error: UiError }) {
  return (
    <div data-testid="degrade-notice">
      <ErrorNotice error={error} />
      <p className="muted">
        No chat model is provisioned on this gateway. Start one with{" "}
        <code>kx serve --features inference</code>, or switch chat to the model-free{" "}
        <code>kx/recipes/echo</code> recipe in Settings.
      </p>
    </div>
  );
}
