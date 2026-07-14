/**
 * Pure editors for the stored App envelope's capability wishes — the tolerant,
 * omit-empty rebuilds the editable Tools / Integrations rails re-`SaveApp` with.
 *
 * Kept byte-faithful to the SDK `AppBuilder` assembly so a console-edited App is
 * indistinguishable from an SDK-authored one: `references.tools` mirrors
 * `steering_config.tools.requested_grants`, and a bound credential NAME (never the
 * secret, D81) joins `steering_config.guards.secret_scope` — which tracks the
 * connectors' credentials (attach adds, detach prunes). Empty sub-objects are
 * omitted; keys are rebuilt rather than `delete`d (biome perf rule).
 *
 * Attaching grants NOTHING (SN-8): at RunApp the server intersects these wishes
 * against the caller's grants + the live broker. `SaveApp` re-canonicalizes.
 */

type Env = Record<string, unknown>;

export interface ConnectionEntry {
  descriptor: string;
  credential_ref: string;
}

// ---- reads (tolerant of the opaque parsed JSON) ----

export function readToolGrants(env: Env): Record<string, string> {
  const tools = (env.steering_config as { tools?: { requested_grants?: Record<string, string> } })
    ?.tools;
  return tools?.requested_grants ?? {};
}

export function readReachInherit(env: Env): boolean {
  const tools = (env.steering_config as { tools?: { reach?: string } })?.tools;
  return tools?.reach === "inherit_principal";
}

export function readConnections(env: Env): ConnectionEntry[] {
  const conns = (
    env.references as { connections?: { descriptor?: string; credential_ref?: string }[] }
  )?.connections;
  return (conns ?? []).map((c) => ({
    descriptor: c.descriptor ?? "",
    credential_ref: c.credential_ref ?? "",
  }));
}

export function readSecretScope(env: Env): string[] {
  const guards = (env.steering_config as { guards?: { secret_scope?: string[] } })?.guards;
  return guards?.secret_scope ?? [];
}

// ---- writes (omit-empty, preserve sibling keys) ----

function withReferences(env: Env, references: Env): Env {
  const { references: _drop, ...rest } = env;
  return Object.keys(references).length > 0 ? { ...rest, references } : rest;
}

function withSteeringConfig(env: Env, steering: Env): Env {
  const { steering_config: _drop, ...rest } = env;
  return Object.keys(steering).length > 0 ? { ...rest, steering_config: steering } : rest;
}

/** Rebuild the tool grants (the load-bearing wish) + the `references.tools` mirror + reach. */
export function writeTools(env: Env, grants: Record<string, string>, reachInherit: boolean): Env {
  const ids = Object.keys(grants);

  const refs = (env.references as Env | undefined) ?? {};
  const { tools: _dropRefTools, ...restRefs } = refs;
  const refTools = ids.map((id) => ({ tool_id: id, tool_version: grants[id] }));
  const references: Env = refTools.length > 0 ? { ...restRefs, tools: refTools } : restRefs;

  const sc = (env.steering_config as Env | undefined) ?? {};
  const { tools: _dropScTools, ...restSc } = sc;
  const toolsCfg: Env = {};
  if (ids.length > 0) {
    toolsCfg.requested_grants = grants;
  }
  if (reachInherit) {
    toolsCfg.reach = "inherit_principal";
  }
  const steering: Env = Object.keys(toolsCfg).length > 0 ? { ...restSc, tools: toolsCfg } : restSc;

  return withSteeringConfig(withReferences(env, references), steering);
}

/** Rebuild the connectors + the `secret_scope` of their bound credential names. */
export function writeConnections(env: Env, connections: ConnectionEntry[]): Env {
  const refs = (env.references as Env | undefined) ?? {};
  const { connections: _dropConns, ...restRefs } = refs;
  const references: Env = connections.length > 0 ? { ...restRefs, connections } : restRefs;

  // secret_scope tracks the connectors' non-empty credential names (deduped).
  const scope = Array.from(
    new Set(connections.map((c) => c.credential_ref).filter((r) => r.length > 0)),
  );
  const sc = (env.steering_config as Env | undefined) ?? {};
  const { guards: _dropGuards, ...restSc } = sc;
  const existingGuards = (sc.guards as Env | undefined) ?? {};
  const { secret_scope: _dropSS, ...restGuards } = existingGuards;
  const guards: Env = scope.length > 0 ? { ...restGuards, secret_scope: scope } : restGuards;
  const steering: Env = Object.keys(guards).length > 0 ? { ...restSc, guards } : restSc;

  return withSteeringConfig(withReferences(env, references), steering);
}
