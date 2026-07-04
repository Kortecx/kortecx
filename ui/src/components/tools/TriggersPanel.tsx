/**
 * The triggers panel (D113 / D170.b / T-APP-TRIGGER-TARGET) — the govern surface over
 * the local trigger registry. A trigger binds an inbound EVENT (a webhook POST, a cron
 * schedule, or a bare `SubmitTrigger` RPC) to EITHER a recipe handle OR a saved App (the
 * credentialed App fires unattended with its connections + secret_scope resolved).
 *
 * Register a trigger (name · kind · recipe|App target · webhook auth posture · the auth
 * secret REF NAME · a cron schedule [interval seconds OR a 5-field expr + timezone] · a
 * per-trigger HITL approval posture), list the registered triggers with their folded
 * state, then per row Test (dry-run the binding — fires nothing), Fire (submit the
 * inbound event), or Remove. D81: the auth secret is referenced by NAME only — a row
 * shows `authSecretPresent` (signed), never the secret itself. SN-8: the trigger id /
 * run instance id are server-derived.
 *
 * `kind`/`auth` are chosen via CHIP buttons (never a controlled `<select>` — the UI-3
 * React-controlled-select e2e gotcha). Degrades to a not-wired state on a gateway
 * without triggers (UNIMPLEMENTED). Every state is designed (D142).
 */

import type { TriggerAuthInput, TriggerKindInput, TriggerRow } from "@kortecx/sdk/web";
import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import {
  useDeregisterTrigger,
  useFireTrigger,
  useListTriggers,
  useRegisterTrigger,
  useTestTrigger,
} from "../../kx/use-triggers";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { GlowCard } from "../ds/GlowCard";

const KINDS: readonly TriggerKindInput[] = ["webhook", "cron", "grpc"];
const AUTHS: readonly TriggerAuthInput[] = ["none", "hmac_sha256", "bearer"];

/** A local audit-clock formatter: `never` when unfired, else the locale string. */
function fmtFired(ms: number): string {
  return ms > 0 ? new Date(ms).toLocaleString() : "never";
}

/** Render a cron schedule: legacy interval-seconds (all digits) as `every Ns`, else the
 *  5-field crontab expr with its timezone (T-APP-TRIGGER-TARGET). */
function fmtSchedule(spec: string, timezone: string): string {
  const s = spec.trim();
  if (s.length > 0 && /^\d+$/.test(s)) {
    return `every ${s}s`;
  }
  const tz = timezone && timezone !== "UTC" ? ` ${timezone}` : "";
  return `${s}${tz}`;
}

/**
 * Per-trigger operator actions: Test (dry-run the binding — fires nothing), Fire
 * (submit the inbound event), Remove. Each row owns its own Test/Fire mutations so
 * the inline outcome stays row-scoped (the ConnectionFireRow precedent). Every state
 * designed (D142): idle / pending / ok+detail / error.
 */
