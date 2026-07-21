/**
 * The per-App TRIGGER strip — this App's schedule, shown where the App is.
 *
 * `ScheduleButton` (a cron RegisterTrigger popover that already accepted an `appHandle`
 * target) shipped with zero call sites on the Apps surfaces, and `useListTriggers` had a
 * single consumer — the global Tools → Triggers govern panel. So "scheduled app" was a
 * lane name with no way to schedule anything from the lane, and an App's existing schedule
 * was only discoverable by leaving the App. This strip closes both: register a schedule
 * here, and see/Test/Fire/Remove the triggers already bound to THIS handle.
 *
 * It deliberately reuses the shipped controls rather than restating them — the row actions
 * are {@link TriggerRowActions} and the schedule renders through `fmtSchedule`, both from
 * the govern panel, so the two surfaces cannot disagree about what "fired" or "every 300s"
 * means. The global panel keeps the full register form (webhook/grpc kinds, auth posture,
 * HITL); this strip is the cron affordance an App actually needs.
 *
 * Not-wired (a gateway without the trigger registry) renders NOTHING: on the App page a
 * missing registry is an absent capability, not an error to shout about, and offering a
 * Schedule button that cannot fire is exactly the GR15 dishonesty the rest of this page
 * avoids.
 */

import { toUiError } from "../../kx/errors";
import { useDeregisterTrigger, useListTriggers } from "../../kx/use-triggers";
import { ScheduleButton } from "../sections/ScheduleButton";
import { TriggerRowActions, fmtSchedule } from "../tools/TriggersPanel";

export function AppTriggersStrip({ handle }: { handle: string }) {
  const list = useListTriggers();
  const remove = useDeregisterTrigger();
  // Triggers are a flat local registry; the App's own are the rows whose target IS this
  // App (`appHandle`, not `recipeHandle` — a recipe trigger that happens to share a name
  // is a different binding entirely).
  const mine = list.triggers.filter((t) => t.appHandle === handle);
  const removeErr = remove.error ? toUiError(remove.error) : null;

  if (list.notWired || list.isLoading) {
    return null;
  }

  return (
    <div className="app-triggers-strip" data-testid="app-triggers-strip">
      <div className="chip-row">
        <span className="muted">
          {mine.length === 0
            ? "No schedule — this App runs only when you run it."
            : `${mine.length} trigger${mine.length === 1 ? "" : "s"} fire this App.`}
        </span>
        <ScheduleButton
          appHandle={handle}
          triggerClassName="btn-ghost"
          testId={`app-schedule-${handle}`}
        />
      </div>
      {mine.length > 0 ? (
        <ul className="connections-list" data-testid="app-triggers-list">
          {mine.map((t) => (
            <li
              key={t.name}
              className="connections-list__row"
              data-testid={`app-trigger-${t.name}`}
            >
              <div className="connections-list__head">
                <span
                  className={`status-dot ${t.enabled ? "status-dot--online" : "status-dot--offline"}`}
                  role="img"
                  aria-label={t.enabled ? "enabled" : "disabled"}
                  title={t.enabled ? "enabled" : "disabled"}
                />
                <span className="connections-list__name">{t.name}</span>
                <span className="chip chip--static">
                  <span className="chip__label">{t.kind}</span>
                </span>
                {t.kind === "cron" && t.scheduleSpec ? (
                  <span className="chip chip--static">
                    <span className="chip__label">{fmtSchedule(t.scheduleSpec, t.timezone)}</span>
                  </span>
                ) : null}
                {t.requireApproval ? (
                  <span
                    className="chip chip--static"
                    title="Per-trigger HITL: irreversible actions await an operator grant (D114)"
                  >
                    <span className="chip__label">🛡 approval</span>
                  </span>
                ) : null}
              </div>
              <TriggerRowActions
                trigger={t}
                onRemove={(n) => remove.mutate(n)}
                removeBusy={remove.isPending && remove.variables === t.name}
              />
            </li>
          ))}
        </ul>
      ) : null}
      {removeErr ? (
        <p className="field-error" data-testid="app-triggers-error" role="alert">
          {removeErr.message}
        </p>
      ) : null}
    </div>
  );
}
