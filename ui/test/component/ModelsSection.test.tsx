import { ErrorCode } from "@kortecx/sdk/web";
import { render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { ModelsSection } from "../../src/components/sections/ModelsSection";
import { connectedWrapper } from "../mocks/harness";
import { makeMockClient } from "../mocks/kx-client";

const MODELS = [
  {
    modelId: "qwen3-4b",
    modalities: ["text", "image"],
    description: "Qwen3 4B (GGUF)",
    serving: true,
    contextLen: 8192,
  },
  {
    modelId: "gemma-2b",
    modalities: ["text"],
    description: "Gemma 2B",
    serving: false,
    contextLen: 4096,
  },
];

describe("ModelsSection", () => {
  it("renders a display-only card per served model (modalities, context, serving badge)", async () => {
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });

    expect(screen.getByTestId("models-section")).toBeInTheDocument();
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    // Real fields render — never fabricated.
    expect(screen.getByText("qwen3-4b")).toBeInTheDocument();
    expect(screen.getByText("gemma-2b")).toBeInTheDocument();
    expect(screen.getByText("Qwen3 4B (GGUF)")).toBeInTheDocument();
    expect(screen.getByText(/ctx 8,192 tokens/)).toBeInTheDocument();
    // Modality tag chips (one served model has vision).
    expect(screen.getAllByText("image").length).toBeGreaterThan(0);
    // The serving model shows a "serving" badge; the idle one shows "idle".
    expect(screen.getByText("serving")).toBeInTheDocument();
    expect(screen.getByText("idle")).toBeInTheDocument();

    // SN-8 display-only: listing never routes — the section has NO action control.
    expect(screen.queryAllByRole("button")).toHaveLength(0);
    expect(screen.queryByText(/use model/i)).not.toBeInTheDocument();
  });

  it("always shows the honest-disabled Cloud / coming-soon cards (D129/GR15, never faked)", async () => {
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });

    const connect = await screen.findByTestId("models-cloud-connect");
    const pull = screen.getByTestId("models-cloud-pull");
    expect(connect).toHaveAttribute("aria-disabled", "true");
    expect(pull).toHaveAttribute("aria-disabled", "true");
    expect(connect).toHaveTextContent(/connect a cloud provider/i);
    expect(pull).toHaveTextContent(/pull a model/i);
  });

  it("shows an honest empty state on an FFI-free serve (empty list, not an error)", async () => {
    const mock = makeMockClient({ listModels: async () => [] });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/no models on this serve/i)).toBeInTheDocument());
    expect(screen.queryByTestId("model-card")).not.toBeInTheDocument();
  });

  it("degrades to 'not wired' on a gateway that predates ListModels", async () => {
    const mock = makeMockClient({
      listModels: async () => {
        throw Object.assign(new Error("unimplemented"), { code: ErrorCode.Unimplemented });
      },
    });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getByText(/model discovery not wired/i)).toBeInTheDocument());
    expect(screen.queryByTestId("model-card")).not.toBeInTheDocument();
  });
});
