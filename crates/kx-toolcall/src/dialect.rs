//! Tool-call **dialects** — the one place that knows how a model spells a tool call.
//!
//! A dialect is the pair of delimiters a model wraps around a call plus the shape of
//! the body between them. Three consumers read this module and they MUST agree:
//!
//! 1. [`crate::parse_tool_call`] — what the runtime will ACCEPT back from a model.
//! 2. `kx-inference`'s lazy-GBNF trigger set — when the sampler ARMS.
//! 3. `kx-grammar`'s GBNF root — what the sampler then CONSTRAINS.
//!
//! **Why one module and not three tables.** They used to be maintained separately, and
//! that is exactly the defect this exists to close: the parser accepted four marker
//! dialects while the sampler armed on one, so every native call generated its args
//! completely unconstrained — the sampler logged *awaiting trigger* and engaged zero
//! times, while every "did the tool fire?" check stayed green because the tolerant
//! parser recovered the call either way.
//!
//! And the reverse mistake is worse than the original. llama.cpp feeds the grammar the
//! text starting at the trigger's **first capture group** and replays the already-sampled
//! tokens into it; `llama_grammar_accept_token` ends in a `throw` when the grammar cannot
//! accept them, and there is no `catch` between there and the C boundary. A trigger whose
//! capture group is not a valid grammar prefix therefore does not degrade — it takes the
//! process down. Arming and constraining are two halves of one artefact, so they read one
//! table, and [`ToolDialect::trigger_pattern`] is DERIVED rather than stored so the two
//! cannot drift apart even in principle.
//!
//! **Where a dialect comes from.** Preferably from the model itself: a GGUF carries its
//! own chat template, and a tool-aware template renders the delimiters. [`derive_dialect`]
//! recovers them by rendering a sentinel call through that template and reading the result
//! back, so onboarding a new model is a conformance run rather than a code change.
//! [`KNOWN_DIALECTS`] is the fallback for templates that do not mention tool calls at all,
//! the seed for backends that expose no template, and the fixture set for offline tests.

use std::borrow::Cow;

/// The shape of a call BODY, between a dialect's open and close delimiters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialectShape {
    /// `NAME{ARGS}` — a bare tool name then the verbatim args object, no JSON wrapper
    /// and **no version** (Gemma-4: `<|tool_call>call:fs_list{}`).
    NameThenArgs,
    /// `{"name": NAME, "arguments"|"args"|"parameters": {ARGS}}` — a JSON object after a
    /// marker (Hermes/Qwen `<tool_call>`, Llama `<|python_tag|>`). Also version-less.
    NamedObject,
    /// The runtime's OWN envelope, `{"tool_call":{"name":…,"version":…,"args":{…}}}`.
    /// Structural rather than marker-delimited — it is the only shape carrying a version,
    /// and the only one whose opener is not a literal, so it keeps a bespoke trigger.
    CanonicalEnvelope,
}

/// One tool-call dialect. Owns its strings so a dialect DERIVED from a model's template
/// at load time and one from [`KNOWN_DIALECTS`] are the same type to every consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDialect {
    /// Stable identifier — used in reports, per-dialect engagement counters and as the
    /// GBNF branch-rule stem. Never matched against model output.
    pub id: Cow<'static, str>,
    /// The open delimiter, verbatim. Empty ONLY for [`DialectShape::CanonicalEnvelope`].
    pub open: Cow<'static, str>,
    /// The close delimiter, if the dialect has one (`<|python_tag|>` has none). ALWAYS
    /// optional to both the parser and the grammar: a truncated call must stay decodable,
    /// so requiring a close would strand the sampler at the end of a legal call.
    pub close: Option<Cow<'static, str>>,
    /// An optional literal between the opener and the name (`call:`). Optional in the
    /// grammar too — a model that omits it still produces an accepted call.
    pub call_marker: Option<Cow<'static, str>>,
    /// The body shape.
    pub shape: DialectShape,
}

/// The canonical envelope's trigger, byte-identical to the one shipped before dialects
/// existed. Its capture group is `{"tool_call"` modulo whitespace, which is exactly the
/// prefix `kx-grammar`'s canonical root already accepts.
const CANONICAL_TRIGGER: &str = r#"[\s\S]*?(\{[ \t\n]*"tool_call")"#;

