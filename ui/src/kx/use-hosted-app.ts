/**
 * D213 Experience lane — hosted-app supervisor hooks. Start (or attach to) a hosted
 * app's dev server (`StartHostedApp`) and poll its live status (`GetHostedAppStatus`).
 * `startHostedApp` returns immediately with the loopback URL once running; the caller
 * opens it in a new browser tab. Degrades to a not-wired signal on a gateway built
 * without the `hosted-apps` feature (the console hides the Run control).
 */

import type { HostedAppStatus } from "@kortecx/sdk/web";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useCallback, useEffect, useRef, useState } from "react";
import { useConnection } from "./connection-context";
import { toUiError } from "./errors";
import { queryKeys } from "./query-keys";

/** Start (or attach to) a hosted app's dev server; resolves with its live status. */
export function useStartHostedApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<HostedAppStatus, unknown, { handle: string; rebuild?: boolean }>({
    mutationFn: async ({ handle, rebuild }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.startHostedApp(handle, rebuild ? { rebuild } : {});
    },
    onSuccess: (_status, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.hostedAppStatus(endpoint, handle) });
    },
  });
}

/** Stop a hosted app's dev server. */
export function useStopHostedApp() {
  const { client, endpoint } = useConnection();
  const qc = useQueryClient();
  return useMutation<boolean, unknown, { handle: string }>({
    mutationFn: async ({ handle }) => {
      if (!client) {
        throw new Error("not connected");
      }
      return client.stopHostedApp(handle);
    },
    onSuccess: (_ok, { handle }) => {
      void qc.invalidateQueries({ queryKey: queryKeys.hostedAppStatus(endpoint, handle) });
    },
  });
}

/**
 * Drive the hosted Run control end-to-end. `StartHostedApp` returns while the app is still
 * materializing/installing (its URL empty), so a naive "open on success" click does nothing.
 * This composes `useStartHostedApp` + `useHostedAppStatus`: it starts the app, then opens the
 * live tab as soon as the poll reports it actually running — and surfaces start errors + the
 * not-wired signal (the gateway lacking the `hosted-apps` feature). The generic Play/Stop trio
 * is a later slice; this fixes only the hosted Run.
 */
export function useHostedRun(handle: string) {
  const start = useStartHostedApp();
  const { status, notWired } = useHostedAppStatus(handle, true);
  const [armed, setArmed] = useState(false);
  const openedRef = useRef(false);

  const openLive = useCallback((url: string) => {
    if (!url || openedRef.current) {
      return;
    }
    openedRef.current = true;
    // The status carries an absolute loopback URL (http://127.0.0.1:<port>/); resolve
    // defensively so a relative path would still open against a sane base.
    let href = url;
    try {
      href = new URL(url, window.location.origin).href;
    } catch {
      href = url;
    }
    window.open(href, "_blank", "noopener");
  }, []);

  function launch(rebuild: boolean): void {
    if (notWired) {
      return;
    }
    openedRef.current = false;
    setArmed(true);
    start.mutate(
      { handle, rebuild },
      {
        onSuccess: (s) => {
          if (s.state === "running" && s.url) {
            openLive(s.url);
            setArmed(false);
          }
        },
        onError: () => setArmed(false),
      },
    );
  }

  /** Start (or attach to) the app and open it once it is genuinely serving. */
  function run(): void {
    launch(false);
  }

  /**
   * Restart CLEAN: re-materialize, drop `node_modules`, reinstall, restart the same
   * lane. This is precisely what the wire's `rebuild` flag does — it is not a production
   * build, which is why the control is not labelled "Build". The flag has been plumbed
   * proto → SDK → this hook since the lane shipped, with no UI ever setting it.
   */
  function restart(): void {
    launch(true);
  }

  // Once armed, open as soon as the poll reports the app is actually running (or give up on
  // a failure — the pill/error surfaces the reason).
  useEffect(() => {
    if (!armed) {
      return;
    }
    if (status?.state === "running" && status.url) {
      openLive(status.url);
      setArmed(false);
    } else if (status?.state === "failed") {
      setArmed(false);
    }
  }, [armed, status?.state, status?.url, openLive]);

  // Every non-terminal state must be listed here AND in the poll's `refetchInterval`
  // below — they are two hand-maintained enumerations of the same idea, and omitting a
  // state from either makes the UI go quiet mid-lifecycle and never open the tab.
  // `building` is the production lane's step; the dev lane never reaches it.
  const busy =
    start.isPending ||
    (armed &&
      (status?.state === "materializing" ||
        status?.state === "installing" ||
        status?.state === "building" ||
        status?.state === "starting"));

  let error: string | null = null;
  if (start.isError) {
    error = toUiError(start.error).message;
  } else if (armed && status?.state === "failed") {
    error = status.detail || "The hosted app failed to start.";
  }

  return { run, restart, disabled: notWired, busy, error, status };
}

/** Poll a hosted app's status; polls while starting/running, stops once stopped/failed. */
export function useHostedAppStatus(handle: string | null, enabled: boolean) {
  const { client, endpoint, status } = useConnection();
  const q = useQuery({
    queryKey: queryKeys.hostedAppStatus(endpoint, handle ?? ""),
    enabled: enabled && status === "connected" && client !== null && handle !== null,
    refetchInterval: (query) => {
      const s = (query.state.data as HostedAppStatus | undefined)?.state;
      // Keep in lock-step with `busy` in useHostedRun — a state missing here stops the
      // poll mid-lifecycle, so the app silently never reports Running.
      return s === "running" ||
        s === "starting" ||
        s === "building" ||
        s === "installing" ||
        s === "materializing"
        ? 3000
        : false;
    },
    queryFn: async (): Promise<HostedAppStatus> => {
      if (!client || handle === null) {
        throw new Error("not connected");
      }
      return client.getHostedAppStatus(handle);
    },
  });
  return {
    status: q.data ?? null,
    notWired: q.isError && toUiError(q.error).kind === "not-wired",
    isError: q.isError,
    refetch: q.refetch,
  };
}
