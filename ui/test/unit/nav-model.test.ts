import { describe, expect, it } from "vitest";
import {
  HIDDEN_SECTIONS,
  NAV_SECTIONS,
  SETTINGS_SECTION,
} from "../../src/components/shell/nav-model";

describe("NAV_SECTIONS", () => {
  it("has the eight spec-IA sections in the spec's order", () => {
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual([
      "chat",
      "runs",
      "recipes",
      "datasets",
      "tools",
      "context",
      "monitor",
      "systems",
    ]);
  });

  it("display labels rename; ids/icons stay on the frozen wire-legacy handles", () => {
    const byId = new Map(NAV_SECTIONS.map((s) => [s.id, s]));
    // D136 precedent: recipes displays Blueprints.
    expect(byId.get("recipes")?.label).toBe("Blueprints");
    expect(byId.get("recipes")?.path).toBe("/recipes");
    expect(byId.get("recipes")?.icon).toBe("recipes");
    // The spec IA renames (§2.186 / D141): chat → New Chat, runs → Workflows,
    // systems → Security — ids/icons untouched. PR-2 moved the `runs`
    // section's ROUTE to /workflows (the D141.1 merge; /runs redirects).
    expect(byId.get("chat")?.label).toBe("New Chat");
    expect(byId.get("chat")?.path).toBe("/chat");
    expect(byId.get("runs")?.label).toBe("Workflows");
    expect(byId.get("runs")?.path).toBe("/workflows");
    expect(byId.get("runs")?.icon).toBe("runs");
    expect(byId.get("systems")?.label).toBe("Security");
    expect(byId.get("systems")?.path).toBe("/systems");
  });

  it("Activity is NOT a section (it is the navbar drawer)", () => {
    expect(NAV_SECTIONS.some((s) => s.id === "activity")).toBe(false);
  });

  it("ids and paths are unique across nav + hidden + settings", () => {
    const all = [...NAV_SECTIONS, ...HIDDEN_SECTIONS, SETTINGS_SECTION];
    expect(new Set(all.map((s) => s.id)).size).toBe(all.length);
    expect(new Set(all.map((s) => s.path)).size).toBe(all.length);
  });

  it("every path is absolute and every section has a label/hint/icon", () => {
    for (const s of [...NAV_SECTIONS, ...HIDDEN_SECTIONS]) {
      expect(s.path.startsWith("/")).toBe(true);
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.hint.length).toBeGreaterThan(0);
      expect(s.icon.length).toBeGreaterThan(0);
    }
  });
});

describe("HIDDEN_SECTIONS", () => {
  it("is empty since PR-2 folded Artifacts into the Workflows tabs (D141.1)", () => {
    expect(HIDDEN_SECTIONS).toEqual([]);
  });
});

describe("SETTINGS_SECTION", () => {
  it("is pinned shell chrome, not a ninth nav section", () => {
    expect(SETTINGS_SECTION.id).toBe("settings");
    expect(SETTINGS_SECTION.path).toBe("/settings");
    expect(SETTINGS_SECTION.icon).toBe("settings");
    expect(NAV_SECTIONS.some((s) => s.id === SETTINGS_SECTION.id)).toBe(false);
  });
});
