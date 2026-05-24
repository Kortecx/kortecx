//! HF-shaped chat recipe — apply the model's chat template, generate a reply.
//!
//! Usage:
//!   cargo run -p kx-llamacpp --example chat -- /path/to/chat-model.gguf

use kx_llamacpp::{ChatMessage, Context, ContextParams, Generator, LlamaBackend, Model, Sampler};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let model_path = std::env::args().nth(1).expect("usage: chat <model.gguf>");
    let backend = LlamaBackend::new()?;
    let model = Model::load(&backend, &model_path)?;
    let prompt = model.apply_chat_template(
        None,
        &[
            ChatMessage::system("You are concise."),
            ChatMessage::user("What is the capital of France?"),
        ],
        true,
    )?;
    let vocab = model.vocab();
    let mut ctx = Context::new_with_params(
        &model,
        &ContextParams::new().with_n_ctx(1024).with_n_seq_max(1),
    )?;
    let mut sampler = Sampler::typical(&backend, 0.7, 40, 0.95, 42)?;
    let gen = Generator::new(
        &mut ctx,
        &mut sampler,
        &vocab,
        vocab.tokenize(&prompt, true, true)?,
    )?;
    for token in gen.take(128) {
        print!("{}", String::from_utf8_lossy(&token?.to_piece(&vocab)?));
    }
    println!();
    Ok(())
}
