import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ResultPreview, oneLine } from "../../src/components/ResultPreview";
import { decodeContent } from "../../src/lib/content-decode";

const enc = (s: string) => new TextEncoder().encode(s);
const REF = "ab".repeat(32);

describe("oneLine", () => {
  it("collapses whitespace and trims", () => {
    expect(oneLine("a\n  b\t c  ")).toBe("a b c");
  });
  it("clips to the cap with an ellipsis", () => {
    expect(oneLine("abcdefghij", 4)).toBe("abcd…");
  });
  it("leaves a short string untouched (no ellipsis)", () => {
    expect(oneLine("short", 80)).toBe("short");
  });
});

describe("ResultPreview", () => {
  it("uncommitted (no ref) renders an em dash, never a hash", () => {
    const { container } = render(<ResultPreview resultRef={null} />);
    expect(container.textContent).toBe("—");
    expect(screen.queryByTestId("result-preview")).toBeNull();
    expect(screen.queryByTestId("digest-chip")).toBeNull();
  });

  it("loading shows resolving… + the digest chip", () => {
    render(<ResultPreview resultRef={REF} loading />);
    const el = screen.getByTestId("result-preview");
    expect(el).toHaveAttribute("data-state", "loading");
    expect(el).toHaveTextContent("resolving…");
    expect(screen.getByTestId("digest-chip")).toBeInTheDocument();
  });

  it("missing (uniform-empty item) reads 'unavailable', honestly", () => {
    render(<ResultPreview resultRef={REF} missing />);
    const el = screen.getByTestId("result-preview");
    expect(el).toHaveAttribute("data-state", "missing");
    expect(el).toHaveTextContent("unavailable");
  });

  it("empty result reads '(empty)'", () => {
    render(<ResultPreview resultRef={REF} content={decodeContent(enc(""))} />);
    const el = screen.getByTestId("result-preview");
    expect(el).toHaveAttribute("data-state", "empty");
    expect(el).toHaveTextContent("(empty)");
  });

  it("text result shows the resolved TEXT as the headline (not the hash)", () => {
    render(<ResultPreview resultRef={REF} content={decodeContent(enc("the model said hello"))} />);
    const el = screen.getByTestId("result-preview");
    expect(el).toHaveAttribute("data-state", "text");
    expect(el).toHaveTextContent("the model said hello");
    // the digest is present but SECONDARY (a chip), not the headline
    expect(screen.getByTestId("digest-chip")).toBeInTheDocument();
  });

  it("json result pretty-prints as text (collapsed to one line)", () => {
    render(<ResultPreview resultRef={REF} content={decodeContent(enc('{"a":1}'))} />);
    const el = screen.getByTestId("result-preview");
    expect(el).toHaveAttribute("data-state", "text");
    expect(el).toHaveTextContent('"a": 1');
  });

  it("non-UTF-8 is shown as 'binary · N B' (never a fake hash headline)", () => {
    const bytes = new Uint8Array([0xff, 0xfe, 0xfd]);
    render(<ResultPreview resultRef={REF} content={decodeContent(bytes)} />);
    const el = screen.getByTestId("result-preview");
    expect(el).toHaveAttribute("data-state", "binary");
    expect(el).toHaveTextContent("binary · 3 B");
  });

  it("clips a long text result to the max headline length", () => {
    const long = "x".repeat(500);
    render(<ResultPreview resultRef={REF} content={decodeContent(enc(long))} max={40} />);
    const text = screen.getByTestId("result-preview").querySelector(".result-preview__text");
    expect(text?.textContent?.endsWith("…")).toBe(true);
    // 40 chars + the ellipsis
    expect((text?.textContent ?? "").length).toBe(41);
    // the FULL text stays available in the title (hover / a11y)
    expect(text).toHaveAttribute("title", long);
  });

  it("chip=false omits the digest chip (for inside-a-button containers)", () => {
    render(<ResultPreview resultRef={REF} content={decodeContent(enc("hi"))} chip={false} />);
    expect(screen.getByTestId("result-preview")).toHaveTextContent("hi");
    expect(screen.queryByTestId("digest-chip")).toBeNull();
  });
});
