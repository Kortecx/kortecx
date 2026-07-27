/**
 * The durable script registry — the GOVERNANCE view over registered scripts plus
 * the operator register/deregister controls.
 *
 * A script is a named, versioned program registered once and fired thereafter as
 * an ordinary tool. What is shown per row is what the script **asked for**, never
 * what it may do: the declared wish becomes the tool's requirement, and the
 * runtime refuses any call whose requirement is not a subset of the calling
 * warrant. Registration grants no authority, and listing leaks none.
 *
 * Scripts run in the platform sandbox or not at all — a serve without a sandbox,
 * or without the declared interpreter, refuses the registration rather than
 * running anything on the host.
 */

import type { RegisteredScript } from "@kortecx/sdk/web";
import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDeregisterScript, useListScripts, useRegisterScript } from "../../kx/use-scripts";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { Badge } from "../ds/Badge";
import { GlowCard } from "../ds/GlowCard";

/** The interpreters a serve accepts. Kept in step with the runtime's allowlist —
 * a serve refuses anything else, so offering more here would only produce a
 * failed registration. */
const INTERPRETERS = ["sh", "python3", "node"] as const;
type InterpreterChoice = (typeof INTERPRETERS)[number];

/** Accent stripe keyed by interpreter (display only). */
function interpreterStripe(interpreter: string): string {
  if (interpreter === "python3") return "var(--info)";
  if (interpreter === "node") return "var(--success)";
  return "var(--violet)";
}

export function ScriptsPanel() {
  const { scripts, notWired, isLoading, isError, error, refetch } = useListScripts();
  const deregister = useDeregisterScript();
  const deregError = deregister.error ? toUiError(deregister.error) : null;

  if (isLoading) {
    return <EmptyState title="Loading scripts…" />;
  }
  if (notWired) {
    return (
      <EmptyState
        title="Scripts need a newer gateway"
        detail="This gateway doesn't expose the script registry (an older build)."
      />
    );
  }
  if (isError) {
    return <ErrorNotice error={toUiError(error)} onRetry={() => void refetch()} />;
  }

  return (
    <div data-testid="scripts-panel">
      <p className="muted" data-testid="scripts-authority-note">
        A script runs in the platform <strong>sandbox</strong>, under the calling agent's own
        grants. What each row lists is what the script <em>asked for</em> — a call is refused unless
        the caller already holds it. An agent supplies only the script's input; its arguments and
        environment are fixed here, at registration.
      </p>
      {deregError ? (
        <p className="field-error" data-testid="script-deregister-error" role="alert">
          {deregError.message}
        </p>
      ) : null}
      {scripts.length === 0 ? (
        <EmptyState
          title="No scripts registered"
          detail="Register one below. It becomes callable by an agent only under a warrant that grants it."
        />
      ) : (
        <m.ul
          className="registry-list"
          data-testid="scripts-registered"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {scripts.map((script) => {
            const pending =
              deregister.isPending &&
              deregister.variables?.name === script.scriptName &&
              deregister.variables?.version === script.scriptVersion;
            return (
              <ScriptRow
                key={`${script.scriptName}@${script.scriptVersion}`}
                script={script}
                pending={pending}
                onDeregister={() =>
                  deregister.mutate({
                    name: script.scriptName,
                    version: script.scriptVersion,
                  })
                }
              />
            );
          })}
        </m.ul>
      )}
      <div className="tools-registry-actions">
        <RegisterScriptForm />
      </div>
    </div>
  );
}

function ScriptRow({
  script,
  pending,
  onDeregister,
}: {
  script: RegisteredScript;
  pending: boolean;
  onDeregister: () => void;
}) {
  return (
    <GlowCard
      className="registry-row"
      stripe={interpreterStripe(script.interpreter)}
      variants={fadeUp}
      {...hoverLift}
    >
      <div className="registry-row__main">
        <div className="registry-row__head">
          <span
            className="registry-row__name mono"
            data-testid={`registered-script-${script.scriptName}-${script.scriptVersion}`}
          >
            {script.scriptName}@{script.scriptVersion}
          </span>
          <Badge label={script.interpreter} color={interpreterStripe(script.interpreter)} />
          <Badge label="sandboxed" color="var(--success)" />
        </div>
        {script.description ? (
          <p className="registry-row__desc muted">{script.description}</p>
        ) : null}
        <p
          className="registry-row__desc mono"
          data-testid={`script-call-format-${script.scriptName}-${script.scriptVersion}`}
          title="How an agent is told to call this script — the canonical tool-call envelope"
        >
          {`{"tool_call":{"name":"${script.scriptName}","version":"${script.scriptVersion}","args":{"input":" … "}}}`}
        </p>
        <dl className="registry-row__meta">
          <div>
            {/* "requested", not "granted": the wish is what the declaration asked
                for, and a call is refused unless the caller already holds it. */}
            <dt className="muted">files requested</dt>
            <dd className="mono">{script.fsScope}</dd>
          </div>
          <div>
            <dt className="muted">network requested</dt>
            <dd className="mono">{script.netScope}</dd>
          </div>
          <div>
            <dt className="muted">time budget</dt>
            <dd>{script.wallClockMs > 0 ? `${script.wallClockMs} ms` : "—"}</dd>
          </div>
          <div>
            <dt className="muted">source</dt>
            <dd className="mono" title={script.sourceRef}>
              {script.sourceRef.slice(0, 12)}
            </dd>
          </div>
        </dl>
      </div>
      <button
        type="button"
        className="btn-ghost registry-row__deregister"
        data-testid={`deregister-script-${script.scriptName}-${script.scriptVersion}`}
        disabled={pending}
        title="Deregister this script"
        onClick={onDeregister}
      >
        {pending ? "Removing…" : "Deregister"}
      </button>
    </GlowCard>
  );
}

