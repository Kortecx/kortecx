/** Pure unit tests — no server. Hex, errors, type views, args encoding, Result. */

import { create } from "@bufbuild/protobuf";
import { Code, ConnectError } from "@connectrpc/connect";
import { describe, expect, it, vi } from "vitest";
import {
  ErrorCode,
  KxCatchupRequired,
  KxPermissionDenied,
  KxRunFailed,
  KxUnauthenticated,
  KxUsage,
  KxWaitTimeout,
  fromRpcError,
} from "../src/errors.js";
import {
  CommittedDeltaSchema,
  EventDeltaSchema,
  FailedDeltaSchema,
  MoteSnapshotSchema,
  MoteSnapshotState,
  ProjectionViewSchema,
  ReactTurnSummarySchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { INSTANCE_LEN, REF_LEN, asBytes, decode, encode } from "../src/hexids.js";
import { KxClient } from "../src/node.js";
import { ReactTurn } from "../src/react.js";
import { Result } from "../src/run.js";
import {
  encodeArgs,
  isNonloopbackPlaintext,
  normalizeBaseUrl,
  warnIfPlaintext,
} from "../src/transport.js";
import { Delta, MoteView, Projection, isCommitted, isPending, stateName } from "../src/types.js";
import { type WaitOutcome, WaitState, pollReactResult } from "../src/wait.js";

const fill = (v: number, n: number): Uint8Array => new Uint8Array(n).fill(v);
const dec = (b: Uint8Array): string => new TextDecoder().decode(b);

// --- hex (SN-8 safe: only encode/decode, never derive) -----------------------

describe("hexids", () => {
  it("round-trips and validates lengths", () => {
    expect(encode(new Uint8Array([0x00, 0xab]))).toBe("00ab");
    expect(decode("00AB")).toEqual(new Uint8Array([0x00, 0xab]));
    expect(asBytes("ab".repeat(16), INSTANCE_LEN)).toEqual(fill(0xab, 16));
    expect(asBytes("cd".repeat(32), REF_LEN)).toEqual(fill(0xcd, 32));
  });

  it("rejects bad input", () => {
    expect(() => decode("zz")).toThrow(KxUsage);
    expect(() => asBytes("ab".repeat(8), INSTANCE_LEN)).toThrow(KxUsage); // 8 bytes, not 16
    expect(() => asBytes("cd".repeat(16), REF_LEN)).toThrow(KxUsage); // 16 bytes, not 32
  });

  it("asBytes accepts hex or raw bytes", () => {
    expect(asBytes("ab".repeat(16), 16)).toEqual(fill(0xab, 16));
    expect(asBytes(fill(0x01, 32), 32)).toEqual(fill(0x01, 32));
    expect(() => asBytes(fill(0x01, 4), 32)).toThrow(KxUsage);
  });
});

// --- errors -------------------------------------------------------------------

describe("errors", () => {
  it.each([
    [Code.Unauthenticated, KxUnauthenticated, ErrorCode.Unauthenticated, "UNAUTHENTICATED"],
    [Code.PermissionDenied, KxPermissionDenied, ErrorCode.PermissionDenied, "PERMISSION_DENIED"],
    [Code.ResourceExhausted, KxCatchupRequired, ErrorCode.CatchupRequired, "RESOURCE_EXHAUSTED"],
  ] as const)("fromRpcError maps %s", (code, cls, errCode, grpcName) => {
    const err = fromRpcError(new ConnectError("boom", code));
    expect(err).toBeInstanceOf(cls);
    expect(err.code).toBe(errCode);
    expect(String(err)).toContain("boom");
    expect(err.grpcCode).toBe(grpcName);
  });

  it("carries structured fields", () => {
    expect(new KxCatchupRequired("x", { nextSeq: 7n }).nextSeq).toBe(7n);
    const e = new KxWaitTimeout("t", { instanceId: "aa", terminalMoteId: "bb" });
    expect(e.instanceId).toBe("aa");
    expect(e.terminalMoteId).toBe("bb");
    expect(new KxRunFailed("f").code).toBe(ErrorCode.RunFailed);
  });
});

// --- type views ---------------------------------------------------------------

describe("type views", () => {
  it("stateName + predicates", () => {
    expect(stateName(MoteSnapshotState.COMMITTED)).toBe("COMMITTED");
    expect(stateName(999)).toBe("UNKNOWN");
    expect(isCommitted(MoteSnapshotState.COMMITTED)).toBe(true);
    expect(isPending(MoteSnapshotState.SCHEDULED)).toBe(true);
    expect(isPending(MoteSnapshotState.COMMITTED)).toBe(false);
  });

  it("mote + projection views", () => {
    const snap = create(MoteSnapshotSchema, {
      moteId: fill(0x03, 32),
      state: MoteSnapshotState.COMMITTED,
      ndClass: 1,
      promotion: 1,
      resultRef: fill(0x04, 32),
      moteDefHash: fill(0x05, 32),
      committedSeq: 7n,
    });
    const view = create(ProjectionViewSchema, {
      instanceId: fill(0x01, 16),
      recipeFingerprint: fill(0x02, 32),
      currentSeq: 7n,
      motes: [snap],
    });
    const proj = Projection.fromProto(view);
    expect(proj.currentSeq).toBe(7);
    expect(proj.motes[0]?.state).toBe("COMMITTED");
    expect(proj.motes[0]?.resultRef).toBe("04".repeat(32));
    expect(proj.committed.length).toBe(1);
    expect(proj.mote("03".repeat(32))).not.toBeNull();
    const d = proj.toJSON() as { motes: Record<string, unknown>[] };
    expect(d.motes[0]?.state).toBe("COMMITTED");
    expect(d.motes[0]?.committed_seq).toBe(7);
  });

  it("optional result_ref absent", () => {
    const snap = create(MoteSnapshotSchema, {
      moteId: fill(0x03, 32),
      state: MoteSnapshotState.SCHEDULED,
      moteDefHash: fill(0x05, 32),
    });
    const mv = MoteView.fromProto(snap);
    expect(mv.resultRef).toBeNull();
    expect(mv.committedSeq).toBeNull();
  });

  it("delta views cover the oneof", () => {
    const committed = create(EventDeltaSchema, {
      seq: 5n,
      kind: {
        case: "committed",
        value: create(CommittedDeltaSchema, {
          moteId: fill(0x07, 32),
          resultRef: fill(0x08, 32),
          ndClass: 1,
        }),
      },
    });
    const dv = Delta.fromProto(committed);
    expect(dv?.kind).toBe("committed");
    expect(dv?.moteId).toBe("07".repeat(32));
    expect(dv?.toJSON().seq).toBe(5);

    const failed = create(EventDeltaSchema, {
      seq: 6n,
      kind: {
        case: "failed",
        value: create(FailedDeltaSchema, { moteId: fill(0x09, 32), reasonClass: 3 }),
      },
    });
    expect(Delta.fromProto(failed)?.reasonClass).toBe(3);

    expect(Delta.fromProto(create(EventDeltaSchema, { seq: 1n }))).toBeNull(); // no kind → skipped
  });
});

// --- args encoding + credential resolution ------------------------------------

describe("args + credentials", () => {
  it("encodeArgs variants", () => {
    expect(dec(encodeArgs({ topic: "x" }))).toBe('{"topic":"x"}');
    expect(dec(encodeArgs('{"a":1}'))).toBe('{"a":1}');
    expect(dec(encodeArgs(new TextEncoder().encode('{"a":1}')))).toBe('{"a":1}');
    expect(() => encodeArgs("{not json")).toThrow(KxUsage);
    // @ts-expect-error: a number is not a valid args type
    expect(() => encodeArgs(123)).toThrow(KxUsage);
  });

  it("non-loopback plaintext detection", () => {
    expect(isNonloopbackPlaintext("http://example.com:50151")).toBe(true);
    expect(isNonloopbackPlaintext("http://10.0.0.5:50151")).toBe(true);
    expect(isNonloopbackPlaintext("http://127.0.0.1:50151")).toBe(false);
    expect(isNonloopbackPlaintext("http://localhost:50151")).toBe(false);
    expect(isNonloopbackPlaintext("http://[::1]:50151")).toBe(false);
    expect(isNonloopbackPlaintext("https://example.com")).toBe(false);
  });

  it("normalizeBaseUrl adds a scheme + strips a trailing slash", () => {
    expect(normalizeBaseUrl("http://127.0.0.1:50151/")).toBe("http://127.0.0.1:50151");
    expect(normalizeBaseUrl("127.0.0.1:50151")).toBe("http://127.0.0.1:50151");
    expect(normalizeBaseUrl("https://h:1")).toBe("https://h:1");
  });

  it("warns on a token over non-loopback plaintext", () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    warnIfPlaintext("http://example.com:50151", "t");
    expect(warn).toHaveBeenCalledOnce();
    warn.mockClear();
    warnIfPlaintext("http://127.0.0.1:50151", "t"); // loopback: no warning
    expect(warn).not.toHaveBeenCalled();
    warn.mockRestore();
  });

  it("token and tokenFile are mutually exclusive", () => {
    expect(() => new KxClient("http://127.0.0.1:1", { token: "t", tokenFile: "/x" })).toThrow(
      KxUsage,
    );
  });
});

