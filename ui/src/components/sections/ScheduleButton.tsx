/**
 * A per-item SCHEDULE affordance — a small CRON form (in a popover) that registers a
 * recurring trigger via the SHIPPED `RegisterTrigger` (the local single-user trigger
 * registry), Invoking this workflow/App on a schedule. NO new RPC: local cron ships, so
 * this replaces the stale "Schedule → Cloud" chip (scheduling was never cloud-only).
 * Exactly one of `recipeHandle` / `appHandle` targets the run.
 */

import { useState } from "react";
import { toUiError } from "../../kx/errors";
import { useRegisterTrigger } from "../../kx/use-triggers";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";

export function ScheduleButton({
  recipeHandle,
  appHandle,
  triggerClassName = "linkbtn",
  testId,
  iconOnly = false,
}: {
  recipeHandle?: string;
  appHandle?: string;
  triggerClassName?: string;
  testId?: string;
  /** Render just the calendar glyph (for a compact icon action cluster); the
   *  accessible name still comes from `triggerLabel`. */
  iconOnly?: boolean;
}) {
  const register = useRegisterTrigger();
  const target = appHandle ?? recipeHandle ?? "";
  const [name, setName] = useState("");
  const [cron, setCron] = useState("0 9 * * 1-5");
  const [tz, setTz] = useState("");
  const canSubmit = name.trim().length > 0 && cron.trim().length > 0 && !register.isPending;

  function submit(): void {
    if (!canSubmit) {
      return;
    }
    register.mutate({
      name: name.trim(),
      kind: "cron",
      recipeHandle: recipeHandle ?? "",
      appHandle: appHandle ?? "",
      auth: "none",
      authSecretRef: "",
      scheduleSpec: cron.trim(),
      timezone: tz.trim(),
      enabled: true,
      requireApproval: false,
    });
  }

  return (
    <Popover
      trigger={
        iconOnly ? (
          <Icon name="calendar" size={16} />
        ) : (
          <>
            <Icon name="calendar" size={15} /> Schedule
          </>
        )
      }
      triggerClassName={triggerClassName}
      triggerLabel="Schedule recurring runs"
      triggerTestId={testId ?? `schedule-${target}`}
      align="right"
      direction="down"
      menuTestId={`schedule-form-${target}`}
    >
      {() => (
        <div className="schedule-form">
          <p className="muted">A recurring CRON run — a LOCAL trigger (no cloud).</p>
          <label className="schedule-form__field">
            <span>Name</span>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              data-testid="schedule-name"
              placeholder="daily-report"
              spellCheck={false}
              autoComplete="off"
            />
          </label>
          <label className="schedule-form__field">
            <span>Cron (5-field, or interval seconds)</span>
            <input
              value={cron}
              onChange={(e) => setCron(e.target.value)}
              data-testid="schedule-cron"
              spellCheck={false}
              autoComplete="off"
            />
          </label>
          <label className="schedule-form__field">
            <span>Timezone (optional)</span>
            <input
              value={tz}
              onChange={(e) => setTz(e.target.value)}
              data-testid="schedule-tz"
              placeholder="UTC"
              spellCheck={false}
              autoComplete="off"
            />
          </label>
          <button
            type="button"
            className="btn-primary"
            data-testid="schedule-submit"
            disabled={!canSubmit}
            onClick={submit}
          >
            {register.isPending ? "Scheduling…" : "Schedule"}
          </button>
          {register.isError ? (
            <p className="field-error" data-testid="schedule-error" role="alert">
              {toUiError(register.error).message}
            </p>
          ) : null}
          {register.isSuccess ? (
            <p className="muted" data-testid="schedule-ok">
              Scheduled ✓ — manage it in Tools → Triggers.
            </p>
          ) : null}
        </div>
      )}
    </Popover>
  );
}
