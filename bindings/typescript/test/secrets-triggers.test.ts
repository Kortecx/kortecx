/**
 * MM-3 secrets + D113 triggers friendly views — pure, no server. Mirrors the
 * `toolscout.test.ts` connections precedent: the proto→friendly mappers + the
 * friendly enum unions (SN-8: the SDK only *encodes* server bytes to hex; secret
 * VALUES are write-only and never on a read wire).
 */

import { create } from "@bufbuild/protobuf";
import { describe, expect, it } from "vitest";
import {
  SecretNameSchema,
  TriggerAuth,
  TriggerKind,
  TriggerViewSchema,
} from "../src/gen/kortecx/v1/gateway_pb.js";
import { decodeFixed } from "../src/hexids.js";
import { SecretNameRow } from "../src/secrets.js";
import {
  TriggerRow,
  triggerAuthName,
  triggerAuthToProto,
  triggerKindName,
  triggerKindToProto,
} from "../src/triggers.js";

/** A deterministic 16-byte trigger id (00 01 … 0f) and its hex form. */
const TID = Uint8Array.from({ length: 16 }, (_, i) => i);
const TID_HEX = "000102030405060708090a0b0c0d0e0f";

describe("SecretNameRow.fromProto", () => {
  it("maps name + audit timestamps (bigint → number; never a value)", () => {
    const row = SecretNameRow.fromProto(
      create(SecretNameSchema, {
        name: "github-token",
        createdUnixMs: 1700000000000n,
        updatedUnixMs: 1700000999000n,
      }),
    );
    expect(row).toBeInstanceOf(SecretNameRow);
    expect(row.name).toBe("github-token");
    expect(row.createdUnixMs).toBe(1700000000000);
    expect(row.updatedUnixMs).toBe(1700000999000);
    expect(row.toJSON()).toEqual({
      name: "github-token",
      created_unix_ms: 1700000000000,
      updated_unix_ms: 1700000999000,
    });
  });
});

describe("trigger enum mapping", () => {
  it("maps every friendly kind to the proto enum and back", () => {
    expect(triggerKindToProto("webhook")).toBe(TriggerKind.WEBHOOK);
    expect(triggerKindToProto("cron")).toBe(TriggerKind.CRON);
    expect(triggerKindToProto("grpc")).toBe(TriggerKind.GRPC);
    expect(triggerKindName(TriggerKind.WEBHOOK)).toBe("webhook");
    expect(triggerKindName(TriggerKind.CRON)).toBe("cron");
    expect(triggerKindName(TriggerKind.GRPC)).toBe("grpc");
    // unknown absorbs UNSPECIFIED(0) + any future value
    expect(triggerKindName(TriggerKind.TRIGGER_KIND_UNSPECIFIED)).toBe("unknown");
    expect(triggerKindName(99)).toBe("unknown");
  });

  it("maps every friendly auth posture to the proto enum and back", () => {
    expect(triggerAuthToProto("none")).toBe(TriggerAuth.NONE);
    expect(triggerAuthToProto("hmac_sha256")).toBe(TriggerAuth.HMAC_SHA256);
    expect(triggerAuthToProto("bearer")).toBe(TriggerAuth.BEARER);
    expect(triggerAuthName(TriggerAuth.NONE)).toBe("none");
    expect(triggerAuthName(TriggerAuth.HMAC_SHA256)).toBe("hmac_sha256");
    expect(triggerAuthName(TriggerAuth.BEARER)).toBe("bearer");
    expect(triggerAuthName(TriggerAuth.TRIGGER_AUTH_UNSPECIFIED)).toBe("unknown");
    expect(triggerAuthName(99)).toBe("unknown");
  });
});

describe("TriggerRow.fromProto", () => {
  it("maps the governance row (id → hex, enums → unions, never a secret value)", () => {
    const row = TriggerRow.fromProto(
      create(TriggerViewSchema, {
        triggerId: TID,
        name: "gh-push",
        kind: TriggerKind.WEBHOOK,
        recipeHandle: "kx/recipes/react",
        auth: TriggerAuth.HMAC_SHA256,
        authSecretPresent: true,
        scheduleSpec: "",
        enabled: true,
        lastFireUnixMs: 1700001234000n,
      }),
    );
    expect(row).toBeInstanceOf(TriggerRow);
    expect(row.triggerId).toBe(TID_HEX);
    expect(row.triggerId).toHaveLength(32);
    expect(row.kind).toBe("webhook");
    expect(row.auth).toBe("hmac_sha256");
    expect(row.authSecretPresent).toBe(true);
    expect(row.recipeHandle).toBe("kx/recipes/react");
    expect(row.enabled).toBe(true);
    expect(row.lastFireUnixMs).toBe(1700001234000);
    // The hex id round-trips back to the server bytes.
    expect(decodeFixed(row.triggerId, 16)).toEqual(TID);
    expect(row.toJSON()).toEqual({
      trigger_id: TID_HEX,
      name: "gh-push",
      kind: "webhook",
      recipe_handle: "kx/recipes/react",
      auth: "hmac_sha256",
      auth_secret_present: true,
      schedule_spec: "",
      enabled: true,
      last_fire_unix_ms: 1700001234000,
    });
  });

  it("maps a never-fired cron row (lastFireUnixMs 0)", () => {
    const row = TriggerRow.fromProto(
      create(TriggerViewSchema, {
        triggerId: TID,
        name: "nightly",
        kind: TriggerKind.CRON,
        recipeHandle: "kx/recipes/echo",
        auth: TriggerAuth.NONE,
        scheduleSpec: "86400",
        enabled: false,
        lastFireUnixMs: 0n,
      }),
    );
    expect(row.kind).toBe("cron");
    expect(row.auth).toBe("none");
    expect(row.scheduleSpec).toBe("86400");
    expect(row.enabled).toBe(false);
    expect(row.lastFireUnixMs).toBe(0);
  });
});
