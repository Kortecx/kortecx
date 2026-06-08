import {
  type ErrorCode,
  KxError,
  KxInvalidArgument,
  KxNotFound,
  KxPermissionDenied,
  KxRunFailed,
  KxUnauthenticated,
  KxUnavailable,
  KxUnimplemented,
  KxUsage,
} from "@kortecx/sdk/web";
import { describe, expect, it } from "vitest";
import { toUiError } from "../../src/kx/errors";

describe("toUiError", () => {
  it.each([
    [new KxUnauthenticated("x"), "reauth", false],
    [new KxPermissionDenied("x"), "forbidden", false],
    [new KxNotFound("x"), "not-found", false],
    [new KxUnimplemented("x"), "not-wired", false],
    [new KxInvalidArgument("x"), "bad-input", false],
    [new KxUsage("x"), "bad-input", false],
    [new KxUnavailable("x"), "retry", true],
    [new KxRunFailed("x"), "generic", false],
  ])("maps %o to kind/retryable", (err, kind, retryable) => {
    const ui = toUiError(err);
    expect(ui.kind).toBe(kind);
    expect(ui.retryable).toBe(retryable);
    expect(ui.title).toBeTruthy();
    expect(ui.code).toBeTruthy();
  });

  it("an unknown future ErrorCode falls back to generic", () => {
    const ui = toUiError(new KxError("weird", { code: "brand_new_code" as ErrorCode }));
    expect(ui.kind).toBe("generic");
  });

  it("a raw (non-Kx) error is normalized via fromRpcError", () => {
    const ui = toUiError(new Error("boom"));
    expect(ui.code).toBeTruthy();
    expect(ui.kind).toBeTruthy();
  });
});
