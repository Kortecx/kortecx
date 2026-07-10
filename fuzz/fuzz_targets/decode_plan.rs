#![no_main]
//! Fuzz `kx_planner::decode_plan` — the untrusted model-PROPOSAL decode (the SN-8 fence: a model's
//! plan bytes are lowered into a registered DAG). Documented total + panic-free over arbitrary bytes
//! with DoS caps (MAX_PLAN_STEPS/EDGES/…). The fuzzer asserts that: any panic / OOM / hang is a finding.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // 1 MiB byte cap — larger than production so the fuzzer explores the size boundary too.
    let _ = kx_planner::decode_plan(data, 1 << 20);
});
