//! [`GatewayService`] — the [`KxGateway`] tonic implementation. Read RPCs fold
//! through the read-only seam; `SubmitRun` and `Invoke` proxy through the
//! [`RunSubmitter`]; the signature RPCs and `Invoke` dispatch to the optional
//! [`SignatureCatalog`] / [`RecipeBinder`] seams the host injects (each returns
//! `unimplemented` when its seam is absent — backward-compatible).

use std::pin::Pin;
use std::sync::Arc;

use kx_proto::proto;
use kx_proto::proto::kx_gateway_server::KxGateway;
use tokio_stream::Stream;
use tonic::{Request, Response, Status};

use crate::error::{hash_32, instance_id_16};
use crate::identity::CallerParty;
use crate::reader::{ContentReader, JournalReader};
use crate::submit::{RunSubmitter, SubmitterError};
use crate::{events, view};

/// The id a `RegisterSignature` server-derived from the manifest bytes (SN-8:
/// the client never supplies the id; the host derives it from the decoded entry).
#[derive(Clone, Copy, Debug)]
pub struct RegisteredSignature {
    /// The 32-byte content-addressed signature id.
    pub signature_id: [u8; 32],
}

/// One entry in a `ListSignatures` enumeration: the content-addressed id plus a
/// host-derived human label.
#[derive(Clone, Debug)]
pub struct SignatureSummaryEntry {
    /// The 32-byte content-addressed signature id.
    pub signature_id: [u8; 32],
    /// A short, stable, human-distinguishable label (the catalog stores no name
    /// of its own; a richer name belongs in advisory metadata later).
    pub name: String,
}

/// A failure from the [`SignatureCatalog`] seam.
///
/// The catalog is a PUBLIC discovery surface (authoritative for *what recipes
/// exist*), so — unlike the `Invoke` execution surface, which collapses to a
/// uniform "not authorized" with no existence oracle — these stay honest,
/// distinct codes.
#[derive(Debug)]
pub enum CatalogSeamError {
    /// A DIFFERENT entry already exists at this content-addressed id.
    ImmutabilityConflict,
    /// The `manifest` bytes could not be decoded into a signature entry.
    Malformed(String),
    /// A backend storage failure (durable-backend I/O, a corrupt row).
    Internal(String),
}

/// The signature-catalog seam (the M7 catalog RPCs frozen at D120).
///
/// Spoken in the gateway's WIRE vocabulary — opaque `manifest` bytes + a 32-byte
/// server-derived id — so gateway-core stays off `kx-catalog` (the
/// dependency wall). The host implements it over a `kx_catalog::CatalogRegistry`,
/// decoding/encoding with the catalog's canonical codec and server-deriving the
/// id from the decoded entry. A `None` seam on the service means the host wired
/// no catalog, so the three signature RPCs return `unimplemented`.
pub trait SignatureCatalog: Send + Sync {
    /// Decode `manifest`, server-derive its id, register it (idempotent +
    /// immutable), and return the id.
    ///
    /// # Errors
    /// [`CatalogSeamError`] on a malformed manifest, an immutability conflict, or
    /// a storage failure.
    fn register(&self, manifest: &[u8]) -> Result<RegisteredSignature, CatalogSeamError>;
    /// The encoded manifest for `signature_id`, or `None` if absent.
    fn get(&self, signature_id: &[u8; 32]) -> Option<Vec<u8>>;
    /// Every registered signature as an `(id, name)` summary, in deterministic
    /// (hash) order.
    fn list(&self) -> Vec<SignatureSummaryEntry>;
}

/// A recipe resolved + bound to concrete args, ready to submit. Mirrors
/// `kx_invoke::BoundRun`, but in gateway-core's own vocabulary (`kx_mote` +
/// `kx_warrant` types it already depends on) so the binding seam stays off
/// kx-invoke / kx-catalog (the dependency wall).
pub struct BoundRecipe {
    /// The recipe identity → the `recipe_fingerprint` passed to `RegisterRun`.
    pub recipe_fingerprint: [u8; 32],
    /// The runnable Motes in submission order, each paired with its narrowed
    /// warrant (⊆ the caller's Use authority AND the recipe's step warrant).
    pub motes: Vec<(kx_mote::Mote, kx_warrant::WarrantSpec)>,
    /// The terminal (sink) Mote whose committed result is the invocation output.
    pub terminal_mote_id: kx_mote::MoteId,
}

