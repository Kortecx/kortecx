/**
 * `DeriveApp` — one prompt in, a reviewable App design out (the single Apps chat surface).
 *
 * The served model decides the workflow, its SHAPE (which steps run in parallel) and the
 * capabilities each step needs; the gateway compiles the design through the vetted planner and
 * intersects every named capability against this caller's own ceiling. It VALIDATES ONLY —
 * nothing is saved, no branch is created, no journal is written. The App comes into existence
 * only when the author approves and the form runs `SaveApp` + `ScaffoldApp`.
 *
 * `{ derived: false }` carries an honest refusal (no served model, an inadmissible workflow)
 * surfaced verbatim, and `notices` on a successful design carries what it did NOT get.
 */

import type { AppDerivation, DeriveAppInput } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

/** Derive a reviewable App design from one prompt. Persists nothing. */
export function useDeriveApp() {
  const { client } = useConnection();
  return useMutation<AppDerivation, unknown, DeriveAppInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.deriveApp({ ...input, prompt: input.prompt.trim() });
    },
  });
}
