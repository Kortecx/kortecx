//! Stress tests — load / decode / drop cycles in tight loops to surface
//! resource leaks and Drop ordering bugs (SN-4 v2 #7 extension).
//!
//! These tests don't assert specific memory numbers (that requires
//! `dhat` / `valgrind` infrastructure deferred to P1.13). They DO
//! exercise the load-drop cycle at a frequency the per-feature tests
//! don't, catching:
//!
//!  - A missing `Drop` impl that doesn't release a llama.cpp resource.
//!  - A static / OnceLock leak in our wrapper that retains references.
//!  - An infinite-loop / hang in the load path.
//!
//! If the test completes in reasonable time without panic, OS-level
//! pressure / OOM, or wedging, the wrapper is leak-clean enough for the
//! current scale. P1.13 will add proper RSS-tracked envelope tests.

#![cfg(feature = "model-smoke-test")]

use kx_llamacpp::{Context, ContextParams, LlamaBackend, Model};

const MODEL_PATH: &str = env!("KX_LLAMACPP_SMOKE_TEST_MODEL");

/// 50× load-and-drop a Model from the same file. If Drop is broken, OS
/// file handles or mmap regions accumulate and the test eventually
/// fails on the load call. We bound the test at 50 iterations: enough
/// to surface a typical leak (file descriptor table fills up around
/// 256–1024 fds by default), small enough to run in seconds.
#[test]
fn stress_50_model_load_drop_cycles() {
    let backend = LlamaBackend::new().expect("backend init");
    for i in 0..50 {
        let model = Model::load(&backend, MODEL_PATH)
            .unwrap_or_else(|e| panic!("iteration {i} load failed: {e} — likely a Drop leak"));
        // Touch the vocab to force any lazy init.
        let _ = model.vocab().n_tokens();
        drop(model);
    }
}

/// 20× load-Model + create-Context + decode-empty + drop. The Context
/// allocates KV cache buffers; if those aren't released on Drop, this
/// surfaces faster than the model-only cycle.
#[test]
fn stress_20_load_decode_drop_cycles() {
    let backend = LlamaBackend::new().expect("backend init");
    for i in 0..20 {
        let model = Model::load(&backend, MODEL_PATH).expect("load");
        let vocab = model.vocab();
        let tokens = vocab.tokenize("hello", true, false).expect("tokenize");
        let mut ctx = Context::new_with_params(
            &model,
            &ContextParams::new().with_n_ctx(64).with_n_seq_max(1),
        )
        .unwrap_or_else(|e| panic!("iteration {i} ctx create failed: {e}"));
        let mut batch = kx_llamacpp::Batch::with_capacity(tokens.len() as i32, 1);
        batch.add_many(&tokens, 0, 0);
        ctx.decode(&batch).expect("decode");
        drop(ctx);
        drop(model);
    }
}

/// 100× LlamaBackend init/free cycle. The internal ref-counted mutex
/// should make each cycle ~constant-time; a leak in the counter or the
/// mutex would surface as growth.
#[test]
fn stress_100_backend_init_drop_cycles() {
    for i in 0..100 {
        let backend = LlamaBackend::new()
            .unwrap_or_else(|e| panic!("iteration {i} backend init failed: {e}"));
        drop(backend);
    }
}
