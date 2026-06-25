import { ErrorCode } from "@kortecx/sdk/web";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
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
    loaded: true,
    chatHandle: "kx/recipes/chat",
    engine: "kx-llamacpp",
    canEmbed: true,
    source: "local",
    active: false,
    chatRagHandle: "",
  },
  {
    modelId: "gemma-2b",
    modalities: ["text"],
    description: "Gemma 2B",
    serving: false,
    contextLen: 4096,
    loaded: false,
    chatHandle: "kx/recipes/m-gemma-2b",
    engine: "kx-ollama",
    canEmbed: false,
    source: "ollama",
    active: false,
    chatRagHandle: "",
  },
];

describe("ModelsSection", () => {
  it("renders a card per served model (modalities, context, serving + loaded badges)", async () => {
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
    // POC-3: live residency badges — one loaded, one not.
    expect(screen.getByText("loaded")).toBeInTheDocument();
    expect(screen.getByText("not loaded")).toBeInTheDocument();
    // The loaded model offers Offload; the idle one offers Load.
    expect(screen.getByTestId("model-offload-btn")).toBeInTheDocument();
    expect(screen.getByTestId("model-load-btn")).toBeInTheDocument();
  });

  it("marks the configured embedder with an 'embed' badge (PR-B)", async () => {
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));
    // Exactly the can_embed model carries the badge (the other does not).
    expect(screen.getAllByText("embed")).toHaveLength(1);
  });

  it("loads an idle model (POC-3): clicking Load calls loadModel + refetches", async () => {
    const user = userEvent.setup();
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    await user.click(screen.getByTestId("model-load-btn"));
    await waitFor(() => expect(mock.loadModel).toHaveBeenCalledWith("gemma-2b"));
    // The models query is invalidated ⇒ ListModels is re-read after the mutation.
    await waitFor(() => expect(mock.listModels.mock.calls.length).toBeGreaterThan(1));
  });

  it("surfaces a fail-closed load error honestly (never a fake success)", async () => {
    const user = userEvent.setup();
    const mock = makeMockClient({
      listModels: async () => MODELS,
      loadModel: async () => {
        throw Object.assign(new Error("model not registered"), { code: ErrorCode.NotFound });
      },
    });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    await user.click(screen.getByTestId("model-load-btn"));
    await waitFor(() => expect(screen.getByTestId("model-action-error")).toBeInTheDocument());
  });

  it("shows the honest-disabled Cloud card + an honest-disabled Pull panel when downloads are off (D129/GR15)", async () => {
    // Model Control v2: downloads OFF by default (deny-by-default) ⇒ the Pull panel
    // renders disabled WITH the reason, never a faked control.
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });

    const connect = await screen.findByTestId("models-cloud-connect");
    expect(connect).toHaveAttribute("aria-disabled", "true");
    expect(connect).toHaveTextContent(/connect a cloud provider/i);

    const pullDisabled = await screen.findByTestId("models-pull-disabled");
    expect(pullDisabled).toHaveAttribute("aria-disabled", "true");
    expect(pullDisabled).toHaveTextContent(/KX_SERVE_ALLOW_MODEL_PULL/);
  });

  it("Model Control v2: an enabled Pull panel pulls an Ollama tag + polls to done", async () => {
    const user = userEvent.setup();
    const mock = makeMockClient({
      listModels: async () => MODELS,
      getServerInfo: async () => ({ allowModelPull: true, activeModelId: "" }),
      pullModel: async () => "gemma3:12b",
      getPullStatus: async () => ({
        modelId: "gemma3:12b",
        phase: "done",
        bytesDownloaded: 100,
        bytesTotal: 100,
        detail: "registered",
      }),
    });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });

    // The enabled panel is shown (not the disabled placeholder).
    const tag = await screen.findByTestId("models-pull-tag");
    await user.type(tag, "gemma3:12b");
    await user.click(screen.getByTestId("models-pull-go"));
    // The pull fires with the Ollama tag, then polls to a terminal status.
    await waitFor(() => expect(mock.pullModel).toHaveBeenCalledWith({ ollamaTag: "gemma3:12b" }));
    await waitFor(() => expect(screen.getByTestId("models-pull-progress")).toBeInTheDocument());
  });

  it("Model Control v2: 'Make active' sets the server's active default", async () => {
    const user = userEvent.setup();
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    await user.click(screen.getByTestId("model-make-active-qwen3-4b"));
    await waitFor(() => expect(mock.setActiveModel).toHaveBeenCalledWith("qwen3-4b"));
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

  it("sets a client-local default (POC-5c): the chip toggles to ★ Default and persists", async () => {
    localStorage.clear();
    const user = userEvent.setup();
    const mock = makeMockClient({ listModels: async () => MODELS });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    // No default yet → every card offers "Set as default".
    expect(screen.getByTestId("model-set-default-qwen3-4b")).toBeInTheDocument();
    await user.click(screen.getByTestId("model-set-default-qwen3-4b"));

    // The chosen card flips to the Default badge; the other still offers Set.
    await waitFor(() =>
      expect(screen.getByTestId("model-default-badge-qwen3-4b")).toBeInTheDocument(),
    );
    expect(screen.getByTestId("model-set-default-gemma-2b")).toBeInTheDocument();
    // Persisted client-local (no backend, SN-8: still a recipe enum at bind).
    expect(localStorage.getItem("kortecx.ui.default-model")).toBe("qwen3-4b");

    // Clicking the badge clears the default.
    await user.click(screen.getByTestId("model-default-badge-qwen3-4b"));
    await waitFor(() =>
      expect(screen.getByTestId("model-set-default-qwen3-4b")).toBeInTheDocument(),
    );
    expect(localStorage.getItem("kortecx.ui.default-model")).toBeNull();
  });
});
