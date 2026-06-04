// A loaded-model handle cache for the in-process llama.cpp backend.
//
// WHY A DEDICATED OWNER THREAD (not a `Mutex<Model>` field):
// `kx_llamacpp::LlamaBackend` is `!Send + !Sync` (it carries
// `PhantomData<*const ()>` to pin llama.cpp's not-thread-safe global state to
// one thread), and every `Model<'b>` borrows the backend. So the backend and
// its loaded models cannot be stored in a `Send + Sync` backend struct, nor
// behind a `Mutex` (a reference to a `!Sync` type is itself `!Send`). The clean
// solution — the one the wrapper docs name — is a single owner thread that
// creates the backend and keeps every loaded `Model` as a thread-local. The
// only thing that crosses the thread boundary is the `mpsc` channel handle
// (`Send + Sync` on Rust ≥ 1.72) and plain-data jobs/replies (`InferenceInput`,
// `InferenceParams`, `InferenceOutput` are all `Send`).
//
// WHAT IT FIXES: the pre-cache backend called `Model::load(path)` on EVERY
// dispatch (seconds for a 7B model; catastrophic for a multi-GB multimodal
// model). The owner thread loads each model once, keyed by its
// `kx_model_store` `identity_digest`, and reuses it. A small LRU bounds RAM.
//
// MULTI-MODAL (PR-2): an image dispatch carries already-resolved image bytes +
// the projector (`mmproj`) path. The owner thread reuses the cached base model
// but (PR-2) re-creates the `Mtmd` projector context per dispatch — the base
// model cache (the PR-1 win) is preserved; the projector reload is *measured*
// via `mmproj_loads` and eliminated in the PR-2.5 projector-cache follow-up.
//
// CONCURRENCY: dispatch is serialized by the single owner thread — exactly the
// discipline `kx-llamacpp` mandates ("never `&Context` from two threads").
// Concurrent callers queue on the channel; each blocks until its reply lands.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use kx_content::ContentRef;
use kx_llamacpp::{
    Bitmap, Context, ContextParams, Generator, LlamaBackend, LlamaError, Model, Mtmd, Sampler,
    Vocab,
};
use kx_mote::{InferenceParams, ModelId};
use smallvec::SmallVec;

use crate::llama::BACKEND_NAME;
use crate::types::{InferenceError, InferenceOutput, MEDIA_MARKER};

/// Default number of distinct models kept loaded at once. Small because models
/// are heavyweight; enough for one active model plus a swap.
pub(crate) const DEFAULT_CACHE_CAPACITY: usize = 2;

/// Multi-modal prefill batch size. Larger than llama.cpp's 512 default because
/// a single high-resolution image on a dynamic-resolution VLM can exceed 512
/// image tokens; an undersized batch would make `mtmd_helper_eval_chunks` fail
/// the decode. This is a correctness bound, not a tuning knob.
const MULTIMODAL_N_BATCH: u32 = 2048;
const MULTIMODAL_N_UBATCH: u32 = 512;

/// Convert a `kx_llamacpp::LlamaError` into the public error enum. Localised so
/// the dispatcher's error surface stays stable as `kx-llamacpp`'s variants
/// evolve.
#[allow(clippy::needless_pass_by_value)]
pub(crate) fn map_llama_err(err: LlamaError) -> InferenceError {
    InferenceError::BackendFailure {
        backend: BACKEND_NAME,
        message: format!("{err}"),
    }
}

/// One inference request handed to the owner thread. All fields are `Send`.
struct Job {
    /// Loaded-model cache key (path+modality identity; NOT a weight hash).
    identity: ContentRef,
    /// Where to load the model from on a cache miss.
    path: PathBuf,
    /// Echoed back into `InferenceOutput.model_id`.
    model_id: ModelId,
    /// Text prompt. For a multi-modal job it contains one [`MEDIA_MARKER`] per
    /// image (in order), spliced in by the context assembler.
    prompt: String,
    /// Resolved, size-capped, image-sniffed media bytes — empty for a text job.
    /// The owner thread decodes each via the projector's `stb`-backed helper.
    images: SmallVec<[Vec<u8>; 2]>,
    /// The multi-modal projector path — `Some` iff this is an image job.
    mmproj_path: Option<PathBuf>,
    /// Decoding parameters (already validated against the warrant by the caller).
    params: InferenceParams,
    /// Context window for this dispatch.
    n_ctx: u32,
    /// Wall-clock ceiling in milliseconds (`warrant.resource_ceiling`).
    wall_clock_ms: u64,
    /// Where the owner thread sends the result.
    reply: Sender<Result<InferenceOutput, InferenceError>>,
}

