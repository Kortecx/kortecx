/**
 * WAVE-3 (PR-1): RunTimeline (WATCH) + RunChanges (REVIEW) view tests. The data hooks
 * are mocked so the pure view logic — a card per ReAct turn with a step-type badge, the
 * rejected-turn fail-closed disclosure, the pure-DAG per-step fallback, and the honest
 * post-run-head outputs list — is asserted without a live gateway.
 */

import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { MoteVM, ProjectionVM } from "../../src/kx/use-projection";
import type { ReactTurnVM } from "../../src/kx/use-react-progress";
import type { DecodedContent } from "../../src/lib/content-decode";

let TURNS: ReactTurnVM[] = [];
let TERMINAL: ReactTurnVM | null = null;
let CONTENT: {
  data: DecodedContent | undefined;
  isLoading: boolean;
  error: unknown;
  refetch: () => void;
} = {
  data: { kind: "text", text: "hello output", byteLength: 12, truncated: false },
  isLoading: false,
  error: null,
  refetch: vi.fn(),
};

vi.mock("../../src/kx/use-react-progress", () => ({
  useReactProgress: () => ({ turns: TURNS, terminal: TERMINAL }),
}));
vi.mock("../../src/kx/use-run-step-kinds", () => ({
  useRunStepKinds: () => new Map(),
}));
vi.mock("../../src/kx/use-content", () => ({
  useContent: () => CONTENT,
}));
// A settled run for the fallback path — decouple the view test from stateCode semantics.
vi.mock("../../src/kx/use-projection", () => ({
  allTerminal: (p: ProjectionVM) => p.motes.length > 0,
}));
// Monaco is lazy; render a plain <pre> so the view test stays synchronous.
vi.mock("../../src/components/editor/CodeViewer", () => ({
  CodeViewer: ({ value, testId }: { value: string; testId?: string }) => (
    <pre data-testid={testId}>{value}</pre>
  ),
}));

import { RunTimeline } from "../../src/components/dag/RunTimeline";

function turn(over: Partial<ReactTurnVM> = {}): ReactTurnVM {
  return {
    turn: 1,
    branch: "pending",
    toolId: "",
    toolVersion: "",
    turnMoteId: "aa".repeat(32),
    maxTurns: 8,
    rejectionReason: "",
    callIndex: 0,
    grantedTools: [],
    secretScopeNames: [],
    ...over,
  };
}
function mote(over: Partial<MoteVM> = {}): MoteVM {
  return {
    moteId: "bb".repeat(32),
    stateCode: 0,
    ndClass: 0,
    promotion: 0,
    resultRef: null,
    committedSeq: 1,
    anomaly: null,
    moteDefHash: "",
    parents: [],
    ...over,
  };
}
function projection(motes: MoteVM[] = []): ProjectionVM {
  return { instanceId: "ii".repeat(16), recipeFingerprint: "ff".repeat(16), currentSeq: 1, motes };
}

afterEach(() => {
  TURNS = [];
  TERMINAL = null;
  CONTENT = {
    data: { kind: "text", text: "hello output", byteLength: 12, truncated: false },
    isLoading: false,
    error: null,
    refetch: vi.fn(),
  };
});

describe("RunTimeline (WATCH) + RunChanges (REVIEW)", () => {
  it("renders a card per ReAct turn with a step-type badge + a rejected-turn disclosure", () => {
    TURNS = [
      turn({ turn: 1, branch: "tool", toolId: "search", toolVersion: "1" }),
      turn({ turn: 2, branch: "rejected", rejectionReason: "tool not permitted" }),
      turn({ turn: 3, branch: "answer" }),
    ];
    TERMINAL = TURNS[2] ?? null;
    render(<RunTimeline instanceId="run1" projection={projection()} />);
    expect(screen.getByTestId("run-timeline")).toBeInTheDocument();
    // A tool turn badges as its step type and shows the fired tool id@version.
    expect(screen.getByTestId("run-turn-1")).toHaveTextContent("Tool");
    expect(screen.getByTestId("run-turn-1")).toHaveTextContent("search@1");
    // The rejected turn's fail-closed reason expands as a disclosure.
    expect(screen.getByTestId("run-turn-2-reason")).toHaveTextContent("tool not permitted");
    // Terminal ⇒ honest "at rest".
    expect(screen.getByText(/at rest/i)).toBeInTheDocument();
  });

  it("falls back to a per-step list for a settled pure-DAG run (no ReAct turns)", () => {
    render(
      <RunTimeline
        instanceId="run1"
        projection={projection([mote({ moteDefHash: "cc".repeat(32) })])}
      />,
    );
    expect(screen.getByTestId("run-timeline-steps")).toBeInTheDocument();
    expect(screen.getByText(/run at rest/i)).toBeInTheDocument();
  });

  it("RunChanges lists committed outputs and renders the selected body (post-run head)", () => {
    const withOutput = projection([
      mote({ moteId: "d".repeat(64), resultRef: "e".repeat(64), moteDefHash: "cc".repeat(32) }),
    ]);
    render(<RunTimeline instanceId="run1" projection={withOutput} />);
    expect(screen.getByTestId("run-changes")).toBeInTheDocument();
    expect(screen.getByTestId("run-changes-list")).toBeInTheDocument();
    // The first output auto-selects and renders its committed body.
    expect(screen.getByTestId("run-changes-body")).toHaveTextContent("hello output");
    // Honest framing — not a fabricated before/after diff.
    expect(screen.getByText(/post-run head/i)).toBeInTheDocument();
  });

  it("RunChanges shows an honest empty state when no outputs are committed", () => {
    render(<RunTimeline instanceId="run1" projection={projection()} />);
    expect(screen.getByText(/no committed outputs/i)).toBeInTheDocument();
  });
});
