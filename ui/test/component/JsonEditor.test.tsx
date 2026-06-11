import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { JsonEditor } from "../../src/components/editor/JsonEditor";

// jsdom has no Worker → MonacoMount renders the textarea fallback (the real editor
// is browser-only, exercised by the e2e). The fallback keeps the `args` id/testid.
describe("JsonEditor", () => {
  it("renders the fallback textarea with the `args` handles", () => {
    render(<JsonEditor id="args" value='{"a":1}' onChange={() => {}} />);
    const ta = screen.getByTestId("args");
    expect(ta.tagName).toBe("TEXTAREA");
    expect(ta).toHaveAttribute("id", "args");
    expect(ta).toHaveValue('{"a":1}');
  });

  it("forwards edits via onChange (validation stays in the parent)", () => {
    const onChange = vi.fn();
    render(<JsonEditor id="args" value="" onChange={onChange} />);
    fireEvent.change(screen.getByTestId("args"), { target: { value: "{}" } });
    expect(onChange).toHaveBeenCalledWith("{}");
  });
});
