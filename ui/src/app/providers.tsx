import { QueryClientProvider } from "@tanstack/react-query";
import { LazyMotion, MotionConfig } from "framer-motion";
import { type ReactNode, useState } from "react";
import { type ClientFactory, KxConnectionProvider } from "../kx/connection-context";
import { makeQueryClient } from "./query-client";

/** Load the animation engine as a dynamic chunk (see `motion-features.ts`). */
const loadMotionFeatures = () => import("./motion-features").then((m) => m.domAnimation);

export interface AppProvidersProps {
  children: ReactNode;
  /** Injectable client builder (tests pass a node-client or mock factory). */
  createClient?: ClientFactory;
}

/**
 * Composes the server-state, connection, and motion providers. `LazyMotion strict`
 * is the bundle-regression guard: any `motion.*` usage (which would drag the full
 * eager animation engine back in) THROWS — components must use `m.*`.
 */
export function AppProviders({ children, createClient }: AppProvidersProps) {
  const [queryClient] = useState(() => makeQueryClient());
  return (
    <QueryClientProvider client={queryClient}>
      <KxConnectionProvider createClient={createClient}>
        <MotionConfig reducedMotion="user">
          <LazyMotion strict features={loadMotionFeatures}>
            {children}
          </LazyMotion>
        </MotionConfig>
      </KxConnectionProvider>
    </QueryClientProvider>
  );
}
