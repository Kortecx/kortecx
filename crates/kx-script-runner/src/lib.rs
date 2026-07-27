//! The sandboxed script shim's descriptor codec + result-ref contract.
//!
//! ## Why this crate exists
//!
//! The platform sandbox backends (`bwrap` on Linux, `sandbox-exec`/Seatbelt on
//! macOS) spawn a *body binary* and read a single 64-character lowercase-hex
//! content ref from its stdout — that contract is what lets the runtime verify a
//! body's result against the content store before committing it. It is also
//! exactly what an arbitrary user script cannot honour: a script prints whatever
//! it prints.
//!
//! The bundled `kx-script-runner` binary closes that gap without widening the
//! executor's contract. It IS the body the sandbox spawns. Inside the
//! already-applied sandbox it:
//!
//! 1. reads a [`ScriptDescriptor`] from the path given as `argv[1]`;
//! 2. runs `<interpreter> <script> <args…>` with a **cleared** environment;
//! 3. writes the child's raw stdout to the descriptor's `out_path` — a directory
//!    the warrant mounted read-write;
//! 4. prints `hex(BLAKE3(`[`SCRIPT_RESULT_PREFIX`]` ‖ output))` on its own
//!    stdout, which is the ref the backend parses.
//!
//! The host then reads `out_path`, puts those bytes into the content store, and
//! asserts the resulting ref equals what the shim printed. A truncated,
//! substituted, or partially written result cannot survive that check — so the
//! integrity of a script's result does not rest on this binary being correct,
//! only on BLAKE3 being collision-resistant.
//!
//! ## Why the codec lives here
//!
//! The descriptor is written by the serve and read by this crate's binary. Two
//! hand-rolled copies of a wire format drift, and a drifted copy fails as a
//! confusing decode error at the far end of a sandbox boundary. One definition,
//! both ends: the serve encodes with [`ScriptDescriptor::encode`], the shim
//! decodes with [`ScriptDescriptor::decode`].
//!
//! The format is fixed and length-prefixed rather than JSON because every crate
//! this binary links is parsing surface reachable from inside the sandbox.

use std::fmt;

/// Domain-separation prefix for a script's result object.
///
/// The object a script run commits is `SCRIPT_RESULT_PREFIX ‖ stdout`, and its
/// content ref is the BLAKE3 of that concatenation ([`result_ref_bytes`]). The
/// prefix keeps a script's output from colliding with the same bytes stored by
/// any other producer, so a ref alone identifies both the value and the fact
/// that a sandboxed script produced it.
pub const SCRIPT_RESULT_PREFIX: &[u8] = b"kx-script-runner-result";

/// Descriptor file magic + schema version. A file that does not start with these
/// exact bytes is refused before any length is read.
const MAGIC: &[u8; 8] = b"KXSCRPT1";

/// Hard ceiling on any single length-prefixed field, applied at decode.
///
/// The descriptor is written by the serve, so this is not a trust boundary — it
/// is a corruption backstop, so a garbled file allocates nothing large before
/// failing.
const MAX_FIELD_LEN: usize = 8 * 1024 * 1024;

/// Hard ceiling on the number of argv entries or environment pairs.
const MAX_VEC_LEN: usize = 4096;

/// What the shim should run, and where to put the result.
///
/// Every path is absolute and server-chosen. The environment is explicit and
/// **complete**: the shim clears its own environment and sets exactly these
/// pairs, so nothing the serve happens to be holding — a credential, an API
/// endpoint, an operator knob — is inherited by a script. An empty `env` (the
/// default) means the script runs with no environment at all.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScriptDescriptor {
    /// Absolute path to the interpreter binary, resolved by the host at
    /// registration and mounted execute-only by the warrant.
    pub interpreter_path: String,
    /// Absolute path to the script's source, materialized from the content
    /// store and mounted read-only by the warrant.
    pub script_path: String,
    /// Absolute path the shim writes the child's raw stdout to. Its parent
    /// directory is the only read-write mount in the warrant.
    pub out_path: String,
    /// Arguments appended after the script path.
    pub argv: Vec<String>,
    /// Bytes written to the child's stdin, which is then closed.
    pub stdin_bytes: Vec<u8>,
    /// The child's complete environment (see the type docs). Empty ⇒ none.
    pub env: Vec<(String, String)>,
    /// Wall-clock budget in milliseconds. The shim stops the script when it is
    /// exceeded. 0 ⇒ no budget.
    ///
    /// Enforced HERE, not by the caller alone, because the shim is the
    /// interpreter's direct parent: it can stop precisely the process that
    /// overran, while an outer deadline can only kill the whole sandbox.
    pub wall_clock_ms: u64,
    /// Address-space ceiling in bytes, applied to the shim before it execs so the
    /// interpreter inherits it. 0 ⇒ unset.
    pub mem_bytes: u64,
    /// Refuse — never truncate — once the child's stdout exceeds this many
    /// bytes.
    ///
    /// A truncated result is worse than no result: it reads as a complete
    /// answer and the agent has no way to tell. Overflow is a hard failure.
    pub max_output_bytes: u64,
}

