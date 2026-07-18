/**
 * The fluent App builder — author a durable, reusable App (a `kortecx.app/v1`
 * envelope) over an existing Flow/Chain blueprint (POC-4).
 *
 * ```ts
 * import { app, flow } from "@kortecx/sdk";
 *
 * const a = app("research-assistant")
 *   .blueprint(flow().agent("Research the topic", { tools: ["mcp-echo/echo"] }))
 *   .rule("no-pii", { body: "Never reveal personal data." })
 *   .steer({ maxTurns: 8, maxToolCalls: 6 });
 *
 * await a.save();                       // persist (uploads pending bodies first)
 * await a.run({ topic: "kortecx" });    // compile the blueprint + run it
 * ```
 *
 * An App WRAPS a blueprint (the byte-stable `toBlueprint()` output) with a minimal
 * prompt/rule/skill/memory rail, a 4-axis steering config, and per-step replay
 * intent. It carries NO authority — `run` re-compiles the blueprint and the server
 * re-resolves every warrant from the caller's grants (SN-8). The envelope
 * serializes byte-identically to the Rust `kx-app` + the Python SDK (the golden
 * corpus). PURE DATA (web-safe); `save`/`run`/`export` resolve a client at call time.
 */

import { APP_SCHEMA, type SaveAppResult, type Skill, prettyJson } from "./apps.js";
import type { DagSpecJson } from "./chains.js";
import { getDefaultClient } from "./default-client.js";
import { KxUsage } from "./errors.js";
import { flow } from "./flow.js";
import type { RegisterMcpServerInput } from "./toolscout.js";

/** Anything that can produce a portable blueprint (a Flow or a Chain). */
export interface BlueprintSource {
  toBlueprint(): DagSpecJson;
}

/** The minimal client surface the App terminals need (avoids a node/web cycle). */
export interface AppClient {
  putContent(payload: Uint8Array, opts?: { mediaType?: string }): Promise<{ contentRef: string }>;
  saveApp(envelope: unknown, opts?: { handle?: string }): Promise<SaveAppResult>;
  /** The App run path — SaveApp + RunApp so `references.connections` +
   * `guards.secret_scope` reach the server (a credentialed connector can be dialed).
   * `requireApproval` (opt-in, default `false`) runs under the per-run HITL gate. */
  runApp(
    handle: string,
    opts?: {
      wait?: boolean;
      timeoutMs?: number;
      args?: Record<string, string>;
      requireApproval?: boolean;
    },
  ): Promise<unknown>;
  /** OPTIONAL — used only when a promoting Flow (`asApp`) carried `withMcp` connectors. */
  registerMcpServer?(input: RegisterMcpServerInput): Promise<unknown>;
  /** OPTIONAL — used only when a promoting Flow (`asApp`) carried `withMemory` facts. */
  storeMemory?(content: string | Uint8Array, opts?: { kind?: number }): Promise<unknown>;
  /** OPTIONAL — export a saved App as a portable `kortecx.appbundle/v1` archive
   *  (used by `App.export({ bundle: true, client })`). */
  exportAppBundle?(handle: string, opts?: { withData?: boolean }): Promise<string>;
}

function resolveClient(explicit?: AppClient): AppClient {
  if (explicit !== undefined) return explicit;
  const c = getDefaultClient();
  if (c === undefined) {
    throw new KxUsage(
      "save()/run() need a client — pass { client }, or import from '@kortecx/sdk' (Node) for the " +
        "zero-config default. The browser (@kortecx/sdk/web) entrypoint is explicit-client by design.",
    );
  }
  return c as unknown as AppClient;
}

const HEX64 = /^[0-9a-f]{64}$/;

type ArtifactEntry = { name: string; content_ref: string };
type SkillEntry = { name: string; instructions_ref: string; tools?: Record<string, string> };
type ContextEntry = { name: string; content_ref: string; media_type?: string };
type ToolEntry = { tool_id: string; tool_version: string };
/** G2: a by-reference connection — the MCP endpoint descriptor + the bare credential
 * NAME (never the secret value; the runtime resolves it at dial). */
type ConnectionEntry = { descriptor: string; credential_ref: string };
type DatasetEntry = { dataset_ref: string; cas_refs?: string[] };

/** Tool resolution reach. `Explicit` (default) grants only the declared wish and is
 * OMITTED from the envelope; `InheritPrincipal` grants the caller's whole resolvable
 * tool ceiling (bounded — the server still intersects with your grants ∩ fireable). */
