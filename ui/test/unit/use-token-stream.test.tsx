/**
 * The token stream's DROP signal.
 *
 * `dropped` used to be set only when the iterator threw, so a stream that died via a
 * CLEAN close — the gateway refusing the subscription and closing the socket rather than
 * erroring — left `dropped` false and rendered as a silently empty pane. That is how the
 * run-ownership gate's per-file revocation hid itself: text stopped mid-word, nothing said
 * why, and the pane looked merely slow.
 *
 * A healthy stream always ends with a terminal chunk (`tokens.ts`: "`done` marks the
 * terminal chunk; the stream ends after it"), so exhausting the iterator WITHOUT one is by
 * construction an abnormal end. These pin both directions — a completed stream must not
 * report a drop, or the signal is noise and no one will trust it.
 */

import { renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";

const wsTokensImpl = vi.fn();

// The connection value must be REFERENTIALLY STABLE. `useTokenStream`'s effect lists
// `client` in its deps, so a mock that builds a fresh object per render re-subscribes on
// every commit and the hook never settles — the test then hangs on `streaming` rather than
// failing on the assertion it is actually about.
const conn = {
  client: { wsTokens: (...a: unknown[]) => wsTokensImpl(...a) },
  endpoint: "http://127.0.0.1:50151",
  status: "connected",
};

vi.mock("../../src/kx/connection-context", () => ({
  useConnection: () => conn,
}));

import { useTokenStream } from "../../src/kx/use-token-stream";

/** A stream that yields `chunks` and then simply ends (generator return). */
function streamOf(chunks: { text: string; done: boolean }[]) {
  return async function* () {
    for (const c of chunks) {
      yield c;
    }
  };
}

afterEach(() => {
  vi.restoreAllMocks();
  wsTokensImpl.mockReset();
});

it("a stream that ends WITHOUT a terminal chunk reports dropped", async () => {
  // The gate-revocation shape: two pieces arrive, then the socket closes cleanly with no
  // `done` frame. Before the fix this left `dropped === false` — an empty-looking pane
  // that claimed nothing was wrong.
  wsTokensImpl.mockImplementation(
    streamOf([
      { text: "Hel", done: false },
      { text: "lo", done: false },
    ]),
  );

  const { result } = renderHook(() => useTokenStream("aa", "bb", true));

  await waitFor(() => expect(result.current.streaming).toBe(false));
  expect(result.current.text).toBe("Hello");
  expect(result.current.dropped).toBe(true);
});

it("a stream that ends WITH a terminal chunk does not report dropped", async () => {
  // The other direction, and the one that makes the signal worth having: a normal
  // completion must stay quiet. If this ever flips, `dropped` is noise.
  wsTokensImpl.mockImplementation(
    streamOf([
      { text: "Hel", done: false },
      { text: "lo", done: true },
    ]),
  );

  const { result } = renderHook(() => useTokenStream("aa", "bb", true));

  await waitFor(() => expect(result.current.streaming).toBe(false));
  expect(result.current.text).toBe("Hello");
  expect(result.current.dropped).toBe(false);
});

it("a synchronous open failure reports dropped without streaming", async () => {
  // An old gateway / broker-unwired serve: degrade advisorily, never throw into render.
  wsTokensImpl.mockImplementation(() => {
    throw new Error("no token surface");
  });

  const { result } = renderHook(() => useTokenStream("aa", "bb", true));

  await waitFor(() => expect(result.current.dropped).toBe(true));
  expect(result.current.streaming).toBe(false);
  expect(result.current.text).toBe("");
});
