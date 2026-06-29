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

/// Render the spec to a complete, `root`-rooted GBNF grammar string.
pub(crate) fn render(spec: &ToolEnvelopeSpec) -> String {
    // Defensive: an empty spec must never emit an empty alternation (invalid
    // GBNF). The caller guarantees this is non-empty before arming a grammar;
    // if it ever isn't, fall back to "any JSON object" rather than a broken rule.
    if spec.tools.is_empty() {
        return format!("root ::= object\n{SHARED_RULES}");
    }

    let mut out = String::new();

    // root: the tool_call envelope wrapper.
    out.push_str("root ::= \"{\" ws ");
    out.push_str(&str_terminal("\"tool_call\""));
    out.push_str(" ws \":\" ws call ws \"}\"\n");

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

    out.push_str(&typed_args);
    out.push_str(SHARED_RULES);
    out
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
        // All-optional: the first opens the group, the rest are nested optionals.
        let mut s = format!("( {}", member_fragment(optional[0]));
        for p in &optional[1..] {
            let _ = write!(s, " ( ws \",\" ws {} )?", member_fragment(p));
        }
        s.push_str(" )?");
        s
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
