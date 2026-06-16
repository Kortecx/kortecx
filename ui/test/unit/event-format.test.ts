import { describe, expect, it } from "vitest";
import {
  type EventLike,
  eventSummary,
  eventVisual,
  exportFeedFilename,
  failureReasonLabel,
  feedToNdjson,
  matchesFeedFilter,
  tallyEventsByKind,
} from "../../src/lib/event-format";

describe("eventVisual", () => {
  it("maps known kinds to reused state tones", () => {
    expect(eventVisual("committed")).toEqual({ label: "COMMITTED", tone: "committed" });
    expect(eventVisual("failed")).toEqual({ label: "FAILED", tone: "failed" });
    expect(eventVisual("repudiated")).toEqual({ label: "REPUDIATED", tone: "repudiated" });
    expect(eventVisual("effect_staged")).toEqual({ label: "EFFECT STAGED", tone: "scheduled" });
  });

  it("unknown kind → unknown tone (never crashes)", () => {
    expect(eventVisual("future_kind").tone).toBe("unknown");
  });
});

describe("eventSummary", () => {
  const id = "ab".repeat(32);
  const ref = "cd".repeat(32);

  it("committed with a result ref", () => {
    const s = eventSummary({ seq: 3, kind: "committed", moteId: id, resultRef: ref });
    expect(s).toContain("committed");
    expect(s).toContain("→");
  });

  it("committed without a result ref omits the arrow", () => {
    const s = eventSummary({ seq: 3, kind: "committed", moteId: id });
    expect(s).toContain("committed");
    expect(s).not.toContain("→");
  });

  it("omitResultRef drops the trailing hash (the row renders resolved text instead)", () => {
    const s = eventSummary(
      { seq: 3, kind: "committed", moteId: id, resultRef: ref },
      undefined,
      true,
    );
    expect(s).toContain("committed");
    expect(s).not.toContain("→");
  });

  it("failed / repudiated / effect_staged read naturally", () => {
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id })).toContain("failed");
    expect(eventSummary({ seq: 1, kind: "repudiated", targetMoteId: id })).toContain("repudiated");
    expect(eventSummary({ seq: 1, kind: "effect_staged", moteId: id })).toContain("effect");
  });

  it("a failed row appends the FailureReason label when the wire carried one", () => {
    const s = eventSummary({ seq: 1, kind: "failed", moteId: id, reasonClass: 0 });
    expect(s).toContain("failed");
    expect(s).toContain("TIMED OUT");
  });

  it("a failed row with no reason class shows NO fabricated reason", () => {
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id })).not.toContain("—");
    expect(eventSummary({ seq: 1, kind: "failed", moteId: id, reasonClass: null })).not.toContain(
      "—",
    );
  });

  it("unknown kind falls back to a generic summary", () => {
    expect(eventSummary({ seq: 1, kind: "weird" })).toContain("weird");
  });
});

describe("failureReasonLabel", () => {
  it("maps every journal FailureReason discriminant (0-8)", () => {
    expect(failureReasonLabel(0)).toBe("TIMED OUT");
    expect(failureReasonLabel(1)).toBe("EXECUTOR REFUSED");
    expect(failureReasonLabel(2)).toBe("VALIDATOR REJECTED");
    expect(failureReasonLabel(3)).toBe("WORKER CRASHED");
    expect(failureReasonLabel(4)).toBe("UPSTREAM REPUDIATED");
    expect(failureReasonLabel(5)).toBe("UNSAFE WORLD-MUTATING");
    expect(failureReasonLabel(6)).toBe("COMPENSATED");
    expect(failureReasonLabel(7)).toBe("QUARANTINED");
    expect(failureReasonLabel(8)).toBe("DEAD-LETTERED");
  });

  it("null/undefined → null (no fabricated cause); unknown discriminant → UNKNOWN", () => {
    expect(failureReasonLabel(null)).toBeNull();
    expect(failureReasonLabel(undefined)).toBeNull();
    expect(failureReasonLabel(99)).toBe("UNKNOWN REASON");
  });

  it("omitReason drops the trailing reason (the row renders a badge instead)", () => {
    const s = eventSummary(
      { seq: 1, kind: "failed", moteId: "ab".repeat(32), reasonClass: 0 },
      undefined,
      false,
      true,
    );
    expect(s).toContain("failed");
    expect(s).not.toContain("TIMED OUT");
    expect(s).not.toContain("—");
  });
});

