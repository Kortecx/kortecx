/** PR-4.1 Popover — opens on trigger, closes on Escape + outside click, exposes
 *  a `role="menu"` panel; the new attach-menu interaction pattern. */

import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Popover } from "../../src/components/shell/Popover";

function setup() {
  return render(
    <div>
      <span data-testid="outside">outside</span>
      <Popover trigger="open" triggerLabel="Open menu" triggerTestId="trigger" menuTestId="menu">
        {(close) => (
          <button type="button" data-testid="item" onClick={close}>
            item
          </button>
        )}
      </Popover>
    </div>,
  );
}

describe("Popover", () => {
  it("is closed initially and opens on the trigger", () => {
    setup();
    expect(screen.queryByTestId("menu")).toBeNull();
    fireEvent.click(screen.getByTestId("trigger"));
    expect(screen.getByTestId("menu")).toBeTruthy();
    expect(screen.getByTestId("menu").getAttribute("role")).toBe("menu");
    expect(screen.getByTestId("trigger").getAttribute("aria-expanded")).toBe("true");
  });

  it("closes on Escape", () => {
    setup();
    fireEvent.click(screen.getByTestId("trigger"));
    fireEvent.keyDown(document, { key: "Escape" });
    expect(screen.queryByTestId("menu")).toBeNull();
  });

  it("closes on an outside click", () => {
    setup();
    fireEvent.click(screen.getByTestId("trigger"));
    fireEvent.mouseDown(screen.getByTestId("outside"));
    expect(screen.queryByTestId("menu")).toBeNull();
  });

  it("a menu item can close the panel via the render-prop callback", () => {
    setup();
    fireEvent.click(screen.getByTestId("trigger"));
    fireEvent.click(screen.getByTestId("item"));
    expect(screen.queryByTestId("menu")).toBeNull();
  });
});
