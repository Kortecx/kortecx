#![forbid(unsafe_code)]
#![warn(missing_docs)]
#![warn(clippy::pedantic)]
#![allow(
    clippy::missing_errors_doc,
    clippy::missing_panics_doc,
    clippy::module_name_repetitions,
    clippy::must_use_candidate,
    clippy::doc_markdown,
    clippy::return_self_not_must_use
)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

//! # kx-proto — kortecx P2.1 gRPC schema (the distribution boundary)
//!
//! > **Phase: distributed (P2/P3).** The gRPC schema for the multi-node control
//! > plane — wiring on the same trait seams as the single-node core, *not* a
//! > rewrite of it. You do **not** need this crate to build, run, or understand
//! > single-node kortecx (`kx-runtime`). See the README (How it works).
//!
//! tonic/prost gRPC schema for the coordinator/worker control plane: **submit
//! Mote**, **report commit**, **heartbeat**, **register worker**. This is the
//! first step of P2 (coordinator/worker distribution) and the **cross-language
//! contract** — `protoc`/`buf` generate native Rust, Python, and TypeScript
//! types from the same `proto/kortecx/v1/coordinator.proto`.
//!
//! ## The external `KxGateway` service (M8 / D120)
//!
//! `proto/kortecx/v1/gateway.proto` adds a SECOND, distinct
//! [`proto::kx_gateway_server::KxGateway`] service (and
//! [`proto::kx_gateway_client::KxGatewayClient`]) — the client-facing surface
//! realized by `kx-gateway-core` as a read-fold (`GetProjection`/`GetContent`/
//! `StreamEvents`) + propose-proxy (`SubmitRun` → coordinator `RegisterRun`/
//! `SubmitMote`). It reuses the coordinator value messages (`Mote`/`WarrantSpec`/
//! `ParentRef`/`NdClass`) via `import` and adds NO new journal write path. The
//! `Coordinator` contract is byte-unchanged. Same identity invariant: a
//! `ProjectionView`/`MoteSnapshot` is **server-derived**; the client never
//! computes a `MoteId`.
//!
//! ## Mirrored fields, Rust-side identity
//!
//! The schema mirrors the domain types as real protobuf messages (not opaque
//! bincode blobs) so non-Rust clients can build them with generated types. The
//! load-bearing correctness rule:
//!
//! > `MoteId`, `warrant_ref`, and content refs are computed **Rust-side** from
//! > the *reconstructed canonical* form ([`kx_mote::canonical_config`] bincode).
//! > Protobuf wire bytes are **never** hashed; clients **never** compute a
//! > `MoteId`.
//!
//! Protobuf carries field *values*; the typed `TryFrom`/`From` conversions (on
//! the generated [`proto`] types) rebuild the exact canonical Rust struct, and
//! the round-trip identity test pins the mapping so the schema cannot silently
//! drift from the domain types. A failed decode surfaces as a [`ConvertError`].

/// Generated gRPC message + service types (tonic/prost codegen from
/// `proto/kortecx/v1/coordinator.proto` + `proto/kortecx/v1/gateway.proto`).
/// Includes the `Coordinator` service (`coordinator_server`/`coordinator_client`)
/// and the external `KxGateway` service (`kx_gateway_server`/`kx_gateway_client`).
pub mod proto {
    // Generated code is exempt from the workspace lint policy: documentation and
    // style live in the `.proto`, not in the machine-generated Rust.
    #![allow(
        missing_docs,
        unreachable_pub,
        clippy::all,
        clippy::pedantic,
        clippy::nursery
    )]
    #![allow(rustdoc::all)]
    tonic::include_proto!("kortecx.v1");
}

