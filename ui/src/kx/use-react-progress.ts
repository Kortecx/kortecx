/**
 * Live ReAct-chain progress (PR-2.1 agent mode): poll `ListReactTurns` for one
 * run until the chain settles. THE driver for agent turns — never invoke-wait
 * (the clean-install campaign's F13 lesson: the seed Mote is SWAPPED for a
 * run-salted turn 0, and the chain extends turn by turn; only the durable
 * ReactRound facts narrate it). Terminal = an `answer` or `dead_lettered`
 * branch; the answer TEXT then lives at the answer turn's committed
 * `result_ref` (resolved by the caller).
 */

import { useQuery } from "@tanstack/react-query";
import { useConnection } from "./connection-context";
import { queryKeys } from "./query-keys";

const POLL_MS = 1000;
const PAGE = 32; // caps are ≤8 turns + ≤8 tool rounds — one page always covers a chain

export interface ReactTurnVM {
  readonly turn: number;
  /** `"pending" | "answer" | "tool" | "rejected" | "dead_lettered"` (frozen at append). */
  readonly branch: string;
  /** The fired tool (`id@version`) for a `tool` branch; "" otherwise. */
  readonly toolId: string;
  readonly toolVersion: string;
  /** The turn Mote (hex) — the `answer` branch's committed result is the reply. */
  readonly turnMoteId: string;
  readonly maxTurns: number;
  /** PR-3 (A2): the fail-closed reason a `rejected` turn re-prompts over; "" otherwise. */
  readonly rejectionReason: string;
  /** T-MULTI-ELEMENT-TOOLCALLS: when a turn fires N tools at once, the gateway fans it
   *  into N `tool` rows sharing `turn`, distinguished by `callIndex` (0..N-1). 0 for a
   *  single call + every non-tool branch. */
  readonly callIndex: number;
}

export interface ReactProgress {
  /** All turn rows, ordered by `(turn, callIndex)` (newest fact wins per row). A
   *  multi-tool turn contributes several rows (one per call). */
  readonly turns: ReactTurnVM[];
  /** The settled terminal turn (`answer` / `dead_lettered`), once one exists. */
  readonly terminal: ReactTurnVM | null;
}

function toProgress(turns: ReactTurnVM[]): ReactProgress {
  // The wire is newest-first and a turn's branch is re-announced as it settles
  // (pending → answer/tool); a multi-tool turn fans into N `tool` rows sharing the
  // turn number, distinguished by callIndex. Key on `(turn, callIndex)` so all N tool
  // rows survive (NOT just one) and the newest fact per row wins — the settled Tool
  // rows (higher seq, newest-first) supersede the same-turn Pending at callIndex 0.
  const byRow = new Map<string, ReactTurnVM>();
  for (const t of turns) {
    const key = `${t.turn}:${t.callIndex}`;
    if (!byRow.has(key)) {
      byRow.set(key, t);
    }
  }
  const ordered = [...byRow.values()].sort((a, b) => a.turn - b.turn || a.callIndex - b.callIndex);
  const terminal =
    ordered.find((t) => t.branch === "answer" || t.branch === "dead_lettered") ?? null;
  return { turns: ordered, terminal };
}

/** Poll the chain for `instanceId` (undefined ⇒ idle). Stops on terminal. PR-R1:
 *  `chainSalt` scopes to ONE chain on serve's shared journal (one chain per Invoke)
 *  so concurrent/sequential agent turns never mix their reason→act→observe trails. */
export function useReactProgress(instanceId: string | undefined, chainSalt?: string) {
  const { client, endpoint, status } = useConnection();
  const query = useQuery({
    queryKey: queryKeys.reactTurns(endpoint, instanceId, PAGE, chainSalt),
    enabled: status === "connected" && client !== null && Boolean(instanceId),
    queryFn: async (): Promise<ReactProgress> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      const page = await client.listReactTurns({
        instanceId,
        stepSalt: chainSalt || undefined,
        limit: PAGE,
      });
      return toProgress(
        page.turns.map((t) => ({
          turn: t.turn,
          branch: t.branch,
          toolId: t.toolId,
          toolVersion: t.toolVersion,
          turnMoteId: t.turnMoteId,
          maxTurns: t.maxTurns,
          rejectionReason: t.rejectionReason,
          callIndex: t.callIndex,
        })),
      );
    },
    refetchInterval: (q) => (q.state.data?.terminal ? false : POLL_MS),
    refetchIntervalInBackground: false,
  });
  return {
    turns: query.data?.turns ?? [],
    terminal: query.data?.terminal ?? null,
  };
}
