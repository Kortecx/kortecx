/** PR-4.1 copy hook — writes to the clipboard + flips `copied`, degrading when
 *  the clipboard is unavailable. */

import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useCopyToClipboard } from "../../src/lib/use-copy-to-clipboard";

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("useCopyToClipboard", () => {
  it("writes the text and flips copied true then back", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    // A 200ms window is long enough that the first `waitFor` reliably observes
    // `true` before the reset fires, then `false` after it.
    const { result } = renderHook(() => useCopyToClipboard(200));
    expect(result.current.copied).toBe(false);
    act(() => result.current.copy("hello"));
    expect(writeText).toHaveBeenCalledWith("hello");
    await waitFor(() => expect(result.current.copied).toBe(true));
    await waitFor(() => expect(result.current.copied).toBe(false));
  });

  it("degrades silently when the clipboard is unavailable", () => {
    vi.stubGlobal("navigator", {});
    const { result } = renderHook(() => useCopyToClipboard());
    expect(() => act(() => result.current.copy("x"))).not.toThrow();
    expect(result.current.copied).toBe(false);
  });

  it("degrades silently when writeText rejects", async () => {
    const writeText = vi.fn().mockRejectedValue(new Error("denied"));
    vi.stubGlobal("navigator", { clipboard: { writeText } });
    const { result } = renderHook(() => useCopyToClipboard());
    act(() => result.current.copy("x"));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(result.current.copied).toBe(false);
  });
});