/** Register a script: identity, interpreter, source, and what it needs. */
function RegisterScriptForm() {
  const register = useRegisterScript();
  const [name, setName] = useState("");
  const [version, setVersion] = useState("1");
  const [interpreter, setInterpreter] = useState<InterpreterChoice>("sh");
  const [description, setDescription] = useState("");
  const [source, setSource] = useState("");
  const [mounts, setMounts] = useState("");
  const err = register.error ? toUiError(register.error) : null;

  function submit(event: React.FormEvent): void {
    event.preventDefault();
    // "ro:/path" per line — the smallest thing that expresses a mount without a
    // repeater widget, and it round-trips the summary the rows display.
    const fsMounts = mounts
      .split("\n")
      .map((line) => line.trim())
      .filter((line) => line.length > 0)
      .flatMap((line) => {
        const [mode, ...rest] = line.split(":");
        const path = rest.join(":");
        return mode === "ro" || mode === "rw" || mode === "exec" ? [{ mode, path } as const] : [];
      });
    register.mutate(
      { name, version, interpreter, description, source, fsMounts },
      {
        onSuccess: () => {
          setName("");
          setSource("");
          setDescription("");
          setMounts("");
        },
      },
    );
  }

  return (
    <form className="register-script-form" onSubmit={submit} data-testid="register-script-form">
      <h3>Register a script</h3>
      <p className="muted">
        The source is stored by content hash, so the registry pins the exact bytes that will run. A
        serve without a sandbox — or without this interpreter — refuses rather than running it
        unsandboxed.
      </p>
      <div className="register-script-form__row">
        <div className="register-script-form__field">
          <label htmlFor="script-name">Name</label>
          <input
            id="script-name"
            data-testid="script-name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="report/summarize"
            required
          />
        </div>
        <div className="register-script-form__field">
          <label htmlFor="script-version">Version</label>
          <input
            id="script-version"
            data-testid="script-version"
            value={version}
            onChange={(e) => setVersion(e.target.value)}
            required
          />
        </div>
        <div className="register-script-form__field">
          <label htmlFor="script-interpreter">Interpreter</label>
          <select
            id="script-interpreter"
            data-testid="script-interpreter"
            value={interpreter}
            onChange={(e) => setInterpreter(e.target.value as InterpreterChoice)}
          >
            {INTERPRETERS.map((choice) => (
              <option key={choice} value={choice}>
                {choice}
              </option>
            ))}
          </select>
        </div>
      </div>
      <div className="register-script-form__field">
        <label htmlFor="script-description">Description</label>
        <input
          id="script-description"
          data-testid="script-description"
          value={description}
          onChange={(e) => setDescription(e.target.value)}
          placeholder="What an agent should use this for"
        />
      </div>
      <div className="register-script-form__field">
        <label htmlFor="script-source">Source</label>
        <textarea
          id="script-source"
          data-testid="script-source"
          value={source}
          onChange={(e) => setSource(e.target.value)}
          rows={8}
          placeholder={'read -r input\nprintf "handled: %s" "$input"'}
          required
        />
      </div>
      <div className="register-script-form__field">
        <label htmlFor="script-mounts">Files it needs — one per line, as mode:path</label>
        <textarea
          id="script-mounts"
          data-testid="script-mounts"
          value={mounts}
          onChange={(e) => setMounts(e.target.value)}
          rows={3}
          placeholder={"ro:/srv/data\nrw:/srv/out"}
        />
      </div>
      {err ? (
        <p className="field-error" data-testid="register-script-error" role="alert">
          {err.message}
        </p>
      ) : null}
      <button type="submit" className="btn-primary" disabled={register.isPending}>
        {register.isPending ? "Registering…" : "Register script"}
      </button>
    </form>
  );
}
