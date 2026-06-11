import { createRoute } from "@tanstack/react-router";
import { Suspense, lazy } from "react";
import { ConnectGate } from "../../components/ConnectGate";
import { EmptyState } from "../../components/EmptyState";
import { useConnection } from "../../kx/connection-context";
import { rootRoute } from "./__root";

const SettingsSection = lazy(() =>
  import("../../components/sections/SettingsSection").then((m) => ({
    default: m.SettingsSection,
  })),
);

function SettingsScreen() {
  const { status } = useConnection();
  if (status !== "connected") {
    return <ConnectGate />;
  }
  return (
    <Suspense fallback={<EmptyState title="Loading settings…" />}>
      <SettingsSection />
    </Suspense>
  );
}

export const settingsRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/settings",
  component: SettingsScreen,
});
