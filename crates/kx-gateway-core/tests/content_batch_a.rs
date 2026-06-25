//! Batch A round-trips over a real tonic transport: `PutContent` (the ONE
//! deliberate client write seam — content store only), the EMPTY-`instance_id`
//! uploads scope on `GetContent`, `GetContentBatch` (order, caps, uniform
//! empties, truncation), and `ListModels` (display-only discovery).
//!
//! The load-bearing assertions: the ref is SERVER-DERIVED (SN-8), the size cap
//! fails closed BEFORE the store is touched, and every unauthorized / missing /
//! malformed read is UNIFORM (no existence oracle, D120.1).

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{build_run, spawn, spawn_with_party, MockSubmitter, INSTANCE_ID};
use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_gateway_core::{
    ContentReader, ContentWriter, GatewayError, GatewayService, JournalReader, ModelCatalogView,
    ModelLifecycleControl, ModelLifecycleOutcome, ModelSummaryEntry, ReadOnly, RunSubmitter,
    UploadRecord, UploadsLedger, BATCH_ITEM_CLAMP_BYTES, MAX_BATCH_REFS,
};
use kx_proto::proto;
use tonic::Code;

/// An in-memory [`UploadsLedger`] fake (the host's `uploads.db` stand-in).
#[derive(Default)]
struct MemLedger {
    rows: Mutex<BTreeMap<[u8; 32], UploadRecord>>,
}

impl MemLedger {
    fn row(&self, r: &[u8; 32]) -> Option<UploadRecord> {
        self.rows.lock().unwrap().get(r).cloned()
    }
}

impl UploadsLedger for MemLedger {
    fn record(&self, rec: UploadRecord) -> Result<(), GatewayError> {
        self.rows.lock().unwrap().insert(rec.content_ref, rec);
        Ok(())
    }

    fn contains(&self, content_ref: &[u8; 32]) -> Result<bool, GatewayError> {
        Ok(self.rows.lock().unwrap().contains_key(content_ref))
    }
}

struct FixedCatalog(Vec<ModelSummaryEntry>);

impl ModelCatalogView for FixedCatalog {
    fn list(&self) -> Result<Vec<ModelSummaryEntry>, GatewayError> {
        Ok(self.0.clone())
    }
}

/// A deterministic (FFI-free) [`ModelLifecycleControl`] over a fixed registered
/// set: an in-memory residency map, fail-closed on an unregistered id (POC-3).
struct FixedLifecycle {
    registered: std::collections::BTreeSet<String>,
    resident: Mutex<std::collections::BTreeSet<String>>,
}

impl FixedLifecycle {
    fn new(registered: &[&str], resident: &[&str]) -> Self {
        Self {
            registered: registered.iter().map(|s| (*s).to_string()).collect(),
            resident: Mutex::new(resident.iter().map(|s| (*s).to_string()).collect()),
        }
    }
    fn gate(&self, id: &str) -> Result<(), GatewayError> {
        if self.registered.contains(id) {
            Ok(())
        } else {
            Err(GatewayError::NotFound("model not registered"))
        }
    }
}

impl ModelLifecycleControl for FixedLifecycle {
    fn load(&self, model_id: &str) -> Result<ModelLifecycleOutcome, GatewayError> {
        self.gate(model_id)?;
        let was = self.resident.lock().unwrap().insert(model_id.to_string());
        Ok(ModelLifecycleOutcome {
            model_id: model_id.to_string(),
            loaded: true,
            was_resident: !was, // insert returns true when NEWLY inserted
        })
    }
    fn offload(&self, model_id: &str) -> Result<ModelLifecycleOutcome, GatewayError> {
        self.gate(model_id)?;
        let was = self.resident.lock().unwrap().remove(model_id);
        Ok(ModelLifecycleOutcome {
            model_id: model_id.to_string(),
            loaded: false,
            was_resident: was,
        })
    }
}

fn no_submitter() -> Arc<dyn RunSubmitter> {
    Arc::new(MockSubmitter::default())
}

/// A gateway whose content store is SHARED between the read seam, the write
/// seam, and the test (so assertions can inspect the store directly), plus an
/// uploads ledger. Returns (service, store, ledger).
fn uploads_service(
    cap: Option<u64>,
) -> (GatewayService, Arc<InMemoryContentStore>, Arc<MemLedger>) {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let store = Arc::new(run.content);
    let ledger = Arc::new(MemLedger::default());
    let content: Arc<dyn ContentReader> = store.clone();
    let writer: Arc<dyn ContentWriter> = store.clone();
    let mut service = GatewayService::new(reader, no_submitter(), content)
        .with_content_writer(writer)
        .with_uploads_ledger(ledger.clone());
    if let Some(cap) = cap {
        service = service.with_put_content_cap(cap);
    }
    (service, store, ledger)
}

