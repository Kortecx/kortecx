import type { RecipeInfo } from "@kortecx/sdk/web";
import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useEffect, useState } from "react";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useBlueprintExport } from "../../kx/use-blueprint-export";
import { useRecipeForm, useRecipeSummaries, useRecipes } from "../../kx/use-recipes";
import {
  BLUEPRINT_NAMES_CHANGED_EVENT,
  loadBlueprintNames,
  setBlueprintName,
} from "../../lib/blueprint-names";
import { blueprintInputs } from "../../lib/export-blueprint";
import { humanizeHandle } from "../../lib/humanize-handle";
import { EmptyState } from "../EmptyState";
import { ErrorNotice } from "../ErrorNotice";
import { CodeViewer } from "../editor/CodeViewer";
import { Icon } from "../shell/Icon";

/**
 * The Workflows (definitions) TABLE (PR-A): the runnable workflow blueprints
 * (`ListRecipes`) as aligned rows (Name · Description · Version · ⋯). A row-click
 * opens the {@link WorkflowDetailDrawer} — the "popup" with the workflow's
 * DEFINITION (its free-param contract) + view + the open-in-new-window action
 * (the ONLY new-window button). Authoring/running stays in Blueprints (D141.1):
 * Run/Edit link there; this table is the workflow catalog + definition view.
 */
export function WorkflowsTable() {
  const { endpoint } = useConnection();
  const recipes = useRecipes();
  const summaries = useRecipeSummaries();
  const [names, setNames] = useState<Record<string, string>>(() => loadBlueprintNames(endpoint));
  const [open, setOpen] = useState<string | null>(null);

  useEffect(() => {
    setNames(loadBlueprintNames(endpoint));
    const onChanged = (): void => setNames(loadBlueprintNames(endpoint));
    window.addEventListener(BLUEPRINT_NAMES_CHANGED_EVENT, onChanged);
    return () => window.removeEventListener(BLUEPRINT_NAMES_CHANGED_EVENT, onChanged);
  }, [endpoint]);

  function nameFor(handle: string): string {
    const local = names[handle];
    return local && local.trim() !== "" ? local : humanizeHandle(handle);
  }

  const handles = recipes.data;
  const meta = summaries.data ?? {};

  return (
    <div data-testid="workflows-tab">
      <div className="table-toolbar">
        <Link to="/blueprints/new" className="btnlink" data-testid="new-blueprint">
          + New workflow
        </Link>
      </div>
      <p className="muted">
        Runnable workflow blueprints. Click a row to view its definition, run it, or open the visual
        builder.
      </p>

      {recipes.isLoading ? <EmptyState title="Loading workflows…" /> : null}

      {handles && handles.length === 0 ? (
        <EmptyState
          title="No workflows provisioned"
          detail="This gateway exposes the catalog but provisions no workflow blueprints."
        />
      ) : handles ? (
        <table className="data-table" data-testid="workflows-list">
          <thead>
            <tr>
              <th scope="col">Name</th>
              <th scope="col">Description</th>
              <th scope="col">Version</th>
              <th scope="col" className="data-table__actions">
                Actions
              </th>
            </tr>
          </thead>
          <tbody>
            {handles.map((h) => (
              <WorkflowRow
                key={h}
                handle={h}
                headline={nameFor(h)}
                summary={meta[h]}
                onOpen={() => setOpen(h)}
              />
            ))}
          </tbody>
        </table>
      ) : null}

      {recipes.isError && !recipes.data ? (
        <ErrorNotice error={toUiError(recipes.error)} onRetry={() => void recipes.refetch()} />
      ) : null}

      {open ? (
        <WorkflowDetailDrawer
          handle={open}
          headline={nameFor(open)}
          summary={meta[open]}
          onClose={() => setOpen(null)}
        />
      ) : null}
    </div>
  );
}

function WorkflowRow({
  handle,
  headline,
  summary,
  onOpen,
}: {
  handle: string;
  headline: string;
  summary: RecipeInfo | undefined;
  onOpen: () => void;
}) {
  const tags = summary?.tags ?? [];
  return (
    <m.tr
      className="data-table__row"
      data-testid="workflow-row"
      whileHover={{ backgroundColor: "var(--surface-elev)" }}
    >
      <td>
        <button
          type="button"
          className="data-table__name"
          data-testid={`workflow-open-${handle}`}
          onClick={onOpen}
          title="View workflow definition"
        >
          {headline}
        </button>
        <code className="mono muted card-grid__handle">{handle}</code>
      </td>
      <td className="muted">
        {summary?.description || "—"}
        {tags.length > 0 ? (
          <span className="card-grid__tags">
            {tags.map((t) => (
              <span key={t} className="chip chip--tag">
                {t}
              </span>
            ))}
          </span>
        ) : null}
      </td>
      <td className="muted">{summary?.version ? `v${summary.version}` : "—"}</td>
      <td className="data-table__actions">
        <button
          type="button"
          className="linkbtn"
          data-testid={`workflow-view-${handle}`}
          onClick={onOpen}
        >
          View
        </button>
        <Link
          to="/recipes"
          search={{ handle }}
          className="linkbtn"
          data-testid={`recipe-pick-${handle}`}
          title="Run this workflow"
        >
          Run
        </Link>
      </td>
    </m.tr>
  );
}

