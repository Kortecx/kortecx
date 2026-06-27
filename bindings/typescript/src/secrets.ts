/**
 * The local OS-keychain secret store views (MM-3 / D110). A `SecretRef` NAME is
 * what a connection's / trigger's `credential_ref` points at; the secret VALUE is
 * write-only — it appears ONLY as a `PutSecret` argument and is never returned on
 * any read. `ListSecretNames` surfaces NAMES + audit timestamps only. Kept in its
 * own module (the `alerts.ts`/`toolscout.ts` module-per-concern precedent).
 *
 * SN-8: `PutSecret`/`DeleteSecret` write host credential material and are gated
 * loopback-only + an authenticated party server-side; the SDK only *carries* the
 * value to the handler and *encodes* nothing sensitive on a read.
 */

import type { SecretName as PbSecretName } from "./gen/kortecx/v1/gateway_pb.js";

/** One stored secret's NAME + audit timestamps (`ListSecretNames`). The VALUE is
 *  never on this wire (write-only). `createdUnixMs`/`updatedUnixMs` are audit-only
 *  wall-clocks (off every hash). */
export class SecretNameRow {
  constructor(
    readonly name: string,
    readonly createdUnixMs: number,
    readonly updatedUnixMs: number,
  ) {}

  static fromProto(s: PbSecretName): SecretNameRow {
    return new SecretNameRow(s.name, Number(s.createdUnixMs), Number(s.updatedUnixMs));
  }

  /** A plain snake_case object (stable wire-shaped serialization for UIs/logs). */
  toJSON() {
    return {
      name: this.name,
      created_unix_ms: this.createdUnixMs,
      updated_unix_ms: this.updatedUnixMs,
    };
  }
}

/** One `ListSecretNames` page (deterministic `(name)` order). */
export interface SecretNamesPage {
  readonly names: readonly SecretNameRow[];
  readonly hasMore: boolean;
}
