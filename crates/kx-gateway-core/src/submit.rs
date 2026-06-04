//! The propose-proxy seam. `SubmitRun` does not write the journal directly — it
//! delegates to a [`RunSubmitter`] which the host wires to the coordinator (the
//! sole journal writer, D40). Keeping this a seam is what lets gateway-core stay
//! off `kx-coordinator` (the dep wall) while still proxying submits.

use kx_proto::proto;
use kx_proto::proto::coordinator_client::CoordinatorClient;
use tonic::transport::Channel;

/// The outcome of admitting one Mote under a registered run.
pub struct SubmitMoteOutcome {
    /// The coordinator-derived Mote identity (re-derived Rust-side, D53).
    pub mote_id: [u8; 32],
    /// The registered run this Mote was admitted under (M1.2 / D64).
    pub instance_id: [u8; 16],
    /// Whether the Mote was newly accepted, an idempotent duplicate, or rejected.
    pub status: SubmitStatus,
}

/// Coordinator submit status (mirrors `proto::SubmitStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitStatus {
    /// Newly accepted.
    Accepted,
    /// Already known (idempotent resubmit).
    Duplicate,
    /// A refusal predicate fired.
    Rejected,
}

/// A failure proxying a submit to the coordinator.
#[derive(Debug, thiserror::Error)]
pub enum SubmitterError {
    /// The coordinator refused the submission (a refusal predicate fired, or the
    /// run is not registered). Carries the coordinator's detail.
    #[error("submission rejected: {0}")]
    Rejected(String),
    /// The submit could not reach / be served by the coordinator.
    #[error("coordinator unavailable: {0}")]
    Transport(String),
}

/// The propose-proxy seam: register a run, then submit each Mote. Wired by the
/// host to a [`TonicCoordinatorSubmitter`] (gRPC) or an in-process coordinator.
#[tonic::async_trait]
pub trait RunSubmitter: Send + Sync {
    /// Register a run for `recipe_fingerprint`; returns the coordinator-assigned,
    /// journaled `instance_id`. Resolves only after the `RunRegistered` entry is
    /// durable (the never-ack-ahead-of-the-journal guarantee). Idempotent.
    async fn register_run(&self, recipe_fingerprint: [u8; 32]) -> Result<[u8; 16], SubmitterError>;

    /// Submit one Mote under the (already-registered) run. The coordinator
    /// re-derives identity; the returned `mote_id` is authoritative (D53).
    async fn submit_mote(
        &self,
        mote: kx_mote::Mote,
        warrant: kx_warrant::WarrantSpec,
        accept_at_least_once: bool,
    ) -> Result<SubmitMoteOutcome, SubmitterError>;
}

/// A [`RunSubmitter`] backed by the generated gRPC `CoordinatorClient`. The
/// channel is cloned per call (tonic clients are cheap to clone and share the
/// connection), so the trait stays `&self`.
pub struct TonicCoordinatorSubmitter {
    client: CoordinatorClient<Channel>,
}

impl TonicCoordinatorSubmitter {
    /// Wrap a connected coordinator client.
    pub fn new(client: CoordinatorClient<Channel>) -> Self {
        Self { client }
    }

    /// Connect to a coordinator at `endpoint` (`http://host:port`).
    pub async fn connect(endpoint: impl Into<String>) -> Result<Self, SubmitterError> {
        let client = CoordinatorClient::connect(endpoint.into())
            .await
            .map_err(|e| SubmitterError::Transport(e.to_string()))?;
        Ok(Self { client })
    }
}

#[tonic::async_trait]
impl RunSubmitter for TonicCoordinatorSubmitter {
    async fn register_run(&self, recipe_fingerprint: [u8; 32]) -> Result<[u8; 16], SubmitterError> {
        let mut client = self.client.clone();
        let resp = client
            .register_run(proto::RegisterRunRequest {
                recipe_fingerprint: recipe_fingerprint.to_vec(),
            })
            .await
            .map_err(|s| SubmitterError::Transport(s.to_string()))?
            .into_inner();
        resp.instance_id.try_into().map_err(|_| {
            SubmitterError::Transport("coordinator returned a non-16-byte instance_id".into())
        })
    }

    async fn submit_mote(
        &self,
        mote: kx_mote::Mote,
        warrant: kx_warrant::WarrantSpec,
        accept_at_least_once: bool,
    ) -> Result<SubmitMoteOutcome, SubmitterError> {
        let mut client = self.client.clone();
        let resp = client
            .submit_mote(proto::SubmitMoteRequest {
                mote: Some(mote.into()),
                warrant: Some(warrant.into()),
                accept_at_least_once,
            })
            .await
            .map_err(|s| SubmitterError::Transport(s.to_string()))?
            .into_inner();

        let status = match proto::SubmitStatus::try_from(resp.status)
            .unwrap_or(proto::SubmitStatus::Unspecified)
        {
            proto::SubmitStatus::Accepted => SubmitStatus::Accepted,
            proto::SubmitStatus::Duplicate => SubmitStatus::Duplicate,
            proto::SubmitStatus::Rejected | proto::SubmitStatus::Unspecified => {
                SubmitStatus::Rejected
            }
        };
        if status == SubmitStatus::Rejected {
            return Err(SubmitterError::Rejected(resp.detail));
        }
        let mote_id = resp
            .mote_id
            .try_into()
            .map_err(|_| SubmitterError::Transport("non-32-byte mote_id".into()))?;
        let instance_id = resp
            .instance_id
            .try_into()
            .map_err(|_| SubmitterError::Transport("non-16-byte instance_id".into()))?;
        Ok(SubmitMoteOutcome {
            mote_id,
            instance_id,
            status,
        })
    }
}
