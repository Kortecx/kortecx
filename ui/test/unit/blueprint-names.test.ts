/** PR-4.1b client-local Blueprint names: per-endpoint, bounded, fail-closed. */

import { beforeEach, describe, expect, it } from "vitest";
import { loadBlueprintNames, setBlueprintName } from "../../src/lib/blueprint-names";

const EP = "http://127.0.0.1:50151";
const HANDLE = "kx/recipes/echo";

describe("blueprint-names store", () => {
  beforeEach(() => localStorage.clear());

  it("sets, trims, and clears names per endpoint, keyed by handle", () => {
    setBlueprintName(EP, HANDLE, "  Triage agent  ");
    expect(loadBlueprintNames(EP)).toEqual({ [HANDLE]: "Triage agent" });
    // Another endpoint never sees it.
    expect(loadBlueprintNames("http://other:1")).toEqual({});
    // An empty rename clears.
    setBlueprintName(EP, HANDLE, "   ");
    expect(loadBlueprintNames(EP)).toEqual({});
  });

  it("caps a name's length and survives corrupt storage", () => {
    setBlueprintName(EP, HANDLE, "x".repeat(500));
    expect(loadBlueprintNames(EP)[HANDLE]).toHaveLength(120);
    localStorage.setItem(`kortecx.ui.blueprint-names:${EP}`, "{not json");
    expect(loadBlueprintNames(EP)).toEqual({});
    localStorage.setItem(`kortecx.ui.blueprint-names:${EP}`, JSON.stringify([1, 2]));
    expect(loadBlueprintNames(EP)).toEqual({});
  });
});
