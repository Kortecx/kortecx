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
}

export interface ReactProgress {
  /** All turns, ascending by turn number (newest fact wins per turn). */
  readonly turns: ReactTurnVM[];
  /** The settled terminal turn (`answer` / `dead_lettered`), once one exists. */
  readonly terminal: ReactTurnVM | null;
}

function toProgress(turns: ReactTurnVM[]): ReactProgress {
  // The wire is newest-first and a turn's branch is re-announced as it settles
  // (pending → answer/tool); keep the NEWEST fact per turn number.
  const byTurn = new Map<number, ReactTurnVM>();
  for (const t of turns) {
    if (!byTurn.has(t.turn)) {
      byTurn.set(t.turn, t);
    }
  }
  const ordered = [...byTurn.values()].sort((a, b) => a.turn - b.turn);
  const terminal =
    ordered.find((t) => t.branch === "answer" || t.branch === "dead_lettered") ?? null;
  return { turns: ordered, terminal };
}

/** Poll the chain for `instanceId` (undefined ⇒ idle). Stops on terminal. */
export function useReactProgress(instanceId: string | undefined) {
  const { client, endpoint, status } = useConnection();
  const query = useQuery({
    queryKey: queryKeys.reactTurns(endpoint, instanceId, PAGE),
    enabled: status === "connected" && client !== null && Boolean(instanceId),
    queryFn: async (): Promise<ReactProgress> => {
      if (!client || !instanceId) {
        throw new Error("not connected");
      }
      const page = await client.listReactTurns({ instanceId, limit: PAGE });
      return toProgress(
        page.turns.map((t) => ({
          turn: t.turn,
          branch: t.branch,
          toolId: t.toolId,
          toolVersion: t.toolVersion,
          turnMoteId: t.turnMoteId,
          maxTurns: t.maxTurns,
          rejectionReason: t.rejectionReason,
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
