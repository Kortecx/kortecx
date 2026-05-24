//! HF-shaped embedding recipe — one-shot `model.embed(text)`.
//!
//! Usage:
//!   cargo run -p kx-llamacpp --example embed -- /path/to/model.gguf "text to embed"

use kx_llamacpp::{LlamaBackend, Model};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let model_path = args.next().expect("usage: embed <model.gguf> <text>");
    let text = args.next().expect("usage: embed <model.gguf> <text>");

    let backend = LlamaBackend::new()?;
    let model = Model::load(&backend, &model_path)?;
    let vector = model.embed(&text)?;
    let l2 = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    println!("{}-dim embedding (L2={:.4})", vector.len(), l2);
    Ok(())
}