export const Reach = {
  Explicit: "explicit",
  InheritPrincipal: "inherit_principal",
} as const;

/** G1: the curated Gmail provider defaults (the bundled `kx-connector-gmail` sidecar).
 * Mirrors the CLI `kx connections add --provider gmail` + the Python SDK. */
const GMAIL_CONNECTOR_COMMAND = "kx-connector-gmail";
const GMAIL_CREDENTIAL_REF = "KX_GMAIL_CREDENTIAL";
/** The curated Discord provider defaults (the bundled `kx-connector-discord`
 * sidecar, #277). Mirrors `withGmail` + the Python SDK. */
const DISCORD_CONNECTOR_COMMAND = "kx-connector-discord";
const DISCORD_CREDENTIAL_REF = "KX_DISCORD_CREDENTIAL";
/** T-APP-TRIGGER-TARGET: the curated Slack provider defaults (the bundled
 * `kx-connector-slack` sidecar). Credential = a bot token `{"bot_token": "xoxb-…"}`. */
const SLACK_CONNECTOR_COMMAND = "kx-connector-slack";
const SLACK_CREDENTIAL_REF = "KX_SLACK_CREDENTIAL";
/** T-APP-TRIGGER-TARGET: the curated Notion provider defaults (the bundled
 * `kx-connector-notion` sidecar). Credential = an integration token `{"token": "…"}`. */
const NOTION_CONNECTOR_COMMAND = "kx-connector-notion";
const NOTION_CREDENTIAL_REF = "KX_NOTION_CREDENTIAL";
type Pending = { rail: string; name: string; body: string; skill?: Skill };

/** A fluent App builder. Each method returns `this`; terminate with
 * {@link AppBuilder.toEnvelope} / {@link AppBuilder.export} / {@link AppBuilder.save} /
 * {@link AppBuilder.run}. */
export class AppBuilder {
  private _blueprint?: DagSpecJson;
  private _description = "";
  private readonly _tags: string[] = [];
  private readonly _context: ContextEntry[] = [];
  private readonly _tools: ToolEntry[] = [];
  private readonly _connections: ConnectionEntry[] = [];
  private readonly _datasets: DatasetEntry[] = [];
  private readonly _secretScope: string[] = [];
  private readonly _prompts: ArtifactEntry[] = [];
  private readonly _rules: ArtifactEntry[] = [];
  private readonly _memory: ArtifactEntry[] = [];
  private readonly _skills: SkillEntry[] = [];
  private readonly _pending: Pending[] = [];
  private _modelRoute = "";
  private readonly _freeParams: Record<string, string> = {};
  private readonly _requestedGrants: Record<string, string> = {};
  /** Tool resolution reach: "" / "explicit" ⇒ the declared wish (default, omitted);
   * "inherit_principal" ⇒ the caller's whole resolvable tool ceiling. */
  private _reach = "";
  private _maxTurns?: number;
  private _maxToolCalls?: number;
  private _branchHandle = "";
  /** Imperative pre-run registrations carried from a Flow via {@link Flow.asApp}
   * (with_mcp connectors / with_memory facts). {@link run} executes them before RunApp.
   * Never part of the envelope (off the golden digest). */
  private _flowMcp: RegisterMcpServerInput[] = [];
  private _flowMemory: string[] = [];

  constructor(
    private readonly _name: string,
    private readonly _version: string = "1",
  ) {}

  /** Capture the run topology from a Flow or Chain via its byte-stable `toBlueprint()`. */
  blueprint(source: BlueprintSource): this {
    this._blueprint = source.toBlueprint();
    return this;
  }

  private addArtifact(
    rail: ArtifactEntry[],
    name: string,
    opts: { ref?: string; body?: string },
    railName: string,
  ): this {
    if (opts.ref !== undefined) {
      if (!HEX64.test(opts.ref)) throw new KxUsage(`${railName} ref must be 64-char lowercase hex`);
      rail.push({ name, content_ref: opts.ref });
    } else if (opts.body !== undefined) {
      this._pending.push({ rail: railName, name, body: opts.body });
    } else {
      throw new KxUsage(`${railName}(${name}) needs either ref or body`);
    }
    return this;
  }

  /** Add a prompt artifact — a named text body in the content store. */
  prompt(name: string, opts: { ref?: string; body?: string }): this {
    return this.addArtifact(this._prompts, name, opts, "prompts");
  }