/// Why a descriptor could not be decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DescriptorError {
    /// The file did not begin with the expected magic + version bytes.
    BadMagic,
    /// The input ended in the middle of a field.
    Truncated {
        /// What the decoder was reading when the input ran out.
        field: &'static str,
    },
    /// A length prefix exceeded its ceiling (a corruption backstop).
    FieldTooLarge {
        /// What the decoder was reading.
        field: &'static str,
        /// The length the prefix claimed.
        len: usize,
    },
    /// A field that must be UTF-8 was not.
    NotUtf8 {
        /// What the decoder was reading.
        field: &'static str,
    },
    /// Bytes remained after the last declared field.
    TrailingBytes {
        /// How many bytes were left over.
        len: usize,
    },
}

impl fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMagic => write!(f, "not a script descriptor (bad magic or version)"),
            Self::Truncated { field } => write!(f, "descriptor truncated while reading {field}"),
            Self::FieldTooLarge { field, len } => {
                write!(f, "descriptor field {field} too large ({len} bytes)")
            }
            Self::NotUtf8 { field } => write!(f, "descriptor field {field} is not valid UTF-8"),
            Self::TrailingBytes { len } => write!(f, "descriptor has {len} trailing bytes"),
        }
    }
}

impl std::error::Error for DescriptorError {}

impl ScriptDescriptor {
    /// Serialize to the fixed length-prefixed schema the shim decodes.
    ///
    /// Total, and deterministic in the descriptor's fields: the same descriptor
    /// always encodes to the same bytes, so a run's descriptor is itself
    /// content-addressable.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(MAGIC);
        put_str(&mut out, &self.interpreter_path);
        put_str(&mut out, &self.script_path);
        put_str(&mut out, &self.out_path);
        put_bytes(&mut out, &self.stdin_bytes);
        out.extend_from_slice(&self.max_output_bytes.to_le_bytes());
        out.extend_from_slice(&self.wall_clock_ms.to_le_bytes());
        out.extend_from_slice(&self.mem_bytes.to_le_bytes());
        put_len(&mut out, self.argv.len());
        for arg in &self.argv {
            put_str(&mut out, arg);
        }
        put_len(&mut out, self.env.len());
        for (key, value) in &self.env {
            put_str(&mut out, key);
            put_str(&mut out, value);
        }
        out
    }

    /// Decode a descriptor, fail-closed.
    ///
    /// Every length is checked against the bytes actually remaining before a
    /// single byte is copied, and trailing bytes are an error rather than
    /// ignored — a descriptor that does not decode exactly is not run.
    ///
    /// # Errors
    ///
    /// [`DescriptorError`] when the magic does not match, the input is
    /// truncated, a length prefix exceeds its ceiling, a string field is not
    /// UTF-8, or bytes remain after the final field.
    pub fn decode(bytes: &[u8]) -> Result<Self, DescriptorError> {
        let mut cur = Cursor { bytes, at: 0 };
        let magic = cur.take(MAGIC.len(), "magic")?;
        if magic != MAGIC {
            return Err(DescriptorError::BadMagic);
        }
        let interpreter_path = cur.take_str("interpreter_path")?;
        let script_path = cur.take_str("script_path")?;
        let out_path = cur.take_str("out_path")?;
        let stdin_bytes = cur.take_bytes("stdin_bytes")?.to_vec();
        let max_output_bytes = cur.take_u64("max_output_bytes")?;
        let wall_clock_ms = cur.take_u64("wall_clock_ms")?;
        let mem_bytes = cur.take_u64("mem_bytes")?;

        let arg_count = cur.take_len("argv_len")?;
        let mut argv = Vec::with_capacity(arg_count);
        for _ in 0..arg_count {
            argv.push(cur.take_str("argv")?);
        }
        let env_count = cur.take_len("env_len")?;
        let mut env = Vec::with_capacity(env_count);
        for _ in 0..env_count {
            let key = cur.take_str("env_key")?;
            let value = cur.take_str("env_value")?;
            env.push((key, value));
        }
        let left = bytes.len() - cur.at;
        if left != 0 {
            return Err(DescriptorError::TrailingBytes { len: left });
        }
        Ok(Self {
            interpreter_path,
            script_path,
            out_path,
            argv,
            stdin_bytes,
            env,
            wall_clock_ms,
            mem_bytes,
            max_output_bytes,
        })
    }
}

