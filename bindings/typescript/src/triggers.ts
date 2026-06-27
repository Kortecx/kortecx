/**
 * The local trigger admin views (D113 / D170.b). A trigger binds an inbound EVENT
 * (a webhook POST, a cron interval, or a bare `SubmitTrigger` RPC) to a recipe
 * handle the event Invokes. The minimal-local single-user trigger; the hosted
 * multi-tenant trigger gateway at scale is CLOUD (GR19). Kept in its own module
 * (the `alerts.ts`/`toolscout.ts` module-per-concern precedent).
 *
 * SN-8: `triggerId`/`instanceId` are server-derived (the SDK only *encodes* the
 * bytes to hex); the auth secret is referenced by NAME only (never the value, a
 * `ListTriggers` row carries `authSecretPresent`, never the secret itself).
 */

import {
  type TriggerView as PbTriggerView,
  TriggerAuth,
  TriggerKind,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** A registrable trigger kind (the kinds a client may create). */
export type TriggerKindInput = "webhook" | "cron" | "grpc";

/** A trigger kind as surfaced on a read. `"unknown"` absorbs UNSPECIFIED(0) + any
 *  future wire value (forward-compatible display). */
export type TriggerKindName = TriggerKindInput | "unknown";

/** A webhook auth posture a client may set. */
export type TriggerAuthInput = "none" | "hmac_sha256" | "bearer";

/** A trigger auth posture as surfaced on a read. `"unknown"` absorbs UNSPECIFIED(0)
 *  + any future wire value (forward-compatible display). */
export type TriggerAuthName = TriggerAuthInput | "unknown";

/** Map a friendly kind to the proto enum (closed input set). */
export function triggerKindToProto(kind: TriggerKindInput): TriggerKind {
  switch (kind) {
    case "webhook":
      return TriggerKind.WEBHOOK;
    case "cron":
      return TriggerKind.CRON;
    case "grpc":
      return TriggerKind.GRPC;
  }
}

/** Map a `TriggerKind` discriminant to a stable name (`"unknown"` if new). */
export function triggerKindName(kind: number): TriggerKindName {
  if (kind === TriggerKind.WEBHOOK) return "webhook";
  if (kind === TriggerKind.CRON) return "cron";
  if (kind === TriggerKind.GRPC) return "grpc";
  return "unknown";
}

/** Map a friendly auth posture to the proto enum (closed input set). */
export function triggerAuthToProto(auth: TriggerAuthInput): TriggerAuth {
  switch (auth) {
    case "none":
      return TriggerAuth.NONE;
    case "hmac_sha256":
      return TriggerAuth.HMAC_SHA256;
    case "bearer":
      return TriggerAuth.BEARER;
  }
}

/** Map a `TriggerAuth` discriminant to a stable name (`"unknown"` if new). */
export function triggerAuthName(auth: number): TriggerAuthName {
  if (auth === TriggerAuth.NONE) return "none";
  if (auth === TriggerAuth.HMAC_SHA256) return "hmac_sha256";
  if (auth === TriggerAuth.BEARER) return "bearer";
  return "unknown";
}

/** A `RegisterTrigger` request shape. The auth secret is referenced by NAME only
 *  (never the value, D81). The server derives the trigger id (SN-8). */
export interface RegisterTriggerInput {
  /** Unique operator handle (derives the trigger id). */
  readonly name: string;
  /** `"webhook"` | `"cron"` | `"grpc"`. */
  readonly kind: TriggerKindInput;
  /** The `kx/recipes/...` handle the event Invokes. */
  readonly recipeHandle: string;
  /** Webhook auth posture (defaults to `"none"`). */
  readonly auth?: TriggerAuthInput;
  /** SecretRef NAME of the HMAC/bearer secret (never the value; defaults to none). */
  readonly authSecretRef?: string;
  /** cron: interval seconds (e.g. `"300"`); empty otherwise. */
  readonly scheduleSpec?: string;
  /** Defaults to `true`. */
  readonly enabled?: boolean;
}

/** The outcome of `registerTrigger` — the server-derived trigger id (hex). */
export interface RegisterTriggerResult {
  /** 16-byte server-derived trigger id, as lowercase hex. */
  readonly triggerId: string;
}

/** One governance row (`ListTriggers`). Never a secret value — `authSecretPresent`
 *  reports only whether a ref NAME is attached. */
export class TriggerRow {
  constructor(
    /** Server-derived trigger id, as lowercase hex. */
    readonly triggerId: string,
    readonly name: string,
    /** `"webhook"` | `"cron"` | `"grpc"` (`"unknown"` absorbs any future value). */
    readonly kind: TriggerKindName,
    readonly recipeHandle: string,
    /** `"none"` | `"hmac_sha256"` | `"bearer"` (`"unknown"` absorbs any future value). */
    readonly auth: TriggerAuthName,
    /** A ref NAME is attached (never the value). */
    readonly authSecretPresent: boolean,
    readonly scheduleSpec: string,
    readonly enabled: boolean,
    /** Audit-only wall-clock; `0` ⇒ never fired. */
    readonly lastFireUnixMs: number,
  ) {}

  static fromProto(t: PbTriggerView): TriggerRow {
    return new TriggerRow(
      encode(t.triggerId),
      t.name,
      triggerKindName(t.kind),
      t.recipeHandle,
      triggerAuthName(t.auth),
      t.authSecretPresent,
      t.scheduleSpec,
      t.enabled,
      Number(t.lastFireUnixMs),
    );
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      trigger_id: this.triggerId,
      name: this.name,
      kind: this.kind,
      recipe_handle: this.recipeHandle,
      auth: this.auth,
      auth_secret_present: this.authSecretPresent,
      schedule_spec: this.scheduleSpec,
      enabled: this.enabled,
      last_fire_unix_ms: this.lastFireUnixMs,
    };
  }
}

/** One `ListTriggers` page (deterministic `(name)` order). */
export interface TriggersPage {
  readonly triggers: readonly TriggerRow[];
  readonly hasMore: boolean;
}

/** The outcome of `submitTrigger` (the inbound EVENT verb) — the registered run
 *  (the PRIOR run when deduped) + whether a prior identical event already fired. */
export interface SubmitTriggerResult {
  /** 16-byte server-derived run instance id, as lowercase hex. */
  readonly instanceId: string;
  /** `true` ⇒ a prior identical event already started this run. */
  readonly deduped: boolean;
}

/** The outcome of `testTrigger` (a dry-run binding validation, fires nothing). */
export interface TestTriggerResult {
  readonly ok: boolean;
  /** Display-only validation prose (empty on success). */
  readonly detail: string;
}