/**
 * The workflow "popup" (PR-A): a slide-over (`.node-drawer`) showing the
 * workflow's DEFINITION — its free-param contract in the read-only Monaco viewer
 * (D141.2) — plus Run · Edit-in-builder · Rename (client-local) · Export
 * (definition) and the ONLY open-in-new-window button (point 4).
 */
function WorkflowDetailDrawer({
  handle,
  headline,
  summary,
  onClose,
}: {
  handle: string;
  headline: string;
  summary: RecipeInfo | undefined;
  onClose: () => void;
}) {
  const { endpoint } = useConnection();
  const form = useRecipeForm(handle);
  const exporter = useBlueprintExport();
  const [draft, setDraft] = useState("");
  const exporting = exporter.pendingHandle === handle;

  useEffect(() => {
    const onKey = (e: KeyboardEvent): void => {
      if (e.key === "Escape") {
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const contract = form.data
    ? JSON.stringify({ handle: form.data.handle, inputs: blueprintInputs(form.data) }, null, 2)
    : null;

  return (
    <>
      <button
        type="button"
        className="node-drawer__scrim"
        aria-label="Close workflow details"
        onClick={onClose}
      />
      <m.aside
        className="node-drawer"
        data-testid="workflow-detail-drawer"
        // biome-ignore lint/a11y/useSemanticElements: a native <dialog> can't ride framer-motion; non-modal side-panel semantics via role+aria-label (the NodeDetailDrawer precedent)
        role="dialog"
        aria-label={`Workflow ${headline}`}
        initial={{ x: 24, opacity: 0 }}
        animate={{ x: 0, opacity: 1 }}
        transition={{ type: "spring", stiffness: 420, damping: 34 }}
      >
        <div className="node-drawer__head">
          <strong>{headline}</strong>
          <button type="button" className="linkbtn" onClick={onClose} aria-label="Close">
            ✕
          </button>
        </div>
        <code className="mono muted" title={handle}>
          {handle}
        </code>
        {summary?.description ? <p className="card-grid__sub">{summary.description}</p> : null}

        <div className="drawer-actions">
          <Link
            to="/recipes"
            search={{ handle }}
            className="btnlink"
            data-testid="workflow-run"
            onClick={onClose}
          >
            Run workflow →
          </Link>
          {/* The ONLY open-in-new-window button lives in the popup (point 4). */}
          <a
            href={`/recipes?handle=${encodeURIComponent(handle)}`}
            target="_blank"
            rel="noopener noreferrer"
            className="linkbtn"
            data-testid="workflow-open-newtab"
          >
            <Icon name="external-link" size={15} /> Open in new window
          </a>
        </div>

        <div className="drawer-section">
          <span className="muted">Definition (free-param contract)</span>
          {form.isLoading ? <EmptyState title="Loading definition…" /> : null}
          {form.error ? <ErrorNotice error={toUiError(form.error)} /> : null}
          {contract ? (
            <CodeViewer
              value={contract}
              language="json"
              testId="workflow-definition"
              ariaLabel={`Workflow definition ${handle}`}
              height={Math.min(360, Math.max(120, contract.split("\n").length * 19 + 24))}
            />
          ) : null}
        </div>

        <label className="drawer-field" htmlFor="workflow-rename-input">
          <span className="muted">Display name (this browser)</span>
          <span className="card-grid__rename">
            <input
              id="workflow-rename-input"
              value={draft}
              data-testid="workflow-rename-input"
              aria-label="Workflow name"
              placeholder={headline}
              onChange={(e) => setDraft(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  setBlueprintName(endpoint, handle, draft);
                }
              }}
              spellCheck={false}
              autoComplete="off"
            />
            <button
              type="button"
              className="linkbtn"
              data-testid="workflow-rename"
              onClick={() => setBlueprintName(endpoint, handle, draft)}
            >
              Save
            </button>
          </span>
        </label>

        <div className="drawer-actions drawer-actions--wrap">
          <Link
            to="/blueprints/new"
            className="linkbtn"
            data-testid="workflow-edit"
            onClick={onClose}
          >
            Edit in builder
          </Link>
          <button
            type="button"
            className="linkbtn"
            data-testid="workflow-export"
            disabled={exporting}
            onClick={() =>
              void exporter.exportBlueprint({
                handle,
                description: summary?.description,
                tags: summary?.tags,
                version: summary?.version,
              })
            }
          >
            {exporting ? "Exporting…" : "Export definition"}
          </button>
        </div>

        {exporter.error ? <ErrorNotice error={toUiError(exporter.error)} /> : null}
      </m.aside>
    </>
  );
}
