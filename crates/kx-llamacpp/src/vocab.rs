//! `Vocab` — borrowed handle to a model's vocabulary.
//!
//! The vocab is owned by [`crate::Model`]; this type is a lifetime-tied borrow
//! that exposes tokenization, detokenization, and special-token queries.

use std::fmt;
use std::ptr::NonNull;

use kx_llamacpp_sys as sys;

use crate::error::LlamaError;
use crate::model::Model;

/// A token id wrapping `i32`.
///
/// Newtype rather than a bare alias so the compiler distinguishes tokens from
/// positions / sequence ids (which are also `i32` on the C side). Construct
/// via `Token(id)`, `Token::from(id)`, or directly out of [`Vocab`] /
/// [`crate::Sampler`] calls. Extract the raw id via `.0`, `.id()`, or
/// `i32::from(token)`.
///
/// Convenience methods ([`Self::is_eog`], [`Self::to_piece`]) mirror the
/// `Vocab` API so token-centric code can call them on the token directly.
///
/// # Examples
///
/// ```
/// use kx_llamacpp::Token;
///
/// let t = Token(42);
/// assert_eq!(t.id(), 42);
/// assert_eq!(t.0, 42);
/// assert_eq!(i32::from(t), 42);
///
/// // Hash + Eq let Token be used as a key.
/// use std::collections::HashSet;
/// let mut seen: HashSet<Token> = HashSet::new();
/// seen.insert(t);
/// assert!(seen.contains(&Token(42)));
/// ```
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Token(pub i32);

impl Token {
    /// The raw token id (alias for `.0`).
    #[inline]
    pub fn id(&self) -> i32 {
        self.0
    }

    /// True if this token is an end-of-generation marker for `vocab`.
    /// Convenience for [`Vocab::is_eog`].
    pub fn is_eog(&self, vocab: &Vocab<'_, '_>) -> bool {
        vocab.is_eog(*self)
    }

    /// Convert this token to its UTF-8 piece (bytes) via `vocab`.
    /// Convenience for [`Vocab::token_to_piece`] with `lstrip = 0`, `special = false`.
    ///
    /// # Errors
    /// See [`Vocab::token_to_piece`].
    pub fn to_piece(&self, vocab: &Vocab<'_, '_>) -> Result<Vec<u8>, LlamaError> {
        vocab.token_to_piece(*self, 0, false)
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({})", self.0)
    }
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<i32> for Token {
    #[inline]
    fn from(id: i32) -> Self {
        Token(id)
    }
}

impl From<Token> for i32 {
    #[inline]
    fn from(t: Token) -> Self {
        t.0
    }
}

/// Borrowed handle to a model's vocabulary.
pub struct Vocab<'m, 'b: 'm> {
    ptr: NonNull<sys::llama_vocab>,
    _model: std::marker::PhantomData<&'m Model<'b>>,
}