// --- Result shape (parity with the CLI render_wait / Python to_dict) ----------

describe("Result", () => {
  it("committed → toJSON", () => {
    const o: WaitOutcome = {
      instanceId: fill(0x01, 16),
      terminalMoteId: fill(0x02, 32),
      state: "COMMITTED",
      resultRef: fill(0x03, 32),
      payload: new TextEncoder().encode("hello"),
    };
    const r = Result.fromOutcome(o);
    expect(r.ok).toBe(true);
    expect(r.text).toBe("hello");
    const d = r.toJSON();
    expect(d.state).toBe("COMMITTED");
    expect(d.result_utf8).toBe("hello");
    expect(d.result_len).toBe(5);
    expect(d.result_hex).toBe(encode(new TextEncoder().encode("hello")));
    const meta = r.toJSON(false);
    expect(meta.result_hex).toBeUndefined();
    expect(meta.result_len).toBe(5);
  });

  it("running → flags a timeout", () => {
    const o: WaitOutcome = {
      instanceId: fill(0x01, 16),
      terminalMoteId: fill(0x02, 32),
      state: "RUNNING",
    };
    const r = Result.fromOutcome(o);
    expect(r.timedOut).toBe(true);
    expect(r.toJSON().timed_out).toBe(true);
  });

  it("binary payload has no utf8", () => {
    const o: WaitOutcome = {
      instanceId: fill(0x01, 16),
      terminalMoteId: fill(0x02, 32),
      state: "COMMITTED",
      resultRef: fill(0x03, 32),
      payload: new Uint8Array([0xff, 0xfe, 0x00]),
    };
    const r = Result.fromOutcome(o);
    expect(r.text).toBeNull();
    expect(r.toJSON().result_utf8).toBeUndefined();
  });
});