/// The `KxGateway` RPC index, generated from the compiled `FileDescriptorSet`.
///
/// This is the MACHINE-KNOWN half of the ControlSurface: the set of RPCs, their
/// request/response types, and whether they stream. It carries no judgement —
/// which domain an RPC belongs to, whether it reads or mutates, and what
/// authority it demands are hand-authored in `kx_gateway_core::control_surface`,
/// because a descriptor cannot know them.
///
/// [`GatewayRpc`] is deliberately not `#[non_exhaustive]`. Adding an RPC to the
/// `.proto` makes every exhaustive `match` over it fail to compile, which is the
/// mechanism that stops a new capability from being silently unreachable.
pub mod control {
    // Generated code is exempt from the workspace lint policy, exactly as `proto`
    // above is. `match_same_arms` in particular fires hard here by construction:
    // a 115-arm table mapping most RPCs to the same answer is the POINT — the
    // arms are exhaustive on purpose so a new RPC cannot be silently omitted, and
    // collapsing them into a wildcard would destroy that guarantee.
    #![allow(
        missing_docs,
        unreachable_pub,
        clippy::all,
        clippy::pedantic,
        clippy::nursery
    )]
    #![allow(rustdoc::all)]
    include!(concat!(env!("OUT_DIR"), "/control_index.rs"));
}

pub use control::GatewayRpc;

mod convert;
mod error;

pub use error::ConvertError;

#[cfg(test)]
mod control_index_tests {
    use super::GatewayRpc;
    use std::collections::BTreeSet;

    /// The generated index must agree with an INDEPENDENT reading of the `.proto`.
    ///
    /// Two derivations of one fact: the build script decodes protoc's descriptor,
    /// this test scrapes the service block with a plain text scan. A build-script
    /// bug that silently drops an RPC cannot satisfy both.
    #[test]
    fn the_generated_index_agrees_with_an_independent_read_of_the_proto() {
        let src = include_str!("../proto/kortecx/v1/gateway.proto");
        let start = src
            .find("service KxGateway")
            .expect("gateway.proto declares service KxGateway");
        let body = &src[start..];
        let end = body.find("\n}").expect("the service block closes");
        let body = &body[..end];

        let scraped: BTreeSet<&str> = body
            .lines()
            .filter_map(|l| l.trim().strip_prefix("rpc "))
            .filter_map(|rest| rest.split('(').next())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();

        let generated: BTreeSet<&str> = GatewayRpc::ALL.iter().map(|r| r.as_str()).collect();

        assert_eq!(
            scraped,
            generated,
            "the generated RPC index and the .proto text disagree — \
             only in .proto: {:?}; only generated: {:?}",
            scraped.difference(&generated).collect::<Vec<_>>(),
            generated.difference(&scraped).collect::<Vec<_>>(),
        );
        assert!(
            generated.len() > 100,
            "expected the full KxGateway surface, got {} rpcs",
            generated.len()
        );
    }

    /// Names, request types and response types are all distinct per RPC, and the
    /// streaming flag is carried rather than defaulted.
    #[test]
    fn the_index_is_well_formed() {
        let names: BTreeSet<&str> = GatewayRpc::ALL.iter().map(|r| r.as_str()).collect();
        assert_eq!(names.len(), GatewayRpc::ALL.len(), "rpc names are unique");

        for rpc in GatewayRpc::ALL {
            assert!(
                rpc.request_type().starts_with("kortecx.v1."),
                "{}: request type is fully qualified, got {:?}",
                rpc.as_str(),
                rpc.request_type()
            );
            assert!(
                rpc.response_type().starts_with("kortecx.v1."),
                "{}: response type is fully qualified, got {:?}",
                rpc.as_str(),
                rpc.response_type()
            );
        }

        // The streaming flag must be REAL, not a constant. If every RPC reported
        // the same value the accessor would be an elaborate way to spell `false`.
        let streaming: Vec<&str> = GatewayRpc::ALL
            .iter()
            .filter(|r| r.server_streaming())
            .map(|r| r.as_str())
            .collect();
        assert!(
            !streaming.is_empty() && streaming.len() < GatewayRpc::ALL.len(),
            "expected a proper subset to stream, got {streaming:?}"
        );
    }
}

