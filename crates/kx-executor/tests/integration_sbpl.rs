//! Integration tests for `profile_from_warrant`'s per-axis SBPL mapping
//! (PR 9a-hardening-1, D46 §6). Verifies the bytes emitted for each
//! `WarrantSpec` axis match the locked spec.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_executor::profile_from_warrant;
use kx_mote::ModelId;
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    WarrantSpec,
};

fn permissive_warrant_with(fs_scope: FsScope, net_scope: NetScope) -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope,
        net_scope,
        syscall_profile_ref: ContentRef::from_bytes([0xAB; 32]),
        tool_grants: BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("local".into()),
            max_input_tokens: 0,
            max_output_tokens: 0,
            max_calls: 0,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 0,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::MacOsSandbox,
        ..Default::default()
    }
}

fn profile_text(w: &WarrantSpec) -> String {
    String::from_utf8(profile_from_warrant(w).as_bytes().to_vec()).expect("UTF-8 profile")
}

// ============================================================================
// fs_scope mappings (D46 §6.1)
// ============================================================================

#[test]
fn fs_scope_read_only_emits_file_read_star() {
    let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
    mounts.insert(PathBuf::from("/usr/local/share"), FsMode::ReadOnly);
    let w = permissive_warrant_with(FsScope { mounts }, NetScope::None);
    let text = profile_text(&w);
    assert!(text.contains("(allow file-read* (subpath \"/usr/local/share\"))"));
    assert!(!text.contains("file-write*"));
    assert!(!text.contains("process-exec"));
}

#[test]
fn fs_scope_read_write_emits_both_read_and_write() {
    let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
    mounts.insert(PathBuf::from("/tmp/workspace"), FsMode::ReadWrite);
    let w = permissive_warrant_with(FsScope { mounts }, NetScope::None);
    let text = profile_text(&w);
    assert!(text.contains("(allow file-read* (subpath \"/tmp/workspace\"))"));
    assert!(text.contains("(allow file-write* (subpath \"/tmp/workspace\"))"));
}

#[test]
fn fs_scope_exec_only_emits_metadata_and_process_exec() {
    let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
    mounts.insert(PathBuf::from("/opt/binaries"), FsMode::ExecOnly);
    let w = permissive_warrant_with(FsScope { mounts }, NetScope::None);
    let text = profile_text(&w);
    assert!(text.contains("(allow file-read-metadata (subpath \"/opt/binaries\"))"));
    assert!(text.contains("(allow process-exec (subpath \"/opt/binaries\"))"));
    // ExecOnly MUST NOT emit file-read* or file-write* — would leak read/write
    // privilege on the exec-only path.
    assert!(!text.contains("(allow file-read* (subpath \"/opt/binaries\"))"));
    assert!(!text.contains("(allow file-write* (subpath \"/opt/binaries\"))"));
}

#[test]
fn fs_scope_emits_mounts_in_btreemap_iteration_order() {
    // BTreeMap iterates by sorted key — same input set, byte-identical output.
    let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
    mounts.insert(PathBuf::from("/z"), FsMode::ReadOnly);
    mounts.insert(PathBuf::from("/a"), FsMode::ReadOnly);
    mounts.insert(PathBuf::from("/m"), FsMode::ReadOnly);
    let w = permissive_warrant_with(FsScope { mounts }, NetScope::None);
    let text = profile_text(&w);
    let a_pos = text.find("subpath \"/a\"").expect("a present");
    let m_pos = text.find("subpath \"/m\"").expect("m present");
    let z_pos = text.find("subpath \"/z\"").expect("z present");
    assert!(a_pos < m_pos, "/a must precede /m");
    assert!(m_pos < z_pos, "/m must precede /z");
}

// ============================================================================
// net_scope mappings (D46 §6.2)
// ============================================================================

#[test]
fn net_scope_none_emits_no_network_rules() {
    let w = permissive_warrant_with(FsScope::empty(), NetScope::None);
    let text = profile_text(&w);
    assert!(
        !text.contains("network-outbound"),
        "NetScope::None must not emit any network-outbound allows"
    );
}

#[test]
fn net_scope_egress_allowlist_emits_per_host_rule() {
    let mut hosts: BTreeSet<Host> = BTreeSet::new();
    hosts.insert(Host("api.example.com".into()));
    hosts.insert(Host("registry.internal".into()));
    let w = permissive_warrant_with(FsScope::empty(), NetScope::EgressAllowlist(hosts));
    let text = profile_text(&w);
    assert!(text.contains("(allow network-outbound (remote ip \"api.example.com:*\"))"));
    assert!(text.contains("(allow network-outbound (remote ip \"registry.internal:*\"))"));
}

