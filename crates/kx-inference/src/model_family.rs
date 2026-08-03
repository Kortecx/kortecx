//! The MODEL-FAMILY REGISTRY — one declarative table describing how each supported
//! OSS model family frames a conversation, read by every backend.
//!
//! # Why this exists
//!
//! Chat templating was implemented twice, in two crates, in two shapes, and the two
//! disagreed about the same model. The in-process backend rendered any arch starting
//! `gemma` in Gemma-4's `<|turn>` vocabulary; the Ollama backend rendered `gemma3` /
//! `gemma2` / `gemma` in Gemma-3's `<start_of_turn>` vocabulary and everything else in
//! `ChatML`. Both were reachable, neither knew about the other, and Gemma-4 over Ollama
//! matched no arm at all — so it was prompted in `ChatML`, a vocabulary it has never
//! been trained on, on every turn.
//!
//! Onboarding a family is now a DATA edit: add a [`Family`] to [`FAMILIES`]. There is no
//! dispatch code to duplicate, and the stop tokens are derived from the template rather
//! than maintained beside it, so the two cannot drift.
//!
//! # What a backend still owns
//!
//! This registry is deliberately about the CHAT SURFACE — turn framing, role naming, the
//! generation prefix, and the turn terminators that follow from them. Tokenization stays
//! with whoever owns the weights (llama.cpp reads the GGUF vocabulary; the Ollama daemon
//! tokenizes server-side), and model ACQUISITION stays with the model store. Those are
//! genuinely different concerns and pretending one table configures them all would be a
//! seam that lies about its own reach.
//!
//! # Precedence
//!
//! A model's OWN embedded template wins wherever one is renderable — that is the
//! model-agnostic path and it needs no entry here. This table is what a backend falls
//! back to: for the Ollama backend that is ALWAYS (it dispatches `/api/generate` with
//! `raw: true`, which applies no template of its own), and for the in-process backend it
//! is the gap-filler for templates the `minja` engine rejects.

/// How one model family frames a conversation.
///
/// The render is `turn_open + role + "\n" + content + turn_close + "\n"` per message,
/// then `turn_open + assistant_role + "\n" + generation_prefix` to open the model's turn.
/// Every supported family to date is expressible in exactly that shape. A family that is
/// not should gain its own render arm rather than bend this one out of shape — none has
/// needed to yet, and the moment one does is the moment to add it, not to add a field
/// that only one entry ever sets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChatTemplate {
    /// Opens a turn (e.g. `<|turn>`, `<start_of_turn>`, `<|im_start|>`).
    pub turn_open: &'static str,
    /// Closes a turn (e.g. `<turn|>`, `<end_of_turn>`, `<|im_end|>`). ALSO the family's
    /// stop token — see [`ChatTemplate::stop_tokens`].
    pub turn_close: &'static str,
    /// What the model's own role is called (Gemma says `model`, `ChatML` says
    /// `assistant`). Getting this wrong is silent: the turn still renders.
    pub assistant_role: &'static str,
    /// The name of the system role, or `None` when the family HAS NO system role and a
    /// system message must be rendered as its own turn under [`ChatTemplate::user_role`].
    /// Gemma-3 is the `None` case — its template maps system onto a user turn, and as its
    /// OWN turn rather than merged into the user's.
    pub system_role: Option<&'static str>,
    /// The user role's name.
    pub user_role: &'static str,
    /// Text appended AFTER the generation turn opens, before the model writes. Gemma-4
    /// opens a thought channel here; omitting it does not fail — the model emits the
    /// channel markers as CONTENT instead, and they reach the caller as answer text.
    /// Measured, one variable: with the prefix `"The capital of France is Paris."`,
    /// without it `"<|channel>thought\n<channel|>The capital of France is Paris."`.
    pub generation_prefix: &'static str,
}

impl ChatTemplate {
    /// Render `(system, user)` — the two-message shape the serve path uses.
    ///
    /// An EMPTY system renders the user turn alone: the template emits a turn per
    /// MESSAGE, and an absent system message is not an empty one. A leading empty turn
    /// is something no model was trained to see.
    #[must_use]
    pub fn render_system_user(&self, system: &str, user: &str) -> String {
        let mut out = String::new();
        if !system.is_empty() {
            // No system role ⇒ the system text becomes its own USER turn.
            self.push_turn(&mut out, self.system_role.unwrap_or(self.user_role), system);
        }
        self.push_turn(&mut out, self.user_role, user);
        self.push_generation_turn(&mut out);
        out
    }

