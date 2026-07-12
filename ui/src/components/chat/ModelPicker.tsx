import type { ModelSummary } from "@kortecx/sdk/web";
import { useDefaultModel } from "../../kx/use-default-model";
import { useModels } from "../../kx/use-models";

/** The honest, existing specs for a model — only fields ListModels actually returns
 *  (no params/quantization on this wire). Joined for a hover tooltip. */
function modelSpecs(m: ModelSummary): string {
  return [
    m.description && m.description !== m.modelId ? m.description : null,
    m.engine ? m.engine.replace(/^kx-/, "") : null,
    m.modalities.includes("image") ? "vision" : null,
    m.contextLen ? `${m.contextLen.toLocaleString()} ctx` : null,
    m.source || null,
    m.canEmbed ? "embedder" : null,
    m.active ? "active" : null,
    m.loaded ? "loaded" : null,
  ]
    .filter(Boolean)
    .join(" · ");
}

/**
 * The composer model control (Batch A): a dropdown over `ListModels`, ALWAYS
 * visible on a ListModels-capable gateway (user-directed 2026-06-12 review
 * feedback — an unmounted control reads as missing). A model-less serve shows
 * an honest, disabled empty state instead of a fake knob; only a gateway that
 * predates the RPC (or one still loading) renders nothing. The selection only
 * ever rides as a recipe ENUM free-param the SERVER validates at binding
 * (SN-8) — picking a model here grants nothing.
 *
 * Selection precedence when the user has NOT explicitly picked (`value` unset):
 * the server's ACTIVE default (Model Control v2 — shared across surfaces), then the
 * client-local default (set in the Models section, this browser), then the first
 * listed. Each option shows its engine so an Ollama ∥ llama.cpp switch is unmistakable.
 */
export function ModelPicker({
  value,
  onChange,
}: {
  value: string | undefined;
  onChange: (modelId: string) => void;
}) {
  const { models, unsupported, loading } = useModels();
  const { defaultModelId } = useDefaultModel();
  if (unsupported || loading || models === undefined) {
    return null;
  }
  if (models.length === 0) {
    return (
      <span className="modelpicker modelpicker--empty" data-testid="model-picker-empty">
        <span className="modelpicker__label">Model</span>
        <span className="muted" title="Start kx serve with KX_SERVE_MODEL_GGUF to serve a model.">
          none on this serve
        </span>
      </span>
    );
  }
  // Precedence: explicit pick → server active → client-local default → first.
  const serverActive = models.find((m) => m.active)?.modelId;
  const selected =
    models.find((m) => m.modelId === (value ?? serverActive ?? defaultModelId)) ?? models[0];
  return (
    <label className="modelpicker" data-testid="model-picker">
      <span className="modelpicker__label">Model</span>
      <select
        value={selected?.modelId ?? ""}
        onChange={(e) => onChange(e.target.value)}
        aria-label="Model"
        // The specs (engine · vision · context · …) surface on hover — the picker
        // itself tooltips the SELECTED model (native <option title> is unreliable).
        title={selected ? modelSpecs(selected) || undefined : undefined}
        data-testid="model-picker-select"
      >
        {models.map((m) => (
          <option key={m.modelId} value={m.modelId} title={modelSpecs(m) || undefined}>
            {m.modelId}
          </option>
        ))}
      </select>
    </label>
  );
}
