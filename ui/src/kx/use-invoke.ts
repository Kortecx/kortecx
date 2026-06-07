/**
 * Start a run by invoking a published recipe handle with JSON args. Returns the
 * run's hex instance id (no wait) so the caller can route to the live run-detail
 * view and watch it execute. The built-in `kx/recipes/echo` recipe makes this work
 * against a plain `kx serve` with no extra wiring.
 */

import type { Run } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

export interface InvokeVars {
  handle: string;
  args: Record<string, unknown>;
}

export function useInvoke() {
  const { client } = useConnection();
  return useMutation<string, unknown, InvokeVars>({
    mutationFn: async ({ handle, args }) => {
      if (!client) {
        throw new Error("not connected");
      }
      const run = (await client.invoke(handle, args)) as Run;
      return run.instanceId; // hex (16B)
    },
  });
}
