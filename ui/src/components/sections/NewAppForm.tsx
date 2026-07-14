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
import { Link } from "@tanstack/react-router";
import { type FormEvent, useState } from "react";
import { fadeUp } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useAttachments } from "../../kx/use-attachments";
import { useDatasets } from "../../kx/use-datasets";
import { useModels } from "../../kx/use-models";
import { useScaffoldApp } from "../../kx/use-scaffold-app";
import { composeCapabilityPrompt } from "../../lib/app-capability-prompt";
import { GlowCard } from "../ds/GlowCard";
import { ScaffoldProgress } from "./ScaffoldProgress";

export function NewAppForm({ onClose }: { onClose: () => void }) {
  const { client } = useConnection();
  const scaffold = useScaffoldApp();
  const datasets = useDatasets();
  const models = useModels();
  const attach = useAttachments();
  const [name, setName] = useState("");
  const [goal, setGoal] = useState("");
  const [prompt, setPrompt] = useState("");
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
      // The App's agent step runs the PROMPT (the instruction); the GOAL is the
      // description. A comprehensive capability rule teaches the model what a Kortecx
      // App is + how to drive the runtime (tools/connections/datasets/skills/files).
      const promptText = prompt.trim() || goal.trim();
      // Attachments already uploaded (PutContent) ride as by-reference context files.
      const readyFiles = attach.attachments.filter((a) => a.status === "ready" && a.ref);
      let builder = app(name.trim())
        .describe(goal.trim())
        .blueprint(flow().agent(promptText))
        .rule("capabilities", {
          body: composeCapabilityPrompt(
            goal.trim(),
            readyFiles.map((a) => a.filename),
          ),
        });
      const trimmedModel = model.trim();
      if (trimmedModel !== "") {
        builder = builder.steer({ model: trimmedModel });
      }
      for (const a of readyFiles) {
        builder = builder.context(a.filename, a.ref as string, { mediaType: a.mediaType });
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
  const canSubmit = nameOk && goalOk && !busy && !attach.uploading && scaffolding === null;

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
            rows={2}
            value={goal}
            onChange={(e) => setGoal(e.target.value)}
            aria-label="App goal"
            disabled={busy}
          />
          <textarea
            className="input"
            data-testid="new-app-prompt"
            placeholder="Prompt (optional) — the instruction the model runs each time. Defaults to the goal. A comprehensive capability prompt (how to use the runtime's tools, connections, datasets & files) is added automatically."
            rows={3}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            aria-label="App prompt"
            disabled={busy}
          />
          <div className="register-tool-form__row">
            <select
              className="mono"
              data-testid="new-app-model"
              value={model}
              onChange={(e) => setModel(e.target.value)}
              aria-label="Model"
              disabled={busy}
              title="Pick a local model, or leave on the served default. External providers connect via Integrations."
            >
              <option value="">Served default</option>
              {(models.models ?? []).map((m) => (
                <option key={m.modelId} value={m.modelId}>
                  {m.modelId}
                  {m.modalities.includes("image") ? " (vision)" : ""}
                </option>
              ))}
            </select>
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

          {/* Attachments — uploaded (PutContent) and attached to the App as
              by-reference context files the model reads at run. */}
          <fieldset className="new-app-form__rail" data-testid="new-app-attachments">
            <legend className="muted">Attachments (context files)</legend>
            <input
              type="file"
              multiple
              data-testid="new-app-attach-input"
              onChange={(e) => {
                if (e.target.files && e.target.files.length > 0) {
                  attach.addFiles(e.target.files);
                  e.target.value = "";
                }
              }}
              aria-label="Attach context files"
              disabled={busy}
            />
            {attach.attachments.length > 0 ? (
              <div className="chips">
                {attach.attachments.map((a) => (
                  <span
                    key={a.id}
                    className="chip"
                    data-testid={`new-app-attachment-${a.filename}`}
                  >
                    {a.filename}
                    {a.status !== "ready" ? " · uploading…" : ""}
                    <button
                      type="button"
                      className="context-strip__remove"
                      aria-label={`Remove ${a.filename}`}
                      onClick={() => attach.remove(a.id)}
                      disabled={busy}
                    >
                      ✕
                    </button>
                  </span>
                ))}
              </div>
            ) : null}
          </fieldset>

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
          <p className="muted">
            Prefer to compose the structure yourself?{" "}
            <Link to="/blueprints/new" className="linkbtn" data-testid="new-app-build-visual">
              Build in the visual builder →
            </Link>
          </p>
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
