/**
 * A DISABLED control must say WHY without a hover.
 *
 * The incident this exists for: a gateway served without the hosted-app supervisor greyed
 * the Run control and put the reason in a `title` attribute. The console had degraded
 * honestly — and the operator read it as "there is no start button" and spent a whole
 * diagnosis on it. A `title` is not an explanation: it needs a pointing device, a hover, a
 * delay, and the knowledge that hovering would help.
 *
 * So the assertion is deliberately NOT "a title exists". It is that the reason is reachable
 * as TEXT — the thing a person sees, a screen reader announces, and a test can find without
 * simulating a mouse.
 */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

let DISABLED = true;

vi.mock("../../src/kx/use-hosted-app", () => ({
  useHostedRun: () => ({
    run: vi.fn(),
    disabled: DISABLED,
    busy: false,
    error: null,
  }),
  useHostedAppStatus: () => ({ status: null, notWired: true }),
  useStopHostedApp: () => ({ mutate: vi.fn(), isPending: false }),
}));

import { HostedRunButton } from "../../src/components/apps/HostedControls";

afterEach(() => {
  DISABLED = true;
  vi.clearAllMocks();
});

describe("a disabled affordance carries its reason without a hover", () => {
  it("renders the reason as text, not only as a title attribute", () => {
    render(<HostedRunButton handle="team/apps/landing" />);

    const control = screen.getByTestId("hosted-run-team/apps/landing");
    expect(control).toHaveAttribute("aria-disabled", "true");

    // THE ASSERTION. `textContent` is what a sighted user reads and a screen reader
    // announces; a `title` is neither. Before the fix this element's text was empty —
    // the control was an icon and nothing else.
    expect(control.textContent?.trim()).not.toBe("");
    expect(control.textContent?.toLowerCase()).toMatch(/unavailable|not available/);
  });

  it("the visible reason names the CAUSE, so it is actionable rather than merely honest", () => {
    render(<HostedRunButton handle="team/apps/landing" />);
    // Searching the accessible tree by TEXT — this query cannot be satisfied by a title.
    const why = screen.getByTestId("hosted-run-reason-team/apps/landing");
    expect(why.textContent?.toLowerCase()).toContain("hosted");
  });

  it("an ENABLED control is unaffected — the reason chip is absent, not merely hidden", () => {
    DISABLED = false;
    render(<HostedRunButton handle="team/apps/landing" />);
    // The control-arm of the pair: without it, a component that ALWAYS rendered the
    // reason would pass both assertions above.
    expect(screen.queryByTestId("hosted-run-reason-team/apps/landing")).toBeNull();
    expect(screen.getByTestId("hosted-run-team/apps/landing").tagName).toBe("BUTTON");
  });
});
