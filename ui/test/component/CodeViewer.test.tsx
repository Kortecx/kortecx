import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { CodeViewer } from "../../src/components/editor/CodeViewer";

// jsdom → the read-only `<pre>` fallback carrying the same test handle + content.
describe("CodeViewer", () => {
  it("renders a read-only <pre> with the content text", () => {
    render(<CodeViewer value='{\n  "k": 1\n}' testId="cv" />);
    const pre = screen.getByTestId("cv");
    expect(pre.tagName).toBe("PRE");
    expect(pre).toHaveTextContent('"k": 1');
  });

  it("renders plaintext values too", () => {
    render(<CodeViewer value="hello world" testId="cv2" />);
    expect(screen.getByTestId("cv2")).toHaveTextContent("hello world");
  });
});
