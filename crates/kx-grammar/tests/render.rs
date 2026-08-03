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
        // Resolves to the SAME granted tool the grammar pinned (exact grant).
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

/// THE PARITY TEST. One fixture, both legs — the two renderers must constrain the
/// SAME things.
///
/// This exists because they did not. The GBNF leg rendered the declared parameters
/// while the Ollama leg emitted `"args": {"type":"object"}`, so an identical warrant
/// constrained one engine and not the other. Nothing compared them, which is exactly
/// how the disagreement survived: each leg had tests, and each leg passed its own.
///
/// What both legs deliberately leave to `validate_args` — numeric bounds and length
/// caps — is asserted as ABSENT on both, so parity cannot drift in that direction
/// either.
#[test]
fn both_legs_constrain_the_same_declared_params() {
    let schema = InputSchema {
        params: vec![
            ParamSpec {
                name: "op".into(),
                ty: ParamType::Enum {
                    allowed: ["add", "sub"].iter().map(|s| (*s).into()).collect(),
                },
                required: true,
            },
            ParamSpec {
                name: "a".into(),
                ty: ParamType::Int {
                    min: Some(0),
                    max: Some(9),
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

    // --- llama.cpp leg ---
    let gbnf = spec.to_gbnf();
    assert!(gbnf.contains("args0 ::="), "GBNF emits a typed args rule");

    // --- Ollama leg ---
    let args = &spec.to_ollama_format()["properties"]["tool_call"]["properties"]["args"];
    assert_eq!(args["type"], "object");
    assert_eq!(
        args["properties"]["op"],
        serde_json::json!({ "type": "string", "enum": ["add", "sub"] }),
        "the enum's allowed values reach Ollama, not just the word \"enum\""
    );
    assert_eq!(
        args["properties"]["a"],
        serde_json::json!({"type":"integer"})
    );
    assert_eq!(
        args["properties"]["verbose"],
        serde_json::json!({"type":"boolean"})
    );
    assert_eq!(
        args["required"],
        serde_json::json!(["op", "a"]),
        "only the REQUIRED params are required; declared order preserved"
    );
    assert_eq!(
        args["additionalProperties"],
        serde_json::json!(false),
        "deny_unknown closes the object"
    );

    // --- what NEITHER leg constrains, asserted on both ---
    assert!(
        !gbnf.contains("[0-9]") || !gbnf.contains("minimum"),
        "GBNF leaves the int bound to validate_args"
    );
    for absent in ["minimum", "maximum", "maxLength"] {
        assert!(
            args["properties"]["a"].get(absent).is_none()
                && args["properties"]["op"].get(absent).is_none(),
            "Ollama leaves {absent} to validate_args, same as the GBNF leg"
        );
    }
}

/// A typed spec splits into one arm PER TOOL, and each arm pins its own
/// name/version pair. The flat envelope could not: it enumerated names and left
/// `version` free, so a model could pair one tool's name with another's version.
#[test]
fn typed_arms_pin_each_name_to_its_own_version() {
    let schema = InputSchema {
        params: vec![ParamSpec {
            name: "key".into(),
            ty: ParamType::Str { max_len: 256 },
            required: true,
        }],
        deny_unknown: true,
    };
    let spec = ToolEnvelopeSpec::new(vec![
        ToolSpec::with_schema("kv/get", "2", schema),
        ToolSpec::new("calc/add", "1"), // untyped: keeps generic-object args
    ]);
    let arms = spec.to_ollama_format();
    let arms = arms["oneOf"]
        .as_array()
        .expect("typed spec ⇒ per-tool arms");
    assert_eq!(arms.len(), 2, "one arm per granted tool");

    let props = |i: usize| arms[i]["properties"]["tool_call"]["properties"].clone();
    // Canonical order sorts calc/add before kv/get.
    assert_eq!(props(0)["name"]["enum"], serde_json::json!(["calc/add"]));
    assert_eq!(props(0)["version"]["enum"], serde_json::json!(["1"]));
    assert_eq!(
        props(0)["args"],
        serde_json::json!({"type":"object"}),
        "an untyped tool in a mixed set keeps generic-object args"
    );
    assert_eq!(props(1)["name"]["enum"], serde_json::json!(["kv/get"]));
    assert_eq!(props(1)["version"]["enum"], serde_json::json!(["2"]));
    assert_eq!(props(1)["args"]["properties"]["key"]["type"], "string");
}

/// The union splices the tool arms BESIDE the answer arm rather than nesting a
/// `oneOf` inside a `oneOf`, and the arms stay mutually exclusive.
#[test]
fn a_typed_union_stays_one_level_deep_and_disjoint() {
    let schema = InputSchema {
        params: vec![ParamSpec {
            name: "key".into(),
            ty: ParamType::Str { max_len: 256 },
            required: true,
        }],
        deny_unknown: true,
    };
    let spec = ToolEnvelopeSpec::new(vec![ToolSpec::with_schema("kv/get", "1", schema)]);
    let union = spec.to_ollama_union_format();
    let arms = union["oneOf"].as_array().expect("union ⇒ oneOf arms");
    assert_eq!(arms.len(), 2, "one tool arm + the answer arm");
    for arm in arms {
        assert!(
            arm.get("oneOf").is_none(),
            "no nested alternation: {}",
            serde_json::to_string(arm).unwrap()
        );
    }
    // Disjoint by required key — `oneOf` matches exactly one arm.
    assert_eq!(arms[0]["required"], serde_json::json!(["tool_call"]));
    assert_eq!(arms[1]["required"], serde_json::json!(["answer"]));
    assert_eq!(arms[1]["additionalProperties"], serde_json::json!(false));
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

/// T-GEMMA3-TOOL-LOOP-ANSWER-FORCE: the GBNF renderer (llama.cpp) reads ONLY `spec.tools`
/// — `answer_only` (like `strict`/`answerable`) never enters the grammar. So arming
/// answer-force is a NO-OP on llama.cpp: its lazy/triggered GBNF is byte-identical and it
/// already completes the loop (the gemma3 gap is Ollama-only). This pins that invariant so
/// a future GBNF edit can't silently start honoring the flag.
#[test]
fn gbnf_ignores_answer_only() {
    let base = ToolEnvelopeSpec::new(vec![ToolSpec::new("slack/read_channel", "1")]);
    let answer_only = ToolEnvelopeSpec::new(vec![ToolSpec::new("slack/read_channel", "1")])
        .with_answer_only(true);
    assert_eq!(
        base.to_gbnf(),
        answer_only.to_gbnf(),
        "answer_only must not change the llama.cpp GBNF (llama.cpp already completes the loop)"
    );
}

/// **Characterising the typed-args path.**
///
/// ⚠ This doc block used to say `ToolSpec::with_schema` "has no production caller —
/// `build_tool_grammar` builds every live spec with `ToolSpec::new`". That was written
/// describing the state BEFORE the change that added this very test, and it stopped being
/// true in the same commit: `build_tool_grammar` calls `with_schema` for any tool whose
/// schema is `deny_unknown`. Left uncorrected it invites the reader to treat every
/// assertion below as characterising dead code.
///
/// The shapes a real registry produces are pinned here.
///
/// The load-bearing case is ALL-OPTIONAL, because it is what a paginated tool declares:
/// `fleet/page` has one optional `cursor`, and the roster call that failed live was a
/// malformed args object for exactly that schema.
#[test]
fn an_all_optional_schema_can_express_every_subset_the_validator_accepts() {
    let schema = InputSchema {
        params: vec![
            ParamSpec {
                name: "a".into(),
                ty: ParamType::Bool,
                required: false,
            },
            ParamSpec {
                name: "b".into(),
                ty: ParamType::Bool,
                required: false,
            },
        ],
        deny_unknown: true,
    };
    let gbnf =
        ToolEnvelopeSpec::new(vec![ToolSpec::with_schema("t", "1", schema.clone())]).to_gbnf();

    // `validate_args` accepts {} , {"a":…} , {"b":…} and {"a":…,"b":…} — every subset, in any
    // order, because it parses a MAP. A grammar that cannot express one of those is STRICTER
    // than the validator it is supposed to front, and the model loses a legal call.
    for args in [
        r#"{}"#,
        r#"{"a":true}"#,
        r#"{"b":true}"#,
        r#"{"a":true,"b":true}"#,
    ] {
        assert!(
            kx_tool_registry::validate_args(&schema, args.as_bytes()).is_ok(),
            "the validator accepts {args}"
        );
    }
    // The grammar must therefore offer `b` WITHOUT `a`. Rendered as
    // `( a ( "," b )? )?` it cannot: dropping `a` drops `b` with it.
    let args_rule = gbnf
        .lines()
        .find(|l| l.starts_with("args0 ::="))
        .unwrap_or_default()
        .to_string();
    // Each optional must be reachable as the FIRST member, or the grammar is narrower than
    // the validator: rendered as `( a ( "," b )? )?` the language is {} | {a} | {a,b}, and a
    // model that wants only `b` is forced to also send `a`.
    assert!(
        args_rule.contains(r#""\"b\"" ws ":" ws ( "true" | "false" ) )? ws "}""#)
            || args_rule.split(r#""\"b\"""#).count() >= 3,
        "every optional must be reachable as the first member: {args_rule}"
    );
}
