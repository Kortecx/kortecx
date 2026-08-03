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

  it("shows the honest-disabled Cloud card + an honest-disabled Pull panel when downloads are off", async () => {
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
    // Persisted client-local (no backend, still a recipe enum at bind).
    expect(localStorage.getItem("kortecx.ui.default-model")).toBe("qwen3-4b");

    // Clicking the badge clears the default.
    await user.click(screen.getByTestId("model-default-badge-qwen3-4b"));
    await waitFor(() =>
      expect(screen.getByTestId("model-set-default-qwen3-4b")).toBeInTheDocument(),
    );
    expect(localStorage.getItem("kortecx.ui.default-model")).toBeNull();
  });
});

// --- The OffloadModel IN-USE GUARD -------------------------------------------
//
// A refusal is a SUCCESSFUL call that evicted nothing. The failure mode being guarded
// against is the console treating that as done: the button stops spinning, nothing
// changes, and the operator concludes the control is broken.

describe("ModelsSection — the offload in-use guard", () => {
  const IN_USE = {
    modelId: "qwen3-4b",
    loaded: true,
    wasResident: true,
    refused: true,
    usageChecked: true,
    inUseBy: [{ kind: "hosted app", handle: "apps/local/desk", detail: "hosted server running" }],
  };

  it("shows what an offload would disrupt instead of silently doing nothing", async () => {
    const mock = makeMockClient({
      listModels: async () => MODELS,
      offloadModel: async () => IN_USE,
    });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    await userEvent.click(screen.getByTestId("model-offload-btn"));

    const banner = await screen.findByTestId("model-offload-refused-qwen3-4b");
    // The REASON is text, not a tooltip — a greyed control with a hover-only
    // explanation reads as "broken", which is the misreading being prevented.
    expect(banner.textContent).toMatch(/in use by/i);
    expect(banner.textContent).toMatch(/apps\/local\/desk/);
    expect(banner.textContent).toMatch(/would disrupt/i);
    // And the override is offered explicitly rather than left to be guessed at.
    expect(screen.getByTestId("model-offload-force-qwen3-4b")).toBeInTheDocument();
  });

  it("the override re-calls with force, and only then evicts", async () => {
    const calls: Array<{ modelId: string; force?: boolean }> = [];
    const mock = makeMockClient({
      listModels: async () => MODELS,
      offloadModel: async (...args: unknown[]) => {
        const [modelId, opts] = args as [string, { force?: boolean } | undefined];
        calls.push({ modelId, force: opts?.force });
        return opts?.force ? { ...IN_USE, loaded: false, refused: false } : IN_USE;
      },
    });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    await userEvent.click(screen.getByTestId("model-offload-btn"));
    await screen.findByTestId("model-offload-refused-qwen3-4b");
    // The FIRST call must NOT have carried force — otherwise the guard is bypassed by
    // the very control meant to respect it, and the warning is theatre.
    expect(calls[0]).toEqual({ modelId: "qwen3-4b", force: undefined });

    await userEvent.click(screen.getByTestId("model-offload-force-qwen3-4b"));
    await waitFor(() => expect(calls).toHaveLength(2));
    expect(calls[1]).toEqual({ modelId: "qwen3-4b", force: true });
  });

  it("a clean offload shows no warning (the banner is not always-on)", async () => {
    const mock = makeMockClient({
      listModels: async () => MODELS,
      offloadModel: async () => ({
        modelId: "qwen3-4b",
        loaded: false,
        wasResident: true,
        refused: false,
        usageChecked: true,
        inUseBy: [],
      }),
    });
    render(<ModelsSection />, { wrapper: connectedWrapper(mock.client) });
    await waitFor(() => expect(screen.getAllByTestId("model-card")).toHaveLength(2));

    await userEvent.click(screen.getByTestId("model-offload-btn"));
    await waitFor(() => expect(mock.offloadModel).toHaveBeenCalled());
    expect(screen.queryByTestId("model-offload-refused-qwen3-4b")).not.toBeInTheDocument();
  });
});
