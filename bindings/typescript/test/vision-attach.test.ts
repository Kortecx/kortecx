/** PR-B2 vision chaining — `chat({ image })` binds kx/recipes/vision. Pure, no server. */

import { describe, expect, it } from "vitest";
import { PutResult } from "../src/content.js";
import { KxUsage } from "../src/errors.js";
import { KxClient } from "../src/node.js";
import { RecipeForm, RecipeFormField } from "../src/recipes.js";
import type { Args } from "../src/transport.js";

/** A KxClient whose put/form/invoke are doubled so the test captures the wire shape
 * `chat({ image })` produces without a server. */
class VisionFake extends KxClient {
  putCalls: Uint8Array[] = [];
  invoked: { handle: string; args: Args } | undefined;
  constructor(
    private readonly fields: RecipeFormField[] | null,
    defaultModel = "",
  ) {
    super("http://127.0.0.1:1", { defaultModel });
  }
  override async putContent(payload: Uint8Array): Promise<PutResult> {
    this.putCalls.push(payload);
    return new PutResult("ab".repeat(32), BigInt(payload.length), false);
  }
  override async getRecipeForm(handle: string): Promise<RecipeForm> {
    if (this.fields === null) throw new Error("kx/recipes/vision not provisioned");
    return new RecipeForm(handle, this.fields);
  }
  // biome-ignore lint/suspicious/noExplicitAny: a test double for the wait path.
  override async invoke(handle: string, args: Args): Promise<any> {
    this.invoked = { handle, args };
    return { text: "a cat" };
  }
}

const visionForm = (allowed: string[]) => [
  new RecipeFormField("prompt", "str", true, 4096, []),
  new RecipeFormField("image_ref", "bytes", true, 64, []),
  new RecipeFormField("model", "enum", true, null, allowed),
];

describe("chat({ image })", () => {
  it("uploads raw bytes and binds kx/recipes/vision with {prompt, image_ref, model}", async () => {
    const c = new VisionFake(visionForm(["gemma3:12b"]));
    const bytes = new Uint8Array([0x89, 0x50, 0x4e, 0x47]);
    const out = await c.chat("what is in this image?", { image: bytes });

    expect(out).toBe("a cat");
    expect(c.putCalls).toHaveLength(1);
    expect(c.putCalls[0]).toEqual(bytes);
    expect(c.invoked?.handle).toBe("kx/recipes/vision");
    expect(c.invoked?.args).toEqual({
      image_ref: "ab".repeat(32),
      prompt: "what is in this image?",
      model: "gemma3:12b",
    });
  });

  it("passes an existing { ref } through without uploading", async () => {
    const c = new VisionFake(visionForm(["m"]));
    await c.chat("ocr please", { image: { ref: "cd".repeat(32) } });
    expect(c.putCalls).toHaveLength(0);
    expect((c.invoked?.args as Record<string, unknown>).image_ref).toBe("cd".repeat(32));
  });

  it("prefers the default model when it is a legal ENUM value", async () => {
    const c = new VisionFake(visionForm(["a", "b", "gemma3:12b"]), "gemma3:12b");
    await c.chat("hi", { image: new Uint8Array([1]) });
    expect((c.invoked?.args as Record<string, unknown>).model).toBe("gemma3:12b");
  });

  it("rejects dataset + image together (vision-RAG not supported)", async () => {
    const c = new VisionFake(visionForm(["m"]));
    await expect(
      c.chat("hi", { image: new Uint8Array([1]), dataset: "docs" }),
    ).rejects.toBeInstanceOf(KxUsage);
  });

  it("honest-degrades to a clear error when no vision model is served", async () => {
    const c = new VisionFake(null); // getRecipeForm throws ⇒ no image-capable model
    await expect(c.chat("hi", { image: new Uint8Array([1]) })).rejects.toBeInstanceOf(KxUsage);
  });

  it("plain chat (no image) is unaffected", async () => {
    const c = new VisionFake(visionForm(["m"]));
    // No image ⇒ never touches put/form; routes to the normal chat invoke.
    const out = await c.chat("hello");
    expect(out).toBe("a cat");
    expect(c.putCalls).toHaveLength(0);
    expect(c.invoked?.handle).toBe("kx/recipes/chat");
  });
});
