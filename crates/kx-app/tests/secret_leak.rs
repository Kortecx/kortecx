//! The security boundary (SN-8 / BLOCKER #5): an App envelope carries REFERENCES
//! and an authorship claim ONLY — never authority. The server re-resolves every
//! axis at bind from the importer's OWN grants. These negatives pin that the
//! serializer is structurally incapable of emitting a secret value or an authority
//! key, no matter how the envelope is populated.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_app::{
    AppEnvelope, ArtifactRef, ConnectionRef, ContextRef, DatasetRef, References, SkillRef,
    SteeringConfig,
};
use serde_json::{json, Value};

/// A maximally-populated envelope — every rail + every steering axis set — so the
/// scans below see the whole shape, not a minimal subset.
fn fully_populated() -> AppEnvelope {
    let blueprint = json!({
        "seed": 0,
        "steps": [
            { "kind": "model", "prompt": "go", "tool_contract": { "mcp-echo/echo": "1" },
              "params": { "max_turns": "8", "max_tool_calls": "6" } }
        ]
    });
    let mut env = AppEnvelope::new("kitchen-sink", blueprint);
    env.description = "every rail set".to_string();
    env.tags = vec!["demo".to_string(), "agentic".to_string()];
    env.references = References {
        context: vec![ContextRef {
            name: "spec".into(),
            content_ref: "a".repeat(64),
            media_type: "text/markdown".into(),
        }],
        tools: vec![kx_app::ToolRef {
            tool_id: "mcp-echo/echo".into(),
            tool_version: "1".into(),
        }],
        connections: vec![ConnectionRef {
            descriptor: "https://mcp.example/sse".into(),
            credential_ref: "MCP_TOKEN".into(),
        }],
        datasets: vec![DatasetRef {
            dataset_ref: "team/ds/docs".into(),
            cas_refs: vec!["b".repeat(64)],
        }],
        prompts: vec![ArtifactRef {
            name: "system".into(),
            content_ref: "c".repeat(64),
        }],
        rules: vec![ArtifactRef {
            name: "no-pii".into(),
            content_ref: "d".repeat(64),
        }],
        skills: vec![SkillRef {
            name: "researcher".into(),
            instructions_ref: "e".repeat(64),
            tools: [("mcp-echo/echo".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        }],
        memory: vec![ArtifactRef {
            name: "notes".into(),
            content_ref: "f".repeat(64),
        }],
        // The composition rail belongs in a "every rail set" fixture too: an App handle is
        // by-name like every other entry here, and carries no authority and no bytes.
        apps: vec![kx_app::AppRef {
            handle: "team/apps/upstream".into(),
        }],
    };
    env.steering_config = SteeringConfig::default();
    env.steering_config.model.model_route = "kx-serve:gemma".into();
    env.steering_config.tools.requested_grants = [("mcp-echo/echo".to_string(), "1".to_string())]
        .into_iter()
        .collect();
    env.steering_config.guards.max_turns = Some(8);
    env.steering_config.guards.secret_scope = vec!["MCP_TOKEN".to_string()];
    env.branch_handle = "team/apps/kitchen-sink".into();
    env
}

/// Reference bodies are NEVER inlined — only the content ref is serialized.
#[test]
fn a_planted_secret_body_never_appears_in_the_envelope_bytes() {
    // Imagine the user authored a rule whose BODY held a secret. The envelope only
    // ever references the body by content_ref; the bytes never travel in the envelope.
    const PLANTED: &str = "sk-PLANTED-SECRET-7f3c91a2-DO-NOT-LEAK";
    let mut env = AppEnvelope::new("x", json!({ "steps": [] }));
    env.references.rules.push(ArtifactRef {
        name: "policy".into(),
        content_ref: "a".repeat(64),
    });
    // (the body lives in the content store, never here)
    let bytes = env.to_canonical_json().unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(
        !text.contains(PLANTED),
        "a referenced artifact body must never inline into the envelope"
    );
}

/// No authority key appears anywhere in the structural envelope (the opaque
/// value-bags — blueprint params, free_params, requested_grants, per_step — are
/// allowed arbitrary keys, but the envelope's own structure must carry none).
#[test]
fn no_authority_key_in_the_structural_envelope() {
    // Exact keys that would mint or smuggle authority. `requested_grants`,
    // `credential_ref`, `secret_scope` are deliberately NOT here: a wish, a NAME,
    // and a list of NAMES respectively — never authority bytes.
    const FORBIDDEN: &[&str] = &[
        "warrant",
        "warrant_spec",
        "warrantspec",
        "tool_grants",
        "grants",
        "secret",
        "secrets",
        "secret_bytes",
        "credential",
        "credentials",
        "password",
        "api_key",
        "apikey",
        "private_key",
        "token",
        "access_token",
        "instance_id",
        "instanceid",
    ];
    // Keys whose VALUES are opaque user/wish bags — record the key, do not descend.
    const NO_DESCEND: &[&str] = &[
        "blueprint",
        "input_schema",
        "free_params",
        "requested_grants",
        "per_step",
        "params",
        "args",
        "tool_contract",
        "tools",
    ];

    fn walk(v: &Value, forbidden: &[&str], no_descend: &[&str]) {
        if let Value::Object(map) = v {
            for (k, val) in map {
                assert!(
                    !forbidden.contains(&k.as_str()),
                    "authority key {k:?} must never appear in an App envelope"
                );
                if !no_descend.contains(&k.as_str()) {
                    walk(val, forbidden, no_descend);
                }
            }
        } else if let Value::Array(a) = v {
            for item in a {
                walk(item, forbidden, no_descend);
            }
        }
    }

    let env = fully_populated();
    let value = serde_json::to_value(&env).unwrap();
    walk(&value, FORBIDDEN, NO_DESCEND);
}

/// A connection descriptor that smuggles URL userinfo is rejected at validation.
#[test]
fn url_userinfo_in_a_connection_is_rejected() {
    let mut env = fully_populated();
    env.references.connections[0].descriptor = "https://user:pw@mcp.example/sse".into();
    assert!(env.validate().is_err());
}

/// The fully-populated envelope is itself valid (the negatives above are the only failures).
#[test]
fn the_kitchen_sink_envelope_validates() {
    fully_populated().validate().unwrap();
}
