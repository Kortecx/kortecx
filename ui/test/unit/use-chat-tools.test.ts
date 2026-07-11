/**
 * A4: the pure lowering of a tool-attached chat turn. `toolContractFrom` splits the
 * picked `${name}@${version}` keys into the `{ name: version }` contract, and
 * `buildAgentTurnRequest` lowers to a single MODEL step carrying that exact contract +
 * the bounded-loop budget (as canonical u32 `params` bytes) + the request-level context
 * union. Byte-checked like the builder golden corpus.
 */

import { proto } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { buildAgentTurnRequest, toolContractFrom } from "../../src/kx/use-chat";

describe("toolContractFrom (A4)", () => {
  it("splits composite keys on the LAST @ and defaults a bare/empty version to 1", () => {
    expect(toolContractFrom(["web-search@1", "mcp-echo/echo@2", "bare", "trailing@"])).toEqual({
      "web-search": "1",
      "mcp-echo/echo": "2",
      bare: "1",
      trailing: "1",
    });
  });

  it("is empty for no tools", () => {
    expect(toolContractFrom([])).toEqual({});
  });
});

describe("buildAgentTurnRequest (A4)", () => {
  it("lowers to one MODEL step with the exact contract + budget bytes", () => {
    const req = buildAgentTurnRequest("do it", { "web-search": "1" }, 8, 6, undefined, []);
    expect(req.steps).toHaveLength(1);
    const step = req.steps?.[0];
    expect(step?.kind).toBe(proto.WorkflowStepKind.MODEL);
    expect(step?.toolContract).toEqual({ "web-search": "1" });
    expect(step?.modelId).toBe("");
    // The budget rides as canonical-JSON u32 bytes under the frozen param keys.
    const dec = new TextDecoder();
    expect(dec.decode(step?.params?.max_turns)).toBe("8");
    expect(dec.decode(step?.params?.max_tool_calls)).toBe("6");
    // No context attached ⇒ no context bundles.
    expect(req.contextBundles ?? []).toEqual([]);
  });

  it("threads the picked model + the context union into the request", () => {
    const req = buildAgentTurnRequest("x", { t: "1" }, 8, 6, "gemma-4-12b", ["ctx/a", "ctx/b"]);
    expect(req.steps?.[0]?.modelId).toBe("gemma-4-12b");
    expect(req.contextBundles).toEqual(["ctx/a", "ctx/b"]);
  });
});
