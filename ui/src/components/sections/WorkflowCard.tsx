import type { RecipeInfo } from "@kortecx/sdk/web";
import { Link } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, hoverLift } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { toUiError } from "../../kx/errors";
import { useBlueprintExport } from "../../kx/use-blueprint-export";
import { setBlueprintName } from "../../lib/blueprint-names";
import { ErrorNotice } from "../ErrorNotice";
import { Icon } from "../shell/Icon";
import { Popover } from "../shell/Popover";
import { ScheduleButton } from "./ScheduleButton";

/**
 * One workflow as a clean, high-level card: a display-name headline (client-local
 * rename > humanized handle) + description, and a top-right action cluster — Run,
 * Schedule (the shipped local CRON trigger), an honest-disabled Share ("Cloud"),
 * and a kebab for the secondary actions (Edit in builder · Rename · Export · Open
 * in new tab). No lock (workflows aren't lockable — that's an App capability) and
 * no raw handle chip (the section stays high-level). Clicking the name opens the
 * run-input form.
 */
export function WorkflowCard({
  handle,
  headline,
  customName,
  summary,
  onRun,
}: {
  handle: string;
  /** Display headline (local rename > humanized handle). */
  headline: string;
  /** The current client-local custom name (seeds the rename draft), or null. */
  customName: string | null;
  /** Advisory catalog metadata (description/tags/version), if the gateway has it. */
  summary: RecipeInfo | undefined;
  /** Open this workflow's input form (the run-form drawer). */
  onRun: (handle: string) => void;
}) {
  const { endpoint } = useConnection();
  const exporter = useBlueprintExport();
  const [renaming, setRenaming] = useState(false);
  const [draft, setDraft] = useState(customName ?? "");

  const description = summary?.description ?? "";
  const tags = summary?.tags ?? [];
  const version = summary?.version ?? "";
  const exporting = exporter.pendingHandle === handle;

  function saveRename(): void {
    setBlueprintName(endpoint, handle, draft);
    setRenaming(false);
  }

  return (
    <m.article
      className="glow-card glow-card--hover card-grid__card"
      data-testid="workflow-card"
      variants={fadeUp}
      {...hoverLift}
    >
      <div className="card-grid__head">
        <button
          type="button"
          className="card-grid__title card-grid__title-btn"
          data-testid={`workflow-open-${handle}`}
          title="Run this workflow"
          onClick={() => onRun(handle)}
        >
          {headline}
        </button>
        <div className="card-grid__head-actions">
          <button
            type="button"
            className="iconbtn"
            data-testid={`workflow-run-${handle}`}
            title="Run this workflow"
            aria-label="Run"
            onClick={() => onRun(handle)}
          >
            <Icon name="play" size={16} />
          </button>
          <ScheduleButton
            recipeHandle={handle}
            triggerClassName="iconbtn"
            iconOnly
            testId={`workflow-schedule-${handle}`}
          />
          <span
            className="iconbtn iconbtn--disabled"
            data-testid={`workflow-share-${handle}`}
            aria-disabled="true"
            title="Sharing across parties is a Cloud capability"
          >
            <Icon name="share" size={16} />
          </span>
          <Popover
            trigger={<Icon name="menu" size={16} />}
            triggerClassName="iconbtn"
            triggerLabel="Workflow actions"
            triggerTestId={`workflow-menu-${handle}`}
            align="right"
            direction="down"
            menuTestId={`workflow-menu-panel-${handle}`}
          >
            {(close) => (
              <>
                <Link
                  to="/blueprints/new"
                  role="menuitem"
                  className="popover__item"
                  data-testid="workflow-edit"
                  title="Open the visual builder to edit a copy of this workflow"
                  onClick={close}
                >
                  <Icon name="settings" size={15} />
                  <span>Edit in builder</span>
                </Link>
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid="workflow-rename"
                  onClick={() => {
                    setDraft(customName ?? "");
                    setRenaming(true);
                    close();
                  }}
                >
                  <Icon name="settings" size={15} />
                  <span>Rename</span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  className="popover__item"
                  data-testid="workflow-export"
                  disabled={exporting}
                  title="Export this workflow's definition (contract + metadata) as JSON"
                  onClick={() => {
                    void exporter
                      .exportBlueprint({ handle, description, tags, version })
                      .then(close);
                  }}
                >
                  <Icon name="download" size={15} />
                  <span>{exporting ? "Exporting…" : "Export"}</span>
                </button>
                <Link
                  to="/recipes"
                  search={{ handle }}
                  target="_blank"
                  rel="noopener noreferrer"
                  role="menuitem"
                  className="popover__item"
                  data-testid="workflow-open-newtab"
                  onClick={close}
                >
                  <Icon name="external-link" size={15} />
                  <span>Open in new tab</span>
                </Link>
              </>
            )}
          </Popover>
        </div>
      </div>

      {description ? <p className="card-grid__sub">{description}</p> : null}

      {renaming ? (
        <span className="card-grid__rename">
          <input
            value={draft}
            data-testid="workflow-rename-input"
            aria-label="Workflow name"
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                saveRename();
              }
              if (e.key === "Escape") {
                setRenaming(false);
              }
            }}
            spellCheck={false}
            autoComplete="off"
          />
          <button type="button" className="linkbtn" onClick={saveRename}>
            Save
          </button>
        </span>
      ) : null}

      {exporter.error ? <ErrorNotice error={toUiError(exporter.error)} /> : null}
    </m.article>
  );
}