/// `Send + Sync` handle to the model-cache owner thread. Cheap to clone (clones
/// share one worker + the load counters).
#[derive(Clone, Debug)]
pub(crate) struct ModelCache {
    tx: Sender<Job>,
    /// Number of cold `Model::load`s performed — the observable proof that a
    /// cache hit did NOT reload (and the ops metric for "the reload is gone").
    loads: Arc<AtomicU64>,
    /// Number of `Mtmd` projector loads performed. In PR-2 this increments once
    /// per image dispatch (the projector is re-created each time); the PR-2.5
    /// projector cache will make it increment once per distinct model. Exposed
    /// as the measured signal that justifies that follow-up.
    mmproj_loads: Arc<AtomicU64>,
}

impl ModelCache {
    /// Spawn the owner thread and return a handle to it. The thread lives until
    /// every `ModelCache` clone (and thus every `Sender`) is dropped, at which
    /// point `recv` errors and it exits cleanly.
    pub(crate) fn spawn(capacity: usize) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let loads = Arc::new(AtomicU64::new(0));
        let mmproj_loads = Arc::new(AtomicU64::new(0));
        let worker_loads = Arc::clone(&loads);
        let worker_mmproj = Arc::clone(&mmproj_loads);
        // `std::thread::spawn` panics on OS thread-exhaustion — an unrecoverable
        // condition for which panic is the correct behavior (no Result to leak
        // into the infallible backend constructors).
        let _handle = std::thread::spawn(move || {
            owner_loop(&rx, capacity.max(1), &worker_loads, &worker_mmproj);
        });
        Self {
            tx,
            loads,
            mmproj_loads,
        }
    }

    /// Number of cold model loads performed so far.
    pub(crate) fn loads(&self) -> u64 {
        self.loads.load(Ordering::Relaxed)
    }

    /// Number of projector (`Mtmd`) loads performed so far.
    pub(crate) fn mmproj_loads(&self) -> u64 {
        self.mmproj_loads.load(Ordering::Relaxed)
    }

    /// Submit a job and block until the owner thread replies.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn dispatch(
        &self,
        identity: ContentRef,
        path: PathBuf,
        model_id: ModelId,
        prompt: String,
        images: SmallVec<[Vec<u8>; 2]>,
        mmproj_path: Option<PathBuf>,
        params: InferenceParams,
        n_ctx: u32,
        wall_clock_ms: u64,
    ) -> Result<InferenceOutput, InferenceError> {
        let (reply_tx, reply_rx) = mpsc::channel();
        let job = Job {
            identity,
            path,
            model_id,
            prompt,
            images,
            mmproj_path,
            params,
            n_ctx,
            wall_clock_ms,
            reply: reply_tx,
        };
        self.tx
            .send(job)
            .map_err(|_| InferenceError::BackendFailure {
                backend: BACKEND_NAME,
                message: "model-cache owner thread is gone (send failed)".to_string(),
            })?;
        reply_rx
            .recv()
            .map_err(|_| InferenceError::BackendFailure {
                backend: BACKEND_NAME,
                message: "model-cache owner thread died mid-job (recv failed)".to_string(),
            })?
    }
}

