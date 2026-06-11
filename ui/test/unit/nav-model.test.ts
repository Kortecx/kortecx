import { describe, expect, it } from "vitest";
import { NAV_SECTIONS, SETTINGS_SECTION } from "../../src/components/shell/nav-model";

describe("NAV_SECTIONS", () => {
  it("has the console sections in order", () => {
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual([
      "activity",
      "monitor",
      "chat",
      "runs",
      "recipes",
      "artifacts",
      "datasets",
      "tools",
      "systems",
    ]);
  });

  it("displays Blueprints over the frozen recipes wire (D136)", () => {
    const recipes = NAV_SECTIONS.find((s) => s.id === "recipes");
    expect(recipes?.label).toBe("Blueprints");
    // The wire surface never renames: id, path, icon stay `recipes`.
    expect(recipes?.path).toBe("/recipes");
    expect(recipes?.icon).toBe("recipes");
  });

  it("ids and paths are unique", () => {
    const ids = new Set(NAV_SECTIONS.map((s) => s.id));
    const paths = new Set(NAV_SECTIONS.map((s) => s.path));
    expect(ids.size).toBe(NAV_SECTIONS.length);
    expect(paths.size).toBe(NAV_SECTIONS.length);
  });

  it("every path is absolute and every section has a label/hint/icon", () => {
    for (const s of NAV_SECTIONS) {
      expect(s.path.startsWith("/")).toBe(true);
      expect(s.label.length).toBeGreaterThan(0);
      expect(s.hint.length).toBeGreaterThan(0);
      expect(s.icon.length).toBeGreaterThan(0);
    }
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
