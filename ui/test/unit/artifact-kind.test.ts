import { describe, expect, it } from "vitest";
import { artifactKindVisual } from "../../src/lib/artifact-kind";

describe("artifactKindVisual", () => {
  it("maps each decoded kind to a label + glyph", () => {
    expect(artifactKindVisual("json").label).toBe("JSON");
    expect(artifactKindVisual("text").label).toBe("Text");
    expect(artifactKindVisual("binary").label).toBe("Binary");
    expect(artifactKindVisual("empty").label).toBe("Empty");
    for (const k of ["json", "text", "binary", "empty"] as const) {
      expect(artifactKindVisual(k).glyph.length).toBeGreaterThan(0);
    }
  });
});
