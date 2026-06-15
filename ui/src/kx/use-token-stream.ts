/**
 * Live token stream for ONE model mote over the WebSocket bridge (`wsTokens`,
 * PR-4.2 / T-STREAM1). A browser cannot speak gRPC server-streaming, so the WS
 * bridge is the only in-browser live token surface (default :50152).
 *
 * The pieces accumulate into ONE growing string (this is a single assistant
 * message, not a ring of events). The stream is ADVISORY + out-of-band: the
 * committed result (fetched on commit by `use-chat`) is the authority and
 * overwrites this. It degrades silently — an old gateway, a broker-unwired serve,
 * or a token-less (non-model) mote yields `dropped`/no text, and the chat poll
 * path still finalizes the turn. On unmount/mote-change we call the iterator's
 * `.return()` so the generator's `finally` closes the socket immediately.
 */

import { useEffect, useState } from "react";
import { useConnection } from "./connection-context";

export interface TokenStreamState {
  /** The accumulated streamed text (pieces concatenated in order). */
  readonly text: string;
  /** Actively receiving tokens for the current mote. */
  readonly streaming: boolean;
  /** The stream ended early / errored (advisory — the poll path still finalizes). */
  readonly dropped: boolean;
}

export function useTokenStream(
  instanceId: string | undefined,
  moteId: string | undefined,
  enabled: boolean,
): TokenStreamState {
  const { client, status } = useConnection();
  const [text, setText] = useState("");
  const [streaming, setStreaming] = useState(false);
  const [dropped, setDropped] = useState(false);

  useEffect(() => {
    if (!enabled || status !== "connected" || !client || !instanceId || !moteId) {
      // Reset when disabled / between motes so a stale stream never leaks across turns.
      setText("");
      setStreaming(false);
      setDropped(false);
      return;
    }
    let cancelled = false;
    setText("");
    setStreaming(true);
    setDropped(false);

    // Open the stream defensively: a client without the token surface (an old
    // SDK) or a synchronous open failure degrades to no-streaming — the chat poll
    // path still finalizes the turn (advisory, never load-bearing).
    let iterator: AsyncIterator<{ text: string; done: boolean }>;
    try {
      iterator = client.wsTokens(instanceId, moteId)[Symbol.asyncIterator]();
    } catch {
      setStreaming(false);
      setDropped(true);
      return;
    }

    void (async () => {
      try {
        for (;;) {
          const { value, done } = await iterator.next();
          if (done || cancelled) {
            break;
          }
          setText((prev) => prev + value.text);
          if (value.done) {
            break;
          }
        }
      } catch {
        if (!cancelled) {
          setDropped(true);
        }
      } finally {
        if (!cancelled) {
          setStreaming(false);
        }
      }
    })();

    return () => {
      cancelled = true;
      // Resume the suspended generator at its `finally` → close the socket now.
      void iterator.return?.(undefined);
    };
  }, [client, status, instanceId, moteId, enabled]);

  return { text, streaming, dropped };
}
