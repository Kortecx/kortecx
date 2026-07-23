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

/** The App's default model route (`steering_config.model.model_route`) — what a step
 *  naming no model of its own binds at run. "" ⇒ unset (the served model applies). */
export function readModelRoute(env: Env): string {
  const model = (env.steering_config as { model?: { model_route?: string } })?.model;
  return model?.model_route ?? "";
}

/** The names of the skills attached to the App (`references.skills`). A read-only view
 *  needs only the names: the instructions ride in CAS, and which of a skill's tool
 *  wishes survive is decided at run (SN-8). */
export function readSkillNames(env: Env): string[] {
  const skills = (env.references as { skills?: { name?: string }[] })?.skills;
  return (skills ?? []).map((s) => s.name ?? "").filter((n) => n !== "");
}

/** One blueprint step, tolerantly read. */
type BlueprintStep = {
  prompt?: string;
  model_id?: string;
  kind?: string;
  skills?: string[];
  connections?: string[];
  datasets?: string[];
};

/** The App's blueprint steps, or `[]` for a hosted App / an unparsed envelope. */
function blueprintSteps(env: Env): BlueprintStep[] {
  const bp = env.blueprint as { steps?: BlueprintStep[] } | undefined;
  return bp?.steps ?? [];
}

/** A short, human label for a blueprint step: its prompt's first words, else its
 *  1-based position. Purely for the "binds to" line — never identity. */
function stepLabel(step: BlueprintStep, index: number): string {
  const text = (step.prompt ?? "").trim().replace(/\s+/g, " ");
  if (text === "") {
    return `step ${index + 1}`;
  }
  return text.length > 32 ? `${text.slice(0, 32)}…` : text;
}

/**
 * Where a declared capability BINDS: the labels of the steps that NAME it on `axis`, or a
 * single "the entry step" when no step does.
 *
 * This is the runtime's own rule stated on screen (`RunApp` binds a capability no step
 * claims to the entry agentic step, and per-step otherwise), so a rail can show an attached
 * skill WITHOUT implying it is app-wide when the truth is per-node. Matching is
 * case-insensitive on the declared name — the same rule the runtime resolves by.
 */
export function bindingTargets(
  env: Env,
  axis: "skills" | "connections" | "datasets",
  name: string,
): string[] {
  const steps = blueprintSteps(env);
  const bound = steps
    .map((s, i) => ({ s, i }))
    .filter(({ s }) => (s[axis] ?? []).some((n) => n.toLowerCase() === name.toLowerCase()));
  if (bound.length === 0) {
    return ["the entry step"];
  }
  return bound.map(({ s, i }) => stepLabel(s, i));
}

/** Scrub `name` from every step's `axis` binding — used when a rail DETACHES a
 *  declaration, so the blueprint never names a binding to a `references` entry that is gone.
 *  Returns the env unchanged when nothing bound it (a hosted App, or an already-unbound
 *  declaration). Omit-empty: a step whose last binding on an axis is removed drops the key. */
export function unbindFromSteps(
  env: Env,
  axis: "skills" | "connections" | "datasets",
  name: string,
): Env {
  const bp = env.blueprint as { steps?: BlueprintStep[] } | undefined;
  if (!bp?.steps) {
    return env;
  }
  let changed = false;
  const steps = bp.steps.map((s) => {
    const cur = s[axis];
    if (!cur || !cur.some((n) => n.toLowerCase() === name.toLowerCase())) {
      return s;
    }
    changed = true;
    const next = cur.filter((n) => n.toLowerCase() !== name.toLowerCase());
    const { [axis]: _drop, ...rest } = s;
    return next.length > 0 ? { ...rest, [axis]: next } : rest;
  });
  if (!changed) {
    return env;
  }
  return { ...env, blueprint: { ...bp, steps } };
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
