import { Link, Outlet, createRootRoute, useRouterState } from "@tanstack/react-router";
import { AnimatePresence, motion } from "framer-motion";
import { pageFade } from "../../app/motion";
import { useConnection } from "../../kx/connection-context";

function RootLayout() {
  const { status, endpoint, disconnect } = useConnection();
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  return (
    <div className="app">
      <header className="app__nav">
        <span className="app__brand">kortecx</span>
        <nav className="app__links">
          <Link
            to="/runs"
            className="navlink"
            activeProps={{ className: "navlink navlink--active" }}
          >
            Runs
          </Link>
          <Link
            to="/connect"
            className="navlink"
            activeProps={{ className: "navlink navlink--active" }}
          >
            Connect
          </Link>
        </nav>
        <span className="app__conn" data-testid="conn-status" data-status={status}>
          {status === "connected" ? (
            <>
              <span className="dot dot--ok" />
              <span className="mono">{endpoint}</span>
              <button type="button" className="linkbtn" onClick={disconnect}>
                disconnect
              </button>
            </>
          ) : (
            <>
              <span className="dot dot--off" />
              not connected
            </>
          )}
        </span>
      </header>
      <main className="app__main">
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

export const rootRoute = createRootRoute({ component: RootLayout });
