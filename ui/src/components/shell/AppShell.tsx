import { Outlet, useRouterState } from "@tanstack/react-router";
import { AnimatePresence, motion } from "framer-motion";
import { useCallback, useState } from "react";
import { pageFade } from "../../app/motion";
import { Navbar } from "./Navbar";
import { Sidebar } from "./Sidebar";

const SIDEBAR_KEY = "kortecx.ui.sidebar";

function loadCollapsed(): boolean {
  try {
    return localStorage.getItem(SIDEBAR_KEY) === "1";
  } catch {
    return false;
  }
}

function persistCollapsed(collapsed: boolean): void {
  try {
    localStorage.setItem(SIDEBAR_KEY, collapsed ? "1" : "0");
  } catch {
    /* best-effort */
  }
}

/**
 * The application shell: a top navbar + a collapsible left sidebar + the animated
 * route outlet. The collapse state persists (localStorage). Route transitions keep
 * the established `pageFade` (honoring reduced-motion via the global MotionConfig).
 */
export function AppShell() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const [collapsed, setCollapsed] = useState<boolean>(() => loadCollapsed());

  const toggle = useCallback(() => {
    setCollapsed((c) => {
      const next = !c;
      persistCollapsed(next);
      return next;
    });
  }, []);

  return (
    <div className={collapsed ? "shell shell--collapsed" : "shell"} data-testid="app-shell">
      <a href="#main" className="skip-link">
        Skip to content
      </a>
      <Navbar onToggleSidebar={toggle} />
      <Sidebar collapsed={collapsed} />
      <main className="shell__main" id="main">
        <AnimatePresence mode="wait">
          <motion.div
            key={pathname}
            initial={pageFade.initial}
            animate={pageFade.animate}
            exit={pageFade.exit}
            transition={pageFade.transition}
          >
            <Outlet />
          </motion.div>
        </AnimatePresence>
      </main>
    </div>
  );
}
