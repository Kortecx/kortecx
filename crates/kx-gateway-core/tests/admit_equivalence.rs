//! The extracted enforcers agree with the RPCs they were extracted from.
//!
//! `admit_script` and `admit_registration` exist so the NL proposer can ask
//! "would this registration be refused?" WITHOUT restating the rules. That is
//! only true while the extraction is faithful, and "I moved the code carefully"
//! is not a property a test can check later.
//!
//! ## The shape of the proof, and the trap it walked into first
//!
//! Each case runs through BOTH paths against a service whose registry admin
//! ALWAYS SUCCEEDS, so the only possible source of a refusal is the prologue.
//!
//! The first version of this file asserted ONLY
//! `admit(&r).is_err() == register(r).is_err()`, and mutation testing showed
//! that assertion is **VACUOUS**: after the extraction both paths call the same
//! function, so deleting a check from `admit_script` deletes it from
//! `RegisterScript` too and the two still agree. It was testing `f(x) == f(x)`.
//! Every one of the twelve cases passed with the source-must-be-non-empty check
//! removed entirely.
//!
//! So each case now carries its EXPECTED verdict, and the suite asserts three
//! things:
//!
//! 1. the RPC's answer matches the expectation — this is what catches a check
//!    being weakened or dropped, because the expectation is written down
//!    independently of the code;
//! 2. `admit` agrees with the RPC — this is what catches the drift the
//!    extraction exists to prevent, someone re-inlining a check into the handler
//!    that the proposer will never see;
//! 3. the case table contains both accepted and refused cases, so neither
//!    assertion can pass by the whole table landing on one side.
//!
//! The lesson generalises: an equivalence test between two things that share an
//! implementation proves nothing about either. It needs a third, independent
//! statement of the answer.
//!
//! ## Why the always-succeeding admin is the right stub, not a cheat
//!
//! A REAL admin would refuse for host reasons — an interpreter absent from the
//! allowlist, a sandbox this machine cannot build, an SSRF-vetted host — and
//! those refusals are deliberately NOT in the extracted functions (they need
//! configuration and a live sandbox). Stubbing them out is what isolates the
//! pure half, which is the half being claimed equivalent.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;

use common::{build_run, spawn, MockSubmitter, INSTANCE_ID};
use kx_gateway_core::{
    GatewayError, GatewayService, JournalReader, ReadOnly, RegisteredScriptEntry,
    RegisteredToolEntry, RunSubmitter, ScriptAdmin, ScriptAdminError, ScriptRegistration,
    ToolAdminError, ToolRegistration, ToolRegistryAdmin,
};
use kx_proto::proto;

/// A script admin that registers anything. See the module doc: the host half of
/// admission is deliberately absent from `admit_script`, so it must be absent
/// here too or the comparison would be measuring the stub.
struct AlwaysOkScripts;

impl ScriptAdmin for AlwaysOkScripts {
    fn register(&self, _reg: ScriptRegistration) -> Result<[u8; 16], ScriptAdminError> {
        Ok([7u8; 16])
    }
    fn deregister(&self, _n: &str, _v: &str) -> Result<bool, GatewayError> {
        Ok(true)
    }
    fn list(
        &self,
        _limit: usize,
        _after: Option<(String, String)>,
    ) -> Result<(Vec<RegisteredScriptEntry>, bool), GatewayError> {
        Ok((Vec::new(), false))
    }
    fn get(
        &self,
        _n: &str,
        _v: &str,
    ) -> Result<Option<(RegisteredScriptEntry, Vec<u8>)>, GatewayError> {
        Ok(None)
    }
}

/// A tool admin that registers anything, for the same reason.
struct AlwaysOkTools;

impl ToolRegistryAdmin for AlwaysOkTools {
    fn register(&self, _reg: ToolRegistration) -> Result<[u8; 16], ToolAdminError> {
        Ok([9u8; 16])
    }
    fn deregister(&self, _n: &str, _v: &str) -> Result<bool, GatewayError> {
        Ok(true)
    }
    fn discover(
        &self,
        _limit: usize,
        _after: Option<(String, String)>,
    ) -> Result<(Vec<RegisteredToolEntry>, bool), GatewayError> {
        Ok((Vec::new(), false))
    }
}

