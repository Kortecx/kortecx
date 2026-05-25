// Example binary: compiled as a separate crate; carries its own allow for
// ergonomic .unwrap()/.expect() in demo code. Production library code is
// held to the workspace deny policy.
#![allow(clippy::unwrap_used, clippy::expect_used)]

//! HF-shaped generate recipe — the smallest one-shot generation in kortecx.
//!
//! Usage:
//!   cargo run -p kx-llamacpp --example generate -- /path/to/model.gguf "Once upon a time"

use kx_llamacpp::{Context, ContextParams, Generator, LlamaBackend, Model, Sampler};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().expect("usage: generate <model.gguf> <prompt>");
    let prompt = args.next().expect("usage: generate <model.gguf> <prompt>");

    let backend = LlamaBackend::new()?;
    let model = Model::load(&backend, &model_path)?;
    let vocab = model.vocab();
    let mut ctx = Context::new_with_params(
        &model,
        &ContextParams::new().with_n_ctx(512).with_n_seq_max(1),
    )?;
    let mut sampler = Sampler::greedy(&backend)?;
    let prompt_tokens = vocab.tokenize(&prompt, true, false)?;
    let gen = Generator::new(&mut ctx, &mut sampler, &vocab, prompt_tokens)?;
    for token in gen.take(64) {
        let token = token?;
        let piece = token.to_piece(&vocab)?;
        print!("{}", String::from_utf8_lossy(&piece));
    }
    println!();
    Ok(())
}
