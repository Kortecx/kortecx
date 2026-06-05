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

mod convert;
mod error;

pub use error::ConvertError;