    /// Render an arbitrary `(role, content)` sequence, mapping `assistant` onto the
    /// family's own name for it and folding `system` when the family has no system role.
    #[must_use]
    pub fn render_messages<'a>(
        &self,
        messages: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> String {
        let mut out = String::new();
        for (role, content) in messages {
            let mapped = match role {
                "assistant" => self.assistant_role,
                "system" => self.system_role.unwrap_or(self.user_role),
                other => other,
            };
            self.push_turn(&mut out, mapped, content);
        }
        self.push_generation_turn(&mut out);
        out
    }

    fn push_turn(&self, out: &mut String, role: &str, content: &str) {
        out.push_str(self.turn_open);
        out.push_str(role);
        out.push('\n');
        out.push_str(content);
        out.push_str(self.turn_close);
        out.push('\n');
    }

    fn push_generation_turn(&self, out: &mut String) {
        out.push_str(self.turn_open);
        out.push_str(self.assistant_role);
        out.push('\n');
        out.push_str(self.generation_prefix);
    }

    /// The stop tokens this template implies. DERIVED, never maintained separately: a
    /// backend dispatching in raw mode supplies the only stops there are, so a family
    /// whose terminator was not listed would render correctly and then run on past the
    /// turn boundary.
    #[must_use]
    pub fn stop_tokens(&self) -> [&'static str; 1] {
        [self.turn_close]
    }
}

/// One registered family: the architecture ids that select it, and its chat surface.
#[derive(Debug, Clone, Copy)]
pub struct Family {
    /// Architecture ids this entry claims, matched EXACTLY. Ollama reports these as
    /// `details.family` (`"gemma4"`); llama.cpp reports them as the leading token of
    /// `llama_model_desc` (`"gemma4 12B Q4_K - Medium"`).
    ///
    /// ⚠ Exact, never a prefix. A prefix match is what made `gemma3` receive Gemma-4's
    /// vocabulary — the families share a name stem and share no control tokens.
    pub ids: &'static [&'static str],
    /// The family's chat surface.
    pub template: ChatTemplate,
}

/// Gemma-4: the `<|turn>` vocabulary, a real system role, and a thought channel opened
/// by the generation turn. Its 17 KB tool-calling jinja template is one `minja` rejects,
/// and the Ollama model card ships `TEMPLATE {{ .Prompt }}` with a daemon-side
/// `RENDERER gemma4` that a `raw: true` dispatch never reaches — so BOTH backends need
/// this entry, for different reasons.
const GEMMA4: ChatTemplate = ChatTemplate {
    turn_open: "<|turn>",
    turn_close: "<turn|>",
    assistant_role: "model",
    system_role: Some("system"),
    user_role: "user",
    generation_prefix: "<|channel>thought\n<channel|>",
};

/// Gemma-3 / Gemma-2 / Gemma: the `<start_of_turn>` vocabulary and NO system role — the
/// template maps a system message onto its own `user` turn. Mirrors what `/api/show`
/// returns for `gemma3:12b` rather than a tidier prompt of our own invention.
const GEMMA3: ChatTemplate = ChatTemplate {
    turn_open: "<start_of_turn>",
    turn_close: "<end_of_turn>",
    assistant_role: "model",
    system_role: None,
    user_role: "user",
    generation_prefix: "",
};

/// `ChatML` — Qwen / Yi / many Mistral GGUFs.
///
/// ⚠ **Deliberately NOT in [`FAMILIES`].** It is the shape a backend falls back to, not
/// a family that needs claiming, and the distinction is load-bearing: the Ollama
/// backend's `None` is a CONTRACT its caller relies on, and that caller's own `ChatML`
/// (`kx_gateway::model_exec::chatml_with`) emits an EMPTY system turn where
/// [`ChatTemplate::render_system_user`] omits it. Registering `qwen` here therefore
/// changed the prompt bytes of every empty-system Qwen turn on the Ollama path — a
/// silent change to the byte-sensitive model path, caught by the fail-safe test that
/// exists to assert the absent branch.
///
/// The rule that follows: **an entry belongs here only when the family needs a template
/// the caller does not already produce.** A family whose handling is unchanged must stay
/// unregistered, or "known" quietly starts meaning something different from "special".
pub const CHATML: ChatTemplate = ChatTemplate {
    turn_open: "<|im_start|>",
    turn_close: "<|im_end|>",
    assistant_role: "assistant",
    system_role: Some("system"),
    user_role: "user",
    generation_prefix: "",
};

