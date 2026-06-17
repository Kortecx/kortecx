/**
 * The toolscout views — advisory tool discovery + TaskBundle preview, as surfaced
 * by `ListToolManifests` / `ScoreTaskBundle` (W1.A5). Kept in its own module so
 * `types.ts` stays a thin aggregator, mirroring the Rust core's module-per-concern
 * discipline.
 *
 * SN-8: every score/verdict here is ADVISORY/DISPLAY-ONLY — a score can surface a
 * tool, never grant one. The sole grant gate stays the exact (toolId, toolVersion)
 * equality check in lowering + the broker. The {@link BundleScore.verdict} is a
 * server-side DRY-RUN of the real lowering gate against the SERVER-built warrant
 * (no client warrant input); nothing submits, nothing journals. Fingerprints are
 * server-derived; the SDK only *encodes* the bytes to hex.
 */

import type { MessageInitShape } from "@bufbuild/protobuf";
import { LowerVerdict } from "./gen/kortecx/v1/gateway_pb.js";
import type {
  KeywordSet as PbKeywordSet,
  ManifestScore as PbManifestScore,
  McpServer as PbMcpServer,
  RegisteredTool as PbRegisteredTool,
  ScoreTaskBundleResponse as PbScoreTaskBundleResponse,
  ToolManifest as PbToolManifest,
  ScoreTaskBundleRequestSchema,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** The lowering dry-run verdict. `"unknown"` absorbs UNSPECIFIED(0) + any future value. */
export type LowerVerdictName = "unavailable" | "would-lower" | "refused" | "unknown";

/** Map a `LowerVerdict` discriminant to a stable name (`"unknown"` if new). */
export function lowerVerdictName(verdict: number): LowerVerdictName {
  if (verdict === LowerVerdict.UNAVAILABLE) return "unavailable";
  if (verdict === LowerVerdict.WOULD_LOWER) return "would-lower";
  if (verdict === LowerVerdict.REFUSED) return "refused";
  return "unknown";
}

/** Normalized intent keywords under one BCP-47-ish language tag. */
export class KeywordSet {
  constructor(
    readonly lang: string,
    readonly words: readonly string[],
  ) {}

  static fromProto(k: PbKeywordSet): KeywordSet {
    return new KeywordSet(k.lang, k.words);
  }
}

/** One registered tool's advisory manifest (ranking/display material ONLY — the
 *  broker never reads manifests; listing it leaks no authority). */
export class ToolManifest {
  constructor(
    /** The grant-set identity half (exact). */
    readonly toolId: string,
    /** The other identity half (exact). */
    readonly toolVersion: string,
    /** Free-form human text; NEVER parsed for enforcement. */
    readonly description: string,
    readonly keywords: readonly KeywordSet[],
    /** The 32B blake3 ToolFingerprint content hash, as lowercase hex (display/join key). */
    readonly fingerprintHash: string,
    /** `"Builtin"` | `"Mcp"` (display). */
    readonly kind: string,
  ) {}

  static fromProto(m: PbToolManifest): ToolManifest {
    return new ToolManifest(
      m.toolId,
      m.toolVersion,
      m.description,
      m.keywords.map((k) => KeywordSet.fromProto(k)),
      encode(m.fingerprintHash),
      m.kind,
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      tool_id: this.toolId,
      tool_version: this.toolVersion,
      description: this.description,
      keywords: this.keywords.map((k) => ({ lang: k.lang, words: [...k.words] })),
      fingerprint_hash: this.fingerprintHash,
      kind: this.kind,
    };
  }
}

/** One manifest's advisory rank against the bundle intent, in integer basis points
 *  (0..=10000; floats never cross the wire — SN-8 no-persisted-confidence). */
export class ManifestScore {
  constructor(
    readonly toolId: string,
    readonly toolVersion: string,
    /** The advisory rank in basis points (10000 = exact keyword/phrase). */
    readonly scoreBp: number,
    /** Hex fingerprint — joins back to {@link KxClientBase.listToolManifests}. */
    readonly fingerprintHash: string,
  ) {}

  static fromProto(s: PbManifestScore): ManifestScore {
    return new ManifestScore(s.toolId, s.toolVersion, s.scoreBp, encode(s.fingerprintHash));
  }
}

/** The advisory `ScoreTaskBundle` outcome: every registered manifest ranked
 *  best-first + the lowering-gate DRY-RUN verdict. DISPLAY-ONLY (SN-8) — the
 *  broker re-gates any future real dispatch. */
export class BundleScore {
  constructor(
    /** The 32B blake3 TaskBundle content fingerprint, as lowercase hex. */
    readonly bundleFingerprint: string,
    /** EVERY registered manifest, best-first (deterministic tiebreak). */
    readonly ranked: readonly ManifestScore[],
    /** The lowering dry-run verdict (`"unknown"` absorbs any future value). */
    readonly verdict: LowerVerdictName,
    /** Display-only availability/refusal prose. */
    readonly verdictDetail: string,
  ) {}

  static fromProto(r: PbScoreTaskBundleResponse): BundleScore {
    return new BundleScore(
      encode(r.bundleFingerprint),
      r.ranked.map((s) => ManifestScore.fromProto(s)),
      lowerVerdictName(r.verdict),
      r.verdictDetail,
    );
  }
}

/** One keyword set in a client-authored bundle spec (plain input shape). */
export interface KeywordSetInput {
  readonly lang: string;
  readonly words: readonly string[];
}

/** One sequenced tool in a client-authored TaskBundle spec. Advisory metadata
 *  rides along; identity is the exact (toolId, toolVersion) pair. */
export interface BundleToolInput {
  readonly toolId: string;
  readonly toolVersion: string;
  /** Advisory ToolMeta description (defaults to `""`). */
  readonly description?: string;
  /** Advisory ToolMeta keywords (defaults to none). */
  readonly keywords?: readonly KeywordSetInput[];
}

/** A client-authored TaskBundle spec for {@link KxClientBase.scoreTaskBundle}. */
export interface BundleSpec {
  /** The task instruction (server size-capped; validated fail-closed). */
  readonly intent: string;
  /** Advisory BCP-47-ish tags (server count/size-capped; defaults to none). */
  readonly languageTags?: readonly string[];
  /** The ordered tool sequence (non-empty; duplicate names refused server-side). */
  readonly tools: readonly BundleToolInput[];
  /** Advisory ranking cut in basis points, 0..=10000 (defaults to 0 = no cut). */
  readonly toleranceThresholdBp?: number;
}

/** Map a {@link BundleSpec} to the `ScoreTaskBundleRequest` init shape (defaults applied). */
export function bundleSpecToProto(
  spec: BundleSpec,
): MessageInitShape<typeof ScoreTaskBundleRequestSchema> {
  return {
    intent: spec.intent,
    languageTags: [...(spec.languageTags ?? [])],
    toolSequence: spec.tools.map((t) => ({
      toolId: t.toolId,
      toolVersion: t.toolVersion,
      description: t.description ?? "",
      keywords: (t.keywords ?? []).map((k) => ({ lang: k.lang, words: [...k.words] })),
    })),
    toleranceThresholdBp: spec.toleranceThresholdBp ?? 0,
  };
}

// --- PR-6a declarative tools registry (DiscoverTools / RegisterTool) ----------
//
// DISTINCT from the advisory ToolManifest above: the durable registry INVENTORY
// (what is registered, by whom, with what authority). Registration grants NO
// authority — a tool fires only under a server-issued warrant (SN-8); `toolId`
// is server-derived. DIALING a registered external MCP server is Cloud / PR-6b.

/** One durable-registry row (`DiscoverTools`). `netScope` is a display summary;
 *  authority never rides this wire (SN-8). */
export class RegisteredTool {
  constructor(
    /** 16-byte server-derived id, as lowercase hex. */
    readonly toolId: string,
    readonly toolName: string,
    readonly toolVersion: string,
    /** `"Builtin"` | `"Mcp"` (display). */
    readonly kind: string,
    readonly description: string,
    readonly idempotencyClass: string,
    /** `"HumanAuthored"` | `"SelfGenerated"`. */
    readonly provenance: string,
    /** `"Approved"` | `"PendingHumanReview"`. */
    readonly registrationStatus: string,
    /** The vetted egress endpoint (empty = no egress). */
    readonly serverHost: string,
    /** `"none"` | `"egress:host[,host]"`. */
    readonly netScope: string,
    readonly isBuiltin: boolean,
  ) {}

  static fromProto(t: PbRegisteredTool): RegisteredTool {
    return new RegisteredTool(
      encode(t.toolId),
      t.toolName,
      t.toolVersion,
      t.kind,
      t.description,
      t.idempotencyClass,
      t.provenance,
      t.registrationStatus,
      t.serverHost,
      t.netScopeSummary,
      t.isBuiltin,
    );
  }
}

/** One `DiscoverTools` page (deterministic `(name, version)` order). */
export interface RegisteredToolsPage {
  readonly tools: readonly RegisteredTool[];
  readonly hasMore: boolean;
}

/** A declared, typed tool input parameter (the MCP inputSchema analogue — CLOSED
 *  set, no float, SN-8). `ty` in `str|bytes|int|bool|enum`. */
export interface ToolParam {
  readonly name: string;
  /** `str` | `bytes` | `int` | `bool` | `enum` (defaults to `str`). */
  readonly ty?: string;
  /** str/bytes byte cap (0 = server default). */
  readonly maxLen?: number;
  /** Defaults to `true`. */
  readonly required?: boolean;
  /** enum: the permitted exact values. */
  readonly allowed?: readonly string[];
}

/** One registered external MCP server (PR-6b-1 `ListMcpServers`). The credential
 *  VALUE is never on the wire — only whether a ref NAME is attached (D81). */
export class McpServer {
  constructor(
    /** 16-byte server-derived connection id, as lowercase hex. */
    readonly connectionId: string,
    readonly serverName: string,
    /** `"stdio"` | `"http"`. */
    readonly transport: string,
    /** The command (stdio) or URL (http). */
    readonly endpoint: string,
    /** `"connected"` | `"unreachable"` | `"unknown"`. */
    readonly health: string,
    readonly toolCount: number,
    readonly credentialRefPresent: boolean,
    /** PR-6b-3: the firing posture — `"stateful"` | `"stateless"`. */
    readonly sessionMode: string,
  ) {}

  static fromProto(s: PbMcpServer): McpServer {
    return new McpServer(
      encode(s.connectionId),
      s.serverName,
      s.transport,
      s.endpoint,
      s.health,
      s.toolCount,
      s.credentialRefPresent,
      s.sessionMode,
    );
  }
}

/** One `ListMcpServers` page (deterministic `(name)` order). */
export interface McpServersPage {
  readonly servers: readonly McpServer[];
  readonly hasMore: boolean;
}

/** The outcome of `registerMcpServer` — the server-derived connection id, the
 *  tools discovered + registered, and the folded health. */
export interface RegisterServerResult {
  /** 16-byte server-derived connection id, as lowercase hex. */
  readonly connectionId: string;
  readonly discovered: number;
  /** `"connected"` | `"unreachable"` | `"unknown"`. */
  readonly health: string;
}

/** A `RegisterMcpServer` request shape (PR-6b-1). The runtime DIALS the server;
 *  the host is SSRF-vetted (admission + dial). A credential is referenced by NAME
 *  only (never the secret, D81). The server derives the connection/tool ids (SN-8). */
export interface RegisterMcpServerInput {
  readonly name: string;
  /** `"stdio"` | `"http"` (defaults to `"stdio"`). */
  readonly transport?: string;
  /** stdio: the program path; http: the endpoint URL. */
  readonly endpoint: string;
  /** stdio command-line args (ignored for http). */
  readonly args?: readonly string[];
  /** http: refuse plaintext `http://` (defaults to `false`). */
  readonly tlsRequired?: boolean;
  /** OPTIONAL secret-less credential ref NAME (env var / vault key). */
  readonly credentialRef?: string;
  /** PR-6b-3 firing posture: `"stateful"` | `"stateless"` (defaults to `"stateless"`). */
  readonly sessionMode?: string;
}

/** A `RegisterTool` request shape. The host is SSRF-vetted; the server derives
 *  identity + capability (the client never sends a warrant / toolId, SN-8). */
export interface RegisterToolInput {
  readonly name: string;
  readonly version: string;
  /** The external MCP endpoint `host[:port]` (SSRF-vetted; required). */
  readonly serverHost: string;
  readonly description?: string;
  /** `Token` | `Readback` | `Staged` | `AtLeastOnce` (defaults to `Readback`). */
  readonly idempotencyClass?: string;
  /** The tool's name on the remote server (defaults to `name`). */
  readonly remoteName?: string;
  readonly params?: readonly ToolParam[];
  /** Refuse keys not in `params` (defaults to `true`). */
  readonly denyUnknownParams?: boolean;
}
