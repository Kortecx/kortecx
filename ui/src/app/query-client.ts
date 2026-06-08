/**
 * The TanStack Query client. Retry policy is keyed on the stable error kind: never
 * retry auth/permission/not-found/not-wired/bad-input (retrying cannot help and
 * would hammer the gateway); retry transient transport errors with capped backoff.
 */

import { QueryClient } from "@tanstack/react-query";
import { toUiError } from "../kx/errors";

export function makeQueryClient(): QueryClient {
  return new QueryClient({
    defaultOptions: {
      queries: {
        retry: (failureCount, error) => {
          const ui = toUiError(error);
          if (!ui.retryable) {
            return false;
          }
          return failureCount < 3;
        },
        retryDelay: (attempt) => Math.min(1000 * 2 ** attempt, 8000),
        refetchOnWindowFocus: false,
        staleTime: 0,
      },
      mutations: { retry: false },
    },
  });
}
