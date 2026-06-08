/**
 * The run history hook — the FORWARD SEAM for the additive `ListRuns` RPC (UI-2).
 * Today it reads the per-endpoint session history (localStorage, recorded when a
 * run is submitted from the console). When `ListRuns` lands, only this hook's
 * source swaps; the `RunRecord` shape and every consumer (Runs view, RunPicker,
 * metrics fold) stay byte-for-byte the same.
 */

import { useCallback, useEffect, useState } from "react";
import { type RunRecord, clearRuns, loadRuns, recordRun } from "../lib/recent-runs";
import { useConnection } from "./connection-context";

export interface UseRuns {
  readonly runs: RunRecord[];
  add(run: RunRecord): void;
  refresh(): void;
  clear(): void;
}

export function useRuns(): UseRuns {
  const { endpoint } = useConnection();
  const [runs, setRuns] = useState<RunRecord[]>(() => loadRuns(endpoint));

  // Reload when the gateway changes — never mix two endpoints' histories.
  useEffect(() => {
    setRuns(loadRuns(endpoint));
  }, [endpoint]);

  const add = useCallback((run: RunRecord) => setRuns(recordRun(endpoint, run)), [endpoint]);
  const refresh = useCallback(() => setRuns(loadRuns(endpoint)), [endpoint]);
  const clear = useCallback(() => {
    clearRuns(endpoint);
    setRuns([]);
  }, [endpoint]);

  return { runs, add, refresh, clear };
}
