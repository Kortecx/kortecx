import { render } from "@testing-library/react";
import type { ReactNode } from "react";
import { describe, expect, it } from "vitest";
import { renderMarkdown } from "../../src/components/chat/markdown";

function mount(node: ReactNode): HTMLElement {
  const { container } = render(<div>{node}</div>);
  return container;
}

describe("renderMarkdown — safe, dependency-free chat markdown", () => {
  it("**bold** → <strong>, *italic* → <em>, `code` → <code>", () => {
    const c = mount(renderMarkdown("**b** *i* `c`"));
    expect(c.querySelector("strong")?.textContent).toBe("b");
    expect(c.querySelector("em")?.textContent).toBe("i");
    expect(c.querySelector("code")?.textContent).toBe("c");
  });

  it("# heading → <h1>, ## → <h2>, ### → <h3>", () => {
    const c = mount(renderMarkdown("# Title\n\n## Sub\n\n### Deep"));
    expect(c.querySelector("h1")?.textContent).toBe("Title");
    expect(c.querySelector("h2")?.textContent).toBe("Sub");
    expect(c.querySelector("h3")?.textContent).toBe("Deep");
  });

  it("a fenced block → <pre><code> with a VERBATIM body (no inline re-parse, no HTML)", () => {
    const c = mount(renderMarkdown("```\nx = **not bold**\n```"));
    const code = c.querySelector("pre code");
    expect(code?.textContent).toBe("x = **not bold**");
    expect(code?.querySelector("strong")).toBeNull();
  });

  it("- items → <ul><li>, 1. items → <ol><li>", () => {
    const u = mount(renderMarkdown("- a\n- b"));
    expect(u.querySelectorAll("ul li")).toHaveLength(2);
    const o = mount(renderMarkdown("1. a\n2. b"));
    expect(o.querySelectorAll("ol li")).toHaveLength(2);
  });

  it("a safe link → <a href> with rel; a javascript: link renders TEXT only (no anchor)", () => {
    const ok = mount(renderMarkdown("[site](https://example.com)"));
    const a = ok.querySelector("a");
    expect(a?.getAttribute("href")).toBe("https://example.com");
    expect(a?.getAttribute("rel")).toBe("noopener noreferrer");
    expect(a?.getAttribute("target")).toBe("_blank");

    const bad = mount(renderMarkdown("[click](javascript:void)"));
    expect(bad.querySelector("a")).toBeNull();
    expect(bad.textContent).toContain("click");
  });

  it("plain text round-trips with no markup", () => {
    const c = mount(renderMarkdown("just a plain answer"));
    expect(c.textContent).toBe("just a plain answer");
    expect(c.querySelector("strong")).toBeNull();
    expect(c.querySelector("a")).toBeNull();
  });
});
