//! A tiny hex codec for the byte ids the gateway speaks (16-byte `instance_id`,
//! 32-byte content refs / Mote ids / signature ids). Hand-rolled to keep the
//! dependency surface minimal (Rule 6) — `hex` is not in the workspace lockfile
//! and a length-checked encode/decode is a dozen lines. Encoding is lowercase;
//! decoding accepts either case.

use std::fmt;

/// A hex-decoding failure. Carries enough to render a precise CLI usage error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HexError {
    /// The input had an odd number of hex digits (can't form whole bytes).
    OddLength,
    /// A character was not a hex digit (`0-9a-fA-F`).
    BadDigit(char),
    /// A fixed-length decode got the wrong number of bytes.
    WrongLength {
        /// The number of bytes the field requires.
        want: usize,
        /// The number of bytes the input decoded to.
        got: usize,
    },
}

impl fmt::Display for HexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HexError::OddLength => write!(f, "hex string has an odd number of digits"),
            HexError::BadDigit(c) => write!(f, "not a hex digit: {c:?}"),
            HexError::WrongLength { want, got } => {
                write!(
                    f,
                    "expected {want} bytes ({} hex chars), got {got}",
                    want * 2
                )
            }
        }
    }
}

impl std::error::Error for HexError {}

/// Lowercase-hex-encode `bytes`.
#[must_use]
pub fn encode(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(DIGITS[(b >> 4) as usize] as char);
        out.push(DIGITS[(b & 0x0f) as usize] as char);
    }
    out
}

/// Lowercase-hex-encode an `Option`, rendering `None` as `-` (the dash the
/// human renderers print for an absent ref).
#[must_use]
pub fn encode_opt(bytes: Option<&[u8]>) -> String {
    bytes.map_or_else(|| "-".to_string(), encode)
}

fn nibble(c: u8) -> Result<u8, HexError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(HexError::BadDigit(c as char)),
    }
}

/// Decode a hex string to bytes. Rejects odd length and non-hex digits.
pub fn decode(s: &str) -> Result<Vec<u8>, HexError> {
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(HexError::OddLength);
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        out.push((nibble(pair[0])? << 4) | nibble(pair[1])?);
    }
    Ok(out)
}

/// Decode a hex string to a fixed-size array, rejecting a wrong byte count.
pub fn decode_fixed<const N: usize>(s: &str) -> Result<[u8; N], HexError> {
    let v = decode(s)?;
    v.clone().try_into().map_err(|_| HexError::WrongLength {
        want: N,
        got: v.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_empty_16_and_32() {
        for n in [0usize, 16, 32] {
            let bytes: Vec<u8> = (0..n).map(|i| u8::try_from(i).unwrap()).collect();
            let s = encode(&bytes);
            assert_eq!(s.len(), n * 2);
            assert_eq!(decode(&s).unwrap(), bytes);
        }
    }

    #[test]
    fn encode_is_lowercase_decode_is_case_insensitive() {
        assert_eq!(encode(&[0xab, 0xcd, 0xef]), "abcdef");
        assert_eq!(decode("ABCDEF").unwrap(), vec![0xab, 0xcd, 0xef]);
        assert_eq!(decode("AbCdEf").unwrap(), vec![0xab, 0xcd, 0xef]);
    }

    #[test]
    fn rejects_odd_length_and_non_hex() {
        assert_eq!(decode("abc"), Err(HexError::OddLength));
        assert_eq!(decode("zz"), Err(HexError::BadDigit('z')));
        assert_eq!(decode("0g"), Err(HexError::BadDigit('g')));
    }

    #[test]
    fn decode_fixed_enforces_length() {
        let id16 = encode(&[7u8; 16]);
        assert_eq!(decode_fixed::<16>(&id16).unwrap(), [7u8; 16]);
        // 16 bytes into a 32-byte field is a length error, not a panic.
        assert_eq!(
            decode_fixed::<32>(&id16),
            Err(HexError::WrongLength { want: 32, got: 16 })
        );
        // Non-hex propagates from `decode`.
        assert!(matches!(
            decode_fixed::<16>("zz"),
            Err(HexError::BadDigit('z'))
        ));
    }

    #[test]
    fn encode_opt_renders_dash_for_none() {
        assert_eq!(encode_opt(None), "-");
        assert_eq!(encode_opt(Some(&[0xaa])), "aa");
    }
}