fn service() -> GatewayService {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let submitter: Arc<dyn RunSubmitter> = Arc::new(MockSubmitter::default());
    GatewayService::new(reader, submitter, Arc::new(run.content))
        .with_script_admin(Arc::new(AlwaysOkScripts))
        .with_tool_admin(Arc::new(AlwaysOkTools))
}

fn ok_script() -> proto::RegisterScriptRequest {
    proto::RegisterScriptRequest {
        script_name: "tidy".into(),
        script_version: "1".into(),
        description: "trims whitespace".into(),
        interpreter: "sh".into(),
        source: b"cat".to_vec(),
        ..Default::default()
    }
}

fn ok_tool() -> proto::RegisterToolRequest {
    proto::RegisterToolRequest {
        tool_name: "lookup".into(),
        tool_version: "1".into(),
        description: "reads a key".into(),
        idempotency_class: "Token".into(),
        server_host: "tools.example.com:443".into(),
        ..Default::default()
    }
}

/// Every case: `admit_script` refuses exactly when `RegisterScript` refuses.
#[tokio::test]
async fn admit_script_matches_register_script() {
    let over_cap = "x".repeat(4 * 1024 + 1);
    let many: Vec<String> = (0..65).map(|i| format!("a{i}")).collect();

    let cases: Vec<(&str, proto::RegisterScriptRequest, bool)> = vec![
        ("the accepted baseline", ok_script(), false),
        (
            "empty name",
            proto::RegisterScriptRequest {
                script_name: String::new(),
                ..ok_script()
            },
            true,
        ),
        (
            "whitespace-only name (trim, not is_empty)",
            proto::RegisterScriptRequest {
                script_name: "   ".into(),
                ..ok_script()
            },
            true,
        ),
        (
            "empty version",
            proto::RegisterScriptRequest {
                script_version: String::new(),
                ..ok_script()
            },
            true,
        ),
        (
            "description over the 4 KiB cap",
            proto::RegisterScriptRequest {
                description: over_cap.clone(),
                ..ok_script()
            },
            true,
        ),
        (
            "description exactly AT the cap is fine",
            proto::RegisterScriptRequest {
                description: "y".repeat(4 * 1024),
                ..ok_script()
            },
            false,
        ),
        (
            "empty source",
            proto::RegisterScriptRequest {
                source: Vec::new(),
                ..ok_script()
            },
            true,
        ),
        (
            "too many argv",
            proto::RegisterScriptRequest {
                argv: many.clone(),
                ..ok_script()
            },
            true,
        ),
        (
            "too many net_hosts",
            proto::RegisterScriptRequest {
                net_hosts: many.clone(),
                ..ok_script()
            },
            true,
        ),
        (
            "exactly 64 argv is fine",
            proto::RegisterScriptRequest {
                argv: many[..64].to_vec(),
                ..ok_script()
            },
            false,
        ),
        (
            "too many env",
            proto::RegisterScriptRequest {
                env: (0..65)
                    .map(|i| proto::ScriptEnv {
                        key: format!("K{i}"),
                        value: "v".into(),
                    })
                    .collect(),
                ..ok_script()
            },
            true,
        ),
        (
            "too many fs_mounts",
            proto::RegisterScriptRequest {
                fs_mounts: (0..65)
                    .map(|i| proto::ScriptMount {
                        path: format!("/tmp/{i}"),
                        mode: "ro".into(),
                    })
                    .collect(),
                ..ok_script()
            },
            true,
        ),
    ];

    let total = cases.len();
    let mut client = spawn(service()).await;
    let mut refusals = 0usize;
    for (label, req, expect_refused) in cases {
        let via_rpc = client.register_script(req.clone()).await.is_err();
        let via_admit = kx_gateway_core::admit_script_for_test(&req).is_err();
        // (1) The INDEPENDENT statement of the answer. This is the assertion the
        //     equivalence check cannot make for itself.
        assert_eq!(
            via_rpc, expect_refused,
            "{label}: RegisterScript refused={via_rpc}, expected refused={expect_refused}"
        );
        // (2) The extraction is faithful — no check lives in the handler that the
        //     NL proposer would never see.
        assert_eq!(
            via_admit, via_rpc,
            "{label}: admit_script says err={via_admit}, RegisterScript says err={via_rpc}"
        );
        if via_rpc {
            refusals += 1;
        }
    }
    // (3) Both sides are represented, so neither assertion can pass by the whole
    //     table landing on one answer.
    assert!(
        refusals > 0 && refusals < total,
        "expected a mix of accepted and refused cases, got {refusals} refusals of {total}"
    );
}