  /** Add a rule artifact (a governance/behavior note). */
  rule(name: string, opts: { ref?: string; body?: string }): this {
    return this.addArtifact(this._rules, name, opts, "rules");
  }

  /** Add a memory artifact (a named context note). */
  memory(name: string, opts: { ref?: string; body?: string }): this {
    return this.addArtifact(this._memory, name, opts, "memory");
  }

  /** Add a skill — a named (instructions + tool wish SET) bundle ≈ an Agent. */
  skill(skill: Skill): this {
    if (skill.instructionsRef !== undefined) {
      if (!HEX64.test(skill.instructionsRef)) {
        throw new KxUsage("skill instructionsRef must be 64-char lowercase hex");
      }
      const entry: SkillEntry = { name: skill.name, instructions_ref: skill.instructionsRef };
      if (skill.tools && Object.keys(skill.tools).length > 0) entry.tools = { ...skill.tools };
      this._skills.push(entry);
    } else if (skill.instructions !== undefined) {
      this._pending.push({ rail: "skills", name: skill.name, body: skill.instructions, skill });
    } else {
      throw new KxUsage(`skill ${skill.name} needs instructions or instructionsRef`);
    }
    return this;
  }

  /** Reference a context item by content ref (carries `mediaType`). */
  context(name: string, ref: string, opts: { mediaType?: string } = {}): this {
    if (!HEX64.test(ref)) throw new KxUsage("context ref must be 64-char lowercase hex");
    const entry: ContextEntry = { name, content_ref: ref };
    if (opts.mediaType) entry.media_type = opts.mediaType;
    this._context.push(entry);
    return this;
  }

  /** Request a tool the App wants to use. Records a tool WISH
   * (`steering_config.tools.requested_grants`) — the surface the server actually
   * intersects with your grants at run — and keeps a display entry on
   * `references.tools`. A wish is never authority: the server grants only
   * `wish ∩ your-grants ∩ fireable`. */
  useTool(toolId: string, toolVersion = "1"): this {
    this._tools.push({ tool_id: toolId, tool_version: toolVersion });
    if (!(toolId in this._requestedGrants)) this._requestedGrants[toolId] = toolVersion;
    return this;
  }

  /** Ground the App on a dataset (declarative RAG-on-App). At run, `RunApp` grants the
   * entry step the read-only `retrieve` tool and steers it to search `datasetRef` live in
   * the loop — the App self-grounds instead of needing a hand-authored blueprint. INGEST
   * the corpus first with `kx datasets ingest <datasetRef> …` (the "reference-existing"
   * model; a named dataset absent from the server fails closed at run). `casRefs` (64-hex
   * content refs the dataset spans) are recorded for a future self-contained ingest;
   * today grounding uses the pre-ingested named dataset. */
  dataset(datasetRef: string, opts: { casRefs?: string[] } = {}): this {
    const entry: DatasetEntry = { dataset_ref: datasetRef };
    if (opts.casRefs && opts.casRefs.length > 0) {
      for (const r of opts.casRefs) {
        if (!HEX64.test(r)) throw new KxUsage("dataset casRef must be 64-char lowercase hex");
      }
      entry.cas_refs = [...opts.casRefs];
    }
    this._datasets.push(entry);
    return this;
  }

  /** Alias for {@link dataset} — ground the App on a dataset (RAG-on-App). */
  rag(datasetRef: string, opts: { casRefs?: string[] } = {}): this {
    return this.dataset(datasetRef, opts);
  }

  /** G2: declare a by-reference connection the App uses. `descriptor` is the MCP
   * endpoint (a stdio command or an `http(s)` URL, no userinfo); `credentialRef` is the
   * bare secret NAME the runtime resolves at DIAL time (never the value). By default the
   * credential is added to `guards.secret_scope` so the run warrant permits dialing it
   * (`RunApp` narrows `SecretScope::AllowList` to these); pass `{ scopeSecret: false }`
   * for a credential-less connection. The pointer is a bare name, so a shared App
   * resolves each operator's OWN credentials — register it with `kx connections add`. */
  withConnection(
    descriptor: string,
    credentialRef = "",
    opts: { scopeSecret?: boolean } = {},
  ): this {
    this._connections.push({ descriptor, credential_ref: credentialRef });
    if ((opts.scopeSecret ?? true) && credentialRef && !this._secretScope.includes(credentialRef)) {
      this._secretScope.push(credentialRef);
    }
    return this;
  }