#[test]
fn net_scope_hosts_emit_in_btreeset_iteration_order() {
    let mut hosts: BTreeSet<Host> = BTreeSet::new();
    hosts.insert(Host("c.example".into()));
    hosts.insert(Host("a.example".into()));
    hosts.insert(Host("b.example".into()));
    let w = permissive_warrant_with(FsScope::empty(), NetScope::EgressAllowlist(hosts));
    let text = profile_text(&w);
    let a_pos = text.find("a.example").expect("a present");
    let b_pos = text.find("b.example").expect("b present");
    let c_pos = text.find("c.example").expect("c present");
    assert!(a_pos < b_pos);
    assert!(b_pos < c_pos);
}

// ============================================================================
// syscall_profile_ref audit-trail line (D46 §6.3 — opaque per-platform; the
// PR 9a-hardening-2 follow-up resolves to body bytes; this PR ships the
// audit-trail comment)
// ============================================================================

#[test]
fn syscall_profile_ref_emits_audit_comment() {
    let w = permissive_warrant_with(FsScope::empty(), NetScope::None);
    let text = profile_text(&w);
    // 0xAB byte pattern from `permissive_warrant_with`.
    assert!(text.contains(";; syscall-profile-ref: "));
    assert!(text.contains(&"ab".repeat(32)));
}

// ============================================================================
// resource_ceiling NOT in profile (D46 §6.4)
// ============================================================================

#[test]
fn resource_ceiling_does_not_appear_in_sbpl_profile() {
    // Non-zero resource_ceiling MUST NOT produce any rules in the profile —
    // resource enforcement is `LocalResourceManager` via setrlimit, not SBPL.
    let mut w = permissive_warrant_with(FsScope::empty(), NetScope::None);
    w.resource_ceiling = ResourceCeiling {
        cpu_milli: 1000,
        mem_bytes: 1 << 30,
        wall_clock_ms: 60_000,
        fd_count: 256,
        disk_bytes: 1 << 30,
    };
    let text = profile_text(&w);
    // None of these SBPL forms map resource limits; their absence is the
    // layering boundary signal.
    assert!(!text.contains("limit-"));
    assert!(!text.contains("rlimit"));
    assert!(!text.contains("cpu_milli"));
}

// ============================================================================
// Deny-default template invariants (D46 §5)
// ============================================================================

#[test]
fn deny_default_template_is_present_in_every_profile() {
    let w = permissive_warrant_with(FsScope::empty(), NetScope::None);
    let text = profile_text(&w);
    // Template imports Apple's system.sb baseline (so Rust-runtime startup
    // mmap calls succeed) before the deny-default + per-axis allows. See
    // `DENY_DEFAULT_TEMPLATE` in `backends/macos_sandbox.rs` for the
    // rationale.
    assert!(text.starts_with("(version 1)\n"));
    assert!(text.contains("(import \"system.sb\")"));
    assert!(text.contains("(deny default)"));
}

#[test]
fn empty_warrant_produces_minimal_profile_with_only_template_and_audit_line() {
    let w = permissive_warrant_with(FsScope::empty(), NetScope::None);
    let text = profile_text(&w);
    // No allow rules (fs_scope empty + net_scope None) — only the deny-
    // default template + the syscall-profile-ref audit comment.
    assert!(!text.contains("(allow file-"));
    assert!(!text.contains("(allow network-"));
    assert!(!text.contains("(allow process-exec"));
}

// ============================================================================
// Escaping (D46 §6.1 anti-injection)
// ============================================================================

#[test]
fn paths_with_double_quotes_are_escaped() {
    let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
    mounts.insert(PathBuf::from(r#"/tmp/"quoted""#), FsMode::ReadOnly);
    let w = permissive_warrant_with(FsScope { mounts }, NetScope::None);
    let text = profile_text(&w);
    // The literal " in the path is escaped to \" — preserving the SBPL
    // S-expression's quoting.
    assert!(text.contains(r#"/tmp/\"quoted\""#));
}

#[test]
fn paths_with_backslashes_are_escaped() {
    let mut mounts: BTreeMap<PathBuf, FsMode> = BTreeMap::new();
    mounts.insert(PathBuf::from(r"/tmp/with\backslash"), FsMode::ReadOnly);
    let w = permissive_warrant_with(FsScope { mounts }, NetScope::None);
    let text = profile_text(&w);
    assert!(text.contains(r"/tmp/with\\backslash"));
}