/// A bind failure the host's [`RecipeBinder`] surfaces. The gateway collapses
/// [`BinderError::NotAuthorized`] to a UNIFORM `permission_denied` (no existence
/// oracle on the execution surface); [`BinderError::InvalidArgs`] is the only
/// distinct, caller-actionable code.
#[derive(Debug)]
pub enum BinderError {
    /// Unauthorized OR not-found OR not-a-workflow OR body-unavailable — collapsed
    /// by the host so an unauthorized caller learns nothing about what exists.
    NotAuthorized,
    /// Argument validation / parse / unbound slot / uncompilable / empty recipe.
    InvalidArgs(String),
    /// An internal binder failure (storage, etc.).
    Internal(String),
}

/// The recipe-binding seam (the `Invoke` path). The host implements it with
/// `kx_invoke::bind_snapshot` over its provisioned ledgers + the per-handle
/// free-param contract, resolving the caller's Use authority from the
/// authoritative grant ledger (never a caller-supplied warrant — SN-8). It does
/// NO journal write (that is the [`RunSubmitter`]'s job). A `None` seam on the
/// service ⇒ `Invoke` returns `unimplemented`.
#[tonic::async_trait]
pub trait RecipeBinder: Send + Sync {
    /// Resolve `handle` + `args` for the SERVER-DERIVED `party` into a runnable,
    /// least-privilege [`BoundRecipe`].
    ///
    /// # Errors
    /// [`BinderError`] — `NotAuthorized` (uniform, no oracle) or `InvalidArgs`.
    async fn bind(
        &self,
        party: &str,
        handle: &str,
        args: &[u8],
    ) -> Result<BoundRecipe, BinderError>;
}

/// The backend behind the external `KxGateway` service: a read-only journal +
/// content reader (the read-fold) and a [`RunSubmitter`] (the propose-proxy).
/// Holds no writer; auth/ownership stay cloud-side (the host wraps this with
/// middleware). Construct with [`GatewayService::new`]; wire the optional
/// catalog seam with [`GatewayService::with_signature_catalog`].
#[derive(Clone)]
pub struct GatewayService {
    reader: Arc<dyn JournalReader>,
    submitter: Arc<dyn RunSubmitter>,
    content: Arc<dyn ContentReader>,
    /// The optional signature-catalog seam (the host injects a concrete catalog).
    /// `None` ⇒ the three signature RPCs return `unimplemented`.
    catalog: Option<Arc<dyn SignatureCatalog>>,
    /// The optional recipe-binding seam (the host injects a kx-invoke-backed
    /// binder). `None` ⇒ `Invoke` returns `unimplemented`.
    binder: Option<Arc<dyn RecipeBinder>>,
}

impl GatewayService {
    /// Wire a gateway over a read-only journal seam, a propose-proxy, and a
    /// read-only content seam. No catalog seam (the signature RPCs stay
    /// `unimplemented` until [`GatewayService::with_signature_catalog`] wires one).
    pub fn new(
        reader: Arc<dyn JournalReader>,
        submitter: Arc<dyn RunSubmitter>,
        content: Arc<dyn ContentReader>,
    ) -> Self {
        Self {
            reader,
            submitter,
            content,
            catalog: None,
            binder: None,
        }
    }

    /// Wire the signature-catalog seam (the host's concrete `kx-catalog`-backed
    /// impl). Enables `ListSignatures` / `GetSignature` / `RegisterSignature`.
    #[must_use]
    pub fn with_signature_catalog(mut self, catalog: Arc<dyn SignatureCatalog>) -> Self {
        self.catalog = Some(catalog);
        self
    }

