/**
 * Shell-level open/close state for the navbar approvals drawer, so the bell (in the
 * Navbar), the drawer (rendered by AppShell), and the `/apps?tab=approvals` deep-link
 * migration all share ONE flag without prop-drilling through the shell.
 */

import { type ReactNode, createContext, useContext, useMemo, useState } from "react";

interface ApprovalsContextValue {
  readonly open: boolean;
  readonly show: () => void;
  readonly close: () => void;
  readonly toggle: () => void;
}

const ApprovalsContext = createContext<ApprovalsContextValue | null>(null);

export function ApprovalsProvider({ children }: { children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const value = useMemo<ApprovalsContextValue>(
    () => ({
      open,
      show: () => setOpen(true),
      close: () => setOpen(false),
      toggle: () => setOpen((o) => !o),
    }),
    [open],
  );
  return <ApprovalsContext.Provider value={value}>{children}</ApprovalsContext.Provider>;
}

/** Access the approvals-drawer open state. Safe outside a provider (returns a no-op). */
export function useApprovalsDrawer(): ApprovalsContextValue {
  return (
    useContext(ApprovalsContext) ?? {
      open: false,
      show: () => {},
      close: () => {},
      toggle: () => {},
    }
  );
}