function TriggerRowActions({
  trigger,
  onRemove,
  removeBusy,
}: {
  trigger: TriggerRow;
  onRemove: (name: string) => void;
  removeBusy: boolean;
}) {
  const test = useTestTrigger();
  const fire = useFireTrigger();
  const name = trigger.name;

  const testErr = test.error ? toUiError(test.error) : null;
  const fireErr = fire.error ? toUiError(fire.error) : null;

  return (
    <>
      <div className="connections-list__actions chip-row">
        <button
          type="button"
          className="chip"
          data-testid={`trigger-test-${name}`}
          disabled={test.isPending}
          onClick={() => test.mutate({ name })}
        >
          <span className="chip__label">{test.isPending ? "Testing…" : "Test"}</span>
        </button>
        <button
          type="button"
          className="chip"
          data-testid={`trigger-fire-${name}`}
          disabled={fire.isPending}
          onClick={() => fire.mutate({ name })}
        >
          <span className="chip__label">{fire.isPending ? "Firing…" : "Fire"}</span>
        </button>
        <button
          type="button"
          className="chip chip--danger"
          data-testid={`trigger-remove-${name}`}
          disabled={removeBusy}
          onClick={() => onRemove(name)}
        >
          <span className="chip__label">Remove</span>
        </button>
      </div>

      {testErr ? (
        <p className="field-error" data-testid={`trigger-test-error-${name}`} role="alert">
          {testErr.message}
        </p>
      ) : test.data ? (
        <p
          className={test.data.ok ? "register-tool__result" : "field-error"}
          data-testid={`trigger-test-result-${name}`}
          role={test.data.ok ? undefined : "alert"}
        >
          {test.data.ok
            ? `Binding OK${test.data.detail ? ` — ${test.data.detail}` : ""}`
            : `Binding failed — ${test.data.detail || "the recipe/App handle did not resolve."}`}
        </p>
      ) : null}

      {fireErr ? (
        <p className="field-error" data-testid={`trigger-fire-error-${name}`} role="alert">
          {fireErr.message}
        </p>
      ) : fire.data ? (
        <p className="register-tool__result" data-testid={`trigger-fire-result-${name}`}>
          {fire.data.deduped
            ? `Already fired — run ${fire.data.instanceId} (deduped).`
            : `Fired — run ${fire.data.instanceId} started.`}
        </p>
      ) : null}
    </>
  );
}

