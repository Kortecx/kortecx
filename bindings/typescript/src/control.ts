/**
 * The ControlSurface + NL-authoring types — `describeControlSurface` /
 * `proposeControlAction` and the durable Policy/Role registry.
 *
 * ## A proposal writes nothing, and approval is client-held
 *
 * `proposeControlAction` returns the EXACT typed request the runtime would issue and
 * registers nothing. Enacting it means calling that ordinary RPC with the bytes you
 * were shown — never re-deriving them from a rendering. That is why
 * {@link ControlPreview} carries `rpc` (which method to call) alongside the summary,
 * and `request` (the message itself) alongside `requestField`.
 *
 * ## A role NARROWS, never grants
 *
 * Assigning a Policy/Role makes a party's effective tool set the INTERSECTION of every
 * present authority leg, so it can only ever take capability away. Naming a tool the
 * party could not fire anyway simply drops out. An EMPTY role is meaningful and is not
 * the same as having no role: it narrows to nothing.
 *
 * ## What a proposal structurally cannot carry
 *
 * Secrets ride a NAME-only shape and scripts carry no `argv`/`env` — not by convention
 * but because the wire types have no such field. A preview can therefore be displayed,
 * logged and forwarded without ever holding a credential.
 */

/** What ONE gateway RPC is. */
export interface ControlSurfaceEntry {
  /** The RPC's wire name. Generated from the compiled descriptor. */
  readonly rpc: string;
  /** The subsystem it belongs to. */
  readonly domain: string;
  /** `false` means a successful call changes no durable state. */
  readonly mutates: boolean;
  /** `caller_principal` | `operator_global` | `loopback_only`. */
  readonly authority: string;
  /** `true` when the domain is one the NL authoring surface covers. */
  readonly authoring: boolean;
}

/** One `(toolId, toolVersion)` pair a role narrows TO. */
export interface PolicyRoleTool {
  readonly toolId: string;
  readonly toolVersion: string;
}

/**
 * One stored Policy/Role.
 *
 * `tools` EMPTY is a decision, not an absence: a role that names no tool refuses every
 * tool. A party with no role assigned is a different thing entirely — it expresses no
 * narrowing and resolves exactly as it did before any registry existed.
 */
export interface PolicyRole {
  readonly name: string;
  readonly description: string;
  readonly tools: readonly PolicyRoleTool[];
  readonly createdUnixMs: number;
  readonly updatedUnixMs: number;
}

/**
 * The exact typed request the runtime WOULD issue.
 *
 * `rpc` names the method to call to enact it, and `request` is the protobuf message to
 * send — the SAME message the server put in the preview, not a reconstruction.
 * Forwarding it verbatim is the whole point: it is what makes "approve" mean "issue
 * what I was shown".
 */
export interface ControlPreview {
  /** The `GatewayRpc` wire name to call (e.g. `"SaveWorkflow"`). */
  readonly rpc: string;
  /** One-line human rendering. DISPLAY ONLY — never parse this. */
  readonly summary: string;
  /** Which `oneof` arm is set (e.g. `"putPolicyRole"`). */
  readonly requestField: string;
  /** The request message itself, ready to forward. */
  readonly request: unknown;
}

/**
 * The outcome of `proposeControlAction`.
 *
 * A refusal is an ANSWER, not an error: `proposed: false` with a `reason`. An
 * inadmissible ask should be refused before a human is asked to approve it, and a
 * thrown error would hide that behind a stack trace.
 */
export interface ControlProposal {
  readonly proposed: boolean;
  readonly preview?: ControlPreview;
  readonly reason: string;
}
