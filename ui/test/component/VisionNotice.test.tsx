import type { ModelSummary } from "@kortecx/sdk/web";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { VisionNotice } from "../../src/components/chat/VisionNotice";
import type { PendingAttachment } from "../../src/kx/use-attachments";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

function model(modelId: string, modalities: string[]): ModelSummary {
  return { modelId, modalities, active: false } as unknown as ModelSummary;
}

const VLM = model("qwen-vl", ["text", "image"]);
const TEXT_ONLY = model("gemma-4-12b", ["text"]);

function attachment(mediaType: string, status: PendingAttachment["status"]): PendingAttachment {
  return { id: "a1", filename: "shot.png", mediaType, size: 1, objectUrl: "", status };
}

const IMAGE = attachment("image/png", "ready");

/** Both models served, so the "pick a vision model" advice is actionable. */
function renderNotice(bound: ModelSummary | undefined, attachments: readonly PendingAttachment[]) {
  const mock = makeMockClient({ listModels: async () => [VLM, TEXT_ONLY] });
  return render(<VisionNotice boundModel={bound} attachments={attachments} />, {
    wrapper: connectedWrapper(mock.client),
  });
}

describe("VisionNotice (the one capability a swap can actually drop)", () => {
  it("warns when an image is attached and the BOUND model is text-only", async () => {
    renderNotice(TEXT_ONLY, [IMAGE]);
    // Names the bound model + the honest consequence, and points somewhere useful
    // once the peer list has landed.
    await waitFor(() =>
      expect(screen.getByTestId("vision-notice")).toHaveTextContent('Pick a model marked "vision"'),
    );
    expect(screen.getByTestId("vision-notice")).toHaveTextContent("gemma-4-12b is text-only");
    expect(screen.getByTestId("vision-notice")).toHaveTextContent("won't see the attached image");
  });

  it("withholds the peer ADVICE until the model list lands — never guesses 'none'", () => {
    // The lead claim is known from `boundModel` alone, so it shows immediately; but on
    // the first paint `useModels` reports `undefined`, and claiming "No served model
    // has vision" there would print a falsehood on every cold load.
    renderNotice(TEXT_ONLY, [IMAGE]);
    const notice = screen.getByTestId("vision-notice");
    expect(notice).toHaveTextContent("gemma-4-12b is text-only");
    expect(notice).not.toHaveTextContent("No served model has vision");
    expect(notice).not.toHaveTextContent("Pick a model");
  });

  it("is silent when the bound model HAS vision", () => {
    renderNotice(VLM, [IMAGE]);
    expect(screen.queryByTestId("vision-notice")).toBeNull();
  });

  it("is silent with no attachment — a text-only model is the normal choice, not a fault", () => {
    renderNotice(TEXT_ONLY, []);
    expect(screen.queryByTestId("vision-notice")).toBeNull();
  });

  it("is silent for a non-image attachment", () => {
    renderNotice(TEXT_ONLY, [attachment("text/plain", "ready")]);
    expect(screen.queryByTestId("vision-notice")).toBeNull();
  });

  it("is silent for a FAILED image upload (it was never going to be looked at)", () => {
    renderNotice(TEXT_ONLY, [attachment("image/png", "failed")]);
    expect(screen.queryByTestId("vision-notice")).toBeNull();
  });

  it("is silent while nothing is bound yet (loading / model-less serve)", () => {
    renderNotice(undefined, [IMAGE]);
    expect(screen.queryByTestId("vision-notice")).toBeNull();
  });

  it("does not send the user hunting when NO served model has vision", async () => {
    const mock = makeMockClient({ listModels: async () => [TEXT_ONLY] });
    render(<VisionNotice boundModel={TEXT_ONLY} attachments={[IMAGE]} />, {
      wrapper: connectedWrapper(mock.client),
    });
    await waitFor(() =>
      expect(screen.getByTestId("vision-notice")).toHaveTextContent("No served model has vision"),
    );
    expect(screen.getByTestId("vision-notice")).not.toHaveTextContent("Pick a model");
  });
});
