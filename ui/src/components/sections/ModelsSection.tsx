import { m } from "framer-motion";
import { useState } from "react";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useActiveModel } from "../../kx/use-active-model";
import { useDefaultModel } from "../../kx/use-default-model";
import { useModelLifecycle } from "../../kx/use-model-lifecycle";
import { useModelPull } from "../../kx/use-model-pull";
import { useModels } from "../../kx/use-models";
import { useServerInfo } from "../../kx/use-server-info";
import { EmptyState } from "../EmptyState";
import { Badge } from "../ds/Badge";

/**
 * The Models view — the catalog over the models serving this gateway (`ListModels`),
 * plus Model Control v2 switching + acquisition. Display/selection stays a
 * server-validated recipe parameter (SN-8); the controls here manage RAM residency
 * (load/offload), the server's ACTIVE default (`SetActiveModel` — an off-journal
 * advisory hint), and model DOWNLOADS (`PullModel` — operator-gated, deny-by-default).
 * Every state is designed + honest (D142 / GR15 don't-fake-gaps): an FFI-free serve
 * lists empty; downloads OFF render an honest-disabled Pull panel with the reason.
 */
export function ModelsSection() {
  const { models, unsupported, loading } = useModels();
  const { load, offload } = useModelLifecycle();
  const { setActive } = useActiveModel();
  const { defaultModelId, setDefault, clearDefault } = useDefaultModel();
  const hasModels = models !== undefined && models.length > 0;

  // Engine grouping so an Ollama ∥ llama.cpp split is obvious (stable order).
  const engines = hasModels ? [...new Set(models.map((mdl) => mdl.engine))].sort() : [];

  return (
    <section className="screen" data-testid="models-section">
      <div className="section-head">
        <div>
          <h1>Models</h1>
          <p className="muted">
            The models serving this gateway, grouped by engine. Load or offload one, make a model
            the active default for new chats, or pull a new one. Switching never routes a turn
            directly — selection stays a server-validated recipe parameter (SN-8).
          </p>
        </div>
      </div>

      {loading ? <EmptyState title="Loading models…" /> : null}

      {unsupported ? (
        <EmptyState
          title="Model discovery not wired"
          detail="This gateway predates ListModels (an older build). Update it to list the models it serves here."
        />
      ) : null}

      {!loading && !unsupported && models?.length === 0 ? (
        <EmptyState
          title="No models on this serve"
          detail="This gateway serves no model. Run a local Ollama and (re)start kx serve to auto-detect it, set KX_SERVE_MODEL_GGUF for a llama.cpp model, or build with --features inference."
        />
      ) : null}

      {hasModels
        ? engines.map((engine) => (
            <div key={engine || "models"} className="models-engine-group">
              {engine ? (
                <h2 className="models-engine-head" data-testid={`models-engine-${engine}`}>
                  <Badge label={engine.replace(/^kx-/, "")} color="var(--text-2)" />
                </h2>
              ) : null}
              <m.div
                className="card-grid"
                data-testid="models-grid"
                variants={stagger()}
                initial="hidden"
                animate="show"
              >
                {models
                  .filter((mdl) => mdl.engine === engine)
                  .map((mdl) => {
                    const loadingThis = load.isPending && load.variables === mdl.modelId;
                    const offloadingThis = offload.isPending && offload.variables === mdl.modelId;
                    const activatingThis =
                      setActive.isPending && setActive.variables === mdl.modelId;
                    const busy = loadingThis || offloadingThis;
                    const isDefault = mdl.modelId === defaultModelId;
                    const actionError =
                      load.isError && load.variables === mdl.modelId
                        ? toUiError(load.error)
                        : offload.isError && offload.variables === mdl.modelId
                          ? toUiError(offload.error)
                          : setActive.isError && setActive.variables === mdl.modelId
                            ? toUiError(setActive.error)
                            : null;
                    return (
                      <m.article
                        key={mdl.modelId}
                        className="glow-card glow-card--hover card-grid__card"
                        data-testid="model-card"
                        variants={fadeUp}
                        {...hoverLift}
                      >
                        <div className="card-grid__head">
                          <code className="mono card-grid__title" title={mdl.modelId}>
                            {mdl.modelId}
                          </code>
                          <Badge
                            label={mdl.serving ? "serving" : "idle"}
                            color={mdl.serving ? "var(--success)" : "var(--text-3)"}
                            dot
                            pulse={mdl.serving}
                          />
                        </div>
                        {mdl.description ? (
                          <p className="card-grid__sub">{mdl.description}</p>
                        ) : null}
                        {mdl.modalities.length > 0 ? (
                          <div className="card-grid__tags">
                            {mdl.modalities.map((mod) => (
                              <span key={mod} className="chip chip--tag">
                                {mod}
                              </span>
                            ))}
                          </div>
                        ) : null}
                        <div className="card-grid__meta">
                          <span className="card-grid__handle">
                            ctx {mdl.contextLen.toLocaleString()} tokens
                          </span>
                          {/* Model Control v2: the server's ACTIVE default (advisory). */}
                          {mdl.active ? (
                            <Badge label="active" color="var(--accent)" dot pulse />
                          ) : null}
                          {/* Model Control v2: a model pulled at runtime. */}
                          {mdl.source === "pulled-ollama" || mdl.source === "pulled-url" ? (
                            <Badge label="pulled" color="var(--text-2)" />
                          ) : null}
                          {mdl.canEmbed ? (
                            <Badge label="embed" color="var(--accent-2, var(--accent))" />
                          ) : null}
                          <Badge
                            label={mdl.loaded ? "loaded" : "not loaded"}
                            color={mdl.loaded ? "var(--accent)" : "var(--text-3)"}
                            dot
                          />
                        </div>
                        <div className="chip-row" data-testid="model-actions">
                          {/* Model Control v2: the SERVER active default (shared across
                              surfaces) — distinct from the client-local browser default. */}
                          {mdl.active ? (
                            <button
                              type="button"
                              className="chip chip--tag model-active-chip"
                              data-testid={`model-active-badge-${mdl.modelId}`}
                              title="The server's active default model. Click to clear (back to the primary)."
                              disabled={activatingThis}
                              onClick={() => setActive.mutate("")}
                            >
                              ★ Active
                            </button>
                          ) : (
                            <button
                              type="button"
                              className="btn-ghost"
                              data-testid={`model-make-active-${mdl.modelId}`}
                              title="Make this the server's active default model for new chats (shared)."
                              disabled={activatingThis}
                              onClick={() => setActive.mutate(mdl.modelId)}
                            >
                              {activatingThis ? "Activating…" : "Make active"}
                            </button>
                          )}
                          {/* The client-local default for new chats (this browser only). */}
                          {isDefault ? (
                            <button
                              type="button"
                              className="chip chip--tag model-default-chip"
                              data-testid={`model-default-badge-${mdl.modelId}`}
                              title="Default for new chats on THIS browser. Click to clear."
                              onClick={() => clearDefault()}
                            >
                              this browser
                            </button>
                          ) : (
                            <button
                              type="button"
                              className="btn-ghost"
                              data-testid={`model-set-default-${mdl.modelId}`}
                              title="Use this model by default for new chats (this browser only)."
                              onClick={() => setDefault(mdl.modelId)}
                            >
                              Pin here
                            </button>
                          )}
                          {mdl.loaded ? (
                            <button
                              type="button"
                              className="btn-ghost"
                              data-testid="model-offload-btn"
                              disabled={busy}
                              onClick={() => offload.mutate(mdl.modelId)}
                            >
                              {offloadingThis ? "Offloading…" : "Offload"}
                            </button>
                          ) : (
                            <button
                              type="button"
                              className="btn-ghost"
                              data-testid="model-load-btn"
                              disabled={busy}
                              onClick={() => load.mutate(mdl.modelId)}
                            >
                              {loadingThis ? "Loading…" : "Load"}
                            </button>
                          )}
                        </div>
                        {actionError ? (
                          <p
                            className="card-grid__sub"
                            data-testid="model-action-error"
                            role="alert"
                          >
                            {actionError.message}
                          </p>
                        ) : null}
                      </m.article>
                    );
                  })}
              </m.div>
            </div>
          ))
        : null}

      {/* Model Control v2: pull a model (operator-gated). A real panel when downloads
          are enabled; an honest-disabled card with the reason when off (D142/GR15). */}
      <PullPanel />

      {/* Honest-disabled Cloud capability (D129): managed vendor keys + OAuth are Cloud. */}
      <m.div
        className="metrics-grid"
        data-testid="models-cloud"
        variants={stagger()}
        initial="hidden"
        animate="show"
      >
        <div
          className="metric-card metric-card--disabled"
          data-testid="models-cloud-connect"
          aria-disabled="true"
        >
          <span className="metric-card__value">
            <span className="chip--soon">Cloud</span>
          </span>
          <span className="metric-card__label">Connect a cloud provider</span>
          <span className="metric-card__sub">
            Managed vendor keys + OAuth arrive with Cloud (D129).
          </span>
        </div>
      </m.div>
    </section>
  );
}

