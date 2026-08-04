//! GBNF rendering (llama.cpp dialect) of a [`crate::ToolEnvelopeSpec`].
//!
//! Produces a grammar whose ONLY accepting strings are canonical tool-call
//! envelopes `{"tool_call":{"name":<granted>,"version":<granted>,"args":{…}}}`
//! with the `(name, version)` pair drawn from the spec. The args are either a
//! generic JSON object (envelope-first default) or the tool's typed parameter
//! object (the per-tool arg-schema stretch).
//!
//! The grammar is fed to `llama_sampler_init_grammar{,_lazy_patterns}` as the
//! `root`-rooted GBNF. Conservative syntax only (no `\xNN`, no `{n}` counts) so it
//! parses on the pinned llama.cpp.

use std::fmt::Write as _;

use kx_tool_registry::{ParamSpec, ParamType};

use crate::spec::ToolEnvelopeSpec;

/// The shared JSON value rules — emitted once, referenced by the per-tool rules.
/// `ws` is whitespace; `jstring`/`jchar`/`hex` are a conservative JSON string;
/// `integer`/`number` cover the numeric forms; `object`/`array`/`value` are the
/// generic JSON value used for envelope-mode (untyped) args.
const SHARED_RULES: &str = concat!(
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

/// Render the spec to a complete, `root`-rooted GBNF grammar string, arming only the
/// runtime's own canonical envelope. Equivalent to [`render_for`] with
/// [`kx_toolcall::KNOWN_DIALECTS`] filtered to the envelope alone — kept as the shape the
/// goldens pin.
pub(crate) fn render(spec: &ToolEnvelopeSpec) -> String {
    render_for(spec, &[kx_toolcall::CANONICAL_ENVELOPE])
}

/// Render the spec against a set of tool-call **dialects**.
///
/// The root alternates over the dialects, so whichever syntax the model opens with, the
/// sampler can constrain its ARGUMENTS. This is the half of the fix that must land with
/// the trigger set and never separately: llama.cpp replays the trigger's capture group
/// into the grammar and `llama_grammar_accept_token` *throws* when the grammar cannot
/// accept it, with no `catch` before the C boundary. A dialect armed without a matching
/// branch here takes the process down rather than degrading.
///
/// The alternation is PREFIX-FACTORED — `root ::= call-envelope | d0 | d1 …`, each headed
/// by a distinct terminal — rather than inlining every tool into the root. llama.cpp
/// expands non-terminals eagerly at the leftmost position and rejects candidates across
/// every live stack on every sampled token, so a flat root would materialise
/// `dialects × tools` stacks at the very first character.
pub(crate) fn render_for(spec: &ToolEnvelopeSpec, dialects: &[kx_toolcall::ToolDialect]) -> String {
    // Defensive: an empty spec must never emit an empty alternation (invalid
    // GBNF). The caller guarantees this is non-empty before arming a grammar;
    // if it ever isn't, fall back to "any JSON object" rather than a broken rule.
    if spec.tools.is_empty() {
        return format!("root ::= object\n{SHARED_RULES}");
    }

    let mut out = String::new();

    // Which tools may appear in a NATIVE (version-less) branch. The native dialects carry
    // no version, so a bare name is only safe when it resolves back to exactly one grant;
    // otherwise the grammar would make reachable a call the parser answers with a LOUD
    // `Ambiguous`, i.e. a grammar that manufactures refusals. `native_name_resolves_uniquely`
    // is the parser's own predicate, so this stays mechanical rather than asserted.
    let granted: Vec<&str> = spec.tools.iter().map(|t| t.name.as_str()).collect();
    let native: Vec<usize> = (0..spec.tools.len())
        .filter(|&i| {
            gbnf_safe_bare_name(&spec.tools[i].name)
                && kx_toolcall::native_name_resolves_uniquely(&spec.tools[i].name, &granted)
        })
        .collect();

    // The marker dialects that can actually be rendered: well-formed delimiters, and at
    // least one pinnable tool. A dialect with nothing to offer is DROPPED from the root
    // rather than emitted as an empty alternation.
    let armed: Vec<&kx_toolcall::ToolDialect> = dialects
        .iter()
        .filter(|d| {
            d.shape != kx_toolcall::DialectShape::CanonicalEnvelope
                && d.is_well_formed()
                && !native.is_empty()
        })
        .collect();
    let envelope = dialects
        .iter()
        .any(|d| d.shape == kx_toolcall::DialectShape::CanonicalEnvelope);

    // With no marker dialect to alternate over, emit the pre-dialect shape BYTE FOR BYTE:
    // `root` IS the envelope, with no indirection. That keeps the shipped canonical-only
    // grammar — and its golden — provably untouched, so the blast radius of this change is
    // exactly the dialect path and nothing else.
    if armed.is_empty() {
        out.push_str("root ::= \"{\" ws ");
        out.push_str(&str_terminal("\"tool_call\""));
        out.push_str(" ws \":\" ws call ws \"}\"\n");
    } else {
        let mut roots: Vec<String> = Vec::with_capacity(armed.len() + 1);
        if envelope {
            roots.push("call-envelope".to_string());
        }
        for k in 0..armed.len() {
            roots.push(format!("d{k}"));
        }
        let _ = writeln!(out, "root ::= {}", roots.join(" | "));
        out.push_str("call-envelope ::= \"{\" ws ");
        out.push_str(&str_terminal("\"tool_call\""));
        out.push_str(" ws \":\" ws call ws \"}\"\n");
    }

    // One rule per armed dialect, then the per-tool branches beneath it.
    let (dialect_rules, needs_argkey) = render_dialect_rules(spec, &armed, &native);

    // call: one branch per granted tool.
    out.push_str("call ::= ");
    let branches: Vec<String> = (0..spec.tools.len()).map(|i| format!("call{i}")).collect();
    out.push_str(&branches.join(" | "));
    out.push('\n');

    // call{i}: the name/version pair pinned + the args rule for that tool.
    let mut typed_args = String::new();
    for (i, tool) in spec.tools.iter().enumerate() {
        let args_ref = match &tool.arg_schema {
            None => "object".to_string(),
            Some(_) => format!("args{i}"),
        };
        let _ = writeln!(
            out,
            "call{i} ::= \"{{\" ws {name_key} ws \":\" ws {name_val} ws \",\" ws \
             {ver_key} ws \":\" ws {ver_val} ws \",\" ws {args_key} ws \":\" ws {args_ref} ws \"}}\"",
            name_key = str_terminal("\"name\""),
            name_val = json_string_terminal(&tool.name),
            ver_key = str_terminal("\"version\""),
            ver_val = json_string_terminal(&tool.version),
            args_key = str_terminal("\"args\""),
        );
        if let Some(schema) = &tool.arg_schema {
            typed_args.push_str(&render_typed_args(i, &schema.params));
        }
    }

    out.push_str(&dialect_rules);
    if needs_argkey {
        // The three aliases `kx_toolcall`'s named-object decoder accepts. Emitting ONE of
        // them satisfies its "two or more aliases present ⇒ ambiguous ⇒ refuse" rule.
        let _ = writeln!(
            out,
            "argkey ::= {} | {} | {}",
            str_terminal("\"arguments\""),
            str_terminal("\"args\""),
            str_terminal("\"parameters\""),
        );
    }
    out.push_str(&typed_args);
    out.push_str(SHARED_RULES);
    out
}

/// Emit `d{k}` plus its per-tool branches for every armed dialect.
///
/// Split out of [`render_for`] so each half reads on its own: the caller decides WHICH
/// dialects are renderable and what the root looks like, this decides what each one's
/// grammar branch IS. Returns the rules and whether any of them referenced `argkey`.
fn render_dialect_rules(
    spec: &ToolEnvelopeSpec,
    armed: &[&kx_toolcall::ToolDialect],
    native: &[usize],
) -> (String, bool) {
    let mut rules = String::new();
    let mut needs_argkey = false;
    for (k, d) in armed.iter().enumerate() {
        let open = str_terminal(&d.open);
        // The close delimiter is OPTIONAL in every dialect that has one: requiring it would
        // strand a truncated-but-legal call, and omitting it would mask the very token the
        // model's own template trained it to emit next.
        let close = d
            .close
            .as_deref()
            .map_or_else(String::new, |c| format!(" ( {} )?", str_terminal(c)));
        match d.shape {
            kx_toolcall::DialectShape::NameThenArgs => {
                let marker = d
                    .call_marker
                    .as_deref()
                    .map_or_else(String::new, |m| format!(" ( {} )?", str_terminal(m)));
                let _ = writeln!(rules, "d{k} ::= {open}{marker} ws n{k} ws{close}");
                let alts: Vec<String> = native.iter().map(|i| format!("n{k}-{i}")).collect();
                let _ = writeln!(rules, "n{k} ::= {}", alts.join(" | "));
                for &i in native {
                    let _ = writeln!(
                        rules,
                        "n{k}-{i} ::= {name} ws {args}",
                        name = str_terminal(&spec.tools[i].name),
                        args = args_rule_ref(&spec.tools[i], i),
                    );
                }
            }
            kx_toolcall::DialectShape::NamedObject => {
                needs_argkey = true;
                let _ = writeln!(rules, "d{k} ::= {open} ws o{k} ws{close}");
                let alts: Vec<String> = native.iter().map(|i| format!("o{k}-{i}")).collect();
                let _ = writeln!(rules, "o{k} ::= {}", alts.join(" | "));
                for &i in native {
                    let _ = writeln!(
                        rules,
                        "o{k}-{i} ::= \"{{\" ws {name_key} ws \":\" ws {name_val} ws \",\" ws \
                         argkey ws \":\" ws {args} ws \"}}\"",
                        name_key = str_terminal("\"name\""),
                        name_val = json_string_terminal(&spec.tools[i].name),
                        args = args_rule_ref(&spec.tools[i], i),
                    );
                }
            }
            kx_toolcall::DialectShape::CanonicalEnvelope => unreachable!("filtered by render_for"),
        }
    }
    (rules, needs_argkey)
}

/// The args rule a branch refers to: the tool's typed `args{i}` when it declares a schema,
/// else the generic `object`. Every dialect branch for tool `i` refers to the SAME rule, so
/// "the native branch constrains args to the typed rule" is literally true rather than a
/// parallel implementation that can drift.
fn args_rule_ref(tool: &crate::spec::ToolSpec, idx: usize) -> String {
    match &tool.arg_schema {
        None => "object".to_string(),
        Some(_) => format!("args{idx}"),
    }
}

/// Every rule name this module generates, for the charset guard.
///
/// ⚠ llama.cpp's GBNF `is_word_char` is `[a-zA-Z0-9-]` — it does **not** include `_`.
/// A rule named `call_envelope` or `n0_0` makes the whole grammar unparseable, which
/// fails the entire dispatch rather than degrading it. Measured, not assumed: the first
/// version of the dialect renderer used underscores and llama.cpp refused it outright.
#[cfg(test)]
pub(crate) fn generated_rule_names(rendered: &str) -> Vec<&str> {
    rendered
        .lines()
        .filter_map(|l| l.split_once("::=").map(|(name, _)| name.trim()))
        .collect()
}

/// True iff a tool id is safe to emit as a BARE GBNF terminal in a native branch.
///
/// A native branch pins the name unquoted, so a name carrying a quote, a backslash, a
/// newline or a `[` would render a grammar llama.cpp REFUSES to parse — and a refused
/// grammar fails the whole dispatch rather than degrading it. Such a tool keeps its
/// (JSON-escaped) canonical-envelope branch and is simply absent from the native ones.
fn gbnf_safe_bare_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'))
}