/// The descriptor's SECOND payoff: proving what a message CANNOT carry.
///
/// The generated `GatewayRpc` index proves every RPC is classified. This module
/// uses the same `FileDescriptorSet` to prove a structural property of the NL
/// surface — that no credential-shaped or execution-shaped field is reachable
/// from `ControlPreview`, transitively, at any depth.
///
/// **Why this cannot be a code review.** `ControlPreview` is a `oneof` over the
/// REAL request messages, so its reachable set grows whenever any of those
/// messages grows a field — in a diff that need not mention `ControlPreview` at
/// all. A reviewer reading the `ControlPreview` hunk would see nothing. The walk
/// sees it, because it re-derives the reachable set from the schema every build.
///
/// **Why the descriptor bytes are `#[cfg(test)]`.** `build.rs` argues that the
/// `.fds` never reaches a shipped rlib, and that argument is load-bearing for
/// the reproducibility gate. `include_bytes!` under `cfg(test)` keeps it true.
#[cfg(test)]
mod control_preview_reachability {
    use prost::Message as _;
    use prost_types::{field_descriptor_proto::Type, DescriptorProto, FileDescriptorSet};
    use std::collections::{BTreeMap, BTreeSet};

    /// The descriptor the build script emitted for THIS build.
    const FDS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/kortecx.fds"));

    /// Field names that must never be reachable from a proposal.
    ///
    /// Two families, one rule. `value` / `secret` / `token` / `password` are
    /// CREDENTIAL-shaped: a proposal is displayed, logged and forwarded, so a
    /// credential reachable from one leaks by design rather than by accident.
    /// `argv` / `env` are EXECUTION-shaped: both are fixed at registration by an
    /// operator and documented "NEVER model-controlled", so a proposal that could
    /// express them would hand the model the one axis it must not have.
    ///
    /// Matching is EXACT, not substring, and deliberately: `auth_secret_ref` and
    /// `credential_ref` name a secret without carrying one, and a substring rule
    /// would flag exactly the fields whose whole purpose is to be a reference.
    /// Exactness is why [`the_preview_arms_are_pinned`] exists beside this — an
    /// exact list cannot anticipate a future `api_key`, so a NEW ARM is what the
    /// suite forces a human to look at.
    const FORBIDDEN: &[&str] = &["value", "secret", "token", "password", "argv", "env"];

    /// The arms `ControlPreview` is allowed to have.
    ///
    /// Pinned by name so adding one is a deliberate edit to this list, at which
    /// point the walk below re-derives that arm's reachable set. This is the
    /// half that survives the exact-match limitation above.
    const EXPECTED_ARMS: &[&str] = &[
        "kortecx.v1.AssignPolicyRoleRequest",
        "kortecx.v1.ProposedScript",
        "kortecx.v1.ProposedSecretName",
        "kortecx.v1.PutPolicyRoleRequest",
        "kortecx.v1.RegisterMcpServerRequest",
        "kortecx.v1.RegisterToolRequest",
        "kortecx.v1.RegisterTriggerRequest",
        "kortecx.v1.SaveWorkflowRequest",
    ];

    /// Every `kortecx.v1` message in the descriptor, by fully-qualified name.
    fn messages() -> BTreeMap<String, DescriptorProto> {
        let set = FileDescriptorSet::decode(FDS).expect("the build script emitted a valid .fds");
        let mut out = BTreeMap::new();
        for file in &set.file {
            let pkg = file.package();
            for m in &file.message_type {
                collect(pkg, m, &mut out);
            }
        }
        out
    }

    /// Add `m` and every nested message under it.
    fn collect(prefix: &str, m: &DescriptorProto, out: &mut BTreeMap<String, DescriptorProto>) {
        let full = format!("{prefix}.{}", m.name());
        for nested in &m.nested_type {
            collect(&full, nested, out);
        }
        out.insert(full, m.clone());
    }

