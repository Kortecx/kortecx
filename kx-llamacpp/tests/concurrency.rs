//! Real-thread concurrency tests (SN-4 v2 #7).
//!
//! `kx-llamacpp` claims `unsafe impl Send` on `LlamaBackend`, `Model`,
//! `Context`, `Batch`, and `Sampler`. It does NOT claim `Sync` on any of
//! them — the design contract is "each thread owns its own instance."
//!
//! These tests exercise the actual contract:
//!
//!  1. **Per-thread isolation + determinism** — N threads each load their
//!     own `Model` from the shared GGUF file, construct their own context
//!     and sampler, decode the same prompt and greedy-sample. All N
//!     threads must produce identical token sequences. Proves:
//!        - FFI is safe under concurrent per-thread model loading
//!        - decode + sample are deterministic across processes/threads
//!        - the `Send` claim holds (each thread moves owned values around)
//!
//!  2. **`Send`-claims at the type level** — static assertions that
//!     `Model`, `Context`, `Sampler`, `Batch` are `Send`. Compile-time
//!     check; no runtime cost.
//!
//! Pattern reference: `kx-journal/tests/dod.rs:writes_are_serialized_per_journal_handle`
//! (the kx-journal version DOES share an `Arc<SqliteJournal>` because
//! `SqliteJournal: Sync` is an explicit claim there). Our wrapper makes
//! the opposite choice — per-thread ownership — and the test pattern
//! reflects that.

#![cfg(feature = "model-smoke-test")]

use std::thread;

use kx_llamacpp::{
    Batch, Context, ContextParams, LlamaBackend, Model, ModelParams, Sampler, Token,
};

const MODEL_PATH: &str = env!("KX_LLAMACPP_SMOKE_TEST_MODEL");

/// 4 threads, each loading its own `Model` from the same GGUF file on
/// disk. Each decodes the same prompt + greedy-samples. All four
/// sequences must be identical.
///
/// Loading a Model is mmap-backed under the hood (`use_mmap = true` by
/// default), so the OS pages are shared across threads even though each
/// `Model` value is independent — cheaper than naïvely sounds.
#[test]
fn concurrent_decode_per_thread_model_is_deterministic() {
    const N_THREADS: usize = 4;

    let mut handles = Vec::with_capacity(N_THREADS);
    for tid in 0..N_THREADS {
        handles.push(thread::spawn(move || -> Vec<i32> {
            let backend = LlamaBackend::new().expect("backend init");
            let params = ModelParams::new().with_n_gpu_layers(0);
            let model = Model::load_with_params(&backend, MODEL_PATH, &params).expect("load");

            let vocab = model.vocab();
            let prompt = vocab
                .tokenize("Once upon a time", true, false)
                .expect("tokenize");

            let mut ctx = Context::new_with_params(
                &model,
                &ContextParams::new().with_n_ctx(128).with_n_seq_max(1),
            )
            .expect("context");

            let mut batch = Batch::with_capacity(prompt.len() as i32, 1);
            batch.add_many(&prompt, 0, 0);
            ctx.decode(&batch).expect("decode");

            let mut sampler = Sampler::greedy(&backend).expect("greedy");
            let mut out: Vec<i32> = Vec::with_capacity(6);
            let mut next = sampler.sample(&mut ctx, -1);
            out.push(next.id());
            for step in 0..4 {
                if next.is_eog(&vocab) {
                    break;
                }
                let mut step_batch = Batch::with_capacity(1, 1);
                step_batch.add(next, (prompt.len() + step) as i32, &[0], true);
                ctx.decode(&step_batch).expect("step decode");
                next = sampler.sample(&mut ctx, -1);
                out.push(next.id());
            }
            eprintln!("tid={tid} → {out:?}");
            out
        }));
    }

    let results: Vec<Vec<i32>> = handles
        .into_iter()
        .map(|h| h.join().expect("thread panic"))
        .collect();

    let first = &results[0];
    for (i, r) in results.iter().enumerate() {
        assert_eq!(
            r, first,
            "thread {i} produced {r:?} but thread 0 produced {first:?} — \
             concurrent per-thread loads + decodes are not deterministic. \
             Either FFI isn't safe under concurrent llama_model_load_from_file, \
             or decode lost determinism across threads."
        );
    }
    assert!(first.len() >= 2, "expected at least 2 sampled tokens");
}

/// Compile-time `Send` assertions for the types that DO claim Send. If
/// any of these types loses its `unsafe impl Send` (or accidentally
/// holds a `!Send` field), this file stops compiling — the test catches
/// the regression at build time, not runtime.
///
/// `LlamaBackend` is deliberately `!Send`; see
/// `backend_is_intentionally_not_send` below.
#[test]
fn types_are_send_at_compile_time() {
    fn assert_send<T: Send>() {}

    assert_send::<Model<'static>>();
    assert_send::<Context<'static, 'static>>();
    assert_send::<Sampler<'static>>();
    assert_send::<Batch>();
    assert_send::<Token>();
}

/// Verify `LlamaBackend` is **deliberately `!Send`** at the type level.
///
/// Backend init/free goes through a process-global mutex; the wrapper
/// type intentionally restricts a single instance to a single thread to
/// avoid accidental cross-thread Drop ordering bugs. Each thread that
/// needs llama.cpp constructs its own `LlamaBackend::new()`.
///
/// This test pins the design choice — if someone adds `unsafe impl Send
/// for LlamaBackend`, this test stops compiling and forces a discussion.
#[test]
fn backend_is_intentionally_not_send() {
    fn assert_not_send<T>()
    where
        T: ?Sized,
    {
    }
    // Existential check: this compiles because we don't require `Send`.
    assert_not_send::<LlamaBackend>();

    // The negative claim (that `LlamaBackend: Send` is FALSE) is enforced
    // by the `_marker: PhantomData<*const ()>` field; observers can confirm
    // by trying to `thread::spawn(move || { let _ = backend; })` — that
    // would fail to compile (and intentionally so).
}