  /** G1: declare the bundled Gmail connector (the curated provider default) — equivalent
   * to `withConnection("kx-connector-gmail", "KX_GMAIL_CREDENTIAL")`. Register it on the
   * runtime with `kx connections add --provider gmail`. */
  withGmail(): this {
    return this.withConnection(GMAIL_CONNECTOR_COMMAND, GMAIL_CREDENTIAL_REF);
  }

  /** Declare the bundled Discord connector (the curated provider default) —
   * equivalent to `withConnection("kx-connector-discord", "KX_DISCORD_CREDENTIAL")`.
   * Register with `kx connections add --provider discord` (a bot token by name). To let an
   * agent FIRE a Discord tool, grant it by the registered CONNECTION NAME — e.g.
   * `tools: ["discord/read_channel"]` — NOT the descriptor `kx-connector-discord/*` (an
   * agent grant is namespaced by the connection name, so the descriptor form will not
   * dispatch against a "discord"-named connection). */
  withDiscord(): this {
    return this.withConnection(DISCORD_CONNECTOR_COMMAND, DISCORD_CREDENTIAL_REF);
  }

  /** T-APP-TRIGGER-TARGET: declare the bundled Slack connector (the curated provider
   * default) — equivalent to `withConnection("kx-connector-slack", "KX_SLACK_CREDENTIAL")`.
   * Register with `kx connections add --provider slack` (a bot token `{"bot_token": "xoxb-…"}`
   * by name). To let an agent FIRE a Slack tool, grant it by the registered CONNECTION
   * NAME — e.g. `tools: ["slack/read_channel"]` — NOT the descriptor `kx-connector-slack/*`
   * (an agent grant is namespaced by the connection name, so the descriptor form will not
   * dispatch against a "slack"-named connection). */
  withSlack(): this {
    return this.withConnection(SLACK_CONNECTOR_COMMAND, SLACK_CREDENTIAL_REF);
  }

  /** T-APP-TRIGGER-TARGET: declare the bundled Notion connector (the curated provider
   * default) — equivalent to `withConnection("kx-connector-notion", "KX_NOTION_CREDENTIAL")`.
   * Register with `kx connections add --provider notion` (an integration token `{"token": "…"}`
   * by name). To let an agent FIRE a Notion tool, grant it by the registered CONNECTION
   * NAME — e.g. `tools: ["notion/search"]` — NOT the descriptor `kx-connector-notion/*` (an
   * agent grant is namespaced by the connection name, so the descriptor form will not
   * dispatch against a "notion"-named connection). */
  withNotion(): this {
    return this.withConnection(NOTION_CONNECTOR_COMMAND, NOTION_CREDENTIAL_REF);
  }

  /** Add secret NAMES to `guards.secret_scope` — the run warrant's
   * `SecretScope::AllowList` narrows to these so a granted connector may be dialed inside
   * the agentic loop (G2/#285). The VALUE never travels (D81). A scope name is bounded
   * server-side by the referenced connections — pair it with the matching
   * {@link withConnection} / {@link withGmail} / {@link withDiscord}, or the entry is
   * inert (fails closed). Usually implicit via `withConnection(..., { scopeSecret: true })`. */
  secrets(names: string | string[]): this {
    for (const name of typeof names === "string" ? [names] : names) {
      if (name && !this._secretScope.includes(name)) this._secretScope.push(name);
    }
    return this;
  }

  /** Carry a promoting Flow's imperative side-channels (with_mcp connectors + with_memory
   * facts) so {@link run} registers them before RunApp. Set by {@link Flow.asApp}; never
   * part of the envelope. */
  carryFlowSideChannels(mcp: readonly RegisterMcpServerInput[], memory: readonly string[]): void {
    this._flowMcp = [...mcp];
    this._flowMemory = [...memory];
  }

  /** Set steering knobs (a WISH the server re-resolves at bind — never authority).
   *
   * `reach` selects how the tool wish is resolved: {@link Reach.Explicit} (default)
   * grants only the declared wish; {@link Reach.InheritPrincipal} grants the caller's
   * whole resolvable tool ceiling (bounded — never omnipotence). */
  steer(opts: {
    model?: string;
    maxTurns?: number;
    maxToolCalls?: number;
    requestedGrants?: Record<string, string>;
    reach?: string;
    freeParams?: Record<string, string>;
  }): this {
    if (opts.model) this._modelRoute = opts.model;
    if (opts.maxTurns !== undefined) this._maxTurns = opts.maxTurns;
    if (opts.maxToolCalls !== undefined) this._maxToolCalls = opts.maxToolCalls;
    if (opts.requestedGrants) Object.assign(this._requestedGrants, opts.requestedGrants);
    if (opts.reach) {
      if (opts.reach !== Reach.Explicit && opts.reach !== Reach.InheritPrincipal) {
        throw new Error(
          `reach must be "${Reach.Explicit}" or "${Reach.InheritPrincipal}", got "${opts.reach}"`,
        );
      }
      this._reach = opts.reach;
    }
    if (opts.freeParams) Object.assign(this._freeParams, opts.freeParams);
    return this;
  }

