/**
 * The host secret-store panel (MM-3 / D110) — the govern surface over the local
 * OS-keychain secret store. Add/overwrite a secret (name + value), list the stored
 * NAMES with their audit timestamps, and remove one.
 *
 * D81: the secret VALUE is WRITE-ONLY. It appears ONLY as the `PutSecret` argument
 * and is NEVER read back — this panel surfaces names + timestamps only (the value
 * input is `type=password` and is cleared the moment it is stored). A `SecretRef`
 * NAME is what a Connection's / Trigger's `credential_ref` points at.
 *
 * Degrades to a not-wired state on a gateway without the secret store (UNIMPLEMENTED).
 * Every state is designed (D142): not-wired / error / loading / empty / list.
 */

import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDeleteSecret, useListSecretNames, usePutSecret } from "../../kx/use-secrets";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { GlowCard } from "../ds/GlowCard";

/** A local audit-clock formatter: `—` when never set, else the locale string. */
function fmtTs(ms: number): string {
  return ms > 0 ? new Date(ms).toLocaleString() : "—";
}

export function SecretsPanel() {
  const list = useListSecretNames();
  const put = usePutSecret();
  const remove = useDeleteSecret();

  const [name, setName] = useState("");
  const [value, setValue] = useState("");

  const canSubmit = name.trim().length > 0 && value.length > 0;

  const onSubmit = (e: FormEvent) => {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    put.mutate(
      { name: name.trim(), value },
      {
        onSuccess: () => {
          // Write-only: drop the plaintext value from memory the instant it is stored.
          setName("");
          setValue("");
        },
      },
    );
  };

  const putErr = put.error ? toUiError(put.error) : null;
  const removeErr = remove.error ? toUiError(remove.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="secrets-panel">
      <h2>Secrets</h2>
      <p className="muted">
        The local OS-keychain secret store. A secret&apos;s <strong>value is write-only</strong> —
        it is never shown again after you store it (D81); this panel lists only NAMES and audit
        timestamps. Reference a secret by its NAME from a Connection or a Trigger&apos;s auth.
      </p>

      {list.notWired ? (
        <EmptyState
          title="Secret store not enabled"
          detail="This gateway does not expose the local secret store (an older or restricted build)."
        />
      ) : list.isError ? (
        <ErrorNotice error={toUiError(list.error)} onRetry={() => void list.refetch()} />
      ) : list.isLoading ? (
        <EmptyState title="Loading secrets…" />
      ) : list.names.length === 0 ? (
        <EmptyState
          title="No secrets stored"
          detail="Add one below to reference it by name from a connection or a trigger's auth."
        />
      ) : (
        <ul className="connections-list" data-testid="secrets-list">
          {list.names.map((s) => {
            const busy = remove.isPending && remove.variables === s.name;
            return (
              <li key={s.name} className="connections-list__row" data-testid={`secret-${s.name}`}>
                <div className="connections-list__head">
                  <span className="status-dot status-dot--online" role="img" aria-label="stored" />
                  <span className="connections-list__name">{s.name}</span>
                  <span className="chip chip--static" title="The value is write-only (never shown)">
                    <span className="chip__label">write-only</span>
                  </span>
                </div>
                <div className="connections-list__meta muted">
                  <span>created {fmtTs(s.createdUnixMs)}</span>
                  <span>· updated {fmtTs(s.updatedUnixMs)}</span>
                </div>
                <div className="connections-list__actions chip-row">
                  <button
                    type="button"
                    className="chip chip--danger"
                    data-testid={`secret-remove-${s.name}`}
                    disabled={busy}
                    onClick={() => remove.mutate(s.name)}
                  >
                    <span className="chip__label">Remove</span>
                  </button>
                </div>
              </li>
            );
          })}
        </ul>
      )}

      {removeErr ? (
        <p className="field-error" data-testid="secret-action-error" role="alert">
          {removeErr.kind === "forbidden" ? "Not permitted: " : ""}
          {removeErr.message}
        </p>
      ) : remove.isSuccess ? (
        <p className="register-tool__result" data-testid="secret-action-result">
          Secret removed.
        </p>
      ) : null}

      <form onSubmit={onSubmit} className="register-tool-form" data-testid="secret-add-form">
        <h3>Add a secret</h3>
        <p className="muted">
          The value is sent once and stored write-only. You can overwrite it later by adding the
          same name again; it is never displayed back.
        </p>
        <div className="register-tool-form__row">
          <input
            type="text"
            data-testid="secret-add-name"
            placeholder="secret name (e.g. github_token)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="Secret name"
            autoComplete="off"
          />
          <input
            type="password"
            data-testid="secret-add-value"
            placeholder="secret value (write-only)"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            aria-label="Secret value (write-only)"
            autoComplete="new-password"
          />
        </div>
        <button
          type="submit"
          data-testid="secret-add-submit"
          disabled={put.isPending || !canSubmit}
        >
          {put.isPending ? "Storing…" : "Store secret"}
        </button>
      </form>

      {putErr ? (
        <p className="field-error" data-testid="secret-add-error" role="alert">
          {putErr.kind === "forbidden" ? "Not permitted: " : ""}
          {putErr.message}
        </p>
      ) : put.isSuccess ? (
        <p className="register-tool__result" data-testid="secret-add-result">
          Secret stored — reference it by name. The value is not shown again.
        </p>
      ) : null}
    </GlowCard>
  );
}
