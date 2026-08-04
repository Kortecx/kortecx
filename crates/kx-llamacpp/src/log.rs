//! Grammar-engagement counters, read from llama.cpp's own log stream.
//!
//! **Why this exists.** A lazy GBNF is a bet on the model's output dialect, and the bet is
//! silent when it loses: the tolerant parser recovers the call either way, so every "did
//! the tool fire?" check stays green while the sampler never engages and the arguments go
//! completely unconstrained. Reading the code tells you a grammar is *wired*; nothing in
//! this workspace could tell you it *engaged*.
//!
//! llama.cpp already knows. `llama_grammar_accept_impl` logs `Grammar triggered on …` the
//! moment the sampler arms, and `Grammar still awaiting trigger …` for every token it
//! buffers without matching. Those lines go through `llama_log_internal`, which applies
//! **no level filter** — the default callback prints everything — so installing our own
//! callback observes the subject itself rather than a proxy for it. If the sampler never
//! arms, the counter reads zero because the event never happened, which is the property a
//! post-hoc scan of the emitted text could never have.
//!
//! **Attribution is by thread, not by atomic snapshot.** The callback is invoked
//! synchronously on whichever thread called `llama_sampler_accept` — no queue, no worker,
//! no deferral — so a thread-local counter attributes exactly even when several model
//! caches decode concurrently on their own owner threads.
//!
//! **Output is preserved by default.** `llama_log_set` REPLACES llama.cpp's default
//! callback, which is what prints every model-load diagnostic. Counting must not cost
//! anyone their logs, so the default forwards every line to stderr verbatim; set
//! `KX_LLAMACPP_LOG=quiet` to drop the sub-warning chatter (the per-token "awaiting"
//! line is emitted once per sampled token) while keeping warnings and errors.

use std::cell::Cell;
use std::ffi::{c_char, c_void, CStr};
use std::sync::Once;

use kx_llamacpp_sys as sys;

/// The upstream log prefix emitted when the lazy grammar ARMS. Matches both the regex and
/// the token trigger forms (`Grammar triggered on regex: …` / `… on token …`) — we pass
/// patterns only today, but counting both means a future switch to trigger tokens cannot
/// silently zero the metric.
pub const TRIGGERED_PREFIX: &str = "Grammar triggered on";
/// The upstream log prefix emitted for every token buffered without matching a trigger.
pub const AWAITING_PREFIX: &str = "Grammar still awaiting trigger";

thread_local! {
    static ARMED: Cell<u64> = const { Cell::new(0) };
    static ENGAGED: Cell<u64> = const { Cell::new(0) };
    static AWAITING: Cell<u64> = const { Cell::new(0) };
}

/// Record that a lazy-grammar stage was constructed on this thread.
///
/// Counted by the caller rather than by llama.cpp, because llama.cpp has no "a grammar was
/// installed" log line — and without it, `engaged == 0` cannot be told apart from "no
/// grammar was ever derived for this turn", which is a completely different defect.
/// Lives beside the other two counters so all three share one attribution model: the owner
/// thread that builds the sampler is the thread that decodes with it.
pub fn note_grammar_armed() {
    ARMED.with(|c| c.set(c.get().saturating_add(1)));
}

/// What the sampler did with a lazy grammar over some span of decoding.
///
/// The three fields exist to separate a real zero from a broken instrument. `armed` is
/// counted by us where the stage is constructed; the other two come from llama.cpp:
///
/// | reading | meaning |
/// |---|---|
/// | `armed == 0` | no grammar was derived — a dispatch or grant defect, not a trigger one |
/// | `armed > 0`, both others `0` | **the instrument is broken** — the callback never fired |
/// | `armed > 0`, `awaiting > 0`, `engaged == 0` | armed, saw tokens, never matched |
/// | `engaged > 0` | the sampler engaged |
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GrammarEngagement {
    /// Lazy-grammar stages constructed (counted by `kx-inference`, not by llama.cpp).
    pub armed: u64,
    /// Times the sampler matched a trigger and began constraining.
    pub engaged: u64,
    /// Tokens buffered while still awaiting a trigger.
    pub awaiting: u64,
}

impl GrammarEngagement {
    /// This reading minus an earlier one — the delta over a span of decoding.
    #[must_use]
    pub fn since(self, before: Self) -> Self {
        Self {
            armed: self.armed.saturating_sub(before.armed),
            engaged: self.engaged.saturating_sub(before.engaged),
            awaiting: self.awaiting.saturating_sub(before.awaiting),
        }
    }

    /// The shipped defect, as a predicate: a grammar was armed, the sampler buffered
    /// tokens, and it never engaged.
    #[must_use]
    pub const fn armed_and_never_engaged(self) -> bool {
        self.engaged == 0 && self.awaiting > 0
    }

    /// True when the counters cannot be believed: something armed a grammar and decoded,
    /// yet llama.cpp reported neither an engagement nor a single awaiting token. Callers
    /// must report this as an instrument failure, never as "it did not engage".
    #[must_use]
    pub const fn looks_uninstrumented(self) -> bool {
        self.armed > 0 && self.engaged == 0 && self.awaiting == 0
    }
}