/// The content ref of a script's result object — `BLAKE3(prefix ‖ output)`.
///
/// The host reconstructs the same object from the bytes it reads back and
/// compares; this is the single definition both sides use.
#[must_use]
pub fn result_ref_bytes(output: &[u8]) -> [u8; 32] {
    let mut hasher = blake3::Hasher::new();
    hasher.update(SCRIPT_RESULT_PREFIX);
    hasher.update(output);
    *hasher.finalize().as_bytes()
}

/// Render 32 bytes as the 64 lowercase-hex characters the sandbox backends parse.
#[must_use]
pub fn hex32(bytes: &[u8; 32]) -> String {
    const NIBBLES: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(64);
    for byte in bytes {
        out.push(char::from(NIBBLES[usize::from(byte >> 4)]));
        out.push(char::from(NIBBLES[usize::from(byte & 0x0F)]));
    }
    out
}

// ---------------------------------------------------------------------------
// codec internals
// ---------------------------------------------------------------------------

/// Append a `u32` length prefix. Lengths are bounded by [`MAX_FIELD_LEN`] /
/// [`MAX_VEC_LEN`] at decode; a value that cannot fit encodes saturated, and the
/// decoder then rejects it rather than reading a wrapped length.
fn put_len(out: &mut Vec<u8>, len: usize) {
    let len = u32::try_from(len).unwrap_or(u32::MAX);
    out.extend_from_slice(&len.to_le_bytes());
}

fn put_bytes(out: &mut Vec<u8>, bytes: &[u8]) {
    put_len(out, bytes.len());
    out.extend_from_slice(bytes);
}

fn put_str(out: &mut Vec<u8>, s: &str) {
    put_bytes(out, s.as_bytes());
}