/// The owner thread's loop. Owns the (`!Send`) backend and the loaded-model LRU
/// as thread-locals; serves jobs until the channel closes.
fn owner_loop(rx: &Receiver<Job>, capacity: usize, loads: &AtomicU64, mmproj_loads: &AtomicU64) {
    let backend = match LlamaBackend::new() {
        Ok(b) => b,
        Err(e) => {
            // Backend init failed: reply the same error to every queued job so
            // callers fail fast instead of hanging.
            let err = map_llama_err(e);
            while let Ok(job) = rx.recv() {
                let _ = job.reply.send(Err(err.clone()));
            }
            return;
        }
    };

    // LRU of loaded models. Each `Model<'_>` borrows `backend`; both are locals
    // of this function, so the borrow is valid for the whole loop and `lru` is
    // dropped before `backend`.
    let mut lru: Vec<(ContentRef, Model<'_>)> = Vec::with_capacity(capacity);

    while let Ok(job) = rx.recv() {
        let result = run_job(&backend, &mut lru, capacity, loads, mmproj_loads, &job);
        // A dropped reply receiver (caller gave up) is not our problem.
        let _ = job.reply.send(result);
    }
}

/// Resolve-or-load the model, then run (multi-modal or text) generation.
fn run_job<'b>(
    backend: &'b LlamaBackend,
    lru: &mut Vec<(ContentRef, Model<'b>)>,
    capacity: usize,
    loads: &AtomicU64,
    mmproj_loads: &AtomicU64,
    job: &Job,
) -> Result<InferenceOutput, InferenceError> {
    let start = Instant::now();
    let timeout = Duration::from_millis(job.wall_clock_ms);
    let model = get_or_load(backend, lru, capacity, loads, job.identity, &job.path)
        .map_err(map_llama_err)?;
    check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;

    let (bytes, output_tokens) = if job.images.is_empty() {
        generate(backend, model, job, start, timeout)?
    } else {
        let mmproj = job
            .mmproj_path
            .as_deref()
            .ok_or(InferenceError::Unsupported {
                reason: "multimodal job is missing its projector (mmproj) path",
            })?;
        generate_multimodal(backend, model, mmproj, mmproj_loads, job, start, timeout)?
    };
    Ok(InferenceOutput {
        bytes,
        output_tokens,
        backend_name: BACKEND_NAME,
        model_id: job.model_id.clone(),
        elapsed: start.elapsed(),
    })
}

/// Return a reference to the cached model for `identity`, loading + inserting it
/// (and evicting the LRU front at capacity) on a miss. By construction the
/// wanted model is the last LRU entry on return.
fn get_or_load<'a, 'b>(
    backend: &'b LlamaBackend,
    lru: &'a mut Vec<(ContentRef, Model<'b>)>,
    capacity: usize,
    loads: &AtomicU64,
    identity: ContentRef,
    path: &Path,
) -> Result<&'a Model<'b>, LlamaError> {
    if let Some(pos) = lru.iter().position(|(id, _)| *id == identity) {
        // Hit: move to the most-recently-used end.
        let entry = lru.remove(pos);
        lru.push(entry);
    } else {
        // Miss: evict LRU front if full, then load + insert.
        if lru.len() >= capacity {
            let _evicted = lru.remove(0);
        }
        let model = Model::load(backend, path)?;
        loads.fetch_add(1, Ordering::Relaxed);
        lru.push((identity, model));
    }
    // Non-empty by construction (a hit re-pushes; a miss pushes).
    let idx = lru.len() - 1;
    Ok(&lru[idx].1)
}

/// Build the sampler for `params` (greedy when `temperature_bps == 0`, else
/// temp/top-k/top-p). Shared by the text and multi-modal generation paths.
fn build_sampler<'b>(
    backend: &'b LlamaBackend,
    params: &InferenceParams,
) -> Result<Sampler<'b>, InferenceError> {
    if params.temperature_bps == 0 {
        Sampler::greedy(backend).map_err(map_llama_err)
    } else {
        #[allow(clippy::cast_precision_loss)]
        let temp = (params.temperature_bps as f32) / 10_000.0;
        #[allow(clippy::cast_precision_loss)]
        let top_p = (params.top_p_bps as f32) / 10_000.0;
        #[allow(clippy::cast_possible_wrap)]
        let top_k = params.top_k as i32;
        Sampler::typical(backend, temp, top_k, top_p, params.seed).map_err(map_llama_err)
    }
}

/// The token-emission loop: pull tokens from `generator`, detokenize into bytes,
/// and stop on the output-token cap, EOG, or the wall-clock ceiling. Shared by
/// the text path (prompt-prefilled `Generator`) and the multi-modal path
/// (`Generator::from_prefilled` after the mtmd prefill).
fn run_generation(
    generator: Generator<'_, '_, '_, '_, '_>,
    vocab: &Vocab<'_, '_>,
    job: &Job,
    start: Instant,
    timeout: Duration,
) -> Result<(Vec<u8>, u32), InferenceError> {
    let mut output_bytes: Vec<u8> = Vec::with_capacity(
        usize::try_from(job.params.max_output_tokens.saturating_mul(4)).unwrap_or(2048),
    );
    let mut output_tokens: u32 = 0;

    for token_result in generator {
        check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;
        let token = token_result.map_err(map_llama_err)?;
        vocab
            .token_to_piece_into(token, 0, false, &mut output_bytes)
            .map_err(map_llama_err)?;
        output_tokens = output_tokens.saturating_add(1);
        if output_tokens >= job.params.max_output_tokens {
            break;
        }
        if vocab.is_eog(token) {
            break;
        }
    }

    Ok((output_bytes, output_tokens))
}