impl<'m, 'b: 'm> Vocab<'m, 'b> {
    pub(crate) fn from_raw(ptr: NonNull<sys::llama_vocab>) -> Self {
        Self {
            ptr,
            _model: std::marker::PhantomData,
        }
    }

    /// The raw `llama_vocab` pointer, for the grammar-sampler FFI
    /// (`llama_sampler_init_grammar{,_lazy_patterns}` take a `const
    /// llama_vocab *`). `pub(crate)` so the unsafe ptr never escapes the crate;
    /// the grammar sampler stores this pointer internally, so a sampler built
    /// from it MUST NOT outlive the model that owns this vocab (upheld at the
    /// call site, which builds + consumes the sampler within one generation
    /// scope — see [`crate::SamplerChainBuilder::add_grammar`]).
    pub(crate) fn as_ptr(&self) -> *const sys::llama_vocab {
        self.ptr.as_ptr()
    }

    /// Number of distinct tokens in the vocabulary.
    pub fn n_tokens(&self) -> i32 {
        // SAFETY: ptr borrowed from a live model.
        unsafe { sys::llama_vocab_n_tokens(self.ptr.as_ptr()) }
    }

    /// Beginning-of-sentence token (or -1 / sentinel if the vocab has none).
    pub fn bos(&self) -> Token {
        Token(unsafe { sys::llama_vocab_bos(self.ptr.as_ptr()) })
    }

    /// End-of-sentence token.
    pub fn eos(&self) -> Token {
        Token(unsafe { sys::llama_vocab_eos(self.ptr.as_ptr()) })
    }

    /// Newline token.
    pub fn nl(&self) -> Token {
        Token(unsafe { sys::llama_vocab_nl(self.ptr.as_ptr()) })
    }

    /// Is this token an end-of-generation marker? (EOS, EOT, EOM — varies by model.)
    pub fn is_eog(&self, token: Token) -> bool {
        unsafe { sys::llama_vocab_is_eog(self.ptr.as_ptr(), token.0) }
    }

    /// Tokenize `text` into a vector of token ids.
    ///
    /// * `add_special` — prepend the BOS token when the model conventions
    ///   call for it.
    /// * `parse_special` — interpret special-token markup (e.g. `<|im_start|>`).
    ///
    /// # Errors
    /// [`LlamaError::TokenizeFailed`] if the underlying call returns an
    /// unrecoverable error after the standard resize-and-retry path.
    pub fn tokenize(
        &self,
        text: &str,
        add_special: bool,
        parse_special: bool,
    ) -> Result<Vec<Token>, LlamaError> {
        // First pass with a tight upper-bound estimate: BPE tokenizers produce
        // at most `bytes.len()` tokens (each token covers >= 1 byte), plus 2
        // for BOS + safety. This eliminates the second call for the typical
        // case (prompts > 7 bytes); the resize-and-retry path remains for the
        // rare case the estimate is wrong.
        let bytes = text.as_bytes();
        let mut capacity = bytes.len() + 2;
        let mut out: Vec<i32> = vec![0; capacity];

        // SAFETY: vocab ptr is valid; text+len define a byte slice; out provides
        // a writable buffer of `capacity` i32 slots.
        let rc = unsafe {
            sys::llama_tokenize(
                self.ptr.as_ptr(),
                bytes.as_ptr().cast::<core::ffi::c_char>(),
                bytes.len() as i32,
                out.as_mut_ptr(),
                capacity as i32,
                add_special,
                parse_special,
            )
        };

        let written = if rc < 0 {
            capacity = (-rc) as usize;
            out = vec![0; capacity];
            let rc2 = unsafe {
                sys::llama_tokenize(
                    self.ptr.as_ptr(),
                    bytes.as_ptr().cast::<core::ffi::c_char>(),
                    bytes.len() as i32,
                    out.as_mut_ptr(),
                    capacity as i32,
                    add_special,
                    parse_special,
                )
            };
            if rc2 < 0 {
                return Err(LlamaError::TokenizeFailed(rc2));
            }
            rc2
        } else {
            rc
        };

        out.truncate(written.max(0) as usize);
        Ok(out.into_iter().map(Token).collect())
    }

    /// Convert a single token to its UTF-8 piece (bytes, since some tokens
    /// are sub-character byte fragments).
    ///
    /// * `lstrip` — number of leading whitespace bytes to strip (passed
    ///   directly to llama.cpp; typically 0).
    /// * `special` — render special tokens visibly (`<|im_start|>` rather than
    ///   the empty string).
    ///
    /// # Errors
    /// [`LlamaError::DetokenizeFailed`] on a non-recoverable C-side failure.
    pub fn token_to_piece(
        &self,
        token: Token,
        lstrip: i32,
        special: bool,
    ) -> Result<Vec<u8>, LlamaError> {
        let mut buf: Vec<u8> = Vec::with_capacity(32);
        self.token_to_piece_into(token, lstrip, special, &mut buf)?;
        Ok(buf)
    }

    /// Append the bytes for `token` directly to `out` (zero per-call alloc when
    /// `out` already has capacity). Used by [`Self::detokenize`] to avoid
    /// allocating a fresh `Vec<u8>` per token in the generation hot loop.
    ///
    /// # Errors
    /// [`LlamaError::DetokenizeFailed`] on a non-recoverable C-side failure.
    pub fn token_to_piece_into(
        &self,
        token: Token,
        lstrip: i32,
        special: bool,
        out: &mut Vec<u8>,
    ) -> Result<usize, LlamaError> {
        // Try the existing tail of `out` (whatever spare capacity it has) up
        // to 32 bytes — most tokens fit there. If not, grow once.
        let start = out.len();
        let mut tail = 32usize.max(out.capacity().saturating_sub(start));
        out.resize(start + tail, 0);

        let rc = unsafe {
            sys::llama_token_to_piece(
                self.ptr.as_ptr(),
                token.0,
                out[start..].as_mut_ptr().cast::<core::ffi::c_char>(),
                tail as i32,
                lstrip,
                special,
            )
        };

        let written = if rc < 0 {
            tail = (-rc) as usize;
            out.resize(start + tail, 0);
            let rc2 = unsafe {
                sys::llama_token_to_piece(
                    self.ptr.as_ptr(),
                    token.0,
                    out[start..].as_mut_ptr().cast::<core::ffi::c_char>(),
                    tail as i32,
                    lstrip,
                    special,
                )
            };
            if rc2 < 0 {
                out.truncate(start);
                return Err(LlamaError::DetokenizeFailed {
                    token: token.0,
                    rc: rc2,
                });
            }
            rc2 as usize
        } else {
            rc as usize
        };

        out.truncate(start + written);
        Ok(written)
    }

    /// Convenience: detokenize a slice of tokens into a UTF-8 String.
    ///
    /// Lossy conversion: invalid UTF-8 bytes are replaced with U+FFFD.
    /// Uses a single growing byte buffer; no per-token allocation.
    /// For raw byte access use [`Self::token_to_piece_into`] in a loop.
    ///
    /// # Errors
    /// Propagates the first [`LlamaError::DetokenizeFailed`] from
    /// [`Self::token_to_piece_into`].
    pub fn detokenize(&self, tokens: &[Token], special: bool) -> Result<String, LlamaError> {
        // Heuristic capacity: average 4 bytes per token (varies by tokenizer).
        let mut out: Vec<u8> = Vec::with_capacity(tokens.len() * 4);
        for &t in tokens {
            self.token_to_piece_into(t, 0, special, &mut out)?;
        }
        Ok(String::from_utf8_lossy(&out).into_owned())
    }
}
