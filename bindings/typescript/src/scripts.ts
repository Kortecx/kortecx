/**
 * The declarative SCRIPT registry — `RegisterScript` / `DeregisterScript` /
 * `ListScripts` / `GetScript` (the `alerts.ts` / `toolscout.ts` module-per-concern
 * precedent).
 *
 * A script is a named, versioned program the operator registers once and the
 * runtime fires as an ordinary tool: the same registry, the same
 * `(name, version)` grant key, the same broker. This module exists because a
 * script's DECLARATION differs — source bytes, an interpreter, a resource wish —
 * not because its authority does.
 *
 * **Registration grants no authority.** The declared wish becomes the tool's
 * requirement, and the runtime still refuses any call whose requirement is not a
 * subset of the calling warrant. A script that wants more than its caller has is
 * refused before it runs; the same script under a sufficient grant fires. The
 * client supplies no warrant and names no id — `scriptId` is server-derived.
 *
 * **A model controls only `input`.** `argv` and `env` are fixed here, at
 * registration, and the sandboxed child's environment is cleared rather than
 * filtered.
 *
 * **Scripts run sandboxed or not at all.** A serve with no sandbox available, or
 * without the declared interpreter, refuses the registration
 * (`failed_precondition`) rather than admitting a script it could only run on the
 * host.
 */

import type {
  RegisteredScript as PbRegisteredScript,
  ScriptEnv as PbScriptEnv,
  ScriptMount as PbScriptMount,
} from "./gen/kortecx/v1/gateway_pb.js";
import { encode } from "./hexids.js";

/** One declared filesystem mount. */
export interface ScriptMount {
  /** Absolute path. */
  path: string;
  /** What the script wants to do there. */
  mode: "ro" | "rw" | "exec";
}

/** One environment pair fixed at registration. Never model-controlled. */
export interface ScriptEnvPair {
  key: string;
  value: string;
}

/** A `RegisterScript` request. */
export interface RegisterScriptInput {
  /** Identity half — the grant-set key. */
  name: string;
  /** Identity half. */
  version: string;
  /** Which interpreter runs the source. The serve validates this against its own
   * closed allowlist and refuses anything else. */
  interpreter: "sh" | "python3" | "node";
  /** The script's source. */
  source: string | Uint8Array;
  /** Free-form; shown to a model in its tool menu, never parsed for enforcement. */
  description?: string;
  /** Fixed arguments appended after the script. NOT model-controlled. */
  argv?: readonly string[];
  /** Fixed environment. NOT model-controlled; omitted ⇒ no environment at all. */
  env?: readonly ScriptEnvPair[];
  /** The filesystem the script DECLARES it needs. Granting is a separate act. */
  fsMounts?: readonly ScriptMount[];
  /** The hosts the script DECLARES it needs. Omitted ⇒ no egress. */
  netHosts?: readonly string[];
  /** Wall-clock budget in ms (omitted ⇒ the serve's default). */
  wallClockMs?: number;
  /** Memory ceiling in bytes (omitted ⇒ unset). */
  memBytes?: number;
  /** Output ceiling in bytes (omitted ⇒ the serve's default). Exceeding it
   * REFUSES the call rather than truncating: a truncated result reads as a
   * complete answer and the caller cannot tell. */
  maxOutputBytes?: number;
}

/**
 * One registered script's inventory row. Every scope field is a DISPLAY summary;
 * authority never rides this wire.
 */
export class RegisteredScript {
  constructor(
    /** 16-byte server-derived id, as lowercase hex. */
    readonly scriptId: string,
    readonly scriptName: string,
    readonly scriptVersion: string,
    /** `"sh"` | `"python3"` | `"node"`. */
    readonly interpreter: string,
    readonly description: string,
    /** Content ref of the EXACT source bytes that run, as lowercase hex. */
    readonly sourceRef: string,
    /** Display: `"none"` | `"ro:/a,rw:/b"`. */
    readonly fsScope: string,
    /** Display: `"none"` | `"egress:host[,host]"`. */
    readonly netScope: string,
    readonly wallClockMs: number,
    readonly maxOutputBytes: number,
  ) {}

  static fromProto(s: PbRegisteredScript): RegisteredScript {
    return new RegisteredScript(
      encode(s.scriptId),
      s.scriptName,
      s.scriptVersion,
      s.interpreter,
      s.description,
      s.sourceRefHex,
      s.fsScopeSummary,
      s.netScopeSummary,
      Number(s.wallClockMs),
      Number(s.maxOutputBytes),
    );
  }
}

/** One page of the script registry, in `(name, version)` order. */
export interface RegisteredScriptsPage {
  scripts: RegisteredScript[];
  hasMore: boolean;
}

/** A script's row plus its registered source. */
export interface ScriptWithSource {
  script: RegisteredScript;
  source: string;
}

/** Encode a source that may be given as text or bytes. */
export function scriptSourceBytes(source: string | Uint8Array): Uint8Array {
  return typeof source === "string" ? new TextEncoder().encode(source) : source;
}

/** Project the declared mounts onto the wire shape. */
export function scriptMountsToProto(mounts: readonly ScriptMount[] | undefined): PbScriptMount[] {
  return (mounts ?? []).map((m) => ({ path: m.path, mode: m.mode }) as PbScriptMount);
}

/** Project the fixed environment onto the wire shape. */
export function scriptEnvToProto(env: readonly ScriptEnvPair[] | undefined): PbScriptEnv[] {
  return (env ?? []).map((e) => ({ key: e.key, value: e.value }) as PbScriptEnv);
}