/// Render `args{idx} ::= …` constraining the object to the declared params:
/// required params in canonical order, optional params as trailing optional
/// groups (the model emits a fixed order the order-tolerant `validate_args`
/// accepts). Numeric bounds / lengths are left to `validate_args` (a tight GBNF
/// digit-range is brittle for weak models — D108.2 envelope-first rationale).
fn render_typed_args(idx: usize, params: &[ParamSpec]) -> String {
    let required: Vec<&ParamSpec> = params.iter().filter(|p| p.required).collect();
    let optional: Vec<&ParamSpec> = params.iter().filter(|p| !p.required).collect();

    let members = if !required.is_empty() {
        let mut s = required
            .iter()
            .map(|p| member_fragment(p))
            .collect::<Vec<_>>()
            .join(" ws \",\" ws ");
        for p in &optional {
            let _ = write!(s, " ( ws \",\" ws {} )?", member_fragment(p));
        }
        s
    } else if optional.is_empty() {
        String::new()
    } else {
        // ALL-OPTIONAL, and the shape matters: `validate_args` parses a MAP, so it accepts
        // EVERY subset of the declared optionals. A grammar that is narrower than the
        // validator it fronts silently costs the model a legal call.
        //
        // The obvious rendering — `( o1 ( "," o2 )? … )?` — hoists `o1` into a mandatory
        // opener, so the language is `{}` | `{o1}` | `{o1,o2}` and `{o2}` alone is
        // UNREPRESENTABLE. Measured on a two-optional schema before this branch was ever
        // reached in production.
        //
        // Instead: alternate over WHICH optional appears first, and let every later one stay
        // independently skippable (the same chain the required branch above uses, which is
        // already correct because a required member anchors it). Declared order is preserved
        // within each alternative — that is the tool's identity contract and the validator
        // does not care about key order. O(n²) in text for a parameter list, which is small.
        let alts: Vec<String> = (0..optional.len())
            .map(|first| {
                let mut s = member_fragment(optional[first]);
                for p in &optional[first + 1..] {
                    let _ = write!(s, " ( ws \",\" ws {} )?", member_fragment(p));
                }
                s
            })
            .collect();
        // The empty object is a legal call for an all-optional schema, so it is an explicit
        // alternative rather than an outer `?` (which would reintroduce the hoist).
        format!("( {} | \"\" )", alts.join(" | "))
    };

    if members.is_empty() {
        format!("args{idx} ::= \"{{\" ws \"}}\"\n")
    } else {
        format!("args{idx} ::= \"{{\" ws {members} ws \"}}\"\n")
    }
}