/// The text generation path: tokenize the prompt, prefill via `Generator::new`,
/// then emit tokens. Behaviorally identical to the pre-multimodal path.
fn generate(
    backend: &LlamaBackend,
    model: &Model<'_>,
    job: &Job,
    start: Instant,
    timeout: Duration,
) -> Result<(Vec<u8>, u32), InferenceError> {
    let ctx_params = ContextParams::new().with_n_ctx(job.n_ctx);
    let mut ctx = Context::new_with_params(model, &ctx_params).map_err(map_llama_err)?;
    let vocab = model.vocab();

    let prompt_tokens = vocab
        .tokenize(&job.prompt, true, false)
        .map_err(map_llama_err)?;
    check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;

    let mut sampler = build_sampler(backend, &job.params)?;
    let generator =
        Generator::new(&mut ctx, &mut sampler, &vocab, prompt_tokens).map_err(map_llama_err)?;
    run_generation(generator, &vocab, job, start, timeout)
}

/// The multi-modal (image) generation path: load the projector, decode the
/// images into bitmaps (fail-closed), splice them in at the media markers,
/// run the mtmd prefill, then emit tokens with the SAME sampler + generation
/// loop as the text path.
fn generate_multimodal(
    backend: &LlamaBackend,
    model: &Model<'_>,
    mmproj: &Path,
    mmproj_loads: &AtomicU64,
    job: &Job,
    start: Instant,
    timeout: Duration,
) -> Result<(Vec<u8>, u32), InferenceError> {
    // A larger batch so a single high-token image does not overflow the decode.
    let ctx_params = ContextParams::new()
        .with_n_ctx(job.n_ctx)
        .with_n_batch(MULTIMODAL_N_BATCH)
        .with_n_ubatch(MULTIMODAL_N_UBATCH);
    let mut ctx = Context::new_with_params(model, &ctx_params).map_err(map_llama_err)?;
    let n_batch = i32::try_from(ctx.n_batch()).unwrap_or(i32::MAX);

    // Load the projector. PR-2: per-dispatch (measured via `mmproj_loads`);
    // PR-2.5 caches it on this owner thread.
    let mtmd = Mtmd::from_file(model, mmproj, 0, true).map_err(map_llama_err)?;
    mmproj_loads.fetch_add(1, Ordering::Relaxed);
    // Defense-in-depth beyond the descriptor's declared modality: the loaded
    // projector itself must accept images.
    if !mtmd.supports_vision() {
        return Err(InferenceError::Unsupported {
            reason: "loaded projector does not support vision (mmproj/model mismatch)",
        });
    }
    check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;

    // Decode each image (untrusted bytes → stb). A decode failure is a typed
    // error, never a panic.
    let bitmaps: Vec<Bitmap> = job
        .images
        .iter()
        .map(|bytes| Bitmap::from_image_buf(&mtmd, bytes))
        .collect::<Result<_, _>>()
        .map_err(map_llama_err)?;
    let bitmap_refs: Vec<&Bitmap> = bitmaps.iter().collect();
    check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;

    // Reconcile our contract marker with the projector's actual marker (handles
    // a future pin bump that changes `mtmd_default_marker()`).
    let real_marker = Mtmd::default_marker();
    let text: Cow<'_, str> = if real_marker == MEDIA_MARKER {
        Cow::Borrowed(&job.prompt)
    } else {
        Cow::Owned(job.prompt.replace(MEDIA_MARKER, real_marker))
    };

    let chunks = mtmd.tokenize(&text, &bitmap_refs).map_err(map_llama_err)?;
    check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;

    // Multi-modal prefill: text token batches + image embedding batches, with
    // `logits_last` so the first sample reads the final position's logits.
    let n_past = mtmd
        .eval_chunks(&mut ctx, &chunks, 0, 0, n_batch, true)
        .map_err(map_llama_err)?;
    check_timeout(start.elapsed(), timeout, job.wall_clock_ms)?;

    let vocab = model.vocab();
    let mut sampler = build_sampler(backend, &job.params)?;
    let generator = Generator::from_prefilled(&mut ctx, &mut sampler, &vocab, n_past);
    run_generation(generator, &vocab, job, start, timeout)
}

/// Timeout guard, identical in semantics to the pre-cache path.
fn check_timeout(
    elapsed: Duration,
    timeout: Duration,
    wall_clock_ms: u64,
) -> Result<(), InferenceError> {
    if elapsed >= timeout {
        Err(InferenceError::Timeout { wall_clock_ms })
    } else {
        Ok(())
    }
}
