/** PR-4.1b blueprint export: definition (contract + advisory metadata) JSON. */

import { RecipeForm, RecipeFormField } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import {
  blueprintInputs,
  exportBlueprintFilename,
  exportBlueprintJson,
} from "../../src/lib/export-blueprint";

const form = new RecipeForm("kx/recipes/echo", [
  new RecipeFormField("topic", "str", true, 256, []),
  new RecipeFormField("mode", "enum", false, null, ["fast", "slow"]),
]);

describe("exportBlueprintFilename", () => {
  it("slugs safely and never empties", () => {
    expect(exportBlueprintFilename("kx/recipes/echo", 42)).toBe(
      "kortecx-blueprint-kx-recipes-echo-42.json",
    );
    expect(exportBlueprintFilename("", 7)).toBe("kortecx-blueprint-blueprint-7.json");
  });
});

describe("blueprintInputs", () => {
  it("maps the contract, omitting absent max_len/allowed", () => {
    expect(blueprintInputs(form)).toEqual([
      { name: "topic", type: "str", required: true, max_len: 256 },
      { name: "mode", type: "enum", required: false, allowed: ["fast", "slow"] },
    ]);
  });
});

describe("exportBlueprintJson", () => {
  it("emits a self-describing envelope with metadata + inputs", () => {
    const doc = JSON.parse(
      exportBlueprintJson(
        {
          handle: "kx/recipes/echo",
          description: "Echoes the topic",
          tags: ["util"],
          version: "1",
        },
        form,
      ),
    );
    expect(doc).toMatchObject({
      kind: "kortecx.blueprint",
      version: 1,
      handle: "kx/recipes/echo",
      description: "Echoes the topic",
      tags: ["util"],
      blueprint_version: "1",
    });
    expect(doc.inputs).toHaveLength(2);
    expect(doc.inputs[0]).toEqual({ name: "topic", type: "str", required: true, max_len: 256 });
  });

  it("defaults missing metadata to empty (advisory only)", () => {
    const doc = JSON.parse(exportBlueprintJson({ handle: "kx/recipes/echo" }, form));
    expect(doc.description).toBe("");
    expect(doc.tags).toEqual([]);
    expect(doc.blueprint_version).toBe("");
  });
});
