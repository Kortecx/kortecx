import { Outlet, useRouterState } from "@tanstack/react-router";
import { AnimatePresence, m } from "framer-motion";
import { Suspense, useCallback, useEffect, useState } from "react";
import { pageFade } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";
import { loadFlag, persistFlag } from "../../lib/ui-flags";
import { DevToolsDock } from "../devtools";
import { ActivityDrawer } from "./ActivityDrawer";
import { Brand } from "./Brand";
import { CommandPalette } from "./CommandPalette";
import { ConnectionStatus } from "./ConnectionStatus";
import { Navbar } from "./Navbar";
import { Sidebar } from "./Sidebar";

const SIDEBAR_KEY = "kortecx.ui.sidebar";
const DEVTOOLS_KEY = "kortecx.ui.devtools";

/** The animated route outlet (shared by the gate and the full shell). */
function RouteOutlet() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  return (
    <AnimatePresence mode="wait">
      <m.div
        key={pathname}
        initial={pageFade.initial}
        animate={pageFade.animate}
        exit={pageFade.exit}
        transition={pageFade.transition}
      >
        <Outlet />
      </m.div>
    </AnimatePresence>
  );
}

/**
 * The application shell. Connect is a LOGIN GATE (D137): until the console is
 * connected to a gateway there is no navbar/sidebar — just the centered route
 * outlet (the connect screen, or a section's own ConnectGate for deep links).
 * Once connected: top navbar + collapsible left sidebar (state persisted) +
 * the ⌘K command palette + the animated outlet.
 */
export function AppShell() {
  const { status } = useConnection();
  const [collapsed, setCollapsed] = useState<boolean>(() => loadFlag(SIDEBAR_KEY));
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [activityOpen, setActivityOpen] = useState(false);
  const [devtoolsOpen, setDevtoolsOpen] = useState<boolean>(() => loadFlag(DEVTOOLS_KEY));
  const connected = status === "connected";

  const toggle = useCallback(() => {
    setCollapsed((c) => {
      const next = !c;
      persistFlag(SIDEBAR_KEY, next);
      return next;
    });
  }, []);

  const toggleDevtools = useCallback(() => {
    setDevtoolsOpen((open) => {
      const next = !open;
      persistFlag(DEVTOOLS_KEY, next);
      return next;
    });
  }, []);

  const openPalette = useCallback(() => setPaletteOpen(true), []);
  const closePalette = useCallback(() => setPaletteOpen(false), []);
  const toggleActivity = useCallback(() => setActivityOpen((open) => !open), []);
  const closeActivity = useCallback(() => setActivityOpen(false), []);

  // Global ⌘K / Ctrl+K — only meaningful once the nav exists (connected).
  useEffect(() => {
    if (!connected) {
      return;
    }
    function onKeyDown(e: KeyboardEvent): void {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((open) => !open);
      }
    }
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [connected]);

  if (!connected) {
    return (
      <div className="gate" data-testid="app-gate">
        <a href="#main" className="skip-link">
          Skip to content
        </a>
        <header className="gate__head">
          <Brand />
          <div className="navbar__spacer" />
          <ConnectionStatus />
        </header>
        <main className="gate__main" id="main">
          <RouteOutlet />
        </main>
      </div>
    );
  }

  return (
    <div
      className={`shell${collapsed ? " shell--collapsed" : ""}${devtoolsOpen ? " shell--dock-open" : ""}`}
      data-testid="app-shell"
    >
      <a href="#main" className="skip-link">
        Skip to content
      </a>
      <Navbar
        onOpenPalette={openPalette}
        onToggleActivity={toggleActivity}
        onToggleDevtools={toggleDevtools}
      />
      <Sidebar collapsed={collapsed} onToggle={toggle} />
      <main className="shell__main" id="main">
        <RouteOutlet />
      </main>
      <CommandPalette open={paletteOpen} onClose={closePalette} />
      <ActivityDrawer open={activityOpen} onClose={closeActivity} />
      {devtoolsOpen ? (
        <Suspense fallback={null}>
          <DevToolsDock onClose={toggleDevtools} />
        </Suspense>
      ) : null}
    </div>
  );
}