  /** Add catalog tags. */
  tags(...tags: string[]): this {
    this._tags.push(...tags);
    return this;
  }

  /** Set the advisory description. */
  describe(text: string): this {
    this._description = text;
    return this;
  }

  /** Set the (optional) per-App project branch handle (reserved; never created here). */
  branch(handle: string): this {
    this._branchHandle = handle;
    return this;
  }

  private referencesDict(): Record<string, unknown> {
    const refs: Record<string, unknown> = {};
    if (this._context.length) refs.context = this._context;
    if (this._tools.length) refs.tools = this._tools;
    if (this._connections.length) refs.connections = this._connections;
    if (this._datasets.length) refs.datasets = this._datasets;
    if (this._prompts.length) refs.prompts = this._prompts;
    if (this._rules.length) refs.rules = this._rules;
    if (this._memory.length) refs.memory = this._memory;
    if (this._skills.length) refs.skills = this._skills;
    return refs;
  }

  private steeringDict(): Record<string, unknown> {
    const steer: Record<string, unknown> = {};
    const model: Record<string, unknown> = {};
    if (this._modelRoute) model.model_route = this._modelRoute;
    if (Object.keys(this._freeParams).length) model.free_params = this._freeParams;
    if (Object.keys(model).length) steer.model = model;
    const tools: Record<string, unknown> = {};
    if (Object.keys(this._requestedGrants).length) tools.requested_grants = this._requestedGrants;
    // Default ("" / explicit) reach is OMITTED — byte-identical to the pre-reach form.
    if (this._reach && this._reach !== Reach.Explicit) tools.reach = this._reach;
    if (Object.keys(tools).length) steer.tools = tools;
    const guards: Record<string, unknown> = {};
    if (this._maxTurns !== undefined) guards.max_turns = this._maxTurns;
    if (this._maxToolCalls !== undefined) guards.max_tool_calls = this._maxToolCalls;
    if (this._secretScope.length) guards.secret_scope = [...new Set(this._secretScope)];
    if (Object.keys(guards).length) steer.guards = guards;
    return steer;
  }

  /** Assemble the `kortecx.app/v1` envelope object (omit-empty, the canonical
   * byte-shape). Requires the blueprint and NO pending body uploads — use
   * {@link AppBuilder.save} (which uploads pending bodies first) or pass artifacts by `ref`. */
  toEnvelope(): Record<string, unknown> {
    if (this._blueprint === undefined) {
      throw new KxUsage("app needs a blueprint — call .blueprint(flow()/chain(...))");
    }
    if (this._pending.length > 0) {
      const names = this._pending.map((p) => `${p.rail}:${p.name}`).join(", ");
      throw new KxUsage(
        `toEnvelope() cannot resolve pending body uploads (${names}); use .save({ client }) or pass artifacts by ref`,
      );
    }
    const env: Record<string, unknown> = {
      schema: APP_SCHEMA,
      name: this._name,
      version: this._version,
      blueprint: this._blueprint,
    };
    if (this._description) env.description = this._description;
    if (this._tags.length) env.tags = [...this._tags];
    const refs = this.referencesDict();
    if (Object.keys(refs).length) env.references = refs;
    const steer = this.steeringDict();
    if (Object.keys(steer).length) env.steering_config = steer;
    if (this._branchHandle) env.branch_handle = this._branchHandle;
    return env;
  }

  /** Write the pretty envelope JSON to `path` (NODE-only — a dynamic `node:fs`
   * import keeps it out of the web/chains static bundle graph). */
  async export(
    path: string,
    opts: { bundle?: boolean; client?: AppClient; withData?: boolean } = {},
  ): Promise<void> {
    const fs = await import("node:fs/promises");
    if (!(opts.bundle ?? false)) {
      await fs.writeFile(path, prettyJson(this.toEnvelope()));
      return;
    }
    const client = resolveClient(opts.client);
    if (client.exportAppBundle === undefined) {
      throw new Error("export({ bundle: true }) requires a client that supports exportAppBundle");
    }
    const saved = await this.save({ client });
    const wire = await client.exportAppBundle(saved.handle, { withData: opts.withData });
    await fs.writeFile(path, wire);
  }