export function TriggersPanel() {
  const list = useListTriggers();
  const register = useRegisterTrigger();
  const remove = useDeregisterTrigger();

  const [name, setName] = useState("");
  const [kind, setKind] = useState<TriggerKindInput>("webhook");
  const [target, setTarget] = useState<"recipe" | "app">("recipe");
  const [recipeHandle, setRecipeHandle] = useState("");
  const [appHandle, setAppHandle] = useState("");
  const [auth, setAuth] = useState<TriggerAuthInput>("none");
  const [authSecretRef, setAuthSecretRef] = useState("");
  const [scheduleSpec, setScheduleSpec] = useState("");
  const [timezone, setTimezone] = useState("");
  const [requireApproval, setRequireApproval] = useState(false);

  // T-APP-TRIGGER-TARGET: exactly one target (recipe | app) is required.
  const handleValue = target === "app" ? appHandle : recipeHandle;
  const canSubmit =
    name.trim().length > 0 &&
    handleValue.trim().length > 0 &&
    (auth === "none" || authSecretRef.trim().length > 0) &&
    (kind !== "cron" || scheduleSpec.trim().length > 0);

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    register.mutate(
      {
        name: name.trim(),
        kind,
        recipeHandle: target === "recipe" ? recipeHandle.trim() : "",
        appHandle: target === "app" ? appHandle.trim() : "",
        auth,
        authSecretRef: auth !== "none" ? authSecretRef.trim() : "",
        scheduleSpec: kind === "cron" ? scheduleSpec.trim() : "",
        timezone: kind === "cron" ? timezone.trim() : "",
        enabled: true,
        requireApproval,
      },
      {
        onSuccess: () => {
          setName("");
          setRecipeHandle("");
          setAppHandle("");
          setAuth("none");
          setAuthSecretRef("");
          setScheduleSpec("");
          setTimezone("");
          setRequireApproval(false);
          setKind("webhook");
          setTarget("recipe");
        },
      },
    );
  };

  const registerErr = register.error ? toUiError(register.error) : null;
  const removeErr = remove.error ? toUiError(remove.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="triggers-panel">
      <h2>Triggers</h2>
      <p className="muted">
        Bind an inbound EVENT — a webhook POST, a cron schedule, or a bare{" "}
        <code>SubmitTrigger</code> RPC — to a <strong>recipe</strong> handle OR a saved{" "}
        <strong>App</strong> (the App fires unattended with its integrations + secret scope
        resolved). Cron takes interval seconds or a 5-field expression in a timezone.{" "}
        <strong>Approval</strong> holds irreversible actions for an operator grant (recommended for
        unattended Apps). Webhook auth references its secret by NAME only (never the value, D81) —
        add that secret on the Secrets tab. Hosted, multi-tenant triggers at scale are a Cloud
        capability.
      </p>

      {list.notWired ? (
        <EmptyState
          title="Triggers not enabled"
          detail="This gateway does not expose the trigger registry (an older or restricted build)."
        />
      ) : list.isError ? (
        <ErrorNotice error={toUiError(list.error)} onRetry={() => void list.refetch()} />
      ) : list.isLoading ? (
        <EmptyState title="Loading triggers…" />
      ) : list.triggers.length === 0 ? (
        <EmptyState
          title="No triggers registered"
          detail="Register one below to Invoke a recipe from a webhook, a cron interval, or an RPC event."
        />
      ) : (
        <ul className="connections-list" data-testid="triggers-list">
          {list.triggers.map((t) => {
            const busy = remove.isPending && remove.variables === t.name;
            return (
              <li key={t.name} className="connections-list__row" data-testid={`trigger-${t.name}`}>
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
                  <span className="chip chip--static" title={`auth: ${t.auth}`}>
                    <span className="chip__label">{t.auth}</span>
                  </span>
                  <span
                    className="chip chip--static"
                    data-testid={`trigger-target-kind-${t.name}`}
                    title={t.appHandle ? "fires a saved App" : "invokes a recipe"}
                  >
                    <span className="chip__label">{t.appHandle ? "app" : "recipe"}</span>
                  </span>
                  {t.authSecretPresent ? (
                    <span
                      className="chip chip--static"
                      data-testid={`trigger-signed-${t.name}`}
                      title="An auth secret ref is attached (signed)"
                    >
                      <span className="chip__label">🔒 signed</span>
                    </span>
                  ) : null}
                  {t.requireApproval ? (
                    <span
                      className="chip chip--static"
                      data-testid={`trigger-hitl-${t.name}`}
                      title="Per-trigger HITL: irreversible actions await an operator grant (D114)"
                    >
                      <span className="chip__label">🛡 approval</span>
                    </span>
                  ) : null}
                </div>
                <div className="connections-list__meta muted">
                  <code className="mono">
                    {t.appHandle ? `app:${t.appHandle}` : t.recipeHandle}
                  </code>
                  {t.kind === "cron" && t.scheduleSpec ? (
                    <span>· {fmtSchedule(t.scheduleSpec, t.timezone)}</span>
                  ) : null}
                  <span>· {t.enabled ? "enabled" : "disabled"}</span>
                  <span>· last fired {fmtFired(t.lastFireUnixMs)}</span>
                </div>
                <TriggerRowActions
                  trigger={t}
                  onRemove={(n) => remove.mutate(n)}
                  removeBusy={busy}
                />
              </li>
            );
          })}
        </ul>
      )}

      {removeErr ? (
        <p className="field-error" data-testid="trigger-action-error" role="alert">
          {removeErr.kind === "forbidden" ? "Not permitted: " : ""}
          {removeErr.message}
        </p>
      ) : remove.isSuccess ? (
        <p className="register-tool__result" data-testid="trigger-action-result">
          Trigger removed.
        </p>
      ) : null}

      <form onSubmit={onSubmit} className="register-tool-form" data-testid="trigger-add-form">
        <h3>Register a trigger</h3>
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Kind</legend>
          <div className="chip-row">
            {KINDS.map((k) => (
              <button
                key={k}
                type="button"
                className={`chip${kind === k ? " chip--active" : ""}`}
                data-testid={`trigger-kind-${k}`}
                aria-pressed={kind === k}
                onClick={() => setKind(k)}
              >
                <span className="chip__label">{k}</span>
              </button>
            ))}
          </div>
        </fieldset>
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Target</legend>
          <div className="chip-row">
            {(["recipe", "app"] as const).map((tgt) => (
              <button
                key={tgt}
                type="button"
                className={`chip${target === tgt ? " chip--active" : ""}`}
                data-testid={`trigger-target-${tgt}`}
                aria-pressed={target === tgt}
                onClick={() => setTarget(tgt)}
              >
                <span className="chip__label">{tgt === "app" ? "App" : "Recipe"}</span>
              </button>
            ))}
          </div>
        </fieldset>
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Webhook auth</legend>
          <div className="chip-row">
            {AUTHS.map((a) => (
              <button
                key={a}
                type="button"
                className={`chip${auth === a ? " chip--active" : ""}`}
                data-testid={`trigger-auth-${a}`}
                aria-pressed={auth === a}
                onClick={() => setAuth(a)}
              >
                <span className="chip__label">{a}</span>
              </button>
            ))}
          </div>
        </fieldset>
        <div className="register-tool-form__row">
          <input
            type="text"
            data-testid="trigger-add-name"
            placeholder="trigger name (e.g. gh-push)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Trigger name"
          />
          {target === "app" ? (
            <input
              type="text"
              data-testid="trigger-add-app"
              placeholder="App handle (a saved App — fires unattended with its integrations)"
              value={appHandle}
              onChange={(e) => setAppHandle(e.target.value)}
              aria-label="App handle"
            />
          ) : (
            <input
              type="text"
              data-testid="trigger-add-recipe"
              placeholder="recipe handle (e.g. kx/recipes/react)"
              value={recipeHandle}
              onChange={(e) => setRecipeHandle(e.target.value)}
              aria-label="Recipe handle"
            />
          )}
        </div>
        {auth !== "none" ? (
          <input
            type="text"
            data-testid="trigger-add-secret-ref"
            placeholder="auth secret ref NAME (from the Secrets tab — never the value)"
            value={authSecretRef}
            onChange={(e) => setAuthSecretRef(e.target.value)}
            aria-label="Auth secret reference name"
          />
        ) : null}
        {kind === "cron" ? (
          <div className="register-tool-form__row">
            <input
              type="text"
              data-testid="trigger-add-schedule"
              placeholder="interval seconds (300) or a 5-field cron (0 9 * * 1-5)"
              value={scheduleSpec}
              onChange={(e) => setScheduleSpec(e.target.value)}
              aria-label="Cron interval seconds or 5-field expression"
            />
            <input
              type="text"
              data-testid="trigger-add-timezone"
              placeholder="timezone for a cron expr (e.g. America/New_York; blank = UTC)"
              value={timezone}
              onChange={(e) => setTimezone(e.target.value)}
              aria-label="Cron timezone"
            />
          </div>
        ) : null}
        <fieldset className="register-tool-form__idempotency">
          <legend className="muted">Approval (HITL)</legend>
          <div className="chip-row">
            <button
              type="button"
              className={`chip${requireApproval ? " chip--active" : ""}`}
              data-testid="trigger-add-require-approval"
              aria-pressed={requireApproval}
              onClick={() => setRequireApproval((v) => !v)}
              title="Withhold irreversible actions until an operator grant (D114) — recommended for unattended Apps"
            >
              <span className="chip__label">
                {requireApproval ? "🛡 approval required" : "autonomous"}
              </span>
            </button>
          </div>
        </fieldset>
        <button
          type="submit"
          data-testid="trigger-add-submit"
          disabled={register.isPending || !canSubmit}
        >
          {register.isPending ? "Registering…" : "Register trigger"}
        </button>
      </form>

      {registerErr ? (
        <p className="field-error" data-testid="trigger-add-error" role="alert">
          {registerErr.kind === "forbidden" ? "Not permitted: " : ""}
          {registerErr.message}
        </p>
      ) : register.isSuccess ? (
        <p className="register-tool__result" data-testid="trigger-add-result">
          Trigger registered — id {register.data.triggerId}.
        </p>
      ) : null}
    </GlowCard>
  );
}
