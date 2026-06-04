//! [`GatewayService`] — the [`KxGateway`] tonic implementation. Read RPCs fold
//! through the read-only seam; `SubmitRun` proxies through the [`RunSubmitter`];
//! the M7 signature RPCs are stubbed (`unimplemented`) at the D120 freeze.

use std::pin::Pin;
use std::sync::Arc;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_server::KxGateway;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::error::{hash_32, instance_id_16};
use crate::reader::{ContentReader, JournalReader};
use crate::submit::{RunSubmitter, SubmitterError};
use crate::{events, view};

/// The backend behind the external `KxGateway` service: a read-only journal +
/// content reader (the read-fold) and a [`RunSubmitter`] (the propose-proxy).
/// Holds no writer; auth/ownership stay cloud-side (the host wraps this with
/// middleware). Construct with [`GatewayService::new`].
#[derive(Clone)]
pub struct GatewayService {
    reader: Arc<dyn JournalReader>,
    submitter: Arc<dyn RunSubmitter>,
    content: Arc<dyn ContentReader>,
}

impl GatewayService {
    /// Wire a gateway over a read-only journal seam, a propose-proxy, and a
    /// read-only content seam.
    pub fn new(
        reader: Arc<dyn JournalReader>,
        submitter: Arc<dyn RunSubmitter>,
        content: Arc<dyn ContentReader>,
    ) -> Self {
        Self {
            reader,
            submitter,
            content,
        }
    }
}

fn submit_status(err: SubmitterError) -> Status {
    match err {
        SubmitterError::Rejected(detail) => Status::failed_precondition(detail),
        SubmitterError::Transport(detail) => Status::unavailable(detail),
    }
}

#[tonic::async_trait]
impl KxGateway for GatewayService {
    async fn submit_run(
        &self,
        request: Request<proto::SubmitRunRequest>,
    ) -> Result<Response<proto::RunHandle>, Status> {
        let req = request.into_inner();
        let recipe_fp = hash_32(
            &req.recipe_fingerprint,
            "recipe_fingerprint must be 32 bytes",
        )?;

        // Register first: returns only after the journaled instance_id (never
        // acks ahead of the journal).
        let instance_id = self
            .submitter
            .register_run(recipe_fp)
            .await
            .map_err(submit_status)?;

        for spec in req.motes {
            let mote_proto = spec
                .mote
                .ok_or_else(|| Status::invalid_argument("SubmitMoteSpec.mote is required"))?;
            // IDENTITY INVARIANT: TryFrom re-derives the MoteId Rust-side; the
            // wire mote_id is advisory only (D53).
            let mote: kx_mote::Mote = mote_proto
                .try_into()
                .map_err(|e: kx_proto::ConvertError| Status::invalid_argument(e.to_string()))?;
            let warrant_proto = spec
                .warrant
                .ok_or_else(|| Status::invalid_argument("SubmitMoteSpec.warrant is required"))?;
            let warrant: kx_warrant::WarrantSpec = warrant_proto
                .try_into()
                .map_err(|e: kx_proto::ConvertError| Status::invalid_argument(e.to_string()))?;
            self.submitter
                .submit_mote(mote, warrant, spec.accept_at_least_once)
                .await
                .map_err(submit_status)?;
        }

        Ok(Response::new(proto::RunHandle {
            instance_id: instance_id.to_vec(),
            recipe_fingerprint: recipe_fp.to_vec(),
        }))
    }

    async fn get_projection(
        &self,
        request: Request<proto::GetProjectionRequest>,
    ) -> Result<Response<proto::ProjectionView>, Status> {
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let view = view::build_view(self.reader.as_ref(), instance_id, req.at_seq)?;
        Ok(Response::new(view))
    }

    async fn get_content(
        &self,
        request: Request<proto::GetContentRequest>,
    ) -> Result<Response<proto::ContentBlob>, Status> {
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let content_ref = hash_32(&req.content_ref, "content_ref must be 32 bytes")?;
        let payload = view::get_owned_content(
            self.reader.as_ref(),
            self.content.as_ref(),
            instance_id,
            content_ref,
        )?;
        Ok(Response::new(proto::ContentBlob { payload }))
    }

    type StreamEventsStream =
        Pin<Box<dyn Stream<Item = Result<proto::EventFrame, Status>> + Send + 'static>>;

    async fn stream_events(
        &self,
        request: Request<proto::StreamEventsRequest>,
    ) -> Result<Response<Self::StreamEventsStream>, Status> {
        let req = request.into_inner();
        let instance_id = instance_id_16(&req.instance_id)?;
        let frames = events::build_frames(self.reader.as_ref(), instance_id, req.since_seq)?;
        let stream = tokio_stream::iter(frames.into_iter().map(Ok));
        Ok(Response::new(Box::pin(stream)))
    }

    async fn list_signatures(
        &self,
        _request: Request<proto::ListSignaturesRequest>,
    ) -> Result<Response<proto::ListSignaturesResponse>, Status> {
        Err(Status::unimplemented(
            "ListSignatures: stubbed at the D120 freeze (M7 catalog read path is M8)",
        ))
    }

    async fn get_signature(
        &self,
        _request: Request<proto::GetSignatureRequest>,
    ) -> Result<Response<proto::GetSignatureResponse>, Status> {
        Err(Status::unimplemented(
            "GetSignature: stubbed at the D120 freeze (M7 catalog read path is M8)",
        ))
    }

    async fn register_signature(
        &self,
        _request: Request<proto::RegisterSignatureRequest>,
    ) -> Result<Response<proto::RegisterSignatureResponse>, Status> {
        Err(Status::unimplemented(
            "RegisterSignature: stubbed at the D120 freeze (M7 catalog write path is M8)",
        ))
    }
}
