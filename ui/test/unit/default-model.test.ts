import { afterEach, describe, expect, it } from "vitest";
import { clearDefaultModel, loadDefaultModel, saveDefaultModel } from "../../src/lib/default-model";

afterEach(() => localStorage.clear());

describe("default-model (POC-5c client-local preference)", () => {
  it("returns undefined when nothing is saved", () => {
    expect(loadDefaultModel()).toBeUndefined();
  });

  it("round-trips a saved default", () => {
    saveDefaultModel("gemma-4-12b");
    expect(loadDefaultModel()).toBe("gemma-4-12b");
  });

  it("clear() removes the default", () => {
    saveDefaultModel("gemma-4-12b");
    clearDefaultModel();
    expect(loadDefaultModel()).toBeUndefined();
  });

  it("treats a blank stored value as unset (corruption-safe)", () => {
    localStorage.setItem("kortecx.ui.default-model", "   ");
    expect(loadDefaultModel()).toBeUndefined();
  });
});
