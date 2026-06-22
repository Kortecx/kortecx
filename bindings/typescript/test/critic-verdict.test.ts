/**
 * T-AGENT2: decode a committed `CriticVerdict` (the `kx/recipes/judge` terminal).
 *
 * Pins the SDK decoder against the EXACT `kx_critic_types::CriticVerdict::encode`
 * byte layout (2-byte LE schema version ‖ fixed-int bincode). Pure unit test, no
 * serve. Mirrors the Python `decode_critic_verdict` + the CLI `critic_verdict_summary`.
 */

import { describe, expect, it } from "vitest";
import { decodeCriticVerdict } from "../src/critic.js";

// Canonical encodings (must match the Rust `CriticVerdict::encode` bytes the
// kx-critic-types `trailing_judge_variants_are_byte_neutral` test pins).
const VALID = new Uint8Array([1, 0, 0, 0, 0, 0]); // version 1 ‖ enum variant 0 (Valid)
// version 1 ‖ Invalid (variant 1) ‖ CriticReason JudgeRejected (variant 5) ‖ code u16
const INVALID_JUDGE_0 = new Uint8Array([1, 0, 1, 0, 0, 0, 5, 0, 0, 0, 0, 0]);
const INVALID_JUDGE_1 = new Uint8Array([1, 0, 1, 0, 0, 0, 5, 0, 0, 0, 1, 0]);

describe("decodeCriticVerdict", () => {
  it("decodes a VALID verdict", () => {
    expect(decodeCriticVerdict(VALID)).toBe("valid");
  });

  it("decodes INVALID judge reasons", () => {
    expect(decodeCriticVerdict(INVALID_JUDGE_0)).toBe(
      "invalid: judge: answer did not satisfy the rubric",
    );
    expect(decodeCriticVerdict(INVALID_JUDGE_1)).toBe(
      "invalid: judge: response was unparseable (fail-closed)",
    );
  });

  it("returns null for a non-verdict payload", () => {
    expect(decodeCriticVerdict(new TextEncoder().encode("Paris"))).toBeNull();
    expect(decodeCriticVerdict(new Uint8Array([]))).toBeNull();
    // wrong schema version
    expect(decodeCriticVerdict(new Uint8Array([2, 0, 0, 0, 0, 0]))).toBeNull();
  });
});
