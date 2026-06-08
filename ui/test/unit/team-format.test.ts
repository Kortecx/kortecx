import { TeamMember, WarrantView } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { formatActionCaps, roleBadgeKind, warrantRows } from "../../src/lib/team-format";

describe("formatActionCaps", () => {
  it("sorts + joins, and renders an em-dash when empty", () => {
    expect(formatActionCaps(["Use", "Read", "Delegate"])).toBe("Delegate · Read · Use");
    expect(formatActionCaps([])).toBe("—");
  });
});

describe("roleBadgeKind", () => {
  it("a Delegate-holding member is a delegate, else a member", () => {
    const delegate = new TeamMember("a", "r", ["Read", "Use", "Delegate"], null);
    const member = new TeamMember("b", "r", ["Read", "Use"], null);
    expect(roleBadgeKind(delegate)).toBe("delegate");
    expect(roleBadgeKind(member)).toBe("member");
  });
});

describe("warrantRows", () => {
  it("projects the headline ceilings + scopes as labelled rows", () => {
    const w = new WarrantView("Bwrap", "m ×3", "None", "", 3, 1000, 30000);
    const rows = warrantRows(w);
    const byLabel = Object.fromEntries(rows.map((r) => [r.label, r.value]));
    expect(byLabel.Executor).toBe("Bwrap");
    expect(byLabel["Max calls"]).toBe("3");
    expect(byLabel.Filesystem).toBe("None"); // empty fs renders "None"
    expect(byLabel["Wall clock (ms)"]).toBe("30000");
  });
});
