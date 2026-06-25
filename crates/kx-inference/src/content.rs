// Object-safe content fetch seam, shared by every multi-modal backend.
//
// Lives in its OWN ungated module (not behind `llamacpp`, not in `kx-content`)
// because BOTH the in-process llama.cpp backend (`llama.rs`, `llamacpp` feature)
// AND the FFI-free Ollama backend (`kx-ollama`, `serve-engine` feature) need to
// fetch a `content_ref`'s bytes for a `Multimodal` dispatch. Keeping it ungated
// here — deps are only `kx_content` (an FFI-free leaf) — is what lets the Ollama
// vision path build under `--features serve-engine` WITHOUT `inference`/llamacpp.
//
// The `ContentStore` trait itself stays generic (associated `Payload` type) for
// its hot-path callers (assembler, executor) that use `&S`; this trait object
// exists solely so a backend can hold a single `Arc<dyn ContentFetcher>`
// regardless of the concrete store.

use kx_content::{ContentRef, ContentStore, NotFound};

/// Object-safe byte fetcher that erases [`ContentStore`]'s associated `Payload`
/// type so a backend can hold a single trait object regardless of the store
/// implementation. Blanket-implemented for every `Send + Sync` `ContentStore`,
/// so callers pass an `Arc<ConcreteStore>` and it coerces to
/// `Arc<dyn ContentFetcher>` directly.
pub trait ContentFetcher: Send + Sync {
    /// Fetch the bytes at `r`, or `None` if the store has no such object.
    fn fetch(&self, r: &ContentRef) -> Option<Vec<u8>>;
}

impl<S> ContentFetcher for S
where
    S: ContentStore + Send + Sync + ?Sized,
{
    fn fetch(&self, r: &ContentRef) -> Option<Vec<u8>> {
        match self.get(r) {
            Ok(payload) => Some(payload.to_vec()),
            Err(NotFound) => None,
        }
    }
}
