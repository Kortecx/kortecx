//! D213 Experience lane — the hosted-app supervisor SEAM.
//!
//! A hosted (Experience) App is a real web project. This seam lets the gateway service
//! start / stop / observe a per-app dev server WITHOUT gateway-core owning any process
//! runtime: the host impl (kx-gateway `hostsupervisor`, behind the `hosted-apps` feature)
//! materializes the app's branch file tree to disk, `npm install`s, spawns the dev server
//! on a loopback port, and supervises it. A `None` seam ⇒ the four hosted RPCs return
//! `unimplemented` — the standard optional-seam degrade. Off-journal / off-digest: the
//! supervisor is a plain host subprocess, never a Mote (D213 — the frozen trio is untouched).

/// A hosted app's dev-server lifecycle state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostedState {
    /// Not running (never started, or explicitly stopped).
    #[default]
    Stopped,
    /// Writing the branch file tree to the working directory.
    Materializing,
    /// `npm install` (first run / `package.json` changed).
    Installing,
    /// The dev server was spawned but is not yet accepting connections.
    Starting,
    /// The dev server is accepting connections; `url` is live.
    Running,
    /// Install / start failed (the `detail` carries the advisory reason).
    Failed,
}

/// A hosted app's status snapshot (state + the live loopback URL + recent logs).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostedStatus {
    /// The saved App handle.
    pub handle: String,
    /// The lifecycle state.
    pub state: HostedState,
    /// The loopback URL when running (`http://127.0.0.1:<port>/`); empty otherwise.
    pub url: String,
    /// A tail of the install / dev-server logs (advisory).
    pub recent_logs: Vec<String>,
    /// The framework label (`"vite_react"` / `"next_js"`).
    pub framework: String,
    /// The loopback dev-server port (0 when not running).
    pub port: u32,
    /// Advisory status / failure text (never authority).
    pub detail: String,
}

/// The host-side hosted-app supervisor seam. The host impl owns the process runtime
/// (materialize → install → spawn+supervise the dev server → kill/reap). `start` returns
/// immediately with the current status (the lifecycle runs in a background task); poll
/// [`HostedAppSupervisor::status`] for progress — the [`crate::AppScaffolder`]
/// propose-proxy contract.
pub trait HostedAppSupervisor: Send + Sync {
    /// Start (or attach to) the hosted app `(principal, handle)`'s dev server. Idempotent:
    /// a running app returns its current status. `rebuild` forces re-materialize + re-install.
    ///
    /// # Errors
    /// [`crate::error::GatewayError::NotFound`] when the app is unknown to the caller;
    /// [`crate::error::GatewayError::InvalidArgument`] when it is not a hosted app;
    /// [`crate::error::GatewayError::Internal`] on a host failure.
    fn start(
        &self,
        principal: &str,
        handle: &str,
        rebuild: bool,
    ) -> Result<HostedStatus, crate::error::GatewayError>;

    /// Stop the hosted app's dev server (kills + reaps the child). Returns `true` iff a
    /// running app was stopped.
    ///
    /// # Errors
    /// [`crate::error::GatewayError::Internal`] on a host failure.
    fn stop(&self, principal: &str, handle: &str) -> Result<bool, crate::error::GatewayError>;

    /// The current status of the hosted app `(principal, handle)` (Stopped when untracked).
    ///
    /// # Errors
    /// [`crate::error::GatewayError::Internal`] on a host failure.
    fn status(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<HostedStatus, crate::error::GatewayError>;

    /// Every hosted app the supervisor is currently tracking for `principal`.
    ///
    /// # Errors
    /// [`crate::error::GatewayError::Internal`] on a host failure.
    fn list(&self, principal: &str) -> Result<Vec<HostedStatus>, crate::error::GatewayError>;
}
