import { useDefaultModel } from "../../kx/use-default-model";
import { useModels } from "../../kx/use-models";

/**
 * The composer model control (Batch A): a dropdown over `ListModels`, ALWAYS
 * visible on a ListModels-capable gateway (user-directed 2026-06-12 review
 * feedback — an unmounted control reads as missing). A model-less serve shows
 * an honest, disabled empty state instead of a fake knob; only a gateway that
 * predates the RPC (or one still loading) renders nothing. The selection only
 * ever rides as a recipe ENUM free-param the SERVER validates at binding
 * (SN-8) — picking a model here grants nothing.
 *
 * POC-5c: when the user has NOT explicitly picked (`value` unset), pre-select the
 * client-local default model (set in the Models section) instead of the first
 * listed — falling back to the first when no default is set or it isn't served here.
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
  const selected = models.find((m) => m.modelId === (value ?? defaultModelId)) ?? models[0];
  return (
    <label className="modelpicker" data-testid="model-picker">
      <span className="modelpicker__label">Model</span>
      <select
        value={selected?.modelId ?? ""}
        onChange={(e) => onChange(e.target.value)}
        aria-label="Model"
        data-testid="model-picker-select"
      >
        {models.map((m) => (
          <option key={m.modelId} value={m.modelId}>
            {m.modelId}
            {m.modalities.includes("image") ? " (vision)" : ""}
            {m.serving ? " · serving" : ""}
            {m.loaded ? " · loaded" : ""}
          </option>
        ))}
      </select>
    </label>
  );
}
