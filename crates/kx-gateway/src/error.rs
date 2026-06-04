//! [`GatewayError`] — the binary's typed failure surface. Thin: CLI / bind /
//! seam-construction / transport errors, each carrying an owned message so the
//! type stays dependency-light (no `#[from]` on a foreign error keeps it simple
//! for `main` to render).

/// A failure starting or running the gateway server.
#[derive(Debug, thiserror::Error)]
pub enum GatewayError {
    /// Bad CLI arguments / configuration.
    #[error("config: {0}")]
    Config(String),
    /// Could not bind / serve on the requested listen address.
    #[error("bind: {0}")]
    Bind(String),
    /// Opening the journal (writer or the gateway's read handle) failed.
    #[error("journal: {0}")]
    Journal(String),
    /// Opening the content store failed.
    #[error("content store: {0}")]
    Content(String),
    /// Connecting / talking to the embedded coordinator failed.
    #[error("coordinator: {0}")]
    Coordinator(String),
    /// A required capability is missing from this build (e.g. the embedded
    /// worker was compiled out with `--no-default-features`).
    #[error("unsupported: {0}")]
    Unsupported(String),
}