  private async resolvePending(client: AppClient): Promise<void> {
    for (const p of this._pending) {
      const enc = new TextEncoder().encode(p.body);
      const ref = (await client.putContent(enc, { mediaType: "text/plain" })).contentRef;
      if (p.rail === "skills") {
        const entry: SkillEntry = { name: p.name, instructions_ref: ref };
        if (p.skill?.tools && Object.keys(p.skill.tools).length > 0)
          entry.tools = { ...p.skill.tools };
        this._skills.push(entry);
      } else {
        const rail =
          p.rail === "prompts" ? this._prompts : p.rail === "rules" ? this._rules : this._memory;
        rail.push({ name: p.name, content_ref: ref });
      }
    }
    this._pending.length = 0;
  }

  /** Upload any pending bodies, then `SaveApp` the canonical envelope. The handle
   * defaults to `apps/local/<sanitized-name>`. */
  async save(opts: { handle?: string; client?: AppClient } = {}): Promise<SaveAppResult> {
    const client = resolveClient(opts.client);
    await this.resolvePending(client);
    return client.saveApp(this.toEnvelope(), { handle: opts.handle });
  }

  /** Save this App and run it via `RunApp` (exactly-once). `args` (the App's input schema)
   * fold server-side into the entry step's prompt.
   *
   * Routes through `SaveApp` + `RunApp` instead of a local `submitWorkflow`
   * recompile — so the App's `references.connections` + `guards.secret_scope` reach the
   * server and a credentialed connector (Gmail / Discord) actually fires inside the
   * agentic loop (the G2/#285 path). Saving is expected: an App is an explicitly-named
   * durable object; the save is idempotent (content-addressed envelope + handle upsert).
   * The server re-resolves every warrant from the caller's grants (SN-8).
   * `requireApproval` (opt-in, default `false`) runs the entry agentic step under the
   * per-run HITL gate, so an irreversible tool call pauses for an explicit grant/deny
   * before it fires. */
  async run(
    args: Readonly<Record<string, unknown>> = {},
    opts: {
      wait?: boolean;
      timeoutMs?: number;
      client?: AppClient;
      requireApproval?: boolean;
    } = {},
  ): Promise<unknown> {
    const client = resolveClient(opts.client);
    await this.resolvePending(client);
    // Imperative side-channels carried from a promoting Flow (Flow.asApp).
    if (this._flowMcp.length > 0 && typeof client.registerMcpServer === "function") {
      for (const spec of this._flowMcp) await client.registerMcpServer(spec);
    }
    if (this._flowMemory.length > 0 && typeof client.storeMemory === "function") {
      for (const fact of this._flowMemory) await client.storeMemory(fact);
    }
    const saved = await client.saveApp(this.toEnvelope());
    const entries = Object.entries(args);
    const strArgs =
      entries.length > 0 ? Object.fromEntries(entries.map(([k, v]) => [k, String(v)])) : undefined;
    return client.runApp(saved.handle, {
      args: strArgs,
      wait: opts.wait ?? true,
      timeoutMs: opts.timeoutMs,
      requireApproval: opts.requireApproval,
    });
  }
}

/** Start an App: `app("my-app").blueprint(flow()...).save()`. The authoring
 * container that WRAPS a blueprint into a durable, reusable App. */
export function app(name: string, opts: { version?: string } = {}): AppBuilder {
  return new AppBuilder(name, opts.version ?? "1");
}

/**
 * POC-5a: author a MINIMAL App envelope (a single agentic step over `goal`) for the
 * "New App" one-shot — save it, then `client.scaffoldApp(handle)` scaffolds the
 * project tree into its branch. The envelope carries NO authority (the server
 * re-resolves warrants at run); the blueprint is a valid single-step DAG.
 */
export function minimalAppEnvelope(
  name: string,
  goal: string,
  opts: { model?: string } = {},
): Record<string, unknown> {
  const builder = app(name).describe(goal).blueprint(flow().agent(goal));
  if (opts.model) builder.steer({ model: opts.model });
  return builder.toEnvelope();
}
