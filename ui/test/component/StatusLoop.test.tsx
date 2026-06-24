/**
 * POC-5c (D168): the New Chat honest status loop. These tests pin the GR15 contract —
 * every displayed phase corresponds to a REAL runtime fact (a durable ReactRound
 * branch or the run projection), and the loop renders NOTHING when idle (never a
 * faked-busy spinner). The word is fact-driven; only the dot animates (CSS).
 */

import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { StatusLoop, derivePhase } from "../../src/components/chat/StatusLoop";
import type { ProjectionVM } from "../../src/kx/use-projection";
import type { ReactTurnVM } from "../../src/kx/use-react-progress";

function turn(partial: Partial<ReactTurnVM>): ReactTurnVM {
  return {
    turn: 0,
    branch: "pending",
    toolId: "",
    toolVersion: "",
    turnMoteId: "ab",
    maxTurns: 8,
    rejectionReason: "",
    callIndex: 0,
    ...partial,
  };
}

function projection(moteCount: number): ProjectionVM {
  return {
    instanceId: "i1",
    recipeFingerprint: "f",
    currentSeq: 1,
    motes: Array.from({ length: moteCount }, (_, i) => ({
      moteId: `m${i}`,
      stateCode: 2,
      ndClass: 1,
      promotion: 0,
      resultRef: null,
      committedSeq: null,
      anomaly: null,
      moteDefHash: "",
      parents: [],
    })),
  };
}

describe("derivePhase (honest, fact-driven — GR15)", () => {
  it("returns null when nothing is in flight (the message renders the answer)", () => {
    expect(
      derivePhase({ busy: false, reactTurns: undefined, activeProjection: undefined }),
    ).toBeNull();
    expect(
      derivePhase({ busy: false, reactTurns: [turn({})], activeProjection: projection(2) }),
    ).toBeNull();
  });

  it("agent: no turns yet → reasoning", () => {
    expect(derivePhase({ busy: true, reactTurns: [], activeProjection: undefined })).toEqual({
      phase: "planning",
      text: "Reasoning…",
    });
  });

  it("agent: a pending turn → reasoning with the real turn/cap", () => {
    const v = derivePhase({
      busy: true,
      reactTurns: [turn({ turn: 1, branch: "pending", maxTurns: 8 })],
      activeProjection: undefined,
    });
    expect(v?.phase).toBe("planning");
    expect(v?.text).toContain("turn 1/8");
  });

  it("agent: a tool turn → tool-call naming the fired tool", () => {
    expect(
      derivePhase({
        busy: true,
        reactTurns: [turn({ turn: 1, branch: "tool", toolId: "mcp-echo", toolVersion: "1" })],
        activeProjection: undefined,
      }),
    ).toEqual({ phase: "tool-call", text: "Calling tool mcp-echo@1" });
  });

  it("agent: a rejected turn → re-planning (a refused proposal, GR15 honest recovery)", () => {
    expect(
      derivePhase({
        busy: true,
        reactTurns: [turn({ branch: "rejected" })],
        activeProjection: undefined,
      })?.phase,
    ).toBe("replanning");
  });

  it("agent: an answer turn → settling", () => {
    expect(
      derivePhase({
        busy: true,
        reactTurns: [turn({ branch: "answer" })],
        activeProjection: undefined,
      })?.phase,
    ).toBe("settling");
  });

  it("agent: a dead-lettered turn → an honest terminal note", () => {
    expect(
      derivePhase({
        busy: true,
        reactTurns: [turn({ branch: "dead_lettered" })],
        activeProjection: undefined,
      }),
    ).toEqual({ phase: "dead-letter", text: "Loop ended without an answer" });
  });

  it("agent: the LATEST turn wins (a tool earlier, pending now → planning)", () => {
    const v = derivePhase({
      busy: true,
      reactTurns: [
        turn({ turn: 0, branch: "tool", toolId: "x", toolVersion: "1" }),
        turn({ turn: 1, branch: "pending" }),
      ],
      activeProjection: undefined,
    });
    expect(v?.phase).toBe("planning");
  });

  it("plain chat: a generating projection → decode", () => {
    expect(
      derivePhase({ busy: true, reactTurns: undefined, activeProjection: projection(2) }),
    ).toEqual({ phase: "decode", text: "Generating…" });
  });

  it("plain chat: no projection yet → submitting", () => {
    expect(
      derivePhase({ busy: true, reactTurns: undefined, activeProjection: undefined })?.phase,
    ).toBe("submitting");
  });
});

describe("StatusLoop render", () => {
  it("renders nothing when idle", () => {
    const { container } = render(
      <StatusLoop chat={{ busy: false, reactTurns: undefined, activeProjection: undefined }} />,
    );
    expect(container).toBeEmptyDOMElement();
  });

  it("renders the fact-driven phase + text with the data-phase attribute", () => {
    render(
      <StatusLoop
        chat={{
          busy: true,
          reactTurns: [turn({ branch: "tool", toolId: "mcp-echo", toolVersion: "1" })],
          activeProjection: undefined,
        }}
      />,
    );
    const el = screen.getByTestId("status-loop");
    expect(el).toHaveAttribute("data-phase", "tool-call");
    expect(el).toHaveAttribute("aria-live", "polite");
    expect(screen.getByTestId("status-loop-text")).toHaveTextContent("mcp-echo@1");
  });
});
