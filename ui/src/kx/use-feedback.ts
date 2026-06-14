/**
 * Submit 👍/👎 feedback on an answer (`SubmitFeedback`, PR-4.1) — a client-origin
 * write into the gateway's rebuildable-to-empty `feedback.db` sidecar (advisory
 * product signal; never truth/identity). The caller principal + the feedback id
 * are server-derived (SN-8). A gateway without the seam throws `KxUnimplemented`,
 * which the UI degrades by hiding the control (don't-fake-gaps).
 */

import type { FeedbackInput } from "@kortecx/sdk/web";
import { useMutation } from "@tanstack/react-query";
import { useConnection } from "./connection-context";

export function useFeedback() {
  const { client } = useConnection();
  return useMutation<string, unknown, FeedbackInput>({
    mutationFn: async (input) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.submitFeedback(input);
    },
  });
}