// --- PutContent ------------------------------------------------------------

#[tokio::test]
async fn put_content_returns_server_derived_ref_and_records_audit_row() {
    let (service, store, ledger) = uploads_service(None);
    let mut client = spawn_with_party(service, "tester").await;

    let resp = client
        .put_content(proto::PutContentRequest {
            payload: b"hello uploads".to_vec(),
            media_type: "text/plain".into(),
            filename: "hello.txt".into(),
        })
        .await
        .unwrap()
        .into_inner();

    // SN-8: the ref is server-derived blake3 of the payload.
    let expected = ContentRef::of(b"hello uploads");
    assert_eq!(resp.content_ref, expected.0.to_vec());
    assert_eq!(resp.size, 13);
    assert!(!resp.deduplicated, "first upload is not a duplicate");
    assert!(
        ContentStore::contains(&*store, &expected),
        "blob landed in the store"
    );

    // The advisory audit row carries the SERVER-resolved principal.
    let row = ledger.row(&expected.0).expect("audit row recorded");
    assert_eq!(row.media_type, "text/plain");
    assert_eq!(row.filename, "hello.txt");
    assert_eq!(row.principal, "tester");

    // Re-upload of identical bytes: same ref, dedup flagged.
    let resp2 = client
        .put_content(proto::PutContentRequest {
            payload: b"hello uploads".to_vec(),
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp2.content_ref, expected.0.to_vec());
    assert!(resp2.deduplicated);
}

#[tokio::test]
async fn put_content_cap_fails_closed_before_the_store() {
    let (service, store, _ledger) = uploads_service(Some(8));
    let mut client = spawn_with_party(service, "tester").await;

    // Exactly at the cap: accepted.
    let ok = client
        .put_content(proto::PutContentRequest {
            payload: vec![0xAB; 8],
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ok.size, 8);

    // One byte over: RESOURCE_EXHAUSTED and the blob is NOT stored.
    let over = vec![0xCD; 9];
    let err = client
        .put_content(proto::PutContentRequest {
            payload: over.clone(),
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::ResourceExhausted);
    assert!(
        !ContentStore::contains(&*store, &ContentRef::of(&over)),
        "an over-cap payload must never touch the store"
    );
}

#[tokio::test]
async fn put_content_without_seams_is_unimplemented() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let service = GatewayService::new(reader, no_submitter(), content);
    let mut client = spawn_with_party(service, "tester").await;

    let err = client
        .put_content(proto::PutContentRequest {
            payload: b"x".to_vec(),
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn put_content_without_resolved_party_is_unauthenticated() {
    let (service, _store, _ledger) = uploads_service(None);
    // `spawn` (no interceptor) stamps no CallerParty.
    let mut client = spawn(service).await;

    let err = client
        .put_content(proto::PutContentRequest {
            payload: b"x".to_vec(),
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

// --- GetContent: the uploads scope ------------------------------------------

#[tokio::test]
async fn uploads_scope_serves_uploaded_refs_and_denies_uniformly() {
    let (service, _store, _ledger) = uploads_service(None);
    let mut client = spawn_with_party(service, "tester").await;

    let put = client
        .put_content(proto::PutContentRequest {
            payload: b"scoped blob".to_vec(),
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap()
        .into_inner();

    // EMPTY instance_id = uploads scope: the uploaded ref is served.
    let blob = client
        .get_content(proto::GetContentRequest {
            content_ref: put.content_ref.clone(),
            instance_id: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(blob.payload, b"scoped blob");

    // An unknown ref denies with the SAME uniform message as the run scope.
    let unknown = client
        .get_content(proto::GetContentRequest {
            content_ref: vec![0x77; 32],
            instance_id: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::PermissionDenied);
    assert_eq!(unknown.message(), "not authorized");

    // The uploads scope does NOT leak into a run scope: a valid run ticket that
    // doesn't own the uploaded ref still denies, uniformly.
    let cross = client
        .get_content(proto::GetContentRequest {
            content_ref: put.content_ref,
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(cross.code(), Code::PermissionDenied);
    assert_eq!(cross.message(), "not authorized");
}

#[tokio::test]
async fn uploads_scope_without_ledger_is_uniformly_denied() {
    // A gateway WITHOUT the uploads sidecar: the empty instance_id is no longer
    // a parse error, but it authorizes nothing — uniform denial, not an oracle
    // about wiring.
    let run = build_run();
    let a_ref = run.a_ref;
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let service = GatewayService::new(reader, no_submitter(), content);
    let mut client = spawn(service).await;

    let err = client
        .get_content(proto::GetContentRequest {
            content_ref: a_ref.0.to_vec(),
            instance_id: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(err.message(), "not authorized");
}

// --- GetContentBatch ---------------------------------------------------------

#[tokio::test]
async fn batch_preserves_order_and_uniformly_empties_bad_refs() {
    let run = build_run();
    let a_ref = run.a_ref;
    let b_ref = ContentRef::of(b"result-of-B");
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let service = GatewayService::new(reader, no_submitter(), content);
    let mut client = spawn(service).await;

    let resp = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: INSTANCE_ID.to_vec(),
            content_refs: vec![
                b_ref.0.to_vec(), // authorized (committed result of the run)
                vec![0x55; 32],   // never existed
                vec![0x01, 0x02], // malformed (not 32 bytes)
                a_ref.0.to_vec(), // authorized
                vec![0xaa; 32],   // exists-adjacent but NOT a committed result (warrant ref)
            ],
            max_bytes_per_item: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(resp.items.len(), 5, "one item per requested ref");
    // Request order preserved; echoed refs match.
    assert_eq!(resp.items[0].payload, b"result-of-B");
    assert_eq!(resp.items[0].full_size, 11);
    assert!(!resp.items[0].truncated);
    assert_eq!(resp.items[3].payload, b"result-of-A");

    // Never-existed, malformed, and exists-but-unauthorized are INDISTINGUISHABLE.
    for bad in [&resp.items[1], &resp.items[2], &resp.items[4]] {
        assert!(bad.payload.is_empty(), "uniform empty payload");
        assert_eq!(bad.full_size, 0, "uniform zero size — no existence oracle");
        assert!(!bad.truncated);
    }
}

#[tokio::test]
async fn batch_ref_count_cap_fails_closed() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let service = GatewayService::new(reader, no_submitter(), content);
    let mut client = spawn(service).await;

    // Exactly at the cap: accepted (all uniformly empty here — unknown refs).
    let at_cap = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: INSTANCE_ID.to_vec(),
            content_refs: vec![vec![0x66; 32]; MAX_BATCH_REFS],
            max_bytes_per_item: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(at_cap.items.len(), MAX_BATCH_REFS);

    // One over: refused outright (never silent truncation).
    let err = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: INSTANCE_ID.to_vec(),
            content_refs: vec![vec![0x66; 32]; MAX_BATCH_REFS + 1],
            max_bytes_per_item: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn batch_truncates_at_client_clamp_with_honest_full_size() {
    let (service, _store, _ledger) = uploads_service(None);
    let mut client = spawn_with_party(service, "tester").await;

    let payload = vec![0xEE; 1000];
    let put = client
        .put_content(proto::PutContentRequest {
            payload: payload.clone(),
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap()
        .into_inner();

    // Uploads scope (empty instance_id) + a 16-byte client clamp.
    let resp = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: Vec::new(),
            content_refs: vec![put.content_ref],
            max_bytes_per_item: Some(16),
        })
        .await
        .unwrap()
        .into_inner();

    let item = &resp.items[0];
    assert_eq!(item.payload.len(), 16, "clamped to the client max");
    assert_eq!(item.payload, vec![0xEE; 16]);
    assert!(item.truncated);
    assert_eq!(item.full_size, 1000, "full_size stays honest");

    // The client can only LOWER the server clamp: a blob bigger than the
    // server's per-item clamp truncates there even under an absurd client max.
    let big_len = usize::try_from(BATCH_ITEM_CLAMP_BYTES).unwrap() + 4096;
    let big = client
        .put_content(proto::PutContentRequest {
            payload: vec![0x42; big_len],
            media_type: String::new(),
            filename: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    let resp = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: Vec::new(),
            content_refs: vec![big.content_ref],
            max_bytes_per_item: Some(u64::MAX),
        })
        .await
        .unwrap()
        .into_inner();
    let item = &resp.items[0];
    assert_eq!(item.payload.len() as u64, BATCH_ITEM_CLAMP_BYTES);
    assert!(item.truncated);
    assert_eq!(item.full_size, big_len as u64);
}

#[tokio::test]
async fn batch_wrong_length_instance_id_is_invalid_argument() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let service = GatewayService::new(reader, no_submitter(), content);
    let mut client = spawn(service).await;

    let err = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: vec![0x11; 8], // neither empty nor 16 bytes
            content_refs: vec![vec![0x66; 32]],
            max_bytes_per_item: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

#[tokio::test]
async fn batch_unowned_run_ticket_is_uniformly_denied() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    let service = GatewayService::new(reader, no_submitter(), content);
    let mut client = spawn(service).await;

    let err = client
        .get_content_batch(proto::GetContentBatchRequest {
            instance_id: vec![0x99; 16], // a well-formed ticket that owns nothing
            content_refs: vec![vec![0x66; 32]],
            max_bytes_per_item: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(err.message(), "not authorized");
}

// --- ListModels ---------------------------------------------------------------

#[tokio::test]
async fn list_models_maps_the_catalog_and_degrades_without_a_seam() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);

    // No seam ⇒ unimplemented (an OLD-host degrade, not an empty lie).
    let bare = GatewayService::new(reader.clone(), no_submitter(), content.clone());
    let mut client = spawn(bare).await;
    let err = client
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);

    // Wired seam ⇒ the display projection maps field-for-field.
    let catalog = FixedCatalog(vec![ModelSummaryEntry {
        model_id: "qwen3-4b".into(),
        modalities: vec!["text".into(), "image".into()],
        description: "Qwen3 4B".into(),
        serving: true,
        context_len: 8192,
        loaded: true,
        chat_handle: "kx/recipes/chat".into(),
        engine: "kx-llamacpp".into(),
        can_embed: true,
    }]);
    let service = GatewayService::new(reader, no_submitter(), content)
        .with_model_catalog_view(Arc::new(catalog));
    let mut client = spawn(service).await;
    let resp = client
        .list_models(proto::ListModelsRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.models.len(), 1);
    let m = &resp.models[0];
    assert_eq!(m.model_id, "qwen3-4b");
    assert_eq!(m.modalities, vec!["text".to_string(), "image".to_string()]);
    assert_eq!(m.description, "Qwen3 4B");
    assert!(m.serving);
    assert_eq!(m.context_len, 8192);
    // POC-3: the additive residency + routing fields map through.
    assert!(m.loaded);
    assert_eq!(m.chat_handle, "kx/recipes/chat");
    // PR-B: the additive embedder flag maps through.
    assert!(m.can_embed);
}

// --- Model lifecycle (POC-3 LoadModel / OffloadModel) --------------------------

#[tokio::test]
async fn load_offload_round_trip_and_unregistered_is_fail_closed() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);

    let lifecycle = Arc::new(FixedLifecycle::new(&["gemma", "qwen"], &["gemma"]));
    let service =
        GatewayService::new(reader, no_submitter(), content).with_model_lifecycle(lifecycle);
    // spawn_with_party stamps a server-derived CallerParty (the auth-gate passes).
    let mut client = spawn_with_party(service, "alice@acme").await;

    // A cold load of a registered model: was_resident=false ⇒ a real load.
    let r = client
        .load_model(proto::LoadModelRequest {
            model_id: "qwen".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(r.model_id, "qwen");
    assert!(r.loaded);
    assert!(!r.was_resident, "cold load");

    // Offload of an already-resident model: was_resident=true.
    let r = client
        .offload_model(proto::OffloadModelRequest {
            model_id: "gemma".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!r.loaded);
    assert!(r.was_resident);

    // The REQUIRED security negative: an UNREGISTERED id is fail-closed NotFound
    // (never a load of an arbitrary path).
    let err = client
        .load_model(proto::LoadModelRequest {
            model_id: "not-registered".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
    let err = client
        .offload_model(proto::OffloadModelRequest {
            model_id: "not-registered".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::NotFound);
}

#[tokio::test]
async fn load_offload_degrade_without_seam_and_require_auth() {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);

    // No lifecycle seam ⇒ unimplemented (an FFI-free / old-host degrade), even
    // with an authenticated caller.
    let bare = GatewayService::new(reader.clone(), no_submitter(), content.clone());
    let mut client = spawn_with_party(bare, "alice@acme").await;
    let err = client
        .load_model(proto::LoadModelRequest {
            model_id: "qwen".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);

    // Seam wired but NO CallerParty (no auth interceptor) ⇒ unauthenticated
    // (a mutating control needs a resolved identity, SN-8).
    let lifecycle = Arc::new(FixedLifecycle::new(&["qwen"], &[]));
    let service =
        GatewayService::new(reader, no_submitter(), content).with_model_lifecycle(lifecycle);
    let mut anon = spawn(service).await;
    let err = anon
        .load_model(proto::LoadModelRequest {
            model_id: "qwen".into(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}
