import { useModels } from "../../kx/use-models";

/**
 * The composer model picker (Batch A): a dropdown over `ListModels`. Hidden
 * when the gateway predates the RPC or serves no model (no fake knob). The
 * selection only ever rides as a recipe ENUM free-param the SERVER validates
 * at binding (SN-8) — picking a model here grants nothing.
 */
export function ModelPicker({
  value,
  onChange,
}: {
  value: string | undefined;
  onChange: (modelId: string) => void;
}) {
  const { models, unsupported } = useModels();
  if (unsupported || !models || models.length === 0) {
    return null;
  }
  const selected = models.find((m) => m.modelId === value) ?? models[0];
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
          </option>
        ))}
      </select>
    </label>
  );
}
