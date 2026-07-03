/**
 * POC-5a "New App" — an inline collapsible authoring panel (the NewBranchForm
 * precedent; NOT a dialog). It saves an App envelope (a single agentic step over
 * the goal) then launches the server-side agentic scaffold, handing off to the
 * honest {@link ScaffoldProgress} poller.
 *
 * The envelope carries NO authority (the server re-resolves warrants at run); the
 * optional model rides as a steering hint only. T-RUNAPP-CONTEXT-RAIL adds the
 * declarative knowledge rail: "Ground on dataset" chips (references.datasets → a
 * live `retrieve@1` grant at run, the App self-grounds) + a guidance rule
 * (references.rules → an entry-step context item). Both resolve server-side at
 * RunApp — the same rails the SDK `.dataset()` / `.rule()` authoring emits.
 *
 * By convention the App's project branch handle IS the saved App handle
 * (one-App-one-branch), so the scaffold + progress poll key on it.
 */

import { app, flow } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useDatasets } from "../../kx/use-datasets";
import { useScaffoldApp } from "../../kx/use-scaffold-app";
import { GlowCard } from "../ds/GlowCard";
import { ScaffoldProgress } from "./ScaffoldProgress";

export function NewAppForm({ onClose }: { onClose: () => void }) {
  const { client } = useConnection();
  const scaffold = useScaffoldApp();
  const datasets = useDatasets();
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [model, setModel] = useState("");
  const [handle, setHandle] = useState("");
  // T-RUNAPP-CONTEXT-RAIL authoring state (the declarative rail).
  const [grounding, setGrounding] = useState<string[]>([]);
  const [rule, setRule] = useState("");
  // Set once the scaffold has launched — switches the panel to the progress view.
  const [scaffolding, setScaffolding] = useState<{
    appHandle: string;
    branchHandle: string;
  } | null>(null);

  const save = useMutation<string, unknown, void>({
    mutationFn: async (): Promise<string> => {
      if (!client) {
        throw new Error("not connected");
      }
      // Build via the App builder so the declarative rail (datasets + a guidance
      // rule) rides the saved envelope; `.save()` uploads any pending rule body
      // first. With no rail selected this is byte-identical to `minimalAppEnvelope`.
      let builder = app(name.trim()).describe(goal.trim()).blueprint(flow().agent(goal.trim()));
      const trimmedModel = model.trim();
      if (trimmedModel !== "") {
        builder = builder.steer({ model: trimmedModel });
      }
      for (const ds of grounding) {
        builder = builder.dataset(ds);
      }
      const trimmedRule = rule.trim();
      if (trimmedRule !== "") {
        builder = builder.rule("guidance", { body: trimmedRule });
      }
      const trimmedHandle = handle.trim();
      const result = await builder.save({
        client,
        ...(trimmedHandle !== "" ? { handle: trimmedHandle } : {}),
      });
      return result.handle;
    },
  });

  const nameOk = name.trim().length > 0;
  const goalOk = goal.trim().length > 0;
  const busy = save.isPending || scaffold.isPending;
  const canSubmit = nameOk && goalOk && !busy && scaffolding === null;

  // Only datasets with an indexed document can ground (honest: grounding turns on
  // AFTER you ingest). No dataset view (hnsw off) / none ingested ⇒ render nothing
  // (don't-fake-gaps, the DatasetPicker precedent).
  const groundable = (datasets.data ?? []).filter((d) => d.docCount > 0);

  function toggleDataset(nameOfDataset: string): void {
    setGrounding((cur) =>
      cur.includes(nameOfDataset)
        ? cur.filter((d) => d !== nameOfDataset)
        : [...cur, nameOfDataset],
    );
  }

  function onSubmit(e: FormEvent): void {
    e.preventDefault();
    if (!canSubmit) {
      return;
    }
    save.mutate(undefined, {
      onSuccess: (appHandle) => {
        scaffold.mutate(
          { handle: appHandle, goal: goal.trim() },
          {
            onSuccess: ({ branchHandle }) => {
              setScaffolding({ appHandle, branchHandle });
            },
          },
        );
      },
    });
  }

  const saveErr = save.error ? toUiError(save.error) : null;
  const scaffoldErr = scaffold.error ? toUiError(scaffold.error) : null;

  return (
    <GlowCard hover={false} variants={fadeUp} data-testid="new-app-form">
      <div className="new-app-form__head">
        <h2>New App</h2>
        <button
          type="button"
          className="linkbtn"
          data-testid="new-app-close"
          aria-label="Close New App"
          onClick={onClose}
        >
          ✕
        </button>
      </div>
      <p className="muted">
        Describe the App and its goal. We save a durable App envelope, then the agent scaffolds a
        starter project tree into the App's own content-addressed branch (the host is never
        written). Browse and edit it after.
      </p>

      {scaffolding === null ? (
        <form onSubmit={onSubmit} className="register-tool-form">
          <input
            type="text"
            data-testid="new-app-name"
            placeholder="App name (e.g. Release Notes Writer)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            aria-label="App name"
            disabled={busy}
          />
          <textarea
            className="input"
            data-testid="new-app-goal"
            placeholder="Goal — what should this App do? (e.g. 'Summarize a changelog into release notes')"
            rows={3}
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            aria-label="App goal"
            disabled={busy}
          />
          <div className="register-tool-form__row">
            <input
              type="text"
              className="mono"
              data-testid="new-app-model"
              placeholder="model id (optional steering hint)"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              aria-label="Model id (optional)"
              disabled={busy}
            />
            <input
              type="text"
              className="mono"
              data-testid="new-app-handle"
              placeholder="handle (optional — derived from the name)"
              value={handle}
              onChange={(e) => setHandle(e.target.value)}
              aria-label="App handle (optional)"
              disabled={busy}
            />
          </div>

          {/* T-RUNAPP-CONTEXT-RAIL: "Ground on dataset" — chip toggles (a controlled
              <select> can't be Playwright-driven; chips are the standing pattern). Only
              shown when a non-empty dataset exists (don't-fake-gaps). */}
          {groundable.length > 0 ? (
            <fieldset className="new-app-form__rail" data-testid="new-app-datasets">
              <legend className="muted">Ground on dataset (RAG)</legend>
              <div className="chips">
                {groundable.map((d) => {
                  const on = grounding.includes(d.name);
                  return (
                    <button
                      key={d.datasetId}
                      type="button"
                      className={on ? "chip chip--active" : "chip"}
                      aria-pressed={on}
                      data-testid={`new-app-dataset-${d.name}`}
                      onClick={() => toggleDataset(d.name)}
                      disabled={busy}
                    >
                      {d.name} ({d.docCount})
                    </button>
                  );
                })}
              </div>
            </fieldset>
          ) : null}

          <textarea
            className="input"
            data-testid="new-app-rule"
            placeholder="Guidance rule (optional) — a behavior note the agent must follow (e.g. 'Always cite sources.')"
            rows={2}
            value={rule}
            onChange={(e) => setRule(e.target.value)}
            aria-label="Guidance rule (optional)"
            disabled={busy}
          />

          <div className="register-tool-form__row">
            <button type="submit" data-testid="new-app-submit" disabled={!canSubmit}>
              {busy ? "Scaffolding…" : "Create & scaffold"}
            </button>
            <button
              type="button"
              className="btn-ghost"
              data-testid="new-app-cancel"
              onClick={onClose}
              disabled={busy}
            >
              Cancel
            </button>
          </div>
        </form>
      ) : (
        <ScaffoldProgress
          branchHandle={scaffolding.branchHandle}
          appHandle={scaffolding.appHandle}
        />
      )}

      {saveErr ? (
        <p className="field-error" data-testid="new-app-save-error" role="alert">
          {saveErr.message}
        </p>
      ) : null}
      {scaffoldErr ? (
        <p className="field-error" data-testid="new-app-scaffold-error" role="alert">
          {scaffoldErr.message}
        </p>
      ) : null}
    </GlowCard>
  );
}