/// A single declared member: `"<key>" ws ":" ws <value-rule>`.
fn member_fragment(p: &ParamSpec) -> String {
    format!(
        "{key} ws \":\" ws {val}",
        key = json_string_terminal(&p.name),
        val = value_fragment(&p.ty),
    )
}

/// The GBNF value fragment for a declared [`ParamType`].
fn value_fragment(ty: &ParamType) -> String {
    match ty {
        ParamType::Bool => "( \"true\" | \"false\" )".to_string(),
        ParamType::Int { .. } => "integer".to_string(),
        ParamType::Str { .. } | ParamType::Bytes { .. } => "jstring".to_string(),
        ParamType::Enum { allowed } => {
            if allowed.is_empty() {
                "jstring".to_string()
            } else {
                let alts: Vec<String> = allowed.iter().map(|v| json_string_terminal(v)).collect();
                format!("( {} )", alts.join(" | "))
            }
        }
    }
}

/// A GBNF terminal matching the JSON encoding of `value` as a string (i.e. the
/// model must emit `"<value>"`, JSON-escaped). Used for tool names, versions,
/// param keys, and enum values.
fn json_string_terminal(value: &str) -> String {
    let json = serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string());
    str_terminal(&json)
}

/// A GBNF double-quoted terminal matching the EXACT characters of `text`,
/// escaping `"` and `\` for GBNF.
fn str_terminal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod rule_name_guard {
    use super::*;
    use crate::spec::{ToolEnvelopeSpec, ToolSpec};

    /// llama.cpp's `is_word_char` for GBNF rule names: `[a-zA-Z0-9-]`. **No underscore.**
    fn is_llamacpp_rule_name(name: &str) -> bool {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
    }

    /// ⚠ THE REGRESSION THIS PINS. The first dialect renderer emitted `call_envelope`
    /// and `n0_0`; llama.cpp refused the whole grammar, which fails the entire dispatch
    /// rather than degrading it. A rule name is not a matter of taste here.
    #[test]
    fn every_generated_rule_name_is_legal_gbnf() {
        let schema = kx_tool_registry::InputSchema {
            params: vec![ParamSpec {
                name: "key".into(),
                ty: ParamType::Int {
                    min: None,
                    max: None,
                },
                required: true,
            }],
            deny_unknown: true,
        };
        let spec = ToolEnvelopeSpec::new(vec![
            ToolSpec::new("calc/add", "1"),
            ToolSpec::with_schema("kv/get", "1", schema),
        ]);
        for rendered in [
            spec.to_gbnf(),
            spec.to_gbnf_for(kx_toolcall::KNOWN_DIALECTS),
        ] {
            for name in generated_rule_names(&rendered) {
                assert!(
                    is_llamacpp_rule_name(name),
                    "generated rule name {name:?} is not `[a-zA-Z0-9-]`, so llama.cpp \
                     refuses the whole grammar.\n--- grammar ---\n{rendered}"
                );
            }
        }
    }

    /// The guard must be able to fail — an underscore is what it exists to catch.
    #[test]
    fn the_rule_name_guard_rejects_an_underscore() {
        assert!(is_llamacpp_rule_name("call-envelope"));
        assert!(is_llamacpp_rule_name("n0-1"));
        assert!(!is_llamacpp_rule_name("call_envelope"));
        assert!(!is_llamacpp_rule_name("n0_1"));
    }
}