// --- F13: react invoke(wait) settles via ListReactTurns ----------------------

describe("pollReactResult (F13 — react wait via ListReactTurns)", () => {
  // A minimal fake gateway: ListReactTurns drives settlement, GetProjection
  // resolves the settled turn's resultRef, GetContent its bytes. Models F13 — the
  // returned terminal_mote_id (seed) never commits; the run-salted answer turn does.
  const fakeGateway = (
    turns: Array<{ branch: string; turnMoteId: Uint8Array }>,
    ans?: {
      moteId: Uint8Array;
      resultRef: Uint8Array;
      payload: Uint8Array;
    },
  ) =>
    ({
      listReactTurns: (_req: unknown) => Promise.resolve({ turns, hasMore: false }),
      getProjection: (_req: unknown) =>
        Promise.resolve({
          motes: ans
            ? [{ moteId: ans.moteId, state: MoteSnapshotState.COMMITTED, resultRef: ans.resultRef }]
            : [],
        }),
      getContent: (_req: unknown) =>
        Promise.resolve({ payload: ans?.payload ?? new Uint8Array(0) }),
    }) as unknown as Parameters<typeof pollReactResult>[0];

  it("settles COMMITTED on an answer branch, resolving the run-salted turn's content", async () => {
    const seed = fill(0x99, 32); // gateway's returned terminal — never commits
    const answer = fill(0x42, 32); // the run-salted settled answer turn
    const gw = fakeGateway(
      [
        { branch: "answer", turnMoteId: answer },
        { branch: "pending", turnMoteId: answer },
      ],
      { moteId: answer, resultRef: fill(0x03, 32), payload: new TextEncoder().encode("final") },
    );
    const out = await pollReactResult(gw, fill(0x07, 16), seed, 5_000);
    expect(out.state).toBe(WaitState.Committed);
    expect(encode(out.terminalMoteId)).toBe(encode(answer)); // NOT the seed
    expect(dec(out.payload as Uint8Array)).toBe("final");
  });

  it("unwraps the Ollama union answer arm to plain text (gemma3 connector-tool-fire)", async () => {
    // Under the Ollama union `format` a settled answer commits {"answer":"…"}; the wait
    // unwraps it (parity with the server-side finalize_agentic_launch, the CLI, and Python).
    const seed = fill(0x99, 32);
    const answer = fill(0x42, 32);
    const gw = fakeGateway([{ branch: "answer", turnMoteId: answer }], {
      moteId: answer,
      resultRef: fill(0x03, 32),
      payload: new TextEncoder().encode('{"answer": "photosynthesis converts light to energy"}'),
    });
    const out = await pollReactResult(gw, fill(0x07, 16), seed, 5_000);
    expect(dec(out.payload as Uint8Array)).toBe("photosynthesis converts light to energy");
  });

  it("leaves prose / tool_call / stray-field bodies untouched (union unwrap is exact)", async () => {
    const seed = fill(0x99, 32);
    const answer = fill(0x42, 32);
    for (const raw of [
      "just prose",
      '{"tool_call":{"name":"x","version":"1","args":{}}}',
      '{"answer":"x","note":"y"}',
    ]) {
      const gw = fakeGateway([{ branch: "answer", turnMoteId: answer }], {
        moteId: answer,
        resultRef: fill(0x03, 32),
        payload: new TextEncoder().encode(raw),
      });
      const out = await pollReactResult(gw, fill(0x07, 16), seed, 5_000);
      expect(dec(out.payload as Uint8Array)).toBe(raw);
    }
  });

  it("ReactTurn.fromProto carries the rejection reason (PR-3/A2)", () => {
    const t = ReactTurn.fromProto(
      create(ReactTurnSummarySchema, {
        turn: 1,
        branch: "rejected",
        rejectionReason: "args do not match inputSchema",
        maxTurns: 8,
        maxToolCalls: 6,
      }),
    );
    expect(t.branch).toBe("rejected");
    expect(t.rejectionReason).toBe("args do not match inputSchema");
    expect(t.toJSON().rejection_reason).toBe("args do not match inputSchema");
  });

  it("settles FAILED on a dead_lettered branch", async () => {
    const dead = fill(0x55, 32);
    const gw = fakeGateway([{ branch: "dead_lettered", turnMoteId: dead }]);
    const out = await pollReactResult(gw, fill(0x07, 16), fill(0x99, 32), 5_000);
    expect(out.state).toBe(WaitState.Failed);
    expect(encode(out.terminalMoteId)).toBe(encode(dead));
  });

  it("returns RUNNING (resumable) while only pending — no false commit", async () => {
    const seed = fill(0x99, 32);
    const gw = fakeGateway([{ branch: "pending", turnMoteId: seed }]);
    const out = await pollReactResult(gw, fill(0x07, 16), seed, 10);
    expect(out.state).toBe(WaitState.Running);
  });
});
