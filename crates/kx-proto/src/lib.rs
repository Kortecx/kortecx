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