    /// `ControlPreview`'s arms, as fully-qualified message names.
    fn preview_arms(all: &BTreeMap<String, DescriptorProto>) -> BTreeSet<String> {
        let preview = all
            .get("kortecx.v1.ControlPreview")
            .expect("ControlPreview is on the wire");
        preview
            .field
            .iter()
            .filter(|f| f.r#type() == Type::Message)
            .map(|f| f.type_name().trim_start_matches('.').to_string())
            .collect()
    }

    /// No field named any of [`FORBIDDEN`] is reachable from `ControlPreview`.
    ///
    /// This is what makes "secrets ride a NAME" and "script argv/env are proposed
    /// EMPTY" properties of the SCHEMA rather than rules the proposer has to keep
    /// remembering. A proposal cannot carry what the wire cannot express.
    #[test]
    fn no_credential_or_argv_field_is_reachable_from_control_preview() {
        let all = messages();
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: Vec<String> = preview_arms(&all).into_iter().collect();
        // `(message, field)` pairs, so the failure NAMES the path rather than
        // just asserting a boolean.
        let mut offenders: Vec<String> = Vec::new();

        while let Some(name) = queue.pop() {
            if !seen.insert(name.clone()) {
                continue;
            }
            let Some(m) = all.get(&name) else { continue };
            for f in &m.field {
                if FORBIDDEN.contains(&f.name()) {
                    offenders.push(format!("{name}.{}", f.name()));
                }
                if f.r#type() == Type::Message {
                    queue.push(f.type_name().trim_start_matches('.').to_string());
                }
            }
        }

        assert!(
            offenders.is_empty(),
            "these fields are reachable from ControlPreview and must not be — a proposal is \
             displayed, logged and forwarded, so anything reachable from one is disclosed by \
             design: {offenders:?}\n\n\
             If the field is credential-shaped, give the domain a REDUCED proposal message (see \
             ProposedSecretName). If it is execution-shaped (argv/env), the same: the point is \
             that the wire cannot express it, not that the proposer declines to fill it in."
        );

        // Anti-vacuity: a walk that reached nothing would pass this trivially.
        // ProposedScript is the arm the reduction exists for, so its presence is
        // the honest floor.
        assert!(
            seen.contains("kortecx.v1.ProposedScript"),
            "the walk did not reach ProposedScript — it is not actually walking"
        );
        assert!(
            seen.len() >= EXPECTED_ARMS.len(),
            "the walk reached {} messages, fewer than the {} arms",
            seen.len(),
            EXPECTED_ARMS.len()
        );
    }

    /// `ControlPreview`'s arm list is exactly [`EXPECTED_ARMS`].
    ///
    /// The exact-match rule above cannot anticipate a field named `api_key`. This
    /// can: a new arm fails here first, which puts a human in front of the new
    /// message's fields before it ever ships.
    #[test]
    fn the_preview_arms_are_pinned() {
        let arms = preview_arms(&messages());
        let expected: BTreeSet<String> = EXPECTED_ARMS.iter().map(|s| (*s).to_string()).collect();
        assert_eq!(
            arms,
            expected,
            "ControlPreview's arms changed. Added: {:?}; removed: {:?}. \
             Update EXPECTED_ARMS *after* checking the new message's fields — this list is the \
             review trigger, not a formality.",
            arms.difference(&expected).collect::<Vec<_>>(),
            expected.difference(&arms).collect::<Vec<_>>(),
        );
    }

    /// The two reduced arms are genuinely reduced: their un-reduced twins DO
    /// carry the forbidden fields.
    ///
    /// Without this, the walk could pass because the reduction was unnecessary.
    /// It asserts the reduction is load-bearing — `PutSecretRequest` really has a
    /// `value`, `RegisterScriptRequest` really has `argv` and `env` — so the
    /// choice to exclude them from the preview is doing work.
    #[test]
    fn the_unreduced_twins_are_why_the_reduction_exists() {
        let all = messages();
        let field_names = |msg: &str| -> BTreeSet<String> {
            all.get(msg)
                .unwrap_or_else(|| panic!("{msg} is on the wire"))
                .field
                .iter()
                .map(|f| f.name().to_string())
                .collect()
        };

        let secret = field_names("kortecx.v1.PutSecretRequest");
        assert!(
            secret.contains("value"),
            "PutSecretRequest lost its `value` field — if the real request no longer carries a \
             value, ProposedSecretName is dead weight and this whole reduction should be revisited"
        );

        let script = field_names("kortecx.v1.RegisterScriptRequest");
        assert!(
            script.contains("argv") && script.contains("env"),
            "RegisterScriptRequest lost argv/env — same reasoning as above for ProposedScript"
        );

        // And the reduced twins do NOT carry them.
        let proposed_secret = field_names("kortecx.v1.ProposedSecretName");
        assert!(
            !proposed_secret.contains("value"),
            "ProposedSecretName grew a `value` — that is the one field it exists to not have"
        );
        let proposed_script = field_names("kortecx.v1.ProposedScript");
        assert!(
            !proposed_script.contains("argv") && !proposed_script.contains("env"),
            "ProposedScript grew argv/env — that is what it exists to not have"
        );
    }
}
