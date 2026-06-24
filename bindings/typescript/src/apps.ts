/**
 * POC-4 App-catalog views — a durable, reusable App (a `kortecx.app/v1` envelope:
 * a portable blueprint wrapped with by-reference references, a 4-axis steering
 * config, and per-step replay intent). Kept in its own module so `types.ts` stays
 * a thin aggregator (the Rust core's module-per-concern discipline, GR3).
 *
 * SN-8: `appRef` is SERVER-DERIVED (blake3 over the canonical envelope) — the
 * client names a handle, never an identity. The catalog lives in an off-journal
 * `apps.db` sidecar (rebuildable-to-empty), scoped to the authoring party; a
 * not-found / not-owned App is UNIFORM (no cross-party existence oracle). The
 * envelope carries NO authority — `runApp` re-compiles the blueprint and the server
 * re-resolves every warrant from the caller's own grants. PURE DATA (web-safe).
 */

import type {
  AppSummary as PbAppSummary,
  GetAppResponse as PbGetAppResponse,
  SaveAppResponse as PbSaveAppResponse,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** The envelope schema/version tag — readers fail closed on a mismatch. */
export const APP_SCHEMA = "kortecx.app/v1";

/** Recursively sort object keys (the canonical-JSON precondition). */
function sortValue(v: unknown): unknown {
  if (Array.isArray(v)) return v.map(sortValue);
  if (v && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const k of Object.keys(v as Record<string, unknown>).sort()) {
      out[k] = sortValue((v as Record<string, unknown>)[k]);
    }
    return out;
  }
  return v;
}

/**
 * The canonical envelope bytes (as a string): keys sorted, compact, UTF-8 —
 * byte-identical to the Rust `kx-app` serializer and the Python SDK (the
 * cross-surface gate, `tests/golden/apps/SPEC.md`).
 */
export function canonicalJson(envelope: unknown): string {
  return JSON.stringify(sortValue(envelope));
}

/** The human export form: pretty (2-space) + sorted keys + a trailing newline. */
export function prettyJson(envelope: unknown): string {
  return `${JSON.stringify(sortValue(envelope), null, 2)}\n`;
}

/**
 * Derive the default 3-segment catalog handle `apps/local/<sanitized>` from an App
 * name (mirrors the `kx app` CLI + the Python SDK). Lowercases, maps invalid chars
 * to `-`, trims, and falls back to `app`.
 */
export function defaultHandle(name: string): string {
  let san = "";
  for (const c of name) {
    if (/[a-z0-9._-]/.test(c)) san += c;
    else if (/[A-Z]/.test(c)) san += c.toLowerCase();
    else san += "-";
  }
  san = san
    .replace(/^[.-]+/, "")
    .replace(/[.-]+$/, "")
    .slice(0, 128);
  return `apps/local/${san || "app"}`;
}

/** A skill: a named (instructions + tool wish SET) bundle ≈ a reusable Agent. */
export interface Skill {
  name: string;
  /** A body uploaded at `save` (use this OR `instructionsRef`). */
  instructions?: string;
  /** A 64-hex content ref already in the store (use this OR `instructions`). */
  instructionsRef?: string;
  /** The skill's tool wish set (id → version). */
  tools?: Record<string, string>;
}

/** An App's catalog/display view (the envelope-derived summary + server id). */
export class AppSummary {
  constructor(
    readonly handle: string,
    /** Server-derived canonical-envelope hash, as hex (16 bytes ⇒ 32 hex chars). */
    readonly appRef: string,
    readonly name: string,
    readonly version: string,
    readonly description: string,
    readonly tags: string[],
    readonly stepCount: number,
    /** POC-5b: the App's project branch is locked (agentic in-CAS edits refused). */
    readonly locked: boolean,
  ) {}

  static fromProto(s: PbAppSummary): AppSummary {
    return new AppSummary(
      s.handle,
      encode(s.appRef),
      s.name,
      s.version,
      s.description,
      [...s.tags],
      s.stepCount,
      s.locked,
    );
  }
}

/** POC-5a: the live App-scaffold phase. */
export type ScaffoldPhase = "planning" | "writing" | "done" | "failed" | "unspecified";

/** POC-5a: the resolved scaffold status (phase + the done/pending skeleton files). */
export interface ScaffoldStatus {
  readonly phase: ScaffoldPhase;
  readonly filesDone: string[];
  readonly filesPending: string[];
  readonly detail: string;
}

/** Map the wire `GetScaffoldStatusResponse.Phase` enum to a stable name. */
export function scaffoldPhaseName(phase: number): ScaffoldPhase {
  switch (phase) {
    case 1:
      return "planning";
    case 2:
      return "writing";
    case 3:
      return "done";
    case 4:
      return "failed";
    default:
      return "unspecified";
  }
}

/** The outcome of a `SaveApp` upsert (server-derived ref + dedup signal). */
export class SaveAppResult {
  constructor(
    readonly appRef: string,
    readonly handle: string,
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbSaveAppResponse): SaveAppResult {
    return new SaveAppResult(encode(r.appRef), r.handle, r.deduplicated);
  }
}

/** A fetched App: its catalog summary + the parsed envelope (`GetApp`). */
export class StoredApp {
  constructor(
    readonly summary: AppSummary,
    readonly envelope: Record<string, unknown>,
  ) {}

  static fromProto(r: PbGetAppResponse): StoredApp {
    const envelope =
      r.envelopeJson.length > 0
        ? (JSON.parse(new TextDecoder().decode(r.envelopeJson)) as Record<string, unknown>)
        : {};
    const summary = r.summary
      ? AppSummary.fromProto(r.summary)
      : new AppSummary("", "", "", "", "", [], 0, false);
    return new StoredApp(summary, envelope);
  }
}