impl ToolDialect {
    /// The llama.cpp lazy-grammar trigger for this dialect.
    ///
    /// **Generated, never stored.** Capture group 1 is precisely the text llama.cpp will
    /// replay into the grammar, so it must be a prefix the GBNF accepts — see the module
    /// docs for why getting that wrong aborts the process rather than degrading. Deriving
    /// it from [`Self::open`] means a dialect cannot be edited into an inconsistent state.
    ///
    /// The leading `[\s\S]*?` is belt-and-braces: this llama.cpp pin falls back to
    /// `regex_search`, so a bare `(<\|tool_call>)` would match too. It is kept because a
    /// pin that returns to full-match-only semantics would silently stop arming, and
    /// because it keeps every pattern isomorphic to the canonical one.
    #[must_use]
    pub fn trigger_pattern(&self) -> String {
        if self.shape == DialectShape::CanonicalEnvelope {
            return CANONICAL_TRIGGER.to_string();
        }
        format!(r"[\s\S]*?({})", ecma_escape(&self.open))
    }

    /// The literal text capture group 1 will contain, which the GBNF must accept as a
    /// prefix. `None` for the canonical envelope, whose opener is a shape rather than a
    /// literal and whose grammar branch already exists.
    #[must_use]
    pub fn grammar_prefix(&self) -> Option<&str> {
        (self.shape != DialectShape::CanonicalEnvelope).then(|| self.open.as_ref())
    }

    /// True when this dialect's literals are safe to embed in a GBNF terminal and in an
    /// ECMAScript regex. A derived dialect comes from a model's template and is therefore
    /// untrusted input: a delimiter carrying a newline, a NUL or a quote would produce a
    /// grammar llama.cpp refuses to parse, which fails the whole dispatch rather than
    /// degrading it. Such a dialect is dropped and the fallback table is used instead.
    #[must_use]
    pub fn is_well_formed(&self) -> bool {
        fn safe(s: &str) -> bool {
            !s.is_empty()
                && s.len() <= 64
                && !s.contains(['\n', '\r', '\t', '"', '\\', '\0'])
                && s.chars().all(|c| !c.is_control())
        }
        if self.shape == DialectShape::CanonicalEnvelope {
            return true;
        }
        safe(&self.open)
            && self.close.as_deref().is_none_or(safe)
            && self.call_marker.as_deref().is_none_or(safe)
    }
}

/// Escape a literal for an ECMAScript regex (`std::regex`'s default grammar).
///
/// Not cosmetic. An unescaped `|` in `<|tool_call>` makes the pattern the alternation
/// `(<)|(tool_call>)`, which matches the first `<` in any prose the model writes — and
/// then feeds that `<` to a grammar that cannot accept it.
#[must_use]
pub fn ecma_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 2);
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '^'
                | '$'
                | '.'
                | '|'
                | '?'
                | '*'
                | '+'
                | '('
                | ')'
                | '['
                | ']'
                | '{'
                | '}'
                | '/'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

const fn borrowed(s: &'static str) -> Cow<'static, str> {
    Cow::Borrowed(s)
}

// The delimiter LITERALS, defined once. `parse.rs` projects its private constants from
// these rather than repeating them, so the set the parser accepts and the set the sampler
// arms on are the same bytes by construction rather than by review.

/// Gemma-4's tool-call opener.
pub const GEMMA_OPEN_LIT: &str = "<|tool_call>";
/// Gemma-4's tool-call close delimiter.
pub const GEMMA_CLOSE_LIT: &str = "<tool_call|>";
/// The optional `call:` literal between Gemma's opener and the tool name.
pub const GEMMA_CALL_MARKER_LIT: &str = "call:";
/// The Hermes/Qwen opener.
pub const HERMES_OPEN_LIT: &str = "<tool_call>";
/// The Hermes/Qwen close tag.
pub const HERMES_CLOSE_LIT: &str = "</tool_call>";
/// Llama 3.x's opener, which has no close delimiter.
pub const PYTHON_TAG_OPEN_LIT: &str = "<|python_tag|>";

/// Byte equality of two `&str` in a const context — the tool that lets `parse.rs` prove,
/// at compile time, that projecting its constants from this table changed no bytes.
#[must_use]
pub const fn str_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}

/// The runtime's own envelope. Always armed: it is what [`crate::parse_tool_call`] has
/// always accepted and what the `ReAct` system prompt teaches.
pub const CANONICAL_ENVELOPE: ToolDialect = ToolDialect {
    id: borrowed("canonical-envelope"),
    open: borrowed(""),
    close: None,
    call_marker: None,
    shape: DialectShape::CanonicalEnvelope,
};

/// Gemma-4's native form, `<|tool_call>call:NAME{ARGS}<tool_call|>` — taken from the
/// model's own embedded chat template, which renders exactly that.
pub const GEMMA4_NATIVE: ToolDialect = ToolDialect {
    id: borrowed("gemma4-native"),
    open: borrowed(GEMMA_OPEN_LIT),
    close: Some(borrowed(GEMMA_CLOSE_LIT)),
    call_marker: Some(borrowed(GEMMA_CALL_MARKER_LIT)),
    shape: DialectShape::NameThenArgs,
};

