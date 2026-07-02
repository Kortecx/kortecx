/**
 * RC-SW1 skill-catalog views — a declarative `kortecx.skill/v1` bundle
 * (instructions + a tool grant-WISH set). Kept in its own module (the Rust
 * core's module-per-concern discipline, GR3).
 *
 * SN-8: `skillRef` and `instructionsRef` are SERVER-DERIVED (blake3 over the
 * canonical manifest / the stored body) — the client sends bytes, never an
 * identity. A wish is never authority: attaching a skill grants nothing; at
 * `runApp` the server intersects the wish against the caller's grants and the
 * live broker (`wish ∩ grants ∩ fireable`). The catalog lives in an
 * off-journal `skills.db` sidecar (rebuildable-to-empty), caller-scoped, with
 * UNIFORM not-found. PURE DATA (web-safe).
 */

import type {
  AddSkillResponse as PbAddSkillResponse,
  GetSkillFormResponse as PbGetSkillFormResponse,
  SkillSummary as PbSkillSummary,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** The manifest schema/version tag — readers fail closed on a mismatch. */
export const SKILL_SCHEMA = "kortecx.skill/v1";

/** The input to `client.skills.add` — a pack (manifest + body) or a stored-form manifest. */
export interface AddSkillInput {
  /** The `kortecx.skill/v1` manifest object (pack form: NO `instructions_ref`). */
  manifest: Record<string, unknown>;
  /**
   * The instructions markdown body (pack form). Omit iff the manifest already
   * names a 64-hex `instructions_ref` (stored form).
   */
  instructions?: string;
}

/** A stored skill's catalog/display view (the manifest-derived summary + server id). */
export class SkillSummary {
  constructor(
    /** Server-derived canonical-manifest hash, as hex (16 bytes ⇒ 32 hex chars). */
    readonly skillRef: string,
    readonly name: string,
    readonly version: string,
    readonly description: string,
    /** 64-hex content-store ref to the instructions body. */
    readonly instructionsRef: string,
    /** The tool grant-WISH set (id → version); a wish, never a grant. */
    readonly tools: Record<string, string>,
    readonly tags: string[],
  ) {}

  static fromProto(s: PbSkillSummary): SkillSummary {
    return new SkillSummary(
      encode(s.skillRef),
      s.name,
      s.version,
      s.description,
      s.instructionsRef,
      { ...s.tools },
      [...s.tags],
    );
  }
}

/** The outcome of an `AddSkill` upsert (server-derived refs + dedup signal). */
export class AddSkillResult {
  constructor(
    readonly skillRef: string,
    readonly name: string,
    readonly instructionsRef: string,
    readonly deduplicated: boolean,
  ) {}

  static fromProto(r: PbAddSkillResponse): AddSkillResult {
    return new AddSkillResult(encode(r.skillRef), r.name, r.instructionsRef, r.deduplicated);
  }
}

/** One wished tool with the ADVISORY `registered` bit (display only, never a grant). */
export interface SkillWish {
  readonly toolId: string;
  readonly toolVersion: string;
  /** Could THIS serve currently fire it (registered/dialed)? Advisory display. */
  readonly registered: boolean;
}

/** The `GetSkillForm` view: summary + wishes + the instructions preview. */
export class SkillForm {
  constructor(
    readonly summary: SkillSummary,
    readonly wishes: SkillWish[],
    /** Server-capped display excerpt (`''` when the skill was added by ref). */
    readonly instructionsPreview: string,
    readonly previewTruncated: boolean,
  ) {}

  static fromProto(r: PbGetSkillFormResponse): SkillForm | null {
    if (!r.found || !r.summary) return null;
    return new SkillForm(
      SkillSummary.fromProto(r.summary),
      r.wishes.map((w) => ({
        toolId: w.toolId,
        toolVersion: w.toolVersion,
        registered: w.registered,
      })),
      r.instructionsPreview,
      r.previewTruncated,
    );
  }
}