/**
 * Model Control v2 — the "Pull a model" panel. Enabled only when the operator set
 * `KX_SERVE_ALLOW_MODEL_PULL` (deny-by-default); otherwise an honest-disabled card
 * with the exact reason. Progress is REAL (polled `GetPullStatus` byte facts), never
 * a fabricated bar (GR15). Two sources: an Ollama tag (quick/easy) or a direct
 * HuggingFace `/resolve/` GGUF URL + its SHA-256 (verified before registration).
 */
function PullPanel() {
  const serverInfo = useServerInfo();
  const { start, status, startError, starting, active, reset } = useModelPull();
  const [tag, setTag] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [url, setUrl] = useState("");
  const [sha256, setSha256] = useState("");

  const allowed = serverInfo.data?.allowModelPull === true;

  if (!allowed) {
    return (
      <m.div
        className="metrics-grid"
        data-testid="models-pull"
        variants={stagger()}
        initial="hidden"
        animate="show"
      >
        <div
          className="metric-card metric-card--disabled"
          data-testid="models-pull-disabled"
          aria-disabled="true"
        >
          <span className="metric-card__value">
            <span className="chip--soon">Off</span>
          </span>
          <span className="metric-card__label">Pull a model</span>
          <span className="metric-card__sub">
            Model downloads are disabled. Set <code>KX_SERVE_ALLOW_MODEL_PULL=1</code> on the server
            to enable them (operator opt-in).
          </span>
        </div>
      </m.div>
    );
  }

  const canPull =
    !starting &&
    !active &&
    (advanced ? url.trim() !== "" && sha256.trim() !== "" : tag.trim() !== "");
  const onPull = () => {
    if (advanced) {
      void start({ url: url.trim(), sha256: sha256.trim() });
    } else {
      void start({ ollamaTag: tag.trim() });
    }
  };

  return (
    <m.div
      className="glow-card models-pull-panel"
      data-testid="models-pull"
      variants={fadeUp}
      initial="hidden"
      animate="show"
    >
      <div className="card-grid__head">
        <h2 className="card-grid__title">Pull a model</h2>
        <Badge label="enabled" color="var(--success)" dot />
      </div>
      <p className="card-grid__sub">
        Download + register a model without a restart. Pulls from the Ollama registry, or a
        HuggingFace <code>/resolve/</code> GGUF URL (verified by SHA-256). It becomes switchable +
        chattable immediately.
      </p>
      {advanced ? (
        <div className="form-stack">
          <input
            className="input"
            data-testid="models-pull-url"
            placeholder="https://huggingface.co/<repo>/resolve/<rev>/<file>.gguf"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            disabled={starting || active}
          />
          <input
            className="input"
            data-testid="models-pull-sha256"
            placeholder="sha256 (hex) — required for a direct URL"
            value={sha256}
            onChange={(e) => setSha256(e.target.value)}
            disabled={starting || active}
          />
        </div>
      ) : (
        <input
          className="input"
          data-testid="models-pull-tag"
          placeholder="Ollama tag, e.g. gemma3:12b"
          value={tag}
          onChange={(e) => setTag(e.target.value)}
          disabled={starting || active}
        />
      )}
      <div className="chip-row">
        <button
          type="button"
          className="btn-primary"
          data-testid="models-pull-go"
          disabled={!canPull}
          onClick={onPull}
        >
          {starting || active ? "Pulling…" : "Pull"}
        </button>
        <button
          type="button"
          className="btn-ghost"
          data-testid="models-pull-advanced"
          onClick={() => setAdvanced((v) => !v)}
          disabled={starting || active}
        >
          {advanced ? "Use an Ollama tag" : "Direct GGUF URL"}
        </button>
      </div>
      {startError ? (
        <p className="card-grid__sub" data-testid="models-pull-error" role="alert">
          {startError}
        </p>
      ) : null}
      {status ? (
        <div className="models-pull-progress" data-testid="models-pull-progress">
          <div className="card-grid__meta">
            <Badge
              label={status.phase}
              color={status.phase === "failed" ? "var(--danger, #e55)" : "var(--accent)"}
              dot
              pulse={!["done", "failed"].includes(status.phase)}
            />
            {status.bytesTotal > 0 ? (
              <span className="card-grid__handle">
                {Math.round((status.bytesDownloaded / status.bytesTotal) * 100)}% (
                {(status.bytesDownloaded / 1e9).toFixed(2)} / {(status.bytesTotal / 1e9).toFixed(2)}{" "}
                GB)
              </span>
            ) : null}
          </div>
          {status.detail ? <p className="card-grid__sub">{status.detail}</p> : null}
          {["done", "failed"].includes(status.phase) ? (
            <button
              type="button"
              className="btn-ghost"
              data-testid="models-pull-dismiss"
              onClick={reset}
            >
              Dismiss
            </button>
          ) : null}
        </div>
      ) : null}
    </m.div>
  );
}
