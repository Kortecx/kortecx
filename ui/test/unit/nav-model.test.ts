import { describe, expect, it } from "vitest";
import { NAV_SECTIONS } from "../../src/components/shell/nav-model";

describe("NAV_SECTIONS", () => {
  it("has the seven console sections", () => {
    expect(NAV_SECTIONS.map((s) => s.id)).toEqual([
      "activity",
      "chat",
      "runs",
      "recipes",
      "artifacts",
      "datasets",
      "systems",
    ]);
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
