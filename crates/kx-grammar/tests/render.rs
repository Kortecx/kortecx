// Integration tests for kx-grammar: the rendered constraint must (1) target the
// EXACT envelope the authority-gate parser accepts, and (2) render deterministic,
// well-formed GBNF / Ollama schemas. Tests use `.unwrap()`/`.expect()` for
// fixture construction (workspace lints deny these in lib code, allow in tests).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]
//! Integration tests for `kx-grammar`: the rendered constraint targets the exact
//! envelope the authority-gate parser accepts, and renders deterministic GBNF /
//! Ollama schemas (incl. the typed-args stretch).

use std::collections::BTreeSet;

use kx_grammar::{GrammarSpec, PermutationSpec, ToolEnvelopeSpec, ToolSpec};
use kx_tool_registry::{InputSchema, ParamSpec, ParamType};
use kx_toolcall::parse_tool_call;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, ToolGrant,
    WarrantSpec,
};

use kx_mote::{ModelId, ToolName, ToolVersion};

/// A warrant granting the given `(tool_id, version)` pairs — mirrors the
/// `kx-toolcall` test helper so the round-trip exercises the REAL gate.
fn warrant_granting(tools: &[(&str, &str)]) -> WarrantSpec {
    let mut tool_grants = BTreeSet::new();
    for (id, ver) in tools {
        tool_grants.insert(ToolGrant {
            tool_id: ToolName((*id).into()),
            tool_version: ToolVersion((*ver).into()),
        });
    }
    WarrantSpec {
        mote_class: MoteClass::WorldMutating,
        nd_class: MoteClass::WorldMutating,
        fs_scope: FsScope::empty(),
        net_scope: NetScope::None,
        syscall_profile_ref: kx_content::ContentRef::from_bytes([0; 32]),
        tool_grants,
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1024,
            max_output_tokens: 256,
            max_calls: 8,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 0,
            mem_bytes: 0,
            wall_clock_ms: 1000,
            fd_count: 0,
            disk_bytes: 0,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// THE contract: an envelope of the exact shape the grammar enforces is ACCEPTED
/// by the real authority-gate parser, for EACH granted tool. If this holds, a
/// grammar-constrained model produces only parser-decodable calls.
#[test]
fn rendered_envelope_shape_is_accepted_by_the_parser() {
    let tools = [("mcp-echo/echo", "1"), ("calc/add", "1"), ("kv/get", "1")];
    let spec = ToolEnvelopeSpec::new(tools.iter().map(|(n, v)| ToolSpec::new(*n, *v)).collect());
    assert!(!spec.is_empty());
    let warrant = warrant_granting(&tools);

    for (name, ver) in tools {
        // Exactly what the grammar's `call{i}` branch admits: name+version pinned,
        // args a JSON object.
        let envelope =
            format!(r#"{{"tool_call":{{"name":"{name}","version":"{ver}","args":{{"x":1}}}}}}"#);
        let decoded = parse_tool_call(envelope.as_bytes(), &warrant, 4096)
            .unwrap_or_else(|e| panic!("grammar-shaped envelope for {name} must parse, got {e:?}"))
            .unwrap_or_else(|| panic!("grammar-shaped envelope for {name} must be a call"));
        // Resolves to the SAME granted tool the grammar pinned (SN-8 exact grant).
        assert_eq!(
            decoded.name,
            ToolName(name.into()),
            "name resolves to the grant"
        );
        assert_eq!(
            decoded.args_bytes,
            br#"{"x":1}"#.to_vec(),
            "args carried verbatim"
        );
    }
}

/// The GBNF has one `call{i}` branch per tool, each pinning the JSON-string name
/// and version, plus a complete `root` and the shared JSON rules.
#[test]
fn gbnf_pins_each_granted_tool() {
    let spec = ToolEnvelopeSpec::new(vec![
        ToolSpec::new("calc/add", "1"),
        ToolSpec::new("mcp-echo/echo", "2"),
    ]);
    let gbnf = spec.to_gbnf();

    assert!(
        gbnf.starts_with("root ::= \"{\" ws "),
        "rooted envelope: {gbnf}"
    );
    assert!(
        gbnf.contains("call ::= call0 | call1\n"),
        "one branch per tool: {gbnf}"
    );
    // Sorted canonical order: calc/add before mcp-echo/echo.
    assert!(
        gbnf.contains(r#""\"calc/add\"""#),
        "calc name pinned as JSON string"
    );
    assert!(
        gbnf.contains(r#""\"mcp-echo/echo\"""#),
        "echo name pinned as JSON string"
    );
    assert!(gbnf.contains(r#""\"version\"""#) && gbnf.contains(r#""\"args\"""#));
    // Shared JSON rules are present so the grammar is self-contained.
    for rule in [
        "object ::=",
        "value ::=",
        "jstring ::=",
        "number ::=",
        "ws ::=",
    ] {
        assert!(gbnf.contains(rule), "missing shared rule {rule}");
    }
    // Envelope-first: args reference the generic object (no args{i} rule).
    assert!(gbnf.contains("ws \"args\"") || gbnf.contains(r#""\"args\"" ws"#));
    assert!(
        !gbnf.contains("args0 ::="),
        "envelope-first has no typed args rule"
    );
}

/// With a per-tool arg schema, the GBNF emits a typed `args{i}` rule: required
/// params mandatory, optionals trailing, enum as alternation, bool as true|false.
#[test]
fn gbnf_renders_typed_args_for_the_stretch() {
    let schema = InputSchema {
        params: vec![
            ParamSpec {
                name: "a".into(),
                ty: ParamType::Int {
                    min: None,
                    max: None,
                },
                required: true,
            },
            ParamSpec {
                name: "op".into(),
                ty: ParamType::Enum {
                    allowed: ["add", "sub"].iter().map(|s| (*s).into()).collect(),
                },
                required: true,
            },
            ParamSpec {
                name: "verbose".into(),
                ty: ParamType::Bool,
                required: false,
            },
        ],
        deny_unknown: true,
    };
    let spec = ToolEnvelopeSpec::new(vec![ToolSpec::with_schema("calc/add", "1", schema)]);
    let gbnf = spec.to_gbnf();

    assert!(gbnf.contains("call0 ::="), "the single tool branch");
    assert!(gbnf.contains("args0 ::="), "a typed args rule is emitted");
    // required `a`(int) then `op`(enum) joined by a comma; optional `verbose` trailing.
    assert!(
        gbnf.contains(r#""\"a\"" ws ":" ws integer"#),
        "int param a: {gbnf}"
    );
    assert!(
        gbnf.contains(r#"( "\"add\"" | "\"sub\"" )"#)
            || gbnf.contains(r#"( "\"sub\"" | "\"add\"" )"#),
        "enum alternation: {gbnf}"
    );
    assert!(
        gbnf.contains(r#"( "true" | "false" )"#),
        "bool value: {gbnf}"
    );
    assert!(
        gbnf.contains(r#"( ws "," ws "\"verbose\""#),
        "optional verbose is a trailing group"
    );
}

/// The Ollama JSON schema constrains the tool name to the granted-id enum.
#[test]
fn ollama_format_enumerates_granted_names() {
    let spec = ToolEnvelopeSpec::new(vec![
        ToolSpec::new("calc/add", "1"),
        ToolSpec::new("kv/get", "1"),
    ]);
    let schema = spec.to_ollama_format();
    let names = &schema["properties"]["tool_call"]["properties"]["name"]["enum"];
    assert_eq!(
        names,
        &serde_json::json!(["calc/add", "kv/get"]),
        "name enum = granted ids"
    );
    let required = &schema["properties"]["tool_call"]["required"];
    assert_eq!(required, &serde_json::json!(["name", "version", "args"]));
}

/// The spec serializes to / from the opaque `Grammar.raw` carrier byte-faithfully.
#[test]
fn spec_round_trips_through_the_carrier() {
    let spec = ToolEnvelopeSpec::new(vec![
        ToolSpec::new("calc/add", "1"),
        ToolSpec::new("kv/get", "1"),
    ]);
    let raw = spec.to_raw().expect("serialize");
    let back = ToolEnvelopeSpec::from_raw(&raw).expect("deserialize");
    assert_eq!(spec, back, "round-trip identity");
    // A corrupt carrier fails CLOSED (never silently unconstrains).
    assert!(ToolEnvelopeSpec::from_raw("not json").is_err());
}

/// Tools are canonicalized: sorted by (name, version) and de-duplicated, so the
/// rendered grammar is deterministic regardless of input order.
#[test]
fn tools_are_canonicalized() {
    let a = ToolEnvelopeSpec::new(vec![
        ToolSpec::new("kv/get", "1"),
        ToolSpec::new("calc/add", "1"),
        ToolSpec::new("calc/add", "1"), // duplicate
    ]);
    let b = ToolEnvelopeSpec::new(vec![
        ToolSpec::new("calc/add", "1"),
        ToolSpec::new("kv/get", "1"),
    ]);
    assert_eq!(a, b, "order + dedup canonicalized");
    assert_eq!(a.tools.len(), 2);
    assert_eq!(a.to_gbnf(), b.to_gbnf(), "deterministic GBNF");
}

/// An empty spec never emits a broken alternation (defensive — callers guard
/// `is_empty`, but a bug must degrade to valid GBNF, not invalid).
#[test]
fn empty_spec_renders_valid_fallback() {
    let spec = ToolEnvelopeSpec::new(vec![]);
    assert!(spec.is_empty());
    let gbnf = spec.to_gbnf();
    assert!(
        gbnf.starts_with("root ::= object\n"),
        "empty falls back to any-object: {gbnf}"
    );
    assert!(!gbnf.contains("call ::= \n"), "no empty alternation");
}

/// Full GBNF golden for the bundled-oracle grant set. This EXACT string is also
/// fed to `kx-llamacpp`'s `smoke_grammar_from_kx_grammar` test (which proves it
/// PARSES + builds a lazy sampler in llama.cpp). If this golden changes, re-sync
/// that smoke test — the two together close the loop: kx-grammar renders shape X,
/// llama.cpp accepts shape X. (kx-llamacpp can't depend on kx-grammar — layering.)
#[test]
fn gbnf_golden_for_bundled_oracles() {
    let spec = ToolEnvelopeSpec::new(vec![
        ToolSpec::new("calc/add", "1"),
        ToolSpec::new("kv/get", "1"),
    ]);
    let expected = concat!(
        "root ::= \"{\" ws \"\\\"tool_call\\\"\" ws \":\" ws call ws \"}\"\n",
        "call ::= call0 | call1\n",
        "call0 ::= \"{\" ws \"\\\"name\\\"\" ws \":\" ws \"\\\"calc/add\\\"\" ws \",\" ws \"\\\"version\\\"\" ws \":\" ws \"\\\"1\\\"\" ws \",\" ws \"\\\"args\\\"\" ws \":\" ws object ws \"}\"\n",
        "call1 ::= \"{\" ws \"\\\"name\\\"\" ws \":\" ws \"\\\"kv/get\\\"\" ws \",\" ws \"\\\"version\\\"\" ws \":\" ws \"\\\"1\\\"\" ws \",\" ws \"\\\"args\\\"\" ws \":\" ws object ws \"}\"\n",
        "object ::= \"{\" ws ( member ( ws \",\" ws member )* )? ws \"}\"\n",
        "member ::= jstring ws \":\" ws value\n",
        "array ::= \"[\" ws ( value ( ws \",\" ws value )* )? ws \"]\"\n",
        "value ::= object | array | jstring | number | \"true\" | \"false\" | \"null\"\n",
        "jstring ::= \"\\\"\" jchar* \"\\\"\"\n",
        "jchar ::= [^\"\\\\] | \"\\\\\" ([\"\\\\/bfnrt] | \"u\" hex hex hex hex)\n",
        "hex ::= [0-9a-fA-F]\n",
        "integer ::= \"-\"? (\"0\" | [1-9] [0-9]*)\n",
        "number ::= integer (\".\" [0-9]+)? ([eEfF] [-+]? [0-9]+)?\n",
        "ws ::= [ \\t\\n]*\n",
    );
    assert_eq!(
        spec.to_gbnf(),
        expected,
        "GBNF golden drift — re-sync the smoke test"
    );
}

// ── RC4c: the listwise-rerank PermutationSpec (Ollama `format` only) ─────────

#[test]
fn permutation_ollama_schema_is_a_fixed_length_int_array() {
    let schema = PermutationSpec::new(5).to_ollama_format();
    assert_eq!(schema["type"], "array");
    assert_eq!(schema["minItems"], 5);
    assert_eq!(schema["maxItems"], 5);
    assert_eq!(schema["uniqueItems"], true);
    assert_eq!(schema["items"]["type"], "integer");
    assert_eq!(schema["items"]["minimum"], 0);
    assert_eq!(schema["items"]["maximum"], 4); // [0, n) ⇒ max == n-1
}

#[test]
fn permutation_carrier_round_trips_and_is_distinct_from_tool_envelope() {
    let raw = GrammarSpec::Permutation(PermutationSpec::new(8))
        .to_raw()
        .unwrap();
    match GrammarSpec::from_raw(&raw).unwrap() {
        GrammarSpec::Permutation(p) => assert_eq!(p.n, 8),
        GrammarSpec::ToolEnvelope(_) => panic!("permutation raw must not decode as tool-envelope"),
    }
    // An existing tool-envelope raw still decodes as ToolEnvelope (back-compat).
    let tool_raw = ToolEnvelopeSpec::new(vec![ToolSpec::new("retrieve", "1")])
        .to_raw()
        .unwrap();
    assert!(matches!(
        GrammarSpec::from_raw(&tool_raw).unwrap(),
        GrammarSpec::ToolEnvelope(_)
    ));
}
