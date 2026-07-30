// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! The ControlSurface classification cannot be defeated.
//!
//! ## What actually stops an unclassified RPC
//!
//! `rustc` does. `GatewayRpc` is generated from the descriptor and is NOT
//! `#[non_exhaustive]`; `control_surface::facet` matches it exhaustively with no
//! wildcard. Adding an RPC to `gateway.proto` therefore fails
//! `cargo check -p kx-gateway-core` with `error[E0004]`. That is Gate A, and it
//! needs no test.
//!
//! ## So what are these for
//!
//! Gate A has exactly two failure modes, and both are edits to THIS repo rather
//! than to the wire:
//!
//! 1. Someone adds a `_ =>` arm to `facet`, converting the compile error into a
//!    silent default.
//! 2. Someone marks the generated enum `#[non_exhaustive]`, which would force
//!    wildcards everywhere and achieve the same thing.
//!
//! Neither is visible to a behavioural test — a table with a wildcard still
//! answers every question, just wrongly for the RPC nobody thought about. The
//! failure is an ABSENCE, so the check has to be over the source. This is the
//! same argument, and the same shape, as `kx-gateway/tests/sidecar_policy.rs`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::path::{Path, PathBuf};

fn control_surface_src() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/control_surface.rs")
}

/// Strip `#[cfg(test)] mod tests { ... }` to the end of file.
///
/// The tests below legitimately write `_ =>` inside match expressions of their
/// own; holding them to the production rule would only teach the next author to
/// move code into a test module.
fn production_source(path: &Path) -> String {
    let src = std::fs::read_to_string(path).unwrap();
    match src.find("\nmod tests {") {
        Some(i) => src[..i].to_string(),
        None => src,
    }
}

/// Isolate the body of `pub const fn facet`.
fn facet_body(src: &str) -> String {
    let start = src
        .find("pub const fn facet(")
        .expect("control_surface.rs declares `pub const fn facet`");
    let rest = &src[start..];
    // The function ends at the first line that is a closing brace in column 0.
    let end = rest.find("\n}").expect("facet's body closes");
    rest[..end].to_string()
}

/// `facet` must have NO wildcard arm.
///
/// A `_ =>` or a binding arm turns the E0004 that protects every future RPC into
/// a silent default. This is the guard's whole job.
#[test]
fn the_classification_match_has_no_wildcard_arm() {
    let src = production_source(&control_surface_src());
    let body = facet_body(&src);

    let mut offenders: Vec<(usize, String)> = Vec::new();
    for (i, line) in body.lines().enumerate() {
        let t = line.trim();
        if t.starts_with("//") {
            continue;
        }
        // `_ => ...` is the literal wildcard; `rpc => ...` / `other => ...` is a
        // binding arm, which is equally exhaustive-defeating.
        let is_wildcard = t.starts_with("_ =>") || t.starts_with("_=>");
        let is_binding = t.split("=>").next().map(str::trim).is_some_and(|lhs| {
            t.contains("=>")
                && !lhs.is_empty()
                && !lhs.contains("GatewayRpc::")
                && !lhs.contains('|')
                && lhs.chars().all(|c| c.is_ascii_lowercase() || c == '_')
        });
        if is_wildcard || is_binding {
            offenders.push((i + 1, t.to_string()));
        }
    }

    assert!(
        offenders.is_empty(),
        "`facet` must stay exhaustive with NO catch-all — a wildcard converts the \
         E0004 that protects every future RPC into a silent default. Offending arms \
         (line, text): {offenders:?}"
    );
}

/// Every arm names a `GatewayRpc` variant explicitly.
///
/// Belt-and-braces for the above: if the arm count ever drops far below the RPC
/// count, something is collapsing arms even without a literal `_`.
#[test]
fn the_classification_names_every_rpc_explicitly() {
    let src = production_source(&control_surface_src());
    let body = facet_body(&src);
    let arms = body.matches("GatewayRpc::").count();
    let total = kx_proto::control::GatewayRpc::ALL.len();
    assert!(
        arms >= total,
        "expected at least one `GatewayRpc::` arm per RPC ({total}), found {arms} — \
         arms are being collapsed"
    );
}

/// The generated enum is exhaustively matchable from ANOTHER crate.
///
/// `#[non_exhaustive]` is a **cross-crate** restriction: it does nothing inside
/// the defining crate, and forbids exhaustive matching everywhere else. So the
/// proof that `GatewayRpc` did not acquire it is not a file scan — it is that
/// `kx_gateway_core::control_surface::facet`, which lives in a DIFFERENT crate
/// from the enum, matches it exhaustively with no wildcard **and compiles**.
///
/// This test therefore asserts nothing at runtime that the build has not already
/// proven; it exists so the reasoning is written down where someone tempted to
/// add `#[non_exhaustive]` to the generator will find it. What it does check at
/// runtime is the observable consequence: `facet` is total over `ALL`.
#[test]
fn the_generated_rpc_enum_is_exhaustively_matchable_from_another_crate() {
    // If GatewayRpc were #[non_exhaustive], `facet`'s wildcard-free match would
    // not have compiled and this binary would not exist.
    for rpc in kx_proto::control::GatewayRpc::ALL {
        let _ = kx_gateway_core::control_surface::facet(*rpc);
    }
    assert!(
        kx_proto::control::GatewayRpc::ALL.len() >= 115,
        "expected the full KxGateway surface, got {}",
        kx_proto::control::GatewayRpc::ALL.len()
    );
}

/// The module doc must keep stating WHY the wildcard is banned.
///
/// The rule is only obeyed if the next author knows it exists. A silent
/// convention is one refactor from gone.
#[test]
fn the_module_records_why_the_match_is_exhaustive() {
    let src = production_source(&control_surface_src());
    let head: String = src.lines().take(60).collect::<Vec<_>>().join("\n");
    for needle in ["E0004", "non_exhaustive", "wildcard"] {
        assert!(
            head.contains(needle),
            "the module doc must explain the exhaustiveness guarantee (missing {needle:?})"
        );
    }
}