describe("tallyEventsByKind (W1a-3)", () => {
  it("counts deltas per kind; unrecognized kinds bucket to unknown", () => {
    const events: EventLike[] = [
      { seq: 1, kind: "committed" },
      { seq: 2, kind: "committed" },
      { seq: 3, kind: "failed" },
      { seq: 4, kind: "future_kind" },
    ];
    const tally = tallyEventsByKind(events);
    expect(tally.committed).toBe(2);
    expect(tally.failed).toBe(1);
    expect(tally.unknown).toBe(1);
    expect(tally.repudiated).toBeUndefined();
  });

  it("an empty buffer tallies to an empty object (no fabricated counts)", () => {
    expect(tallyEventsByKind([])).toEqual({});
  });
});

describe("matchesFeedFilter (W1a-3)", () => {
  const committed: EventLike = {
    seq: 1,
    kind: "committed",
    instanceId: "ab".repeat(16),
    moteId: "cd".repeat(32),
  };
  const failed: EventLike = { seq: 2, kind: "failed", moteId: "ef".repeat(32), reasonClass: 0 };

  it("null kind set + empty query shows everything (the default)", () => {
    expect(matchesFeedFilter(committed, { kinds: null, query: "" })).toBe(true);
    expect(matchesFeedFilter(failed, { kinds: null, query: "" })).toBe(true);
  });

  it("a kind set hides disabled kinds", () => {
    const onlyFailed = { kinds: new Set(["failed"]), query: "" };
    expect(matchesFeedFilter(committed, onlyFailed)).toBe(false);
    expect(matchesFeedFilter(failed, onlyFailed)).toBe(true);
  });

  it("free-text matches instance-hex, mote-hex, and the reason label", () => {
    expect(matchesFeedFilter(committed, { kinds: null, query: "abab" })).toBe(true);
    expect(matchesFeedFilter(committed, { kinds: null, query: "cdcd" })).toBe(true);
    // The failed row's summary carries "TIMED OUT" — a free-text search finds it.
    expect(matchesFeedFilter(failed, { kinds: null, query: "timed out" })).toBe(true);
    expect(matchesFeedFilter(committed, { kinds: null, query: "nomatch" })).toBe(false);
  });
});

describe("feedToNdjson / exportFeedFilename (W1a-3)", () => {
  it("emits one server-derived object per line matching the CLI shape; no payloads", () => {
    const deltas: EventLike[] = [
      {
        seq: 5,
        kind: "committed",
        instanceId: "ab".repeat(16),
        moteId: "cd".repeat(32),
        resultRef: "ef".repeat(32),
        ndClass: 1,
      },
      {
        seq: 6,
        kind: "failed",
        instanceId: "ab".repeat(16),
        moteId: "12".repeat(32),
        reasonClass: 3,
      },
    ];
    const lines = feedToNdjson(deltas).split("\n");
    expect(lines).toHaveLength(2);
    const first = JSON.parse(lines[0] ?? "");
    expect(first).toEqual({
      seq: 5,
      instance_id: "ab".repeat(16),
      type: "committed",
      mote_id: "cd".repeat(32),
      result_ref: "ef".repeat(32),
      // nd_class is the lowercase STRING tag (byte-identical to `kx events --json`),
      // NOT the numeric discriminant — the tri-surface parity contract.
      nd_class: "pure",
    });
    // SN-8: never a payload/secret field — only hex join keys.
    expect(Object.keys(first)).not.toContain("content");
    const second = JSON.parse(lines[1] ?? "");
    expect(second.type).toBe("failed");
    expect(second.reason_class).toBe(3);
  });

  it("maps every nd_class discriminant to its wire string tag (CLI parity)", () => {
    const wire = (ndClass: number | null) =>
      JSON.parse(feedToNdjson([{ seq: 1, kind: "committed", moteId: "ab".repeat(32), ndClass }]))
        .nd_class;
    expect(wire(1)).toBe("pure");
    expect(wire(2)).toBe("read_only_nondet");
    expect(wire(3)).toBe("world_mutating");
    expect(wire(0)).toBe("unspecified");
    expect(wire(null)).toBe("unspecified"); // absent ⇒ honest "unspecified", never fabricated
  });

  it("an empty feed serializes to an empty string", () => {
    expect(feedToNdjson([])).toBe("");
  });

  it("the export filename is a stable, slugged .ndjson name", () => {
    expect(exportFeedFilename(1717)).toBe("kortecx-feed-1717.ndjson");
  });
});
