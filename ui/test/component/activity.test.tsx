import { Delta } from "@kortecx/sdk/web";
import { fireEvent, render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { ActivityFeed } from "../../src/components/activity/ActivityFeed";
import { TimeTravelScrubber } from "../../src/components/activity/TimeTravelScrubber";

describe("ActivityFeed", () => {
  it("empty state when there are no events", () => {
    render(<ActivityFeed events={[]} dropped={false} active={false} />);
    expect(screen.getByText(/no events yet/i)).toBeInTheDocument();
  });

  it("renders rows in the given (newest-first) order", () => {
    const events = [
      new Delta(2, "failed", "aa".repeat(32)),
      new Delta(1, "committed", "bb".repeat(32), "cc".repeat(32)),
    ];
    render(<ActivityFeed events={events} dropped={false} active={true} />);
    const rows = screen.getAllByTestId("event-row");
    expect(rows).toHaveLength(2);
    expect(rows[0]).toHaveAttribute("data-kind", "failed");
    expect(rows[1]).toHaveAttribute("data-kind", "committed");
  });

  it("shows a dropped-stream notice", () => {
    render(
      <ActivityFeed events={[new Delta(1, "committed", "aa".repeat(32))]} dropped active={false} />,
    );
    expect(screen.getByTestId("feed-dropped")).toBeInTheDocument();
  });
});

describe("TimeTravelScrubber", () => {
  it("live by default; Live button disabled", () => {
    const onChange = vi.fn();
    render(<TimeTravelScrubber currentSeq={10} onChange={onChange} />);
    expect(screen.getByTestId("scrubber-seq")).toHaveTextContent("live");
    expect(screen.getByTestId("scrubber-live")).toBeDisabled();
  });

  it("scrubbing pins a seq via onChange", () => {
    const onChange = vi.fn();
    render(<TimeTravelScrubber currentSeq={10} onChange={onChange} />);
    fireEvent.change(screen.getByLabelText(/journal sequence/i), { target: { value: "3" } });
    expect(onChange).toHaveBeenCalledWith(3);
  });

  it("pinned at seq 0 shows #0 and Live clears it", () => {
    const onChange = vi.fn();
    render(<TimeTravelScrubber currentSeq={10} atSeq={0} onChange={onChange} />);
    expect(screen.getByTestId("scrubber-seq")).toHaveTextContent("#0");
    const live = screen.getByTestId("scrubber-live");
    expect(live).toBeEnabled();
    fireEvent.click(live);
    expect(onChange).toHaveBeenCalledWith(undefined);
  });
});
