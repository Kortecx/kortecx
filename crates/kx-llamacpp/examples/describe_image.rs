// Example binary: compiled as a separate crate; carries its own allow for
// ergonomic .unwrap()/.expect() in demo code. Production library code is held
// to the workspace deny policy.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::doc_markdown)]

//! Multi-modal "describe this image" recipe — the smallest one-shot
//! image→text generation in kortecx (PR-2).
//!
//! Loads a vision model + its `mmproj` projector, decodes an image file, runs
//! the mtmd prefill, and greedily generates a description.
//!
//! Usage:
//!   cargo run -p kx-llamacpp --example describe_image -- \
//!     /path/to/model.gguf /path/to/mmproj.gguf /path/to/image.png \
//!     "What is in this image?"
//!
//! Get a small VLM the same way CI does, e.g. ggml-org/Qwen2-VL-2B-Instruct-GGUF
//! (`Qwen2-VL-2B-Instruct-Q4_K_M.gguf` + `mmproj-Qwen2-VL-2B-Instruct-Q8_0.gguf`).

use kx_llamacpp::{Bitmap, Context, ContextParams, Generator, LlamaBackend, Model, Mtmd, Sampler};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let usage = "usage: describe_image <model.gguf> <mmproj.gguf> <image> [question]";
    let model_path = args.next().expect(usage);
    let mmproj_path = args.next().expect(usage);
    let image_path = args.next().expect(usage);
    let question = args
        .next()
        .unwrap_or_else(|| "Describe this image.".to_string());

    let backend = LlamaBackend::new()?;
    let model = Model::load(&backend, &model_path)?;

    // Load the projector and decode the image bytes into a bitmap.
    let mtmd = Mtmd::from_file(&model, &mmproj_path, 0, true)?;
    let image_bytes = std::fs::read(&image_path)?;
    let bitmap = Bitmap::from_image_buf(&mtmd, &image_bytes)?;

    // One media marker per image, inside a ChatML user turn (Qwen2-VL style).
    let marker = Mtmd::default_marker();
    let prompt = format!("<|im_start|>user\n{marker}{question}<|im_end|>\n<|im_start|>assistant\n");
    let chunks = mtmd.tokenize(&prompt, &[&bitmap])?;

    // A larger batch so a high-token image does not overflow the decode.
    let mut ctx = Context::new_with_params(
        &model,
        &ContextParams::new()
            .with_n_ctx(4096)
            .with_n_batch(2048)
            .with_n_ubatch(512)
            .with_n_seq_max(1),
    )?;

    // Multi-modal prefill (text + image), then continue with the ordinary loop.
    let n_batch = i32::try_from(ctx.n_batch()).unwrap_or(i32::MAX);
    let n_past = mtmd.eval_chunks(&mut ctx, &chunks, 0, 0, n_batch, true)?;

    let vocab = model.vocab();
    let mut sampler = Sampler::greedy(&backend)?;
    let gen = Generator::from_prefilled(&mut ctx, &mut sampler, &vocab, n_past);
    for token in gen.take(128) {
        let token = token?;
        if token.is_eog(&vocab) {
            break;
        }
        let piece = token.to_piece(&vocab)?;
        print!("{}", String::from_utf8_lossy(&piece));
    }
    println!();
    Ok(())
}
