//! A pure, deterministic, fail-closed **prompt-template engine** for the
//! authoring layer.
//!
//! `MoteDef` has no prompt field — the identity-bearing instruction text is
//! carried in `config_subset` under [`kx_mote::PROMPT_KEY`], so it folds into
//! [`kx_mote::MoteDef::hash`] → `MoteId` (same prompt ⇒ same identity, different
//! prompt ⇒ different identity). This module lets a recipe author carry an
//! *un-rendered* template under [`TEMPLATE_KEY`] with named `{placeholder}`
//! slots, then bind named parameters into the final prompt **before**
//! [`crate::compile`] — so the *rendered* prompt is what folds into identity.
//!
//! # Where rendering happens (and why)
//!
//! Rendering is an **authoring/bind-time** step (run after
//! [`WorkflowDef::bind_param`](crate::WorkflowDef::bind_param), before
//! [`compile`](crate::compile)), never a dispatch-time step. That is the only
//! placement that keeps the prompt identity-bearing and composable with the
//! existing free-param binding path (the D121 inbound-execution path binds
//! structural slots, then a caller may [`render_prompts`] the text). A prompt
//! substituted at dispatch time would NOT fold into `MoteId`, silently breaking
//! recipe-reuse / fresh-call semantics.
//!
//! # Guarantees
//!
//! - **Pure / total / deterministic.** [`PromptTemplate::parse`] is a function of
//!   its input string only; [`PromptTemplate::render`] is a function of the
//!   template + the (`BTreeMap`-ordered) params — no clock, host, PID, float, or
//!   hash-map iteration order.
//! - **Fail-closed.** A malformed template, an unfilled placeholder, or an
//!   unknown parameter is an error — never a silently dropped or half-rendered
//!   prompt.
//! - **Structure-safe, not content-safe.** The engine validates template
//!   *structure*; it does not sanitize param *content*. A param value is an
//!   opaque `config_subset` byte string — it cannot widen a warrant, mint a
//!   capability, or alter DAG topology. Content trust stays the warrant +
//!   `deterministic_critic`'s job. (A rendered prompt folds into `MoteId`, so
//!   callers must not pass secrets as prompt params — same property as
//!   [`WorkflowDef::bind_param`](crate::WorkflowDef::bind_param).)

use std::collections::{BTreeMap, BTreeSet};

use kx_mote::{ConfigKey, ConfigVal, PROMPT_KEY};

use crate::def::WorkflowDef;
use crate::error::CompileError;

/// The `config_subset` key under which a step carries its **un-rendered** prompt
/// template — read, rendered, and cleared by [`render_prompts`].
///
/// Distinct from [`kx_mote::PROMPT_KEY`] (the key the *rendered* prompt lands
/// under). A step carrying `TEMPLATE_KEY` is "render me"; once rendered, only
/// `PROMPT_KEY` remains, so a rendered `WorkflowDef` is indistinguishable from
/// one authored with a literal prompt (and the pass is idempotent).
pub const TEMPLATE_KEY: &str = "prompt_template";

/// One segment of a parsed [`PromptTemplate`], in source order.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    /// A run of literal characters (with `{{`/`}}` already de-escaped to `{`/`}`).
    Literal(String),
    /// A `{name}` placeholder slot to be filled at render time.
    Slot(String),
}

/// A parsed prompt template: an ordered sequence of literal + `{placeholder}`
/// segments, plus the (sorted) set of placeholder names it declares.
///
/// Build one with [`PromptTemplate::parse`]; fill it with
/// [`PromptTemplate::render`]. Both are pure — the type holds no interior
/// mutability and no allocation beyond its segment list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptTemplate {
    segments: Vec<Segment>,
    declared: BTreeSet<String>,
}

impl PromptTemplate {
    /// Parse a template string with `{name}` placeholder slots.
    ///
    /// Slot names are `[A-Za-z0-9_]+`. `{{` and `}}` are literal escapes for `{`
    /// and `}`. Pure, total, and deterministic — a function of `src` alone.
    ///
    /// # Errors
    ///
    /// [`CompileError::MalformedTemplate`] on an unterminated placeholder, an
    /// empty `{}`, a nested `{`, an unescaped lone `}`, or a placeholder name
    /// with a non-`[A-Za-z0-9_]` character.
    pub fn parse(src: &str) -> Result<Self, CompileError> {
        let chars: Vec<char> = src.chars().collect();
        let mut segments: Vec<Segment> = Vec::new();
        let mut declared: BTreeSet<String> = BTreeSet::new();
        let mut lit = String::new();
        let mut i = 0usize;

        while i < chars.len() {
            let c = chars[i];
            if c == '{' {
                // `{{` → a literal '{'.
                if chars.get(i + 1) == Some(&'{') {
                    lit.push('{');
                    i += 2;
                    continue;
                }
                // Start of a placeholder — flush any pending literal first.
                if !lit.is_empty() {
                    segments.push(Segment::Literal(std::mem::take(&mut lit)));
                }
                i += 1; // consume '{'
                let mut name = String::new();
                let mut closed = false;
                while i < chars.len() {
                    let d = chars[i];
                    if d == '}' {
                        closed = true;
                        i += 1;
                        break;
                    }
                    if d == '{' {
                        return Err(CompileError::MalformedTemplate {
                            reason: "nested '{' inside a placeholder".to_string(),
                        });
                    }
                    name.push(d);
                    i += 1;
                }
                if !closed {
                    return Err(CompileError::MalformedTemplate {
                        reason: format!("unterminated placeholder '{{{name}'"),
                    });
                }
                if name.is_empty() {
                    return Err(CompileError::MalformedTemplate {
                        reason: "empty placeholder '{}'".to_string(),
                    });
                }
                if !name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
                {
                    return Err(CompileError::MalformedTemplate {
                        reason: format!(
                            "placeholder '{name}' has a character outside [A-Za-z0-9_]"
                        ),
                    });
                }
                declared.insert(name.clone());
                segments.push(Segment::Slot(name));
            } else if c == '}' {
                // `}}` → a literal '}'. A lone '}' is a structural error.
                if chars.get(i + 1) == Some(&'}') {
                    lit.push('}');
                    i += 2;
                    continue;
                }
                return Err(CompileError::MalformedTemplate {
                    reason: "unescaped '}' (use '}}' for a literal brace)".to_string(),
                });
            } else {
                lit.push(c);
                i += 1;
            }
        }
        if !lit.is_empty() {
            segments.push(Segment::Literal(lit));
        }
        Ok(Self { segments, declared })
    }

