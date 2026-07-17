#![no_main]
//! Fuzz `kx_toolcall::parse_tool_call` / `parse_tool_calls` — the single non-forkable tool-call
//! authority gate. It decodes UNTRUSTED model output into a warrant-granted call (SN-8: the model
//! cannot authorize a tool the runtime withheld) and documents itself as "Total + panic-free over
//! arbitrary `bytes`" (crates/kx-toolcall/src/parse.rs:913). A panic / OOM / hang is a finding.
//!
//! The gate early-returns `Ok(None)` under an EMPTY warrant, so a default (deny-all) warrant would
//! exercise nothing — this target grants one tool so the accept / refuse / oversize arms are all
//! reachable. Beyond the panic-free invariant it also asserts a ROUND-TRIP FIXPOINT: a parsed call
//! rebuilt into its canonical JSON envelope must re-parse to the identical call.
use libfuzzer_sys::fuzz_target;
use kx_mote::{ToolName, ToolVersion};
use kx_warrant::{ToolGrant, WarrantSpec};

// A fixed literal args cap. NOT derived via `max_args_bytes(&w)`: `WarrantSpec::default()` sets
// `max_output_tokens = 0`, which would force every call down the `Oversize` arm. 4096 is the value
// the crate's own parse tests use — small enough to keep the oversize-refusal arm reachable, large
// enough to admit real calls.
const ARGS_CAP: usize = 4096;

/// A non-empty warrant granting exactly one tool. The `mcp-echo/echo` full-id lets the parser reach
/// the bare-leaf / marked / native / paren name shapes (each resolves against the grant set).
fn granting_warrant() -> WarrantSpec {
    let mut w = WarrantSpec::default();
    w.tool_grants.insert(ToolGrant {
        tool_id: ToolName("mcp-echo/echo".into()),
        tool_version: ToolVersion("1".into()),
    });
    w
}

fuzz_target!(|data: &[u8]| {
    let warrant = granting_warrant();

    // (1) Panic-free invariant over both public gate fns — a panic / OOM / hang is the finding.
    let _ = kx_toolcall::parse_tool_calls(data, &warrant, ARGS_CAP);
    let parsed = kx_toolcall::parse_tool_call(data, &warrant, ARGS_CAP);

    // (2) Round-trip fixpoint: rebuild the CANONICAL `{"tool_call":…}` envelope from the parsed call
    // and assert it re-parses to the identical call. name/version are canonical after the first parse
    // and `args_bytes` is spliced VERBATIM (the envelope path carries an object arg byte-for-byte —
    // args_value_bytes/parse.rs:564 — matching the pinned `args_bytes_are_byte_identical…` invariant),
    // so the canonical envelope is a fixpoint of the parser.
    //
    // SOUND ONLY when `args_bytes` is a valid JSON OBJECT. The Gemma-native / paren arms brace-balance
    // args WITHOUT JSON-validating (resolve_native_call/parse.rs:725), so a native call can legitimately
    // hold non-JSON args (e.g. `{"x":=}`), and the envelope path can unescape a string arg to non-JSON
    // bytes — neither is representable in the JSON normal form, so those are excluded here (still fully
    // covered by the panic-free invariant above). Restricting to objects also guarantees the re-parse
    // takes args_value_bytes' verbatim `{`-branch, so a match is byte-exact, not shape-normalized.
    if let Ok(Some(call)) = parsed {
        let args_is_json_object = serde_json::from_slice::<serde_json::Value>(&call.args_bytes)
            .map(|v| v.is_object())
            .unwrap_or(false);
        if args_is_json_object {
            // A valid JSON object is valid UTF-8, so this never fails; kept as a guard.
            if let Ok(args) = std::str::from_utf8(&call.args_bytes) {
                let name =
                    serde_json::to_string(&call.name.0).expect("string is always serializable");
                let version =
                    serde_json::to_string(&call.version.0).expect("string is always serializable");
                let envelope = format!(
                    r#"{{"tool_call":{{"name":{name},"version":{version},"args":{args}}}}}"#
                );
                let again = kx_toolcall::parse_tool_call(envelope.as_bytes(), &warrant, ARGS_CAP);
                assert!(
                    matches!(&again, Ok(Some(c)) if *c == call),
                    "round-trip fixpoint broke: reparsing the canonical envelope of {call:?} gave {again:?}"
                );
            }
        }
    }
});