/// THE TABLE. Adding a model family is an entry here and nothing else.
pub const FAMILIES: &[Family] = &[
    Family {
        ids: &["gemma4"],
        template: GEMMA4,
    },
    Family {
        ids: &["gemma3", "gemma2", "gemma"],
        template: GEMMA3,
    },
];

/// Resolve an architecture id to its family. `None` ⇒ unregistered, and every caller's
/// documented fallback applies (the in-process backend renders `ChatML`; the Ollama
/// backend returns `None` so its caller keeps the `ChatML` path it always had).
///
/// Matching is EXACT and case-sensitive: these ids come from model metadata, not from a
/// human, and a fuzzy match here silently hands one family another's control tokens.
#[must_use]
pub fn resolve(arch: &str) -> Option<&'static Family> {
    FAMILIES.iter().find(|f| f.ids.contains(&arch))
}

/// Resolve from llama.cpp's `llama_model_desc` (`"gemma4 12B Q4_K - Medium"`), whose
/// leading whitespace-delimited token is the architecture.
#[must_use]
pub fn resolve_from_desc(model_desc: &str) -> Option<&'static Family> {
    resolve(model_desc.split_whitespace().next().unwrap_or_default())
}

/// Every stop token any registered family can terminate a turn with. A raw-mode
/// dispatcher that passes these can never cut a family short or let one run on.
#[must_use]
pub fn all_stop_tokens() -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for f in FAMILIES {
        for s in f.template.stop_tokens() {
            if !out.contains(&s) {
                out.push(s);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ★ THE INCIDENT THIS TABLE EXISTS FOR. Gemma-4 and Gemma-3 share a name stem and
    /// share NO control tokens. A prefix match — which is what the in-process backend
    /// did — hands one the other's vocabulary.
    #[test]
    fn gemma4_and_gemma3_share_no_control_tokens() {
        let g4 = resolve("gemma4").expect("gemma4 registered").template;
        let g3 = resolve("gemma3").expect("gemma3 registered").template;
        assert_ne!(g4.turn_open, g3.turn_open);
        assert_ne!(g4.turn_close, g3.turn_close);
        for tok in [g4.turn_open, g4.turn_close] {
            assert!(
                !g3.render_system_user("S", "U").contains(tok),
                "a gemma3 render must carry no gemma4 marker ({tok})"
            );
        }
        for tok in [g3.turn_open, g3.turn_close] {
            assert!(
                !g4.render_system_user("S", "U").contains(tok),
                "a gemma4 render must carry no gemma3 marker ({tok})"
            );
        }
    }

    /// Matching is exact. `gemma5` must NOT inherit Gemma-4's vocabulary by looking
    /// similar — an unknown family's documented behaviour is the caller's fallback.
    #[test]
    fn resolution_is_exact_never_a_prefix() {
        assert!(
            resolve("gemma5").is_none(),
            "a future family must not be guessed"
        );
        assert!(resolve("gemma4-it").is_none(), "a suffix must not match");
        assert!(resolve("GEMMA4").is_none(), "case-sensitive");
        assert!(resolve("").is_none(), "absent arch");
        assert!(resolve("llama").is_none(), "unregistered");
    }

    /// ★ A ChatML-shaped family must NOT be registered. `resolve` returning `Some` makes
    /// the Ollama backend render instead of returning `None`, and its caller's ChatML
    /// differs from this one on the empty-system case — so claiming such a family
    /// silently rewrites prompt bytes for a path that was already correct.
    #[test]
    fn no_registered_family_is_merely_chatml() {
        for f in FAMILIES {
            assert_ne!(
                f.template, CHATML,
                "{:?} is ChatML-shaped — leave it unregistered so the caller's fallback stands",
                f.ids
            );
        }
    }

    /// The measured Gemma-4 render, byte for byte. The generation prefix is the load-
    /// bearing part: without it the model emits `<|channel>thought\n<channel|>` as
    /// CONTENT and it reaches the caller as answer text.
    #[test]
    fn gemma4_renders_the_measured_template() {
        let t = resolve("gemma4").expect("registered").template;
        assert_eq!(
            t.render_system_user("SYS", "USR"),
            "<|turn>system\nSYS<turn|>\n<|turn>user\nUSR<turn|>\n\
             <|turn>model\n<|channel>thought\n<channel|>"
        );
    }

    /// Gemma-3 has NO system role: the system text is its own `user` turn, not merged
    /// into the user's and not emitted under a `system` label.
    #[test]
    fn gemma3_has_no_system_role_and_renders_system_as_its_own_user_turn() {
        let t = resolve("gemma3").expect("registered").template;
        let out = t.render_system_user("SYS", "USR");
        assert_eq!(
            out,
            "<start_of_turn>user\nSYS<end_of_turn>\n\
             <start_of_turn>user\nUSR<end_of_turn>\n\
             <start_of_turn>model\n"
        );
        assert!(!out.contains("system"), "gemma3 has no system role");
        assert_eq!(out.matches("<start_of_turn>user").count(), 2);
    }

    /// An empty system renders the user turn alone, for every registered family — a
    /// leading empty turn is one no model was trained to see.
    #[test]
    fn an_empty_system_renders_the_user_turn_alone_for_every_family() {
        for f in FAMILIES {
            let out = f.template.render_system_user("", "USR");
            let opens = out.matches(f.template.turn_open).count();
            assert_eq!(
                opens, 2,
                "{:?}: expected the user turn + the generation turn, got {out:?}",
                f.ids
            );
        }
    }

    /// ★ THE DERIVED-STOPS INVARIANT, for EVERY family rather than the one someone
    /// remembered. A raw-mode dispatch supplies the only stops there are, so a family
    /// whose terminator is unlisted renders correctly and then runs past the turn.
    #[test]
    fn every_family_terminator_is_a_declared_stop() {
        let all = all_stop_tokens();
        for f in FAMILIES {
            let rendered = f.template.render_system_user("S", "U");
            assert!(
                rendered.contains(f.template.turn_close),
                "{:?}: the render must carry its own terminator",
                f.ids
            );
            assert!(
                all.contains(&f.template.turn_close),
                "{:?}: terminator {:?} missing from {all:?}",
                f.ids,
                f.template.turn_close
            );
        }
    }

    /// No id may be claimed by two families — the table is a function, and a duplicate
    /// would make resolution depend on entry order.
    #[test]
    fn no_architecture_id_is_claimed_twice() {
        let mut seen: Vec<&str> = Vec::new();
        for f in FAMILIES {
            for id in f.ids {
                assert!(!seen.contains(id), "{id} is claimed by two families");
                seen.push(id);
            }
        }
    }

    #[test]
    fn resolve_from_desc_reads_the_leading_arch_token() {
        assert_eq!(
            resolve_from_desc("gemma4 12B Q4_K - Medium").map(|f| f.ids),
            Some(GEMMA4_IDS)
        );
        assert!(resolve_from_desc("").is_none());
    }
    const GEMMA4_IDS: &[&str] = &["gemma4"];

    /// `assistant` maps onto the family's own name for the model's role.
    #[test]
    fn assistant_maps_to_the_family_role_name() {
        let g4 = resolve("gemma4").expect("registered").template;
        let out = g4.render_messages([("assistant", "prior")]);
        assert!(out.starts_with("<|turn>model\nprior<turn|>\n"), "{out:?}");
        let chatml = CHATML.render_messages([("assistant", "prior")]);
        assert!(chatml.starts_with("<|im_start|>assistant\nprior<|im_end|>\n"));
    }

    /// Deterministic — the greedy + content-addressed replay contract depends on it.
    #[test]
    fn rendering_is_deterministic() {
        let t = resolve("gemma4").expect("registered").template;
        assert_eq!(
            t.render_system_user("S", "U"),
            t.render_system_user("S", "U")
        );
    }
}
