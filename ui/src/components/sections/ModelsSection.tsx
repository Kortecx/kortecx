import { m } from "framer-motion";
import { fadeUp, hoverLift, stagger } from "../../app/motion";
import { toUiError } from "../../kx/errors";
import { useDefaultModel } from "../../kx/use-default-model";
import { useModelLifecycle } from "../../kx/use-model-lifecycle";
import { useModels } from "../../kx/use-models";
import { EmptyState } from "../EmptyState";
import { Badge } from "../ds/Badge";

/**
 * The Models view (Tools group) — a read-only catalog over the models serving this
 * gateway (`ListModels`, Batch A). Display-ONLY (SN-8): listing a model never routes
 * one; selection stays a recipe ENUM free-param the SERVER validates at binding, so
 * there is no "use model" action here. An FFI-free serve returns an honest EMPTY list
 * (not an error); a gateway that predates the RPC degrades to "not wired". Pulling a
 * model / connecting a vendor are managed-Cloud or coming-soon — honest-disabled
 * placeholders (D129 / GR15 don't-fake-gaps), never faked as local actions.
 *
 * Pure renderer: composes the existing `useModels` hook + the shared `Badge` and the
 * adopted card/density language (`.card-grid` / `.glow-card`, PR-4.1b).
 */
export function ModelsSection() {
  const { models, unsupported, loading } = useModels();
  const { load, offload } = useModelLifecycle();
  const { defaultModelId, setDefault, clearDefault } = useDefaultModel();
  const hasModels = models !== undefined && models.length > 0;

  return (
    <section className="screen" data-testid="models-section">
      <div className="section-head">
        <div>
          <h1>Models</h1>
          <p className="muted">
            The models serving this gateway. Load or offload one, and pick the default for new chats
            (client-local). Listing or defaulting a model never routes one — selection stays a
            server-validated recipe parameter (SN-8).
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

      {hasModels ? (
        <m.div
          className="card-grid"
          data-testid="models-grid"
          variants={stagger()}
          initial="hidden"
          animate="show"
        >
          {models.map((mdl) => {
            // POC-3: per-card pending/error state (the mutations are shared, so key
            // on the in-flight variables == this model id).
            const loadingThis = load.isPending && load.variables === mdl.modelId;
            const offloadingThis = offload.isPending && offload.variables === mdl.modelId;
            const busy = loadingThis || offloadingThis;
            // POC-5c: the client-local default for new chats (this browser only).
            const isDefault = mdl.modelId === defaultModelId;
            const actionError =
              load.isError && load.variables === mdl.modelId
                ? toUiError(load.error)
                : offload.isError && offload.variables === mdl.modelId
                  ? toUiError(offload.error)
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
                {mdl.description ? <p className="card-grid__sub">{mdl.description}</p> : null}
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
                  {/* The serving engine (llamacpp / ollama), display only. Empty on
                      an old host that does not report an engine. */}
                  {mdl.engine ? (
                    <Badge label={mdl.engine.replace(/^kx-/, "")} color="var(--text-2)" />
                  ) : null}
                  {/* POC-3: live RAM residency (the LRU snapshot), display only. */}
                  <Badge
                    label={mdl.loaded ? "loaded" : "not loaded"}
                    color={mdl.loaded ? "var(--accent)" : "var(--text-3)"}
                    dot
                  />
                </div>
                {/* POC-3: load/offload controls — warm/evict this model in RAM. All
                    states designed: idle / pending / loaded / error. Token-based
                    `.btn-ghost` ⇒ both themes + AA carry by construction (D142). */}
                <div className="chip-row" data-testid="model-actions">
                  {/* POC-5c: pick the runtime default for new chats (client-local,
                      this browser only — SN-8: still a server-validated recipe enum).
                      A chip/button, never a controlled <select> (Playwright-safe). */}
                  {isDefault ? (
                    <button
                      type="button"
                      className="chip chip--tag model-default-chip"
                      data-testid={`model-default-badge-${mdl.modelId}`}
                      title="Default model for new chats on this browser. Click to clear."
                      onClick={() => clearDefault()}
                    >
                      ★ Default
                    </button>
                  ) : (
                    <button
                      type="button"
                      className="btn-ghost"
                      data-testid={`model-set-default-${mdl.modelId}`}
                      title="Use this model by default for new chats (this browser only)."
                      onClick={() => setDefault(mdl.modelId)}
                    >
                      Set as default
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
                  <p className="card-grid__sub" data-testid="model-action-error" role="alert">
                    {actionError.message}
                  </p>
                ) : null}
              </m.article>
            );
          })}
        </m.div>
      ) : null}

      {/* Honest-disabled Cloud / coming-soon capabilities (D129 / GR15): the OSS
          console LISTS what is served; connecting a vendor or pulling a model are
          managed-Cloud / coming-soon, rendered disabled — never faked. Mirrors the
          PR-C1 `cost-tile-disabled` idiom (metric-card--disabled + chip--soon). */}
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
        <div
          className="metric-card metric-card--disabled"
          data-testid="models-cloud-pull"
          aria-disabled="true"
        >
          <span className="metric-card__value">
            <span className="chip--soon">Soon</span>
          </span>
          <span className="metric-card__label">Pull a model</span>
          <span className="metric-card__sub">Local model download is coming soon.</span>
        </div>
      </m.div>
    </section>
  );
}
