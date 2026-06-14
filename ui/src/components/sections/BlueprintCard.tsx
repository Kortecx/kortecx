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

/**
 * One blueprint as a bordered card (PR-4.1b): a clean display name headline +
 * description subtitle + advisory tag/version chips (the raw handle stays a
 * secondary mono chip) + a per-card action menu (the reused `Popover`/D148).
 * "Edit in builder" is the blueprint's honest settings (the builder IS the
 * editor); Share + Schedule are cross-party / cloud (D129) → honest-disabled
 * "Cloud" chips (D142 don't-fake-gaps). The card title opens the run form.
 */
export function BlueprintCard({
  handle,
  headline,
  customName,
  summary,
  onRun,
  onView,
}: {
  handle: string;
  /** Display headline (local rename > humanized handle). */
  headline: string;
  /** The current client-local custom name (seeds the rename draft), or null. */
  customName: string | null;
  /** Advisory catalog metadata (description/tags/version), if the gateway has it. */
  summary: RecipeInfo | undefined;
  /** Open this blueprint's input form (the run-form drawer). */
  onRun: (handle: string) => void;
  /** Open this blueprint's contract viewer. */
  onView: (handle: string) => void;
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
      data-testid="blueprint-card"
      variants={fadeUp}
      {...hoverLift}
    >
      <div className="card-grid__head">
        <button
          type="button"
          className="card-grid__title"
          data-testid={`recipe-pick-${handle}`}
          title="Open this blueprint's input form"
          onClick={() => onRun(handle)}
        >
          {headline}
        </button>
        <Popover
          trigger={<Icon name="menu" size={16} />}
          triggerClassName="iconbtn"
          triggerLabel="Blueprint actions"
          triggerTestId="blueprint-menu"
          align="right"
          direction="down"
          menuTestId="blueprint-menu-panel"
        >
          {(close) => (
            <>
              <button
                type="button"
                role="menuitem"
                className="popover__item"
                data-testid={`recipe-run-${handle}`}
                onClick={() => {
                  close();
                  onRun(handle);
                }}
              >
                <Icon name="runs" size={15} />
                <span>Run</span>
              </button>
              <Link
                to="/recipes"
                search={{ handle }}
                target="_blank"
                rel="noopener noreferrer"
                role="menuitem"
                className="popover__item"
                data-testid="blueprint-open-newtab"
                onClick={close}
              >
                <Icon name="external-link" size={15} />
                <span>Open in new tab</span>
              </Link>
              <button
                type="button"
                role="menuitem"
                className="popover__item"
                data-testid={`recipe-view-${handle}`}
                title="View the blueprint's contract (inputs + types)"
                onClick={() => {
                  close();
                  onView(handle);
                }}
              >
                <Icon name="recipes" size={15} />
                <span>View contract</span>
              </button>
              <Link
                to="/blueprints/new"
                role="menuitem"
                className="popover__item"
                data-testid="blueprint-edit"
                title="Open the visual builder to edit a copy of this blueprint"
                onClick={close}
              >
                <Icon name="settings" size={15} />
                <span>Edit in builder</span>
              </Link>
              <button
                type="button"
                role="menuitem"
                className="popover__item"
                data-testid="blueprint-rename"
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
                data-testid="blueprint-export"
                disabled={exporting}
                title="Export this blueprint's definition (contract + metadata) as JSON"
                onClick={() => {
                  void exporter.exportBlueprint({ handle, description, tags, version }).then(close);
                }}
              >
                <Icon name="download" size={15} />
                <span>{exporting ? "Exporting…" : "Export"}</span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="popover__item popover__item--disabled"
                data-testid="blueprint-share"
                disabled
                aria-disabled="true"
                title="Share across parties — a managed cloud capability"
              >
                <Icon name="share" size={15} />
                <span>Share</span>
                <span className="chip chip--soon">Cloud</span>
              </button>
              <button
                type="button"
                role="menuitem"
                className="popover__item popover__item--disabled"
                data-testid="blueprint-schedule"
                disabled
                aria-disabled="true"
                title="Schedule recurring runs — a managed cloud capability"
              >
                <Icon name="calendar" size={15} />
                <span>Schedule</span>
                <span className="chip chip--soon">Cloud</span>
              </button>
            </>
          )}
        </Popover>
      </div>

      {description ? <p className="card-grid__sub">{description}</p> : null}

      {renaming ? (
        <span className="card-grid__rename">
          <input
            value={draft}
            data-testid="blueprint-rename-input"
            aria-label="Blueprint name"
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

      {tags.length > 0 ? (
        <div className="card-grid__tags">
          {tags.map((t) => (
            <span key={t} className="chip chip--tag">
              {t}
            </span>
          ))}
        </div>
      ) : null}

      <div className="card-grid__meta">
        <code className="mono card-grid__handle" title={handle}>
          {handle}
        </code>
        {version ? <span className="card-grid__time">v{version}</span> : null}
      </div>

      {exporter.error ? <ErrorNotice error={toUiError(exporter.error)} /> : null}
    </m.article>
  );
}
