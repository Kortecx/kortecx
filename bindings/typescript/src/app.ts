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
import { Chain } from "./chains.js";
import { getDefaultClient } from "./default-client.js";
import { KxUsage } from "./errors.js";

/** Anything that can produce a portable blueprint (a Flow or a Chain). */
export interface BlueprintSource {
  toBlueprint(): DagSpecJson;
}

/** The minimal client surface the App terminals need (avoids a node/web cycle). */
export interface AppClient {
  putContent(payload: Uint8Array, opts?: { mediaType?: string }): Promise<{ contentRef: string }>;
  saveApp(envelope: unknown, opts?: { handle?: string }): Promise<SaveAppResult>;
  submitWorkflow(request: unknown, opts?: { wait?: boolean; timeoutMs?: number }): Promise<unknown>;
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
  private readonly _prompts: ArtifactEntry[] = [];
  private readonly _rules: ArtifactEntry[] = [];
  private readonly _memory: ArtifactEntry[] = [];
  private readonly _skills: SkillEntry[] = [];
  private readonly _pending: Pending[] = [];
  private _modelRoute = "";
  private readonly _freeParams: Record<string, string> = {};
  private readonly _requestedGrants: Record<string, string> = {};
  private _maxTurns?: number;
  private _maxToolCalls?: number;
  private _branchHandle = "";

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

  /** Reference a registered tool (id + version only — never a grant). */
  useTool(toolId: string, toolVersion = "1"): this {
    this._tools.push({ tool_id: toolId, tool_version: toolVersion });
    return this;
  }

  /** Set steering knobs (a WISH the server re-resolves at bind — never authority). */
  steer(opts: {
    model?: string;
    maxTurns?: number;
    maxToolCalls?: number;
    requestedGrants?: Record<string, string>;
    freeParams?: Record<string, string>;
  }): this {
    if (opts.model) this._modelRoute = opts.model;
    if (opts.maxTurns !== undefined) this._maxTurns = opts.maxTurns;
    if (opts.maxToolCalls !== undefined) this._maxToolCalls = opts.maxToolCalls;
    if (opts.requestedGrants) Object.assign(this._requestedGrants, opts.requestedGrants);
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
    if (Object.keys(this._requestedGrants).length) {
      steer.tools = { requested_grants: this._requestedGrants };
    }
    const guards: Record<string, unknown> = {};
    if (this._maxTurns !== undefined) guards.max_turns = this._maxTurns;
    if (this._maxToolCalls !== undefined) guards.max_tool_calls = this._maxToolCalls;
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
  async export(path: string): Promise<void> {
    const fs = await import("node:fs/promises");
    await fs.writeFile(path, prettyJson(this.toEnvelope()));
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

  /** Compile this App's blueprint and run it (exactly-once). The server re-resolves
   * every warrant from the caller's grants (SN-8). */
  async run(
    _args: Readonly<Record<string, unknown>> = {},
    opts: { wait?: boolean; timeoutMs?: number; client?: AppClient } = {},
  ): Promise<unknown> {
    const client = resolveClient(opts.client);
    await this.resolvePending(client);
    const blueprint = this.toEnvelope().blueprint as DagSpecJson;
    const request = Chain.fromBlueprint(blueprint);
    return client.submitWorkflow(request, { wait: opts.wait ?? true, timeoutMs: opts.timeoutMs });
  }
}

/** Start an App: `app("my-app").blueprint(flow()...).save()`. The authoring
 * container that WRAPS a blueprint into a durable, reusable App. */
export function app(name: string, opts: { version?: string } = {}): AppBuilder {
  return new AppBuilder(name, opts.version ?? "1");
}