/// Every case: `admit_registration` refuses exactly when `RegisterTool` refuses.
#[tokio::test]
async fn admit_registration_matches_register_tool() {
    let over_cap = "x".repeat(4 * 1024 + 1);

    let cases: Vec<(&str, proto::RegisterToolRequest, bool)> = vec![
        ("the accepted baseline", ok_tool(), false),
        (
            "empty name",
            proto::RegisterToolRequest {
                tool_name: String::new(),
                ..ok_tool()
            },
            true,
        ),
        (
            "whitespace-only version",
            proto::RegisterToolRequest {
                tool_version: "  ".into(),
                ..ok_tool()
            },
            true,
        ),
        (
            "empty server_host",
            proto::RegisterToolRequest {
                server_host: String::new(),
                ..ok_tool()
            },
            true,
        ),
        (
            "whitespace-only server_host",
            proto::RegisterToolRequest {
                server_host: " ".into(),
                ..ok_tool()
            },
            true,
        ),
        (
            "description over the 4 KiB cap",
            proto::RegisterToolRequest {
                description: over_cap,
                ..ok_tool()
            },
            true,
        ),
        (
            "too many params",
            proto::RegisterToolRequest {
                input_schema: Some(proto::ToolInputSchema {
                    params: (0..65)
                        .map(|i| proto::ToolParamSpec {
                            name: format!("p{i}"),
                            ty: "str".into(),
                            ..Default::default()
                        })
                        .collect(),
                    deny_unknown: true,
                }),
                ..ok_tool()
            },
            true,
        ),
        (
            "exactly 64 params is fine",
            proto::RegisterToolRequest {
                input_schema: Some(proto::ToolInputSchema {
                    params: (0..64)
                        .map(|i| proto::ToolParamSpec {
                            name: format!("p{i}"),
                            ty: "str".into(),
                            ..Default::default()
                        })
                        .collect(),
                    deny_unknown: true,
                }),
                ..ok_tool()
            },
            false,
        ),
    ];

    let total = cases.len();
    let mut client = spawn(service()).await;
    let mut refusals = 0usize;
    for (label, req, expect_refused) in cases {
        let via_rpc = client.register_tool(req.clone()).await.is_err();
        let via_admit = kx_gateway_core::admit_registration_for_test(&req).is_err();
        assert_eq!(
            via_rpc, expect_refused,
            "{label}: RegisterTool refused={via_rpc}, expected refused={expect_refused}"
        );
        assert_eq!(
            via_admit, via_rpc,
            "{label}: admit_registration says err={via_admit}, RegisterTool says err={via_rpc}"
        );
        if via_rpc {
            refusals += 1;
        }
    }
    assert!(
        refusals > 0 && refusals < total,
        "expected a mix of accepted and refused cases, got {refusals} refusals of {total}"
    );
}

/// The extracted functions do NOT claim the host's half of admission.
///
/// This is the boundary being documented as an assertion rather than a comment:
/// a request `admit_script` accepts can still be refused at registration by a
/// real host (an interpreter outside the allowlist, a sandbox it cannot build).
/// If someone later folds those checks in, this test says so — and the NL
/// proposer's contract would change with it, because a preview would start
/// claiming a host guarantee it cannot make.
#[test]
fn admit_does_not_claim_the_hosts_half() {
    // An interpreter no host allowlists. The pure gate accepts it — the sandbox
    // probe is the authority on whether it can actually run.
    let req = proto::RegisterScriptRequest {
        interpreter: "definitely-not-a-real-interpreter".into(),
        ..ok_script()
    };
    assert!(
        kx_gateway_core::admit_script_for_test(&req).is_ok(),
        "admit_script must not pretend to know what this host can execute"
    );

    // A host the operator's allowlist would reject. Same reasoning: SSRF vetting
    // needs configuration the pure function does not have.
    let req = proto::RegisterToolRequest {
        server_host: "169.254.169.254:80".into(),
        ..ok_tool()
    };
    assert!(
        kx_gateway_core::admit_registration_for_test(&req).is_ok(),
        "admit_registration must not pretend to know the operator's egress allowlist"
    );
    let _ = INSTANCE_ID;
}
