import type { ModelSummary } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { resolveAutoModel, resolveBoundModel } from "../../src/lib/auto-model";

function model(modelId: string, active = false): ModelSummary {
  return {
    modelId,
    active,
    modalities: [],
    chatHandle: `kx/recipes/m-${modelId}`,
  } as unknown as ModelSummary;
}

/** A model carrying the gateway's capability flags — what `serving` / `canEmbed` say. */
function capability(
  modelId: string,
  flags: { serving: boolean; canEmbed: boolean },
  active = false,
): ModelSummary {
  return {
    modelId,
    active,
    modalities: [],
    chatHandle: flags.serving ? "kx/recipes/chat" : `kx/recipes/m-${modelId}`,
    serving: flags.serving,
    canEmbed: flags.canEmbed,
  } as unknown as ModelSummary;
}

describe("resolveAutoModel (Model Control v2 — shared Auto resolution)", () => {
  it("returns undefined when nothing is served", () => {
    expect(resolveAutoModel(undefined, undefined)).toBeUndefined();
    expect(resolveAutoModel([], "gemma-4-12b")).toBeUndefined();
  });

  it("prefers the server-active model over a client default and the first listed", () => {
    const models = [model("a"), model("b", true), model("c")];
    expect(resolveAutoModel(models, "a")).toBe("b");
  });

  it("falls back to a SERVED client-local default when no model is active", () => {
    expect(resolveAutoModel([model("a"), model("b")], "b")).toBe("b");
  });

  it("ignores an UNSERVED client default and uses the first listed (never names a stale model)", () => {
    expect(resolveAutoModel([model("a"), model("b")], "gone")).toBe("a");
  });

  it("uses the first listed when there is neither an active model nor a default", () => {
    expect(resolveAutoModel([model("a"), model("b")], undefined)).toBe("a");
  });

  // ── the model Auto offers must be able to answer a chat turn ──────────────────────
  // A fresh install is exactly the state these cases describe: nothing is server-active
  // and the browser has no local default, so the last fallback IS the common path.

  it("prefers the SERVING model over whatever sorts first in the catalog", () => {
    // The shape of a real serve with an embedder registered: the embedding model sorts
    // first, and the chat model is the one the gateway marks `serving`. Picking the
    // first listed opened New Chat on a model whose only endpoint is /api/embed, so the
    // first message dead-lettered.
    const models = [
      capability("embeddinggemma:latest", { serving: false, canEmbed: true }),
      capability("gemma3:12b", { serving: true, canEmbed: false }),
    ];
    expect(resolveAutoModel(models, undefined)).toBe("gemma3:12b");
  });

  it("never offers an embed-only model while a chat-capable one exists", () => {
    // Same rule with NOTHING marked serving — the embed flag alone must disqualify it.
    const models = [
      capability("embeddinggemma:latest", { serving: false, canEmbed: true }),
      capability("some-chat-model", { serving: false, canEmbed: false }),
    ];
    expect(resolveAutoModel(models, undefined)).toBe("some-chat-model");
  });

  it("still names something when EVERY served model only embeds", () => {
    // The accepting control for the rule above: disqualifying every candidate must not
    // leave the picker with no answer at all. One variable from the case before it.
    const models = [capability("embed-only", { serving: false, canEmbed: true })];
    expect(resolveAutoModel(models, undefined)).toBe("embed-only");
  });

  it("does not let `serving` override an ACTIVE model or a served client default", () => {
    // The new step slots in BELOW the two explicit choices; it must not outrank them.
    const chosen = capability("chosen", { serving: false, canEmbed: false }, true);
    const primary = capability("primary", { serving: true, canEmbed: false });
    // ACTIVE wins over serving.
    expect(resolveAutoModel([chosen, primary], undefined)).toBe("chosen");
    // A served client default also wins over serving (same model, no longer active).
    const notActive = capability("chosen", { serving: false, canEmbed: false });
    expect(resolveAutoModel([primary, notActive], "chosen")).toBe("chosen");
  });
});

describe("resolveBoundModel (the label and the bound model are one resolution)", () => {
  it("honors an explicit pick that names a SERVED model", () => {
    const bound = resolveBoundModel([model("a"), model("b", true)], "a", undefined);
    expect(bound.model?.modelId).toBe("a");
    expect(bound.explicit).toBe(true);
    expect(bound.stalePick).toBeUndefined();
  });

  it("binds AUTO — not models[0] — when the pick is not served, and discloses the stale pick", () => {
    // The regression: a pick persisted against another serve (the storage key is
    // global) used to be sent verbatim while the picker labelled "Auto · b" and the
    // turn routed to models[0] = "a". Auto's answer is "b" (server-active); "a" is
    // exactly the wrong one, so this asserts the divergence cannot come back.
    const bound = resolveBoundModel([model("a"), model("b", true)], "gone", undefined);
    expect(bound.model?.modelId).toBe("b");
    expect(bound.explicit).toBe(false);
    expect(bound.stalePick).toBe("gone");
  });

  it("carries the bound model's OWN chatHandle (the id and the route cannot diverge)", () => {
    const bound = resolveBoundModel([model("a"), model("b", true)], "gone", undefined);
    expect(bound.model?.chatHandle).toBe("kx/recipes/m-b");
  });

  it("falls through an unserved pick to a SERVED client default", () => {
    const bound = resolveBoundModel([model("a"), model("b")], "gone", "b");
    expect(bound.model?.modelId).toBe("b");
    expect(bound.explicit).toBe(false);
  });

  it("defers to Auto when there is no pick at all", () => {
    const bound = resolveBoundModel([model("a"), model("b", true)], undefined, undefined);
    expect(bound.model?.modelId).toBe("b");
    expect(bound.explicit).toBe(false);
    expect(bound.stalePick).toBeUndefined();
  });

  it("binds nothing when nothing is served", () => {
    expect(resolveBoundModel([], "a", undefined).model).toBeUndefined();
    expect(resolveBoundModel(undefined, "a", undefined).model).toBeUndefined();
  });

  it("never calls a pick stale while the model list is still loading", () => {
    // `useModels` reports `undefined` on load AND on reconnect — accusing the pick
    // then would flash the disclosure on every cold mount.
    expect(resolveBoundModel(undefined, "a", undefined).stalePick).toBeUndefined();
    expect(resolveBoundModel([], "a", undefined).stalePick).toBeUndefined();
  });

  it("INVARIANT: the bound model is always served, or nothing (never a stale enum)", () => {
    const served = [model("a"), model("b", true)];
    for (const pick of ["gone", "a", "b", undefined]) {
      const bound = resolveBoundModel(served, pick, "also-gone");
      expect(served).toContain(bound.model);
    }
  });
});
