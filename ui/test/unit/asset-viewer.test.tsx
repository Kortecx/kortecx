/**
 * AssetViewer — the shared OSS Data Lab multi-modal renderer. Asserts that media
 * renders from a `blob:` object URL (created once, revoked on unmount — no leak),
 * markdown renders via the React-element renderer (never innerHTML), text renders
 * in the code body, and a truncated media payload degrades to an honest download.
 */

import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { AssetViewer } from "../../src/components/AssetViewer";
import type { DecodedContent } from "../../src/lib/content-decode";

const createObjectURL = vi.fn(() => "blob:mock");
const revokeObjectURL = vi.fn();

beforeAll(() => {
  Object.assign(globalThis.URL, { createObjectURL, revokeObjectURL });
});
afterEach(() => {
  cleanup();
  createObjectURL.mockClear();
  revokeObjectURL.mockClear();
});

function media(kind: "image" | "video" | "audio", mediaType: string): DecodedContent {
  return {
    kind,
    text: "",
    bytes: new Uint8Array([1, 2, 3]),
    mediaType,
    byteLength: 3,
    truncated: false,
  };
}

describe("AssetViewer", () => {
  it("renders an image from a blob object URL (created once)", () => {
    render(<AssetViewer content={media("image", "image/png")} />);
    expect(screen.getByTestId("asset-image").getAttribute("src")).toBe("blob:mock");
    expect(createObjectURL).toHaveBeenCalledTimes(1);
  });

  it("renders video and audio media elements", () => {
    render(<AssetViewer content={media("video", "video/mp4")} />);
    expect(screen.getByTestId("asset-video")).toBeTruthy();
    cleanup();
    render(<AssetViewer content={media("audio", "audio/ogg")} />);
    expect(screen.getByTestId("asset-audio")).toBeTruthy();
  });

  it("revokes the object URL on unmount (no leak)", () => {
    const { unmount } = render(<AssetViewer content={media("image", "image/png")} />);
    unmount();
    expect(revokeObjectURL).toHaveBeenCalledWith("blob:mock");
  });

  it("renders markdown through the element renderer (never innerHTML)", () => {
    const md: DecodedContent = { kind: "markdown", text: "# Hi", byteLength: 4, truncated: false };
    render(<AssetViewer content={md} />);
    expect(screen.getByTestId("asset-markdown")).toBeTruthy();
  });

  it("renders text in the code body", () => {
    const t: DecodedContent = { kind: "text", text: "plain", byteLength: 5, truncated: false };
    render(<AssetViewer content={t} />);
    expect(screen.getByTestId("asset-viewer-body").textContent).toContain("plain");
  });

  it("degrades truncated media to an honest download (no broken element)", () => {
    const big: DecodedContent = {
      kind: "image",
      text: "",
      bytes: new Uint8Array([1]),
      mediaType: "image/png",
      byteLength: 999,
      truncated: true,
    };
    render(<AssetViewer content={big} />);
    expect(screen.queryByTestId("asset-image")).toBeNull();
    expect(screen.getByText("Preview too large")).toBeTruthy();
  });
});