/// The Hermes/Qwen XML-ish form, `<tool_call>{"name":…}</tool_call>`. Distinct from
/// Gemma's opener by the `|`, and the parser's prefix match is exact, so they never
/// collide.
pub const HERMES_XML: ToolDialect = ToolDialect {
    id: borrowed("hermes-xml"),
    open: borrowed(HERMES_OPEN_LIT),
    close: Some(borrowed(HERMES_CLOSE_LIT)),
    call_marker: None,
    shape: DialectShape::NamedObject,
};

/// Llama 3.x's `<|python_tag|>{"name":…}`, which has no close delimiter.
pub const LLAMA_PYTHON_TAG: ToolDialect = ToolDialect {
    id: borrowed("llama-python-tag"),
    open: borrowed(PYTHON_TAG_OPEN_LIT),
    close: None,
    call_marker: None,
    shape: DialectShape::NamedObject,
};

/// The fallback table, in arming order — which is also the GBNF root-alternation order,
/// pinned by a golden so a reorder is a reviewed change rather than an accident.
///
/// This is NOT the primary source. A model that declares its dialect through its chat
/// template ([`derive_dialect`]) is believed first; this covers the models that do not,
/// backends that expose no template, and the offline grammar tests.
pub const KNOWN_DIALECTS: &[ToolDialect] = &[
    CANONICAL_ENVELOPE,
    GEMMA4_NATIVE,
    HERMES_XML,
    LLAMA_PYTHON_TAG,
];

// ---------------------------------------------------------------------------------
// Derivation from a model's own chat template
// ---------------------------------------------------------------------------------

/// Sentinels planted in the probe. Chosen to be absent from every plausible template and
/// to survive JSON escaping unchanged.
pub const PROBE_NAME: &str = "KXPROBENAME";
/// The probe's single argument key.
pub const PROBE_ARG: &str = "KXPROBEARG";
/// The probe's single argument value.
pub const PROBE_VAL: &str = "KXPROBEVAL";
/// The assistant text used by the CONTROL render, which carries no tool call.
pub const PROBE_TEXT: &str = "KXPROBETEXT";

/// The OpenAI-shaped chat messages a caller renders through the model's own template to
/// discover its dialect. The assistant turn carries one tool call built from the
/// sentinels above.
///
/// The caller applies the template (llama.cpp's `apply_chat_template`, or Ollama's
/// template from `/api/show`) and passes the result to [`derive_dialect`] together with
/// the render of [`control_messages_json`].
#[must_use]
pub fn probe_messages_json() -> String {
    format!(
        r#"[{{"role":"user","content":"hi"}},{{"role":"assistant","content":"","tool_calls":[{{"type":"function","function":{{"name":"{PROBE_NAME}","arguments":"{{\"{PROBE_ARG}\":\"{PROBE_VAL}\"}}"}}}}]}}]"#
    )
}

