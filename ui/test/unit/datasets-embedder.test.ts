import { ErrorCode } from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { isNoEmbedder } from "../../src/components/datasets/EmbedderNotice";

describe("isNoEmbedder", () => {
  it("is true only for the gateway's FAILED_PRECONDITION (no embedder wired)", () => {
    expect(isNoEmbedder({ code: ErrorCode.FailedPrecondition })).toBe(true);
  });

  it("is false for other error codes + plain errors", () => {
    expect(isNoEmbedder({ code: ErrorCode.NotFound })).toBe(false);
    expect(isNoEmbedder({ code: ErrorCode.Unimplemented })).toBe(false);
    expect(isNoEmbedder({ code: ErrorCode.InvalidArgument })).toBe(false);
    expect(isNoEmbedder(new Error("boom"))).toBe(false);
  });
});
