//! The public API-surface guard (semver discipline; the D167 Extension Acceptance
//! Gate item 4 — additive-only). Two layers, both build-failing on drift:
//!
//!  1. **Resolution** — `use` every promised prelude symbol. An upstream RENAME or
//!     REMOVAL makes this file fail to compile (the surface is provably real).
//!  2. **Drift** — parse `prelude.rs`, extract the exported set, and `assert_eq!`
//!     it against a frozen `EXPECTED` list. Any ADD/REMOVE forces a deliberate,
//!     reviewed edit to `EXPECTED` (the surface is provably exactly the intended set).
//!
//! Together they make the prelude a pinned, semver'd contract: it cannot drift
//! silently in either direction.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    unused_imports
)]

use std::collections::BTreeSet;

// -- Layer 1: resolution (a rename/removal upstream fails to compile here) -------
use kx_extension_sdk::prelude::{
    // kx-warrant (the capability boundary)
    check_tool_requirement,
    // kx-mcp (transports, secrets, egress, capability bodies)
    classify_ip,
    // kx-mcp-gateway (the dial path)
    connection_id_of,
    vet_resolved_addr,
    CapabilitySink,
    Connection,
    ConnectionHealth,
    CostCeiling,
    CredentialRef,
    DecodeError,
    EgressDenied,
    EgressPolicy,
    EnvSecretStore,
    FsScope,
    GatewayError,
    Host,
    HttpTransport,
    // kx-tool-registry (the tool vocabulary + durable registry)
    IdempotencyClass,
    InMemoryToolRegistry,
    InputSchema,
    IpClass,
    McpCapability,
    McpEndpointId,
    McpGateway,
    McpSession,
    McpSessionCapability,
    McpTransport,
    ModelRoute,
    NetScope,
    ParamSpec,
    ParamType,
    RegisterOutcome,
    RegisteredEntry,
    RegistrationError,
    RegistrationStatus,
    RemoteToolDecl,
    ResolutionError,
    ResourceCeiling,
    SecretRef,
    SecretScope,
    SecretStore,
    SessionError,
    SessionMode,
    SqliteConnectionStore,
    SqliteToolRegistry,
    StdioTransport,
    ToolDef,
    ToolGrant,
    ToolKind,
    // kx-mote (tool identity)
    ToolName,
    ToolProvenance,
    ToolRegistry,
    ToolRequirement,
    ToolVersion,
    TransportError,
    TransportSpec,
    WarrantSpec,
};

/// The frozen public prelude surface. Editing this list is the SEMVER decision:
/// an addition is a minor bump; a removal/rename is a breaking change. Keep it
/// sorted within each group for review legibility.
const EXPECTED: &[&str] = &[
    // kx-mcp-gateway
    "CapabilitySink",
    "Connection",
    "ConnectionHealth",
    "GatewayError",
    "McpGateway",
    "RegisterOutcome",
    "SessionMode",
    "SqliteConnectionStore",
    "TransportSpec",
    "connection_id_of",
    // kx-mcp
    "CredentialRef",
    "DecodeError",
    "EgressDenied",
    "EgressPolicy",
    "EnvSecretStore",
    "HttpTransport",
    "IpClass",
    "McpCapability",
    "McpSession",
    "McpSessionCapability",
    "McpTransport",
    "RemoteToolDecl",
    "SecretStore",
    "SessionError",
    "StdioTransport",
    "TransportError",
    "classify_ip",
    "vet_resolved_addr",
    // kx-tool-registry
    "IdempotencyClass",
    "InMemoryToolRegistry",
    "InputSchema",
    "McpEndpointId",
    "ParamSpec",
    "ParamType",
    "RegisteredEntry",
    "RegistrationError",
    "RegistrationStatus",
    "ResolutionError",
    "SqliteToolRegistry",
    "ToolDef",
    "ToolKind",
    "ToolProvenance",
    "ToolRegistry",
    // kx-warrant
    "CostCeiling",
    "FsScope",
    "Host",
    "ModelRoute",
    "NetScope",
    "ResourceCeiling",
    "SecretRef",
    "SecretScope",
    "ToolGrant",
    "ToolRequirement",
    "WarrantSpec",
    "check_tool_requirement",
    // kx-mote
    "ToolName",
    "ToolVersion",
];

/// Extract the exported identifier set from `prelude.rs` source: for every
/// `pub use … { A, B, C }` statement, the brace-list leaves (honoring an `as` alias).
fn exported_symbols(src: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for chunk in src.split("pub use").skip(1) {
        let stmt = chunk.split(';').next().unwrap_or("");
        let Some(open) = stmt.find('{') else { continue };
        let close = stmt[open..].find('}').map_or(stmt.len(), |c| open + c);
        for raw in stmt[open + 1..close].split(',') {
            let name = raw.trim();
            if name.is_empty() {
                continue;
            }
            // `Foo as Bar` exports `Bar`; a plain `Foo` exports `Foo`.
            let exported = name.split(" as ").last().unwrap_or(name).trim();
            out.insert(exported.to_string());
        }
    }
    out
}

#[test]
fn prelude_surface_matches_the_frozen_set() {
    let parsed = exported_symbols(include_str!("../src/prelude.rs"));
    let expected: BTreeSet<String> = EXPECTED.iter().map(|s| (*s).to_string()).collect();
    assert_eq!(
        parsed, expected,
        "the kx-extension-sdk prelude surface drifted from the frozen set. \
         Adding an export is a SEMVER-MINOR change; removing/renaming is BREAKING. \
         Update EXPECTED in this test deliberately to record the decision."
    );
}
