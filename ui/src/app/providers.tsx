import { QueryClientProvider } from "@tanstack/react-query";
import { MotionConfig } from "framer-motion";
import { type ReactNode, useState } from "react";
import { type ClientFactory, KxConnectionProvider } from "../kx/connection-context";
import { makeQueryClient } from "./query-client";

export interface AppProvidersProps {
  children: ReactNode;
  /** Injectable client builder (tests pass a node-client or mock factory). */
  createClient?: ClientFactory;
}

/** Composes the server-state, connection, and motion providers. */
export function AppProviders({ children, createClient }: AppProvidersProps) {
  const [queryClient] = useState(() => makeQueryClient());
  return (
    <QueryClientProvider client={queryClient}>
      <KxConnectionProvider createClient={createClient}>
        <MotionConfig reducedMotion="user">{children}</MotionConfig>
      </KxConnectionProvider>
    </QueryClientProvider>
  );
}
