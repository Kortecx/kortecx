import { createRootRoute } from "@tanstack/react-router";
import { AppShell } from "../../components/shell/AppShell";

/**
 * The root route renders the {@link AppShell} (navbar + sidebar + animated outlet).
 * All layout/nav/connection logic lives in the shell components (one concern per
 * file); this stays a thin route declaration.
 */
export const rootRoute = createRootRoute({ component: AppShell });
