/**
 * Query keys are ENDPOINT-SCOPED so switching gateways never serves a stale
 * cross-endpoint cache. `atSeq` distinguishes a pinned time-travel snapshot from
 * the live ("live") view.
 */

export const queryKeys = {
  signatures: (endpoint: string) => ["kx", endpoint, "signatures"] as const,
  projection: (endpoint: string, instanceId: string, atSeq?: number) =>
    ["kx", endpoint, "projection", instanceId, atSeq ?? "live"] as const,
  /** A committed artifact blob, scoped by its owning run + content ref. */
  content: (endpoint: string, instanceId: string, ref: string) =>
    ["kx", endpoint, "content", instanceId, ref] as const,
  /** Gateway liveness probe (endpoint-scoped). */
  health: (endpoint: string) => ["kx", endpoint, "health"] as const,
};
