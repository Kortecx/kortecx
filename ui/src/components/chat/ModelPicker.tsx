import type { ModelSummary } from "@kortecx/sdk/web";
import { useDefaultModel } from "../../kx/use-default-model";
import { useModels } from "../../kx/use-models";
import { resolveAutoModel } from "../../lib/auto-model";

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
 * Default is AUTO: the user defers the choice and the runtime picks — the server's
 * ACTIVE default (Model Control v2 — shared across surfaces), then this browser's
 * client-local default (Models section), then the first listed. The first option
 * ("Auto") makes that deferral explicit and honest — it names the model the runtime
 * would pick — and the user STEERS by choosing a concrete model instead. An empty /
 * unset selection IS Auto (the server resolves it to the default at binding, SN-8).
 * Each option shows its engine so an Ollama ∥ llama.cpp switch is unmistakable.
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
  // The model the runtime resolves to when the user defers ("Auto"): server active,
  // then this browser's default (if served), then the first listed. Shared with
  // useChatController so this LABEL never diverges from the model actually bound.
  const autoResolved = resolveAutoModel(models, defaultModelId) ?? "";
  // A concrete steer only when `value` names a served model; otherwise Auto (deferred).
  const picked = value ? models.find((m) => m.modelId === value) : undefined;
  const autoLabel = autoResolved ? `Auto · ${autoResolved}` : "Auto (runtime default)";
  return (
    <label className="modelpicker" data-testid="model-picker">
      <span className="modelpicker__label">Model</span>
      <select
        value={picked?.modelId ?? ""}
        onChange={(e) => onChange(e.target.value)}
        aria-label="Model"
        // The specs (engine · vision · context · …) surface on hover — the picker
        // itself tooltips the SELECTED model (native <option title> is unreliable).
        title={
          picked
            ? modelSpecs(picked) || undefined
            : `Auto — the runtime picks ${autoResolved || "the served default"}`
        }
        data-testid="model-picker-select"
      >
        <option value="" title={autoResolved ? `The runtime picks ${autoResolved}` : undefined}>
          {autoLabel}
        </option>
        {models.map((m) => (
          <option key={m.modelId} value={m.modelId} title={modelSpecs(m) || undefined}>
            {m.modelId}
          </option>
        ))}
      </select>
    </label>
  );
}
