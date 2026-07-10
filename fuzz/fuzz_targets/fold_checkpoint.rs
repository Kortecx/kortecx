#![no_main]
//! Fuzz `kx_projection::FoldCheckpoint::from_bytes` — decodes an untrusted projection checkpoint
//! (version + codec + blake3-digest gated, length-checked, fail-closed). A recovering runtime folds
//! this; a panic / OOM on a corrupt or adversarial checkpoint is a finding.
use libfuzzer_sys::fuzz_target;
use kx_projection::FoldCheckpoint;

fuzz_target!(|data: &[u8]| {
    let _ = FoldCheckpoint::from_bytes(data);
});