/// The calling thread's engagement counters since its own first decode.
#[must_use]
pub fn grammar_engagement() -> GrammarEngagement {
    GrammarEngagement {
        armed: ARMED.with(Cell::get),
        engaged: ENGAGED.with(Cell::get),
        awaiting: AWAITING.with(Cell::get),
    }
}

/// Reset the calling thread's counters. For tests that want an absolute reading rather
/// than a delta.
pub fn reset_grammar_engagement() {
    ARMED.with(|c| c.set(0));
    ENGAGED.with(|c| c.set(0));
    AWAITING.with(|c| c.set(0));
}

static INSTALL: Once = Once::new();

/// Install the counting log callback, once per process.
///
/// Called from [`crate::LlamaBackend::new`] before `llama_backend_init` so init-time lines
/// are captured too. Never uninstalled: the callback is a plain `fn` with a null
/// user-data pointer, so there is nothing that could dangle when the backend refcount
/// returns to zero — uninstalling would be strictly more dangerous than leaving it.
pub fn install() {
    INSTALL.call_once(|| {
        // SAFETY: `kx_log_callback` is `extern "C"`, panic-guarded and allocation-free;
        // the user-data pointer is null and is never dereferenced. `llama_log_set` stores
        // the pointer in a process-global and llama.cpp calls it synchronously from the
        // logging thread. The callback is a `fn` item with `'static` code, so the pointer
        // is valid for the life of the process.
        unsafe { sys::llama_log_set(Some(kx_log_callback), std::ptr::null_mut()) };
    });
}

/// Whether to suppress llama.cpp lines below warning level. Read once.
fn quiet() -> bool {
    static QUIET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *QUIET.get_or_init(|| {
        std::env::var("KX_LLAMACPP_LOG")
            .map(|v| v.trim().eq_ignore_ascii_case("quiet"))
            .unwrap_or(false)
    })
}

/// llama.cpp's log sink: count the two grammar lines, then forward.
///
/// Must not unwind — a panic crossing an `extern "C"` boundary aborts the process — and
/// must not allocate, because it runs once per sampled token.
unsafe extern "C" fn kx_log_callback(
    level: sys::ggml_log_level,
    text: *const c_char,
    _user_data: *mut c_void,
) {
    if text.is_null() {
        return;
    }
    let _ = std::panic::catch_unwind(|| {
        // SAFETY: llama.cpp passes a NUL-terminated C string that outlives this call.
        let bytes = unsafe { CStr::from_ptr(text) }.to_bytes();
        if bytes.starts_with(TRIGGERED_PREFIX.as_bytes()) {
            let _ = ENGAGED.try_with(|c| c.set(c.get().saturating_add(1)));
        } else if bytes.starts_with(AWAITING_PREFIX.as_bytes()) {
            let _ = AWAITING.try_with(|c| c.set(c.get().saturating_add(1)));
        }
        let suppress = quiet()
            && !matches!(
                level,
                sys::ggml_log_level_GGML_LOG_LEVEL_WARN | sys::ggml_log_level_GGML_LOG_LEVEL_ERROR
            );
        if !suppress {
            use std::io::Write as _;
            let _ = std::io::stderr().lock().write_all(bytes);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_with_no_decoding_is_all_zero() {
        reset_grammar_engagement();
        assert_eq!(grammar_engagement().engaged, 0);
        assert_eq!(grammar_engagement().awaiting, 0);
    }

    #[test]
    fn since_is_a_saturating_delta() {
        let before = GrammarEngagement {
            armed: 1,
            engaged: 2,
            awaiting: 30,
        };
        let after = GrammarEngagement {
            armed: 3,
            engaged: 5,
            awaiting: 30,
        };
        assert_eq!(
            after.since(before),
            GrammarEngagement {
                armed: 2,
                engaged: 3,
                awaiting: 0
            }
        );
        // A counter that went backwards (a thread reused across spans) reads 0, never
        // an underflowed enormous number.
        assert_eq!(before.since(after).engaged, 0);
    }

    /// The three readings the counter exists to keep apart.
    #[test]
    fn a_broken_instrument_is_distinguishable_from_a_real_zero() {
        let never_engaged = GrammarEngagement {
            armed: 1,
            engaged: 0,
            awaiting: 72,
        };
        let uninstrumented = GrammarEngagement {
            armed: 1,
            engaged: 0,
            awaiting: 0,
        };
        let engaged = GrammarEngagement {
            armed: 1,
            engaged: 4,
            awaiting: 12,
        };

        assert!(never_engaged.armed_and_never_engaged());
        assert!(!never_engaged.looks_uninstrumented());

        assert!(uninstrumented.looks_uninstrumented());
        assert!(!uninstrumented.armed_and_never_engaged());

        assert!(!engaged.armed_and_never_engaged());
        assert!(!engaged.looks_uninstrumented());
    }

    /// The prefixes are matched against llama.cpp's own text. A submodule bump that
    /// renames them would zero the metric silently, so `kx-llamacpp-sys`'s build script
    /// greps the pinned source for both — this test pins what that grep looks for.
    #[test]
    fn the_watched_prefixes_are_the_upstream_wording() {
        assert_eq!(TRIGGERED_PREFIX, "Grammar triggered on");
        assert_eq!(AWAITING_PREFIX, "Grammar still awaiting trigger");
    }
}
