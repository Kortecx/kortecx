/**
 * A2: the composer TOOL picker — a live multi-select of fireable tools mirroring
 * the Context picker (role="menuitemcheckbox" / aria-checked / ✓; the menu stays
 * open on toggle). Honest-degrades to a not-wired or empty row; never fakes an
 * option. Blueprint/Dataset stay honest-disabled.
 */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Composer, type ToolPickerProps } from "../../src/components/chat/Composer";

/** Render the composer with a tool picker and open the attach menu. */
function openWithTools(tools: ToolPickerProps) {
  render(<Composer disabled={false} onSend={vi.fn()} onPickFiles={vi.fn()} tools={tools} />);
  fireEvent.click(screen.getByTestId("attach-btn"));
}

describe("Composer tool picker (A2)", () => {
  it("lists fireable tools as multi-select checkmark rows", () => {
    const onToggle = vi.fn();
    openWithTools({
      options: ["web-search@1", "mcp-echo/echo@1"],
      attached: ["web-search@1"],
      notWired: false,
      onToggle,
    });
    expect(screen.getByTestId("attach-tool-group")).toBeInTheDocument();
    const picked = screen.getByTestId("attach-tool-option-web-search@1");
    expect(picked.getAttribute("role")).toBe("menuitemcheckbox");
    expect(picked.getAttribute("aria-checked")).toBe("true");
    const other = screen.getByTestId("attach-tool-option-mcp-echo/echo@1");
    expect(other.getAttribute("aria-checked")).toBe("false");
    // Toggling reports the composite `${name}@${version}` key and keeps the menu open.
    fireEvent.click(other);
    expect(onToggle).toHaveBeenCalledWith("mcp-echo/echo@1");
    expect(screen.getByTestId("attach-menu")).toBeInTheDocument();
  });

  it("shows the honest empty row when no tools are fireable", () => {
    openWithTools({ options: [], attached: [], notWired: false, onToggle: vi.fn() });
    expect(screen.getByTestId("attach-tool-group")).toBeInTheDocument();
    expect(screen.getByTestId("attach-tool-empty")).toBeDisabled();
  });

  it("honest-degrades to a not-wired row when the registry is unsupported", () => {
    openWithTools({ options: [], attached: [], notWired: true, onToggle: vi.fn() });
    expect(screen.getByTestId("attach-tool-not-wired")).toBeDisabled();
    expect(screen.queryByTestId("attach-tool-empty")).toBeNull();
  });

  it("keeps Blueprint/Dataset honest-disabled and drops the old attach-tool row", () => {
    openWithTools({ options: [], attached: [], notWired: false, onToggle: vi.fn() });
    expect(screen.getByTestId("attach-blueprint")).toBeDisabled();
    expect(screen.getByTestId("attach-dataset")).toBeDisabled();
    expect(screen.queryByTestId("attach-tool")).toBeNull();
  });
});