/// A bounds-checked forward reader over the descriptor bytes.
struct Cursor<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, len: usize, field: &'static str) -> Result<&'a [u8], DescriptorError> {
        let end = self
            .at
            .checked_add(len)
            .ok_or(DescriptorError::FieldTooLarge { field, len })?;
        if end > self.bytes.len() {
            return Err(DescriptorError::Truncated { field });
        }
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }

    fn take_u32(&mut self, field: &'static str) -> Result<u32, DescriptorError> {
        let raw = self.take(4, field)?;
        let mut buf = [0u8; 4];
        buf.copy_from_slice(raw);
        Ok(u32::from_le_bytes(buf))
    }

    fn take_u64(&mut self, field: &'static str) -> Result<u64, DescriptorError> {
        let raw = self.take(8, field)?;
        let mut buf = [0u8; 8];
        buf.copy_from_slice(raw);
        Ok(u64::from_le_bytes(buf))
    }

    /// A length prefix bounded by [`MAX_VEC_LEN`] (element counts).
    fn take_len(&mut self, field: &'static str) -> Result<usize, DescriptorError> {
        let len = self.take_u32(field)? as usize;
        if len > MAX_VEC_LEN {
            return Err(DescriptorError::FieldTooLarge { field, len });
        }
        Ok(len)
    }

    /// A length-prefixed blob bounded by [`MAX_FIELD_LEN`].
    fn take_bytes(&mut self, field: &'static str) -> Result<&'a [u8], DescriptorError> {
        let len = self.take_u32(field)? as usize;
        if len > MAX_FIELD_LEN {
            return Err(DescriptorError::FieldTooLarge { field, len });
        }
        self.take(len, field)
    }

    fn take_str(&mut self, field: &'static str) -> Result<String, DescriptorError> {
        let raw = self.take_bytes(field)?;
        std::str::from_utf8(raw)
            .map(str::to_owned)
            .map_err(|_| DescriptorError::NotUtf8 { field })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ScriptDescriptor {
        ScriptDescriptor {
            interpreter_path: "/usr/bin/python3".into(),
            script_path: "/tmp/kx/script.py".into(),
            out_path: "/tmp/kx/out/result.bin".into(),
            argv: vec!["--mode".into(), "summary".into()],
            stdin_bytes: b"{\"k\":1}".to_vec(),
            env: vec![("LANG".into(), "C".into())],
            max_output_bytes: 1024,
            wall_clock_ms: 5_000,
            mem_bytes: 256 * 1024 * 1024,
        }
    }

    #[test]
    fn round_trips_every_field() {
        let want = sample();
        let got = ScriptDescriptor::decode(&want.encode());
        assert_eq!(got, Ok(want));
    }

    #[test]
    fn round_trips_the_empty_descriptor() {
        let want = ScriptDescriptor::default();
        let got = ScriptDescriptor::decode(&want.encode());
        assert_eq!(got, Ok(want));
    }

    /// The encode is a pure function of the fields — the serve relies on this to
    /// keep a run's descriptor content-addressable.
    #[test]
    fn encoding_is_deterministic() {
        assert_eq!(sample().encode(), sample().encode());
    }

    /// Distinct descriptors must not encode alike, or two different script runs
    /// would share a descriptor ref.
    #[test]
    fn a_changed_field_changes_the_bytes() {
        let mut other = sample();
        other.argv.push("extra".into());
        assert_ne!(sample().encode(), other.encode());
    }

    #[test]
    fn refuses_foreign_bytes() {
        assert_eq!(
            ScriptDescriptor::decode(b"not a descriptor at all"),
            Err(DescriptorError::BadMagic)
        );
    }

    /// Every truncation point must be refused, not silently short-read. Driving
    /// every prefix length is the only check that cannot miss one boundary.
    #[test]
    fn refuses_every_truncation() {
        let full = sample().encode();
        for cut in 0..full.len() {
            assert!(
                ScriptDescriptor::decode(&full[..cut]).is_err(),
                "prefix of {cut} bytes decoded but should not have"
            );
        }
        assert!(ScriptDescriptor::decode(&full).is_ok());
    }

    #[test]
    fn refuses_trailing_bytes() {
        let mut extra = sample().encode();
        extra.push(0);
        assert_eq!(
            ScriptDescriptor::decode(&extra),
            Err(DescriptorError::TrailingBytes { len: 1 })
        );
    }

    #[test]
    fn refuses_an_oversized_length_prefix() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&u32::MAX.to_le_bytes());
        let got = ScriptDescriptor::decode(&bytes);
        assert!(
            matches!(got, Err(DescriptorError::FieldTooLarge { .. })),
            "expected FieldTooLarge, got {got:?}"
        );
    }

    #[test]
    fn refuses_a_non_utf8_string_field() {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(MAGIC);
        put_bytes(&mut bytes, &[0xFF, 0xFE]);
        assert_eq!(
            ScriptDescriptor::decode(&bytes),
            Err(DescriptorError::NotUtf8 {
                field: "interpreter_path"
            })
        );
    }

    #[test]
    fn hex32_renders_64_lowercase_chars() {
        let hex = hex32(&[0xAB; 32]);
        assert_eq!(hex.len(), 64);
        assert_eq!(hex, "ab".repeat(32));
    }

    /// The prefix is what separates a script's result from the identical bytes
    /// stored by any other producer. Without it the two would share a ref.
    #[test]
    fn the_result_ref_is_domain_separated() {
        let plain = *blake3::hash(b"hello").as_bytes();
        assert_ne!(result_ref_bytes(b"hello"), plain);
    }

    #[test]
    fn the_result_ref_is_a_function_of_the_output() {
        assert_eq!(result_ref_bytes(b"a"), result_ref_bytes(b"a"));
        assert_ne!(result_ref_bytes(b"a"), result_ref_bytes(b"b"));
    }
}
