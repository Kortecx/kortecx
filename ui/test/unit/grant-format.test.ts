import { GrantView } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { formatActions, grantStatusLabel } from "../../src/lib/grant-format";

describe("formatActions", () => {
  it("joins actions, em-dash when empty", () => {
    expect(formatActions(["Read", "Use"])).toBe("Read · Use");
    expect(formatActions([])).toBe("—");
  });
});

describe("grantStatusLabel", () => {
  const grant = (isRoot: boolean, revoked: boolean) =>
    new GrantView("g", "h", ["Use"], "demo", isRoot, revoked);

  it("classifies revoked > root > delegated", () => {
    expect(grantStatusLabel(grant(true, false))).toBe("Root");
    expect(grantStatusLabel(grant(false, false))).toBe("Delegated");
    // Revoked wins regardless of root-ness.
    expect(grantStatusLabel(grant(true, true))).toBe("Revoked");
  });
});
