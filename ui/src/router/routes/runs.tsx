import { createRoute, useNavigate } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { ErrorNotice } from "../../components/ErrorNotice";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useInvoke } from "../../kx/use-invoke";
import { useSignatures } from "../../kx/use-signatures";
import { shortHex } from "../../lib/format";
import { rootRoute } from "./__root";

const DEFAULT_HANDLE = "kx/recipes/echo";
const DEFAULT_ARGS = '{\n  "topic": "hello"\n}';

function RunsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return <RunsContent />;
}

function RunsContent() {
  const navigate = useNavigate();
  const signatures = useSignatures();
  const invoke = useInvoke();
  const [handle, setHandle] = useState(DEFAULT_HANDLE);
  const [argsText, setArgsText] = useState(DEFAULT_ARGS);
  const [argsError, setArgsError] = useState<string | null>(null);

  function submit(e: FormEvent<HTMLFormElement>): void {
    e.preventDefault();
    let args: Record<string, unknown>;
    try {
      const parsed: unknown = JSON.parse(argsText);
      if (parsed === null || typeof parsed !== "object" || Array.isArray(parsed)) {
        throw new Error("args must be a JSON object");
      }
      args = parsed as Record<string, unknown>;
    } catch (err) {
      setArgsError((err as Error).message);
      return;
    }
    setArgsError(null);
    invoke.mutate(
      { handle: handle.trim(), args },
      { onSuccess: (instanceId) => navigate({ to: "/runs/$instanceId", params: { instanceId } }) },
    );
  }

  const invokeError = invoke.error ? toUiError(invoke.error) : null;

  return (
    <section className="screen">
      <h1>Run a recipe</h1>
      <p className="muted">
        The built-in <code>kx/recipes/echo</code> recipe works against any <code>kx serve</code> —
        submit it to watch a run execute.
      </p>

      <form className="invoke-form" onSubmit={submit}>
        <label htmlFor="handle">Recipe handle</label>
        <input
          id="handle"
          value={handle}
          onChange={(e) => setHandle(e.target.value)}
          spellCheck={false}
          autoComplete="off"
        />
        <label htmlFor="args">Args (JSON object)</label>
        <textarea
          id="args"
          value={argsText}
          onChange={(e) => setArgsText(e.target.value)}
          rows={5}
          spellCheck={false}
        />
        {argsError ? (
          <p className="field-error" role="alert">
            {argsError}
          </p>
        ) : null}
        <button type="submit" disabled={invoke.isPending}>
          {invoke.isPending ? "Submitting…" : "Submit run"}
        </button>
      </form>
      {invokeError ? <ErrorNotice error={invokeError} onRetry={() => invoke.reset()} /> : null}

      <h2>Published recipes</h2>
      {signatures.isLoading ? <EmptyState title="Loading recipes…" /> : null}
      {signatures.error ? (
        <ErrorNotice
          error={toUiError(signatures.error)}
          onRetry={() => void signatures.refetch()}
        />
      ) : null}
      {signatures.data && signatures.data.length === 0 ? (
        <EmptyState
          title="No published recipes"
          detail="The built-in kx/recipes/echo recipe still works above."
        />
      ) : null}
      {signatures.data && signatures.data.length > 0 ? (
        <ul className="sig-list">
          {signatures.data.map((s) => (
            <li key={s.signatureId}>
              <button type="button" className="linkbtn" onClick={() => setHandle(s.name)}>
                {s.name}
              </button>
              <code className="mono">{shortHex(s.signatureId)}</code>
            </li>
          ))}
        </ul>
      ) : null}
    </section>
  );
}

export const runsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/runs",
  component: RunsScreen,
});
