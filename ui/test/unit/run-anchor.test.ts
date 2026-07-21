/**
 * The RUN ANCHOR precedence and the search it produces.
 *
 * The two `RunHandle` keys are not interchangeable — `react_chain_salt` is a ReAct chain
 * key the server emits ONLY for a single tool-granted agentic step, `terminal_mote_id` is
 * the sink Mote every shape has — and the whole point of `lib/run-anchor` is that the
 * choice between them is made in ONE place. #362 shipped the scoping machinery with the
 * `||` re-typed at three call sites and omitted at eight more, and nothing caught it
 * because none of this had a test.
 */

import { describe, expect, it } from "vitest";
import {
  type RunAnchors,
  memberMoteSearch,
  runAnchor,
  runViewHref,
  runViewSearch,
} from "../../src/lib/run-anchor";

const SALT = "5a".repeat(32);
const TERMINAL = "7e".repeat(32);
const INSTANCE = "ab".repeat(16);

describe("runAnchor", () => {
  it("prefers the ReAct chain salt when the run has one", () => {
    // The salt pins the agentic chain exactly and is the key the react surfaces already
    // thread through; the terminal Mote is the fallback, not the other way round.
    expect(runAnchor({ reactChainSalt: SALT, terminalMoteId: TERMINAL })).toBe(SALT);
  });

  it("falls back to the terminal Mote — the case that covers MOST runs", () => {
    // A plain single-agent App, a pure pipeline and a multi-step DAG all get an EMPTY
    // salt by design. Anchoring on the salt alone is why those runs shipped unscoped.
    expect(runAnchor({ reactChainSalt: "", terminalMoteId: TERMINAL })).toBe(TERMINAL);
  });

  it("returns '' when the server gave us neither (an old server, a durable-only row)", () => {
    // "" means CANNOT SCOPE. It must never be confused with a real anchor, because a
    // wrong anchor renders an empty run and an absent one renders an honest notice.
    expect(runAnchor({ reactChainSalt: "", terminalMoteId: "" })).toBe("");
    expect(runAnchor({})).toBe("");
  });

  it("treats null exactly like empty (the persisted RunRecord shape)", () => {
    // `RunRecord` stores `null`, the live mutation results store `""`. One helper serves
    // both, so a run reopened from history scopes the same way it did when it started.
    expect(runAnchor({ reactChainSalt: null, terminalMoteId: null })).toBe("");
    expect(runAnchor({ reactChainSalt: null, terminalMoteId: TERMINAL })).toBe(TERMINAL);
  });
});

describe("runViewSearch", () => {
  it("carries the poll-stop hint AND the scope anchor", () => {
    expect(runViewSearch({ reactChainSalt: SALT, terminalMoteId: TERMINAL })).toEqual({
      terminal: TERMINAL,
      chain: SALT,
    });
  });

  it("a salt-less run still gets a scope — chain falls back to the terminal Mote", () => {
    expect(runViewSearch({ reactChainSalt: "", terminalMoteId: TERMINAL })).toEqual({
      terminal: TERMINAL,
      chain: TERMINAL,
    });
  });

  it("OMITS what it does not know rather than sending empty keys", () => {
    // An absent `chain` is what makes the run view render its unscoped notice. Emitting
    // `chain=""` would look scoped to a reader and change nothing for the fold.
    const record: RunAnchors = { reactChainSalt: null, terminalMoteId: null };
    expect(runViewSearch(record)).toEqual({});
    expect(Object.keys(runViewSearch(record))).toHaveLength(0);
  });
});

describe("memberMoteSearch", () => {
  it("anchors on an arbitrary member Mote (a feed row, an inspector selection)", () => {
    // The fold is a connected-component walk, so any Mote of the submission anchors it.
    // There is no terminal id at these sites, so no poll-stop hint rides along.
    expect(memberMoteSearch(SALT)).toEqual({ chain: SALT });
  });

  it("no Mote (a pre-registration delta) ⇒ unscoped, honestly", () => {
    expect(memberMoteSearch(null)).toEqual({});
    expect(memberMoteSearch(undefined)).toEqual({});
  });
});

describe("runViewHref (the open-in-new-window path)", () => {
  it("produces the same search the <Link> sites do", () => {
    expect(runViewHref(INSTANCE, { reactChainSalt: SALT, terminalMoteId: TERMINAL })).toBe(
      `/workflows/${INSTANCE}?terminal=${TERMINAL}&chain=${SALT}`,
    );
  });

  it("no anchors ⇒ a bare path, no dangling '?'", () => {
    expect(runViewHref(INSTANCE, {})).toBe(`/workflows/${INSTANCE}`);
  });
});