    /// Wire the recipe-binding seam (the host's `kx-invoke`-backed binder).
    /// Enables `Invoke` (recipe-by-handle execution).
    #[must_use]
    pub fn with_recipe_binder(mut self, binder: Arc<dyn RecipeBinder>) -> Self {
        self.binder = Some(binder);
        self
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

    async fn invoke(
        &self,
        request: Request<proto::InvokeRequest>,
    ) -> Result<Response<proto::InvokeResponse>, Status> {
        let binder = self.binder.as_ref().ok_or_else(|| {
            Status::unimplemented("Invoke: no recipe binder wired (host provisioned no recipes)")
        })?;
        // SERVER-DERIVED identity (SN-8): the party the auth interceptor resolved
        // and stashed. Absent ⇒ no caller was resolved ⇒ deny. The wire request
        // carries no party field, so a caller cannot assert who it is.
        let party = request
            .extensions()
            .get::<CallerParty>()
            .map(|p| p.0.clone())
            .ok_or_else(|| Status::unauthenticated("no resolved caller identity"))?;
        let req = request.into_inner();

        let bound = binder
            .bind(&party, &req.handle, &req.args)
            .await
            .map_err(|e| match e {
                // Uniform "not authorized" — no existence oracle on the execution
                // surface (unauthorized / unknown handle are indistinguishable).
                BinderError::NotAuthorized => Status::permission_denied("not authorized"),
                BinderError::InvalidArgs(detail) => Status::invalid_argument(detail),
                BinderError::Internal(detail) => Status::internal(detail),
            })?;

        // The SAME propose-proxy as SubmitRun: register first (returns only after
        // the journaled instance_id), then submit each bound Mote. No new write
        // path; the coordinator stays the sole journal writer.
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(submit_status)?;
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false)
                .await
                .map_err(submit_status)?;
        }

        Ok(Response::new(proto::InvokeResponse {
            instance_id: instance_id.to_vec(),
            recipe_fingerprint: bound.recipe_fingerprint.to_vec(),
            // SERVER-DERIVED (from bind → compile, never client-supplied — SN-8).
            terminal_mote_id: bound.terminal_mote_id.as_bytes().to_vec(),
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
        let catalog = self
            .catalog
            .as_ref()
            .ok_or_else(|| Status::unimplemented("ListSignatures: no signature catalog wired"))?;
        let signatures = catalog
            .list()
            .into_iter()
            .map(|e| proto::SignatureSummary {
                signature_id: e.signature_id.to_vec(),
                name: e.name,
            })
            .collect();
        Ok(Response::new(proto::ListSignaturesResponse { signatures }))
    }

    async fn get_signature(
        &self,
        request: Request<proto::GetSignatureRequest>,
    ) -> Result<Response<proto::GetSignatureResponse>, Status> {
        let catalog = self
            .catalog
            .as_ref()
            .ok_or_else(|| Status::unimplemented("GetSignature: no signature catalog wired"))?;
        let id = hash_32(
            &request.into_inner().signature_id,
            "signature_id must be 32 bytes",
        )?;
        // A public discovery surface: `not_found` here is intended (the catalog is
        // authoritative for WHAT recipes exist), NOT collapsed like the Invoke
        // execution surface.
        let manifest = catalog
            .get(&id)
            .ok_or_else(|| Status::not_found("signature not found"))?;
        Ok(Response::new(proto::GetSignatureResponse {
            signature_id: id.to_vec(),
            manifest,
        }))
    }

    async fn register_signature(
        &self,
        request: Request<proto::RegisterSignatureRequest>,
    ) -> Result<Response<proto::RegisterSignatureResponse>, Status> {
        let catalog = self.catalog.as_ref().ok_or_else(|| {
            Status::unimplemented("RegisterSignature: no signature catalog wired")
        })?;
        // The host server-derives the id from the decoded manifest (SN-8) and the
        // registry enforces idempotency + immutability.
        let registered = catalog
            .register(&request.into_inner().manifest)
            .map_err(|e| match e {
                CatalogSeamError::ImmutabilityConflict => {
                    Status::failed_precondition("immutable catalog conflict")
                }
                CatalogSeamError::Malformed(detail) => Status::invalid_argument(detail),
                CatalogSeamError::Internal(detail) => Status::internal(detail),
            })?;
        Ok(Response::new(proto::RegisterSignatureResponse {
            signature_id: registered.signature_id.to_vec(),
        }))
    }
}
