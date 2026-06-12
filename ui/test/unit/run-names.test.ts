/** PR-2.1 client-local run names: per-endpoint, bounded, fail-closed. */

import { beforeEach, describe, expect, it } from "vitest";
import { loadRunNames, setRunName } from "../../src/lib/run-names";

const EP = "http://127.0.0.1:50151";

describe("run-names store", () => {
  beforeEach(() => localStorage.clear());

  it("sets, trims, and clears names per endpoint", () => {
    setRunName(EP, "a".repeat(32), "  incident triage  ");
    expect(loadRunNames(EP)).toEqual({ ["a".repeat(32)]: "incident triage" });
    // Another endpoint never sees it.
    expect(loadRunNames("http://other:1")).toEqual({});
    // An empty rename clears.
    setRunName(EP, "a".repeat(32), "   ");
    expect(loadRunNames(EP)).toEqual({});
  });

  it("caps a name's length and survives corrupt storage", () => {
    setRunName(EP, "b".repeat(32), "x".repeat(500));
    expect(loadRunNames(EP)["b".repeat(32)]).toHaveLength(120);
    localStorage.setItem(`kortecx.ui.run-names:${EP}`, "{not json");
    expect(loadRunNames(EP)).toEqual({});
    localStorage.setItem(`kortecx.ui.run-names:${EP}`, JSON.stringify([1, 2]));
    expect(loadRunNames(EP)).toEqual({});
  });
});