/// The CONTROL render: the same conversation with a plain assistant answer instead of a
/// tool call. Diffing the two renders is what separates the model's turn framing (which
/// both share) from its tool-call delimiters (which only the probe has) — without it we
/// would be guessing where the framing ends, per model.
#[must_use]
pub fn control_messages_json() -> String {
    format!(r#"[{{"role":"user","content":"hi"}},{{"role":"assistant","content":"{PROBE_TEXT}"}}]"#)
}

/// Recover a [`ToolDialect`] from what a model's own template produced for the probe.
///
/// Pure: it takes the two rendered strings and returns the dialect, so it is fully
/// testable against a captured template output with no model present.
///
/// `None` means the template does not render tool calls at all (the sentinel name never
/// appears), or the recovered delimiters are not safe to embed — in both cases the caller
/// falls back to [`KNOWN_DIALECTS`]. Returning `None` is an honest "this model did not
/// tell us", never a guess.
#[must_use]
pub fn derive_dialect(rendered_probe: &str, rendered_control: &str) -> Option<ToolDialect> {
    let name_at = rendered_probe.find(PROBE_NAME)?;

    // The two renders share the conversation up to the assistant turn's content. Whatever
    // follows that common prefix in the probe is the dialect's own opening.
    let common = common_prefix_len(rendered_probe, rendered_control).min(name_at);
    let prefix = rendered_probe.get(common..name_at)?.trim();
    if prefix.is_empty() {
        // The name appears with no delimiter before it — not a dialect we can arm on.
        return None;
    }

    let close = derive_close(rendered_probe, rendered_control);
    dialect_from_affixes(prefix, close.as_deref())
}

/// Build a dialect from the raw text a template emits BEFORE the tool name and AFTER the
/// arguments. Shared by both derivation entry points so they can never disagree about what
/// a given pair of affixes means.
fn dialect_from_affixes(prefix: &str, close: Option<&str>) -> Option<ToolDialect> {
    let prefix = prefix.trim();
    if prefix.is_empty() {
        return None;
    }
    // A `{` means the name is a JSON value inside an object, so the marker we arm on is
    // everything before that brace; otherwise the name is bare and any trailing text
    // (`call:`) is an optional marker the grammar makes skippable.
    let (open, call_marker, shape) = if let Some(brace) = prefix.find('{') {
        (prefix[..brace].trim_end(), None, DialectShape::NamedObject)
    } else if let Some(gt) = prefix.rfind('>') {
        let (o, m) = prefix.split_at(gt + 1);
        let m = m.trim();
        (o, (!m.is_empty()).then_some(m), DialectShape::NameThenArgs)
    } else {
        (prefix, None, DialectShape::NameThenArgs)
    };
    if open.is_empty() {
        return None;
    }

    let dialect = ToolDialect {
        id: Cow::Owned(format!("derived:{open}")),
        open: Cow::Owned(open.to_string()),
        close: close.map(|c| Cow::Owned(c.to_string())),
        call_marker: call_marker.map(|m| Cow::Owned(m.to_string())),
        shape,
    };
    dialect.is_well_formed().then_some(dialect)
}

/// Recover a [`ToolDialect`] from a model's raw **chat-template source** (the Jinja text a
/// GGUF carries in `tokenizer.chat_template`, or what Ollama returns from `/api/show`).
///
/// This is the entry point the runtime uses, because rendering a template with `tool_calls`
/// needs a Jinja engine and `llama_chat_apply_template` only accepts role/content pairs.
/// It reads the emission literals inside the template's own `for … in …tool_calls` loop —
/// the model's own statement of how it spells a call.
///
/// Verified against the two real templates this runtime is proven on:
///
/// ```text
/// gemma-4:  {{- '<|tool_call>call:' + function['name'] + '{' -}} … {{- '}<tool_call|>' -}}
/// qwen2.5:  {{- '\n<tool_call>\n{"name": "' }} … {{- '}\n</tool_call>' }}
/// ```
///
/// `None` when the template never mentions `tool_calls`, when no emission literal can be
/// read, or when the recovered delimiters are not safe to embed. `None` is an honest "the
/// model did not tell us" and sends the caller to [`KNOWN_DIALECTS`]; it is never a guess.
#[must_use]
pub fn derive_dialect_from_template(template_src: &str) -> Option<ToolDialect> {
    // A template may contain SEVERAL `tool_calls` loops — Gemma-4 has a second one that
    // resolves a tool_call_id back to a function name and emits no delimiters. Take the
    // first loop that actually emits fixed text, not merely the first loop.
    for body in tool_call_loop_bodies(template_src) {
        let literals = emitted_literals(body);
        let Some(first) = literals.first() else {
            continue;
        };
        // One literal means the opener only, and the close is genuinely ABSENT (Llama's
        // python tag has none) rather than unknown.
        let close = (literals.len() > 1)
            .then(|| literals[literals.len() - 1].as_str())
            .map(|l| {
                l.trim_matches(|c: char| {
                    c.is_whitespace() || matches!(c, '"' | '}' | ']' | ',' | '\'' | ':')
                })
                .to_string()
            })
            .filter(|c| !c.is_empty());
        if let Some(d) = dialect_from_affixes(first, close.as_deref()) {
            return Some(d);
        }
    }
    None
}

/// The bodies of every `for <var> in …tool_calls` loop, in source order. That loop is
/// where a Jinja chat template emits one call, so its literals are the dialect; anything
/// else in the template describes tool *definitions* or tool *responses*.
///
/// ⚠ The body runs to the loop's OWN `endfor`, counting nested `for`s. Gemma-4's real
/// template iterates the argument map inside the tool-call loop, so stopping at the first
/// `endfor` truncates the body just before the close delimiter is emitted — the dialect
/// then derives with `close: None` and looks almost right. Measured against the model's
/// full 17 KB template; a hand-trimmed excerpt has no nested loop and cannot show it.
fn tool_call_loop_bodies(src: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut search = 0usize;
    while let Some(rel) = src[search..].find("for ") {
        let at = search + rel;
        let head_end = src[at..].find("%}").map_or(src.len(), |e| at + e);
        let head = &src[at..head_end];
        if head.contains(" in ") && head.contains("tool_calls") {
            let after = (head_end + 2).min(src.len());
            out.push(&src[after..after + body_len_to_matching_endfor(&src[after..])]);
        }
        search = at + 4;
    }
    out
}

/// Length of the prefix of `rest` up to the `endfor` that closes the loop it belongs to,
/// skipping over nested `for`/`endfor` pairs.
fn body_len_to_matching_endfor(rest: &str) -> usize {
    // ⚠ `"endfor "` CONTAINS `"for "`. A naive scan counts every loop close as a new loop
    // open, so the depth never returns to zero and the body runs to the end of the file.
    let opens_a_loop = |at: usize| at < 3 || &rest[at - 3..at] != "end";
    let mut depth = 0usize;
    let mut i = 0usize;
    while i < rest.len() {
        let next_for = rest[i..]
            .match_indices("for ")
            .map(|(p, _)| i + p)
            .find(|&p| opens_a_loop(p));
        let next_end = rest[i..].find("endfor").map(|p| i + p);
        match (next_for, next_end) {
            (_, None) => return rest.len(),
            (Some(f), Some(e)) if f < e => {
                depth += 1;
                i = f + 4;
            }
            (_, Some(e)) => {
                if depth == 0 {
                    return e;
                }
                depth -= 1;
                i = e + 6;
            }
        }
    }
    rest.len()
}

/// The string literals a Jinja block EMITS, in order — the contents of `{{ '…' }}` /
/// `{{- "…" -}}` expressions that are plain literals. Expressions that reference variables
/// (the tool name, the arguments) are skipped: only the fixed text is the dialect.
fn emitted_literals(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0usize;
    while i + 1 < bytes.len() {
        if bytes[i] == b'{' && bytes[i + 1] == b'{' {
            let Some(rel) = body[i..].find("}}") else {
                break;
            };
            let expr = &body[i + 2..i + rel];
            if let Some(lit) = sole_string_literal(expr) {
                out.push(lit);
            }
            i += rel + 2;
        } else {
            i += 1;
        }
    }
    out
}

/// If `expr` is a single quoted string literal (modulo Jinja's `-` whitespace markers),
/// return it with the common escapes decoded. Anything with a `+` concatenation or a
/// variable reference is not fixed text, so it contributes nothing to the dialect.
fn sole_string_literal(expr: &str) -> Option<String> {
    let e = expr
        .trim()
        .trim_start_matches('-')
        .trim_end_matches('-')
        .trim();
    let quote = e.chars().next().filter(|c| *c == '\'' || *c == '"')?;
    // The literal must run to the matching close quote and be the WHOLE expression, or
    // this is a concatenation whose leading literal we would misread as the full prefix.
    let rest = &e[1..];
    let close = rest.find(quote)?;
    if rest[close + 1..].trim() != "" {
        // A concatenation such as `'<|tool_call>call:' + function['name'] + '{'` — the
        // leading literal IS the prefix the model emits before the name, so take it.
        let head = &rest[..close];
        return Some(unescape_jinja(head));
    }
    Some(unescape_jinja(&rest[..close]))
}

/// Decode the escapes a Jinja string literal may carry.
fn unescape_jinja(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Recover the close delimiter: what the probe emits after the argument value, once the
/// JSON closers and the trailing turn framing (shared with the control) are removed.
fn derive_close(rendered_probe: &str, rendered_control: &str) -> Option<String> {
    let val_end = rendered_probe.find(PROBE_VAL)? + PROBE_VAL.len();
    let tail = rendered_probe.get(val_end..)?;
    let shared = common_suffix_len(tail, rendered_control);
    let segment = tail.get(..tail.len().saturating_sub(shared))?;
    // Drop the JSON that closes the args object and the call wrapper; whatever literal
    // remains is the dialect's own close delimiter.
    let close = segment
        .trim_matches(|c: char| {
            c.is_whitespace() || matches!(c, '"' | '}' | ']' | ',' | '\'' | ':')
        })
        .trim();
    (!close.is_empty()).then(|| close.to_string())
}

fn common_prefix_len(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.bytes().zip(b.bytes()) {
        if x != y {
            break;
        }
        n += 1;
    }
    // Never split a UTF-8 sequence.
    while n > 0 && !a.is_char_boundary(n) {
        n -= 1;
    }
    n
}

fn common_suffix_len(a: &str, b: &str) -> usize {
    let mut n = 0;
    for (x, y) in a.bytes().rev().zip(b.bytes().rev()) {
        if x != y {
            break;
        }
        n += 1;
    }
    while n > 0 && !a.is_char_boundary(a.len() - n) {
        n -= 1;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- the table's own invariants -------------------------------------------------

    #[test]
    fn only_the_canonical_envelope_has_no_open_marker() {
        for d in KNOWN_DIALECTS {
            assert_eq!(
                d.open.is_empty(),
                d.shape == DialectShape::CanonicalEnvelope,
                "{}: an empty opener is legal only for the structural envelope",
                d.id
            );
        }
    }

    #[test]
    fn dialect_ids_are_unique() {
        let mut seen = std::collections::BTreeSet::new();
        for d in KNOWN_DIALECTS {
            assert!(seen.insert(d.id.as_ref()), "duplicate dialect id {}", d.id);
        }
    }

    /// `<tool_call>` and `<|tool_call>` differ by one byte. The parser's prefix match is
    /// exact so they cannot collide, but a table edit that made one a prefix of the other
    /// would make the grammar alternation ambiguous at the first character.
    #[test]
    fn no_marker_is_a_prefix_of_another() {
        for a in KNOWN_DIALECTS {
            for b in KNOWN_DIALECTS {
                if a.id == b.id || a.open.is_empty() || b.open.is_empty() {
                    continue;
                }
                assert!(
                    !a.open.starts_with(b.open.as_ref()),
                    "{} opener {:?} has {} opener {:?} as a prefix",
                    a.id,
                    a.open,
                    b.id,
                    b.open
                );
            }
        }
    }

    #[test]
    fn every_known_dialect_is_well_formed() {
        for d in KNOWN_DIALECTS {
            assert!(d.is_well_formed(), "{} is not embeddable", d.id);
        }
    }

    // --- the trigger is derived, so it cannot drift ---------------------------------

    #[test]
    fn every_marker_trigger_captures_exactly_its_opener() {
        for d in KNOWN_DIALECTS {
            if d.shape == DialectShape::CanonicalEnvelope {
                assert_eq!(d.trigger_pattern(), CANONICAL_TRIGGER);
                assert_eq!(d.grammar_prefix(), None);
                continue;
            }
            assert_eq!(
                d.trigger_pattern(),
                format!(r"[\s\S]*?({})", ecma_escape(&d.open)),
                "{}",
                d.id
            );
            assert_eq!(d.grammar_prefix(), Some(d.open.as_ref()));
        }
    }

    /// The regression that would fire the trigger on the first `<` of any prose and then
    /// hand that `<` to a grammar that cannot accept it.
    #[test]
    fn the_pipe_in_a_marker_is_escaped() {
        assert_eq!(ecma_escape("<|tool_call>"), r"<\|tool_call>");
        assert_eq!(ecma_escape("<|python_tag|>"), r"<\|python_tag\|>");
        assert!(GEMMA4_NATIVE.trigger_pattern().contains(r"<\|tool_call>"));
        assert!(!GEMMA4_NATIVE.trigger_pattern().contains("(<|tool_call>)"));
    }

    #[test]
    fn ecma_escape_covers_every_metacharacter() {
        assert_eq!(
            ecma_escape(r"\^$.|?*+()[]{}/"),
            r"\\\^\$\.\|\?\*\+\(\)\[\]\{\}\/"
        );
        assert_eq!(ecma_escape("<tool_call>"), "<tool_call>");
    }

    // --- derivation from a real template -------------------------------------------

    /// The exact shape Gemma-4's embedded template emits:
    /// `{{- '<|tool_call>call:' + function['name'] + '{' -}}` … `{{- '}<tool_call|>' -}}`
    #[test]
    fn gemma4_template_output_yields_the_gemma_dialect() {
        let probe = format!(
            "<|turn|>user\nhi<turn|><|turn|>model\n<|tool_call>call:{PROBE_NAME}{{\"{PROBE_ARG}\": \"{PROBE_VAL}\"}}<tool_call|><turn|>"
        );
        let control = format!("<|turn|>user\nhi<turn|><|turn|>model\n{PROBE_TEXT}<turn|>");
        let d = derive_dialect(&probe, &control).expect("gemma template declares a dialect");
        assert_eq!(d.open, "<|tool_call>");
        assert_eq!(d.call_marker.as_deref(), Some("call:"));
        assert_eq!(d.close.as_deref(), Some("<tool_call|>"));
        assert_eq!(d.shape, DialectShape::NameThenArgs);
        // And the derived dialect arms the same trigger the static entry would.
        assert_eq!(d.trigger_pattern(), GEMMA4_NATIVE.trigger_pattern());
    }

    /// Qwen2.5 / Hermes — a model deliberately absent from every hard-coded table.
    #[test]
    fn hermes_template_output_yields_the_xml_dialect() {
        let probe = format!(
            "<|im_start|>assistant\n<tool_call>\n{{\"name\": \"{PROBE_NAME}\", \"arguments\": {{\"{PROBE_ARG}\": \"{PROBE_VAL}\"}}}}\n</tool_call><|im_end|>"
        );
        let control = format!("<|im_start|>assistant\n{PROBE_TEXT}<|im_end|>");
        let d = derive_dialect(&probe, &control).expect("hermes template declares a dialect");
        assert_eq!(d.open, "<tool_call>");
        assert_eq!(d.close.as_deref(), Some("</tool_call>"));
        assert_eq!(d.shape, DialectShape::NamedObject);
        assert_eq!(d.trigger_pattern(), HERMES_XML.trigger_pattern());
    }

    /// Llama's python tag has no close delimiter — `None`, not an empty string.
    #[test]
    fn python_tag_template_output_has_no_close_delimiter() {
        let probe = format!(
            "<|start_header_id|>assistant<|end_header_id|>\n<|python_tag|>{{\"name\": \"{PROBE_NAME}\", \"parameters\": {{\"{PROBE_ARG}\": \"{PROBE_VAL}\"}}}}<|eot_id|>"
        );
        let control =
            format!("<|start_header_id|>assistant<|end_header_id|>\n{PROBE_TEXT}<|eot_id|>");
        let d = derive_dialect(&probe, &control).expect("python-tag template declares a dialect");
        assert_eq!(d.open, "<|python_tag|>");
        assert_eq!(d.close, None);
        assert_eq!(d.shape, DialectShape::NamedObject);
    }

    /// The honest failure: a template that ignores `tool_calls` entirely tells us nothing,
    /// and must say so rather than inventing a delimiter.
    #[test]
    fn a_template_that_ignores_tool_calls_derives_nothing() {
        let probe = "<|im_start|>assistant\n<|im_end|>".to_string();
        let control = format!("<|im_start|>assistant\n{PROBE_TEXT}<|im_end|>");
        assert_eq!(derive_dialect(&probe, &control), None);
    }

    /// A template is untrusted input. A delimiter carrying a quote would render a GBNF
    /// terminal llama.cpp refuses to parse, failing the whole dispatch — drop it instead.
    #[test]
    fn an_unembeddable_delimiter_is_refused_not_shipped() {
        let probe = format!("A<\"weird>{PROBE_NAME}{{\"{PROBE_ARG}\": \"{PROBE_VAL}\"}}B");
        let control = format!("A{PROBE_TEXT}B");
        assert_eq!(derive_dialect(&probe, &control), None);
    }

    #[test]
    fn a_name_with_no_delimiter_before_it_derives_nothing() {
        let probe = format!("A{PROBE_NAME}{{\"{PROBE_ARG}\": \"{PROBE_VAL}\"}}B");
        let control = format!("A{PROBE_TEXT}B");
        assert_eq!(derive_dialect(&probe, &control), None);
    }

    // --- the grammar/parser contract for version-less native names ------------------
    //
    // These live here rather than beside the function because they are about what the
    // GBNF renderer is allowed to pin, which is a dialect concern.

    #[test]
    fn an_unambiguous_bare_name_may_be_pinned_in_a_native_branch() {
        let granted = ["mcp-calc/calc", "mcp-kv/get"];
        assert!(crate::native_name_resolves_uniquely(
            "mcp-calc/calc",
            &granted
        ));
        // A native dialect may also spell the short leaf or the server prefix.
        assert!(crate::native_name_resolves_uniquely("calc", &granted));
        assert!(crate::native_name_resolves_uniquely("mcp-calc", &granted));
    }

    /// The grant set that would make the grammar manufacture a refusal: a bare `fs`
    /// addresses BOTH `fs` and `fs/list`, so no native branch may pin it.
    #[test]
    fn a_bare_name_addressing_two_grants_is_refused_a_native_branch() {
        let granted = ["fs", "fs/list"];
        assert!(!crate::native_name_resolves_uniquely("fs", &granted));
        // …while the unambiguous sibling in the same set is still fine.
        assert!(crate::native_name_resolves_uniquely("fs/list", &granted));
        assert!(crate::native_name_resolves_uniquely("list", &granted));
    }

    #[test]
    fn a_name_addressing_no_grant_gets_no_native_branch() {
        assert!(!crate::native_name_resolves_uniquely(
            "nope",
            &["mcp-calc/calc"]
        ));
        assert!(!crate::native_name_resolves_uniquely(
            "",
            &["mcp-calc/calc"]
        ));
    }

    // --- derivation from a real chat-template SOURCE --------------------------------
    //
    // These fixtures are the VERBATIM tool-call loop from each model's own
    // `tokenizer.chat_template`, read out of the GGUF this runtime is proven on. They are
    // the model declaring its dialect; if a future model ships a shape these cannot read,
    // `derive_dialect_from_template` returns None and the caller falls back — it never
    // guesses.

    /// Verbatim from `gemma-4-12b-it-q4_k_m.gguf`, **including the nested argument
    /// loop**. The nesting is load-bearing: a reader that stops at the first `endfor`
    /// truncates the body just before `'}<tool_call|>'` and derives a dialect with no
    /// close delimiter — which looks almost right. A hand-trimmed excerpt without this
    /// loop cannot catch that, and did not.
    const GEMMA4_TEMPLATE_LOOP: &str = r"
{%- if message['tool_calls'] -%}
    {%- for tool_call in message['tool_calls'] -%}
        {%- set function = tool_call['function'] -%}
        {{- '<|tool_call>call:' + function['name'] + '{' -}}
        {%- if function['arguments'] is mapping -%}
            {%- set ns_args = namespace(found_first=false) -%}
            {%- for key, value in function['arguments'] | dictsort -%}
                {%- if ns_args.found_first %},{% endif -%}
                {%- set ns_args.found_first = true -%}
                {{- key -}}:{{- format_argument(value, escape_keys=False) -}}
            {%- endfor -%}
        {%- elif function['arguments'] is string -%}
            {{- function['arguments'] -}}
        {%- endif -%}
        {{- '}<tool_call|>' -}}
    {%- endfor -%}
{%- endif -%}
";

    /// Verbatim from `qwen2.5-3b-instruct-q4_k_m.gguf`.
    const QWEN25_TEMPLATE_LOOP: &str = r#"
{%- for tool_call in message.tool_calls %}
    {%- if tool_call.function is defined %}
        {%- set tool_call = tool_call.function %}
    {%- endif %}
    {{- '\n<tool_call>\n{"name": "' }}
    {{- tool_call.name }}
    {{- '", "arguments": ' }}
    {{- tool_call.arguments | tojson }}
    {{- '}\n</tool_call>' }}
{%- endfor %}
"#;

    #[test]
    fn gemma4s_own_template_declares_the_gemma_dialect() {
        let d = derive_dialect_from_template(GEMMA4_TEMPLATE_LOOP)
            .expect("gemma-4's template declares a tool-call dialect");
        assert_eq!(d.open, "<|tool_call>");
        assert_eq!(d.call_marker.as_deref(), Some("call:"));
        assert_eq!(d.close.as_deref(), Some("<tool_call|>"));
        assert_eq!(d.shape, DialectShape::NameThenArgs);
        // Derived and hard-coded agree — which is what makes the table a FALLBACK rather
        // than a second source of truth.
        assert_eq!(d.open, GEMMA4_NATIVE.open);
        assert_eq!(d.close, GEMMA4_NATIVE.close);
        assert_eq!(d.trigger_pattern(), GEMMA4_NATIVE.trigger_pattern());
    }

    /// ★ The whole point of deriving: Qwen2.5 is deliberately absent from every hard-coded
    /// table in this runtime, and its dialect is still recovered from the model itself.
    #[test]
    fn qwen25s_own_template_declares_its_dialect_without_being_hardcoded() {
        let d = derive_dialect_from_template(QWEN25_TEMPLATE_LOOP)
            .expect("qwen2.5's template declares a tool-call dialect");
        assert_eq!(d.open, "<tool_call>");
        assert_eq!(d.close.as_deref(), Some("</tool_call>"));
        assert_eq!(d.shape, DialectShape::NamedObject);
        assert_eq!(d.trigger_pattern(), HERMES_XML.trigger_pattern());
    }

    /// Gemma-4's template carries a SECOND `tool_calls` loop that resolves a call id back
    /// to a function name and emits no delimiters. Taking merely the first loop would
    /// derive nothing; the reader must take the first loop that emits fixed text.
    #[test]
    fn a_second_non_emitting_tool_calls_loop_does_not_defeat_the_reader() {
        let with_decoy = format!(
            "{{%- for tc in message['tool_calls'] -%}}{{%- if tc.get('id') -%}}{{%- endif -%}}{{%- endfor -%}}{GEMMA4_TEMPLATE_LOOP}"
        );
        let d = derive_dialect_from_template(&with_decoy).expect("the emitting loop is found");
        assert_eq!(d.open, "<|tool_call>");
    }

    #[test]
    fn a_template_with_no_tool_call_loop_declares_nothing() {
        let plain = "{%- for message in messages -%}{{- message['content'] -}}{%- endfor -%}";
        assert_eq!(derive_dialect_from_template(plain), None);
    }

    #[test]
    fn the_probe_messages_are_valid_json_carrying_every_sentinel() {
        let probe: serde_json::Value =
            serde_json::from_str(&probe_messages_json()).expect("probe messages are JSON");
        let control: serde_json::Value =
            serde_json::from_str(&control_messages_json()).expect("control messages are JSON");
        let probe_s = probe.to_string();
        assert!(probe_s.contains(PROBE_NAME) && probe_s.contains(PROBE_ARG));
        assert!(probe_s.contains(PROBE_VAL));
        assert!(control.to_string().contains(PROBE_TEXT));
        // The control must NOT carry a tool call, or the diff has nothing to separate.
        assert!(!control.to_string().contains(PROBE_NAME));
    }
}
