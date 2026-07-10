#![no_main]
//! Fuzz `kx_journal::decode_entry` — parses an untrusted journal-entry byte record (header/kind/version
//! gated, length-checked, fail-closed). The journal is the runtime's synchronization substrate; a panic
//! or OOM decoding a corrupt/malicious entry is a finding.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = kx_journal::decode_entry(data);
});
