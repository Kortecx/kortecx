import type { ModelSummary } from "@kortecx/sdk/web";
import type { PendingAttachment } from "../../kx/use-attachments";
import { useModels } from "../../kx/use-models";

/**
 * The honest note for the one capability a model swap can actually DROP: vision.
 * An image is attached but the bound model has no `image` modality, so it will not
 * be looked at — `planVisionArgs` either binds a text-only model to the vision
 * recipe or silently falls back to the form's first allowed model, neither of which
 * the composer otherwise admits.
 *
 * Vision is the ONLY axis warned on. Tool-calling is NOT a per-model capability
 * here — the runtime GRAMMAR-forces the tool envelope from the warrant's granted
 * tools (`GrammarSpec::ToolEnvelope`, off-MoteDef at dispatch), so it does not ride
 * on a model's native tool training and no swap can drop it. Warning about it would
 * invent a distinction the runtime does not have.
 *
 * DERIVED state, not a swap EVENT: it lights up for attach-then-swap and
 * swap-then-attach alike, and clears itself when either the image goes or a vision
 * model is picked. No dismiss, no nagging — and silent (renders nothing) whenever
 * the pairing is fine, which is the same standard {@link RunPreflight} holds.
 *
 * Keyed on the BOUND model, never `settings.modelId` — a stale pick binds Auto
 * instead (`resolveBoundModel`), and a notice naming the pick would be lying in
 * exactly the case that reconciliation exists to fix.
 */
export function VisionNotice({
  boundModel,
  attachments,
}: {
  boundModel: ModelSummary | undefined;
  attachments: readonly PendingAttachment[];
}) {
  const { models } = useModels();
  const imagePending = attachments.some(
    (a) => a.mediaType.startsWith("image/") && a.status !== "failed",
  );
  // Stay silent while nothing is bound yet (loading / model-less serve) — the
  // model-less case is already said honestly by the picker's empty state.
  if (!imagePending || !boundModel || boundModel.modalities.includes("image")) {
    return null;
  }
  // Tri-state on PURPOSE. The lead claim is known from `boundModel` alone, but whether
  // a PEER has vision is not known until the list lands — and `undefined` must not
  // collapse to "none do", which would print a falsehood on every cold load. So the
  // advice is withheld until it can be told truthfully, and the true part still shows.
  const anyVision = models?.some((m) => m.modalities.includes("image"));
  return (
    <p className="muted" data-testid="vision-notice">
      {`⚠ ${boundModel.modelId} is text-only — it won't see the attached image.`}
      {anyVision === true ? ` Pick a model marked "vision".` : null}
      {anyVision === false ? " No served model has vision." : null}
    </p>
  );
}