    /// The placeholder names this template declares, in deterministic (sorted) order.
    pub fn slots(&self) -> impl Iterator<Item = &str> {
        self.declared.iter().map(String::as_str)
    }

    /// Render the template into the final prompt string.
    ///
    /// Fail-closed: every key in `params` MUST be a declared slot, and every
    /// declared slot MUST be present in `params`. Deterministic — output bytes
    /// depend only on the (source-order) segments and the param values.
    ///
    /// # Errors
    ///
    /// [`CompileError::UnknownParam`] if a param names no declared slot;
    /// [`CompileError::MissingPlaceholder`] if a declared slot has no param.
    pub fn render(&self, params: &BTreeMap<String, String>) -> Result<String, CompileError> {
        // Unknown-param check first (deterministic first-failure over BTreeMap order).
        for key in params.keys() {
            if !self.declared.contains(key) {
                return Err(CompileError::UnknownParam { name: key.clone() });
            }
        }
        // Every declared slot must be supplied.
        for name in &self.declared {
            if !params.contains_key(name) {
                return Err(CompileError::MissingPlaceholder { name: name.clone() });
            }
        }
        let capacity: usize = self
            .segments
            .iter()
            .map(|s| match s {
                Segment::Literal(l) => l.len(),
                Segment::Slot(n) => params.get(n).map_or(0, String::len),
            })
            .sum();
        let mut out = String::with_capacity(capacity);
        for s in &self.segments {
            match s {
                Segment::Literal(l) => out.push_str(l),
                // Presence is guaranteed by the missing-placeholder check above;
                // the defensive arm keeps this total without an `expect`.
                Segment::Slot(n) => match params.get(n) {
                    Some(v) => out.push_str(v),
                    None => return Err(CompileError::MissingPlaceholder { name: n.clone() }),
                },
            }
        }
        Ok(out)
    }
}

/// Render `template` with `params` and write the result into
/// `config_subset[`[`PROMPT_KEY`]`]` — the author-side convenience for a single
/// step. Identity-bearing: the rendered prompt folds into the step's `MoteId`.
///
/// # Errors
///
/// Propagates [`PromptTemplate::parse`] / [`PromptTemplate::render`] failures
/// (fail-closed — on error the map is left unmodified).
pub fn put_rendered_prompt(
    config_subset: &mut BTreeMap<ConfigKey, ConfigVal>,
    template: &str,
    params: &BTreeMap<String, String>,
) -> Result<(), CompileError> {
    let rendered = PromptTemplate::parse(template)?.render(params)?;
    config_subset.insert(
        ConfigKey(PROMPT_KEY.to_string()),
        ConfigVal(rendered.into_bytes()),
    );
    Ok(())
}

/// The bind-time prompt-render pass: for every step carrying a [`TEMPLATE_KEY`]
/// slot, render its template against `params`, write the result under
/// [`PROMPT_KEY`], and remove the template slot. Returns the number of steps
/// whose prompt was rendered.
///
/// Run this **after** [`WorkflowDef::bind_param`](crate::WorkflowDef::bind_param)
/// and **before** [`compile`](crate::compile) so the rendered prompt is
/// identity-bearing. Idempotent: a second run is a no-op (the template slot is
/// already gone). Steps with no template slot are untouched.
///
/// **Atomic.** The pass stages its work on a clone and commits only on full
/// success — a render failure at any step leaves `def` **byte-unchanged**.
///
/// # Errors
///
/// [`CompileError::RenderPromptStep`] naming the first failing step (template
/// not valid UTF-8, malformed, or with a missing/unknown param).
pub fn render_prompts(
    def: &mut WorkflowDef,
    params: &BTreeMap<String, String>,
) -> Result<usize, CompileError> {
    let template_key = ConfigKey(TEMPLATE_KEY.to_string());
    let prompt_key = ConfigKey(PROMPT_KEY.to_string());

    // Stage on a clone so any failure is a no-op on the caller's `WorkflowDef`.
    let mut staged = def.clone();
    let mut rendered = 0usize;
    for (i, step) in staged.steps.iter_mut().enumerate() {
        let Some(tmpl) = step.config_subset.get(&template_key).cloned() else {
            continue;
        };
        let text = std::str::from_utf8(&tmpl.0).map_err(|_| CompileError::RenderPromptStep {
            step: i,
            reason: "template bytes are not valid UTF-8".to_string(),
        })?;
        let prompt = PromptTemplate::parse(text)
            .and_then(|t| t.render(params))
            .map_err(|e| CompileError::RenderPromptStep {
                step: i,
                reason: e.to_string(),
            })?;
        step.config_subset
            .insert(prompt_key.clone(), ConfigVal(prompt.into_bytes()));
        step.config_subset.remove(&template_key);
        rendered += 1;
    }
    *def = staged;
    Ok(rendered)
}
