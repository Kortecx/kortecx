import type { AppSummary } from "@kortecx/sdk/web";
import { useNavigate } from "@tanstack/react-router";
import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { TEMPLATE_TAG, useApps, useCloneApp, useToggleTemplate } from "../../kx/use-apps";
import { EmptyState } from "../EmptyState";
import { AppViewPopover } from "../apps/AppViewPopover";

/**
 * The Templates tab — reusable App templates. An App tagged `template` shows here;
 * "Use template" clones it (`cloneApp` — content already resident, no transfer) into a
 * new App you then enhance with the App's Chat & edit gate. Mark any existing App as a
 * template below. Built-in-local only, no new RPC: reads the App catalog, writes the
 * reserved tag via SaveApp, clones via CloneApp. Inline naming (no modal) — the whole
 * arc is self-contained here.
 */
export function WorkflowsTemplatesPanel() {
  const { apps, notWired, isLoading } = useApps();
  const toggle = useToggleTemplate();
  const [preview, setPreview] = useState<string | null>(null);

  const templates = apps.filter((a) => a.tags.includes(TEMPLATE_TAG));
  const markable = apps.filter((a) => !a.tags.includes(TEMPLATE_TAG));

  return (
    <div data-testid="workflows-templates">
      {notWired ? (
        <EmptyState
          title="Templates unavailable"
          detail="This gateway does not expose the App catalog."
        />
      ) : isLoading ? (
        <EmptyState title="Loading templates…" />
      ) : templates.length === 0 ? (
        <EmptyState
          title="No templates yet"
          detail="Mark any App as a template to reuse it here — clone it, then enhance the copy with Chat & edit."
          action={
            <span className="chip chip--soon" data-testid="workflows-templates-placeholder">
              Mark an App below
            </span>
          }
        />
      ) : (
        <m.div
          className="card-grid"
          data-testid="workflows-templates-grid"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {templates.map((t) => (
            <TemplateCard
              key={t.handle}
              app={t}
              onPreview={() => setPreview(t.handle)}
              onUnmark={() => toggle.mutate({ handle: t.handle })}
              busy={toggle.isPending}
            />
          ))}
        </m.div>
      )}

      {markable.length > 0 ? (
        <div className="skills-rail" data-testid="templates-markable">
          <h3>Reuse an existing App as a template</h3>
          <p className="muted">
            Marking adds the <code className="mono">template</code> tag; the App still runs and
            edits normally.
          </p>
          <div className="chip-row">
            {markable.map((a) => (
              <button
                key={a.handle}
                type="button"
                className="chip"
                disabled={toggle.isPending}
                title={`Mark ${a.name} as a template`}
                onClick={() => toggle.mutate({ handle: a.handle })}
                data-testid={`template-mark-${a.handle}`}
              >
                + {a.name}
              </button>
            ))}
          </div>
          {toggle.error ? (
            <p className="field-error" data-testid="templates-error">
              {toUiError(toggle.error).message}
            </p>
          ) : null}
        </div>
      ) : null}

      {preview ? <AppViewPopover handle={preview} onClose={() => setPreview(null)} /> : null}
    </div>
  );
}

function TemplateCard({
  app,
  onPreview,
  onUnmark,
  busy,
}: {
  app: AppSummary;
  onPreview: () => void;
  onUnmark: () => void;
  busy: boolean;
}) {
  const navigate = useNavigate();
  const clone = useCloneApp();
  const [naming, setNaming] = useState(false);
  const [name, setName] = useState("");
  const trimmed = name.trim();

  const create = () => {
    if (trimmed === "") {
      return;
    }
    clone.mutate(
      { handle: app.handle, newname: trimmed },
      {
        onSuccess: ({ handle }) => {
          setNaming(false);
          setName("");
          clone.reset();
          void navigate({ to: "/apps/$handle", params: { handle } });
        },
      },
    );
  };

  return (
    <m.article
      variants={fadeUp}
      {...hoverLift}
      className="glow-card glow-card--hover card-grid__card"
      data-testid={`template-card-${app.handle}`}
    >
      <div className="card-grid__head">
        <button
          type="button"
          className="card-grid__title card-grid__title-btn"
          title={`${app.name} — preview`}
          data-testid={`template-preview-${app.handle}`}
          onClick={onPreview}
        >
          {app.name}
        </button>
        <div className="card-grid__head-actions">
          <button
            type="button"
            className="btn-primary"
            data-testid={`template-use-${app.handle}`}
            onClick={() => setNaming((v) => !v)}
          >
            Use template
          </button>
        </div>
      </div>
      {app.description ? <p className="card-grid__sub">{app.description}</p> : null}
      <div className="card-grid__tags">
        <span className="chip chip--tag">
          {app.stepCount} step{app.stepCount === 1 ? "" : "s"}
        </span>
        {app.tags
          .filter((t) => t !== TEMPLATE_TAG)
          .map((t) => (
            <span key={t} className="chip chip--tag">
              {t}
            </span>
          ))}
      </div>
      {naming ? (
        <div className="chip-row" data-testid={`template-clone-form-${app.handle}`}>
          <input
            className="input"
            placeholder="New App name"
            value={name}
            disabled={clone.isPending}
            onChange={(e) => setName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                create();
              }
            }}
            data-testid={`template-clone-name-${app.handle}`}
            aria-label="New App name"
            spellCheck={false}
            autoComplete="off"
          />
          <button
            type="button"
            className="btn-primary"
            disabled={clone.isPending || trimmed === ""}
            onClick={create}
            data-testid={`template-clone-submit-${app.handle}`}
          >
            {clone.isPending ? "Creating…" : "Create"}
          </button>
          <button
            type="button"
            className="btn-ghost"
            disabled={clone.isPending}
            onClick={() => {
              setNaming(false);
              clone.reset();
            }}
          >
            Cancel
          </button>
        </div>
      ) : null}
      {clone.error ? (
        <p className="field-error" data-testid={`template-clone-error-${app.handle}`}>
          {toUiError(clone.error).message}
        </p>
      ) : null}
      <button
        type="button"
        className="linkbtn"
        disabled={busy}
        onClick={onUnmark}
        data-testid={`template-unmark-${app.handle}`}
      >
        Remove from templates
      </button>
    </m.article>
  );
}
