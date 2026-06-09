//! In-process `KxGateway` round-trips over a real tonic transport server. Three
//! enterprise scenarios (operator renders a run DAG; end-user fetches a committed
//! result; resumable event stream) plus the SubmitRun propose-proxy and the SN-8
//! "client never computes a MoteId" boundary.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::sync::Arc;

use common::{
    build_run, sample_mote, sample_warrant, service_from, spawn, spawn_with_party, MockSubmitter,
    INSTANCE_ID, RECIPE_FP,
};
use kx_gateway_core::{
    AssetGrantsView, BinderError, BoundRecipe, CatalogSeamError, GatewayService, GrantEntry,
    GrantView, MembershipView, RecipeBinder, RecipeCatalog, RecipeFormFieldEntry, RecipeParamKind,
    RegisteredSignature, RunSubmitter, SignatureCatalog, SignatureSummaryEntry, TeamMemberEntry,
    TeamMembersView, TeamSummaryEntry, WarrantProjection,
};
use kx_proto::proto;
use tonic::Code;

fn no_submitter() -> Arc<dyn RunSubmitter> {
    Arc::new(MockSubmitter::default())
}

// --- Scenario A — operator renders a live run DAG -------------------------

#[tokio::test]
async fn scenario_a_renders_run_dag_server_derived() {
    let run = build_run();
    let (a, b, c) = (run.a, run.b, run.c);
    let mut client = spawn(service_from(run, no_submitter())).await;

    let view = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: INSTANCE_ID.to_vec(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(view.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(view.recipe_fingerprint, RECIPE_FP.to_vec());
    assert_eq!(view.current_seq, 4);
    assert_eq!(view.motes.len(), 3);

    let snap = |id: kx_mote::MoteId| {
        view.motes
            .iter()
            .find(|m| m.mote_id == id.as_bytes().to_vec())
            .cloned()
            .unwrap_or_else(|| panic!("missing snapshot for {id:?}"))
    };

    // Every mote_id is server-derived (it came from the journal fold, not a request).
    let sa = snap(a);
    assert_eq!(sa.state, proto::MoteSnapshotState::Committed as i32);
    assert!(sa.result_ref.is_some());
    assert_eq!(
        sa.mote_def_hash.len(),
        32,
        "committed snapshot carries its def hash"
    );
    assert!(sa.warrant_ref.is_some());
    assert!(sa.verdict.is_none(), "verdict opaque/absent at freeze");

    let sb = snap(b);
    assert_eq!(sb.state, proto::MoteSnapshotState::Committed as i32);
    assert_eq!(sb.parents.len(), 1, "B is a data-child of A");
    assert_eq!(sb.parents[0].parent_id, a.as_bytes().to_vec());

    let sc = snap(c);
    assert_eq!(sc.state, proto::MoteSnapshotState::Scheduled as i32);
    assert!(sc.committed_seq.is_none());
}

#[tokio::test]
async fn scenario_a_at_seq_is_clamped_to_head() {
    let run = build_run();
    let mut client = spawn(service_from(run, no_submitter())).await;
    // A far-future at_seq must yield the head, never an error that leaks the head.
    let view = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: INSTANCE_ID.to_vec(),
            at_seq: Some(9_999),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(view.current_seq, 4);
}

#[tokio::test]
async fn scenario_a_wrong_instance_is_uniform_denied() {
    let run = build_run();
    let mut client = spawn(service_from(run, no_submitter())).await;
    let err = client
        .get_projection(proto::GetProjectionRequest {
            instance_id: [0x99; 16].to_vec(),
            at_seq: None,
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::PermissionDenied);
    assert_eq!(err.message(), "not authorized");
}

// --- Scenario B — end-user fetches a committed result (no oracle) ----------

#[tokio::test]
async fn scenario_b_get_content_owned_then_uniform_denials() {
    let run = build_run();
    let a_ref = run.a_ref;
    let mut client = spawn(service_from(run, no_submitter())).await;

    // Owned committed ref → bytes.
    let blob = client
        .get_content(proto::GetContentRequest {
            content_ref: a_ref.0.to_vec(),
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(blob.payload, b"result-of-A");

    // Wrong instance and unknown ref must BOTH return the identical denial — no
    // existence oracle (a caller cannot tell "not yours" from "doesn't exist").
    let wrong_instance = client
        .get_content(proto::GetContentRequest {
            content_ref: a_ref.0.to_vec(),
            instance_id: [0x99; 16].to_vec(),
        })
        .await
        .unwrap_err();
    let unknown_ref = client
        .get_content(proto::GetContentRequest {
            content_ref: [0xEE; 32].to_vec(),
            instance_id: INSTANCE_ID.to_vec(),
        })
        .await
        .unwrap_err();

    assert_eq!(wrong_instance.code(), Code::PermissionDenied);
    assert_eq!(unknown_ref.code(), Code::PermissionDenied);
    assert_eq!(wrong_instance.message(), unknown_ref.message());
    assert_eq!(wrong_instance.message(), "not authorized");
}

// --- Scenario C — resumable event stream -----------------------------------

async fn collect_frames(
    client: &mut kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    since_seq: u64,
) -> Vec<proto::EventFrame> {
    let mut stream = client
        .stream_events(proto::StreamEventsRequest {
            instance_id: INSTANCE_ID.to_vec(),
            since_seq,
        })
        .await
        .unwrap()
        .into_inner();
    let mut frames = Vec::new();
    while let Some(frame) = stream.message().await.unwrap() {
        frames.push(frame);
    }
    frames
}

fn committed_mote_ids(frames: &[proto::EventFrame]) -> Vec<Vec<u8>> {
    frames
        .iter()
        .flat_map(|f| &f.deltas)
        .filter_map(|d| match &d.kind {
            Some(proto::event_delta::Kind::Committed(c)) => Some(c.mote_id.clone()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn scenario_c_stream_from_start_then_resume() {
    let run = build_run();
    let (a, b) = (run.a, run.b);
    let mut client = spawn(service_from(run, no_submitter())).await;

    // From the start: A + B committed deltas; the proposed C is NOT surfaced.
    let frames = collect_frames(&mut client, 0).await;
    let committed = committed_mote_ids(&frames);
    assert!(committed.contains(&a.as_bytes().to_vec()));
    assert!(committed.contains(&b.as_bytes().to_vec()));
    let last = frames.last().unwrap();
    assert!(last.journal_boundary);
    assert_eq!(last.next_seq, 4, "next_seq never exceeds head");

    // Resume at A's seq (2): only B's committed delta (seq 3) is newer.
    let resumed = collect_frames(&mut client, 2).await;
    let committed_after = committed_mote_ids(&resumed);
    assert_eq!(committed_after, vec![b.as_bytes().to_vec()]);
    assert!(resumed.iter().all(|f| f.next_seq <= 4));
}

// --- SubmitRun — propose-proxy ordering ------------------------------------

#[tokio::test]
async fn submit_run_registers_first_then_submits() {
    let mock = MockSubmitter::default();
    let svc = service_from(build_run(), Arc::new(mock.clone()));
    let mut client = spawn(svc).await;

    let handle = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: RECIPE_FP.to_vec(),
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(sample_mote().into()),
                warrant: Some(sample_warrant().into()),
                accept_at_least_once: false,
            }],
        })
        .await
        .unwrap()
        .into_inner();

    assert_eq!(handle.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(handle.recipe_fingerprint, RECIPE_FP.to_vec());
    // register_run is proxied BEFORE any submit_mote (never ack ahead of the run).
    let calls = mock.calls();
    assert_eq!(calls.first().map(String::as_str), Some("register_run(34)")); // 0x22 == 34
    assert!(calls.iter().any(|c| c == "submit_mote"));
    assert!(
        calls.iter().position(|c| c.starts_with("register_run"))
            < calls.iter().position(|c| c == "submit_mote")
    );
}

#[tokio::test]
async fn submit_run_rejects_malformed_recipe_fingerprint() {
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    let err = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: vec![0x22; 4], // not 32 bytes
            motes: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::InvalidArgument);
}

// --- PR-2c-3 critic-live: SubmitRun cross-Mote critic ADMISSION (B3 + H5) ---

/// A native deterministic critic Mote wire-spec (`critic_check` + `critic_for`).
fn critic_mote_spec(critic_for: [u8; 32]) -> proto::SubmitMoteSpec {
    use common::{mote_def, sample_warrant};
    use kx_critic_types::{CheckSpec, SchemaSpec, SchemaTag};
    use kx_mote::{GraphPosition, InputDataId, Mote, MoteId, NdClass};

    let mut def = mote_def(0xC0, NdClass::Pure);
    def.critic_check = Some(CheckSpec::Schema(SchemaSpec {
        expected: SchemaTag::Json,
    }));
    def.critic_for = Some(MoteId::from_bytes(critic_for));
    let mote = Mote::new(
        def,
        InputDataId::from_bytes([0x20; 32]),
        GraphPosition(vec![0xC0]),
        smallvec::SmallVec::new(),
    );
    proto::SubmitMoteSpec {
        mote: Some(mote.into()),
        warrant: Some(sample_warrant().into()),
        accept_at_least_once: false,
    }
}

#[tokio::test]
async fn submit_run_refuses_critic_when_critics_unsupported() {
    // H5: the default service cannot evaluate critics → a critic-bearing workflow
    // is refused fail-closed (never admitted into an exit-gate deadlock).
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    let err = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: RECIPE_FP.to_vec(),
            motes: vec![critic_mote_spec([0xAA; 32])],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(
        err.message().contains("critic"),
        "reason surfaced: {}",
        err.message()
    );
}

#[tokio::test]
async fn submit_run_refuses_critic_with_dangling_critic_for() {
    // B3: with critics supported, the whole-DAG validator still runs — a critic
    // whose `critic_for` references no submitted Mote trips R-4 (cross-Mote, NOT
    // enforced by the per-Mote live submit path) and is refused at ingress.
    let svc =
        service_from(build_run(), Arc::new(MockSubmitter::default())).with_critics_supported(true);
    let mut client = spawn(svc).await;
    let err = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: RECIPE_FP.to_vec(),
            motes: vec![critic_mote_spec([0x77; 32])], // critic_for → a non-existent Mote
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
    assert!(
        err.message().contains("critic admission"),
        "reason surfaced: {}",
        err.message()
    );
}

#[tokio::test]
async fn submit_run_admits_critic_free_run_unchanged_when_unsupported() {
    // A critic-free workflow is byte-for-byte unaffected by the admission gate even
    // on a serve that cannot evaluate critics (the gate only engages on a critic).
    let mock = MockSubmitter::default();
    let svc = service_from(build_run(), Arc::new(mock.clone()));
    let mut client = spawn(svc).await;
    let handle = client
        .submit_run(proto::SubmitRunRequest {
            recipe_fingerprint: RECIPE_FP.to_vec(),
            motes: vec![proto::SubmitMoteSpec {
                mote: Some(sample_mote().into()),
                warrant: Some(sample_warrant().into()),
                accept_at_least_once: false,
            }],
        })
        .await
        .expect("a critic-free run is admitted unchanged")
        .into_inner();
    assert_eq!(handle.instance_id, INSTANCE_ID.to_vec());
    assert!(mock.calls().iter().any(|c| c == "submit_mote"));
}

// --- SN-8 — the client never computes a MoteId -----------------------------

#[test]
fn submit_boundary_rederives_mote_id_discarding_wire_advisory() {
    // A genuine Mote and its wire form.
    let mote = sample_mote();
    let expected = *mote.id.as_bytes();
    let mut wire: proto::Mote = mote.into();
    // Tamper with the advisory wire mote_id.
    wire.mote_id = vec![0xFF; 32];
    // The boundary re-derives identity Rust-side (D53/SN-8): the tampered
    // advisory id is discarded; the re-derived id matches the genuine one.
    let rebuilt: kx_mote::Mote = wire.try_into().unwrap();
    assert_eq!(*rebuilt.id.as_bytes(), expected);
    assert_ne!(*rebuilt.id.as_bytes(), [0xFF; 32]);
}

// --- Signature RPCs: the optional catalog seam (R2a) ------------------------

/// A mock [`SignatureCatalog`] that knows exactly one id (`[0x11; 32]`) and can
/// be configured to fail registration with an immutability conflict.
#[derive(Default)]
struct MockCatalog {
    conflict: bool,
}

const MOCK_ID: [u8; 32] = [0x11; 32];

impl SignatureCatalog for MockCatalog {
    fn register(&self, _manifest: &[u8]) -> Result<RegisteredSignature, CatalogSeamError> {
        if self.conflict {
            Err(CatalogSeamError::ImmutabilityConflict)
        } else {
            Ok(RegisteredSignature {
                signature_id: MOCK_ID,
            })
        }
    }

    fn get(&self, signature_id: &[u8; 32]) -> Option<Vec<u8>> {
        (*signature_id == MOCK_ID).then(|| b"mock-manifest".to_vec())
    }

    fn list(&self) -> Vec<SignatureSummaryEntry> {
        vec![SignatureSummaryEntry {
            signature_id: MOCK_ID,
            name: "sig-11111111".to_string(),
        }]
    }
}

fn service_with_catalog(catalog: MockCatalog) -> GatewayService {
    service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_signature_catalog(Arc::new(catalog))
}

#[tokio::test]
async fn signature_rpcs_unimplemented_without_a_catalog_seam() {
    // The default service wires no catalog → all three RPCs are unimplemented
    // (backward-compatible: SubmitRun-only hosts are unaffected).
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()));
    let mut client = spawn(svc).await;
    assert_eq!(
        client
            .list_signatures(proto::ListSignaturesRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .get_signature(proto::GetSignatureRequest {
                signature_id: vec![0u8; 32],
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .register_signature(proto::RegisterSignatureRequest { manifest: vec![] })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
}

#[tokio::test]
async fn signature_rpcs_dispatch_to_the_catalog_seam() {
    let mut client = spawn(service_with_catalog(MockCatalog::default())).await;

    let reg = client
        .register_signature(proto::RegisterSignatureRequest {
            manifest: b"anything".to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(reg.signature_id, MOCK_ID.to_vec());

    let got = client
        .get_signature(proto::GetSignatureRequest {
            signature_id: MOCK_ID.to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(got.manifest, b"mock-manifest".to_vec());

    // A public discovery surface: an unknown id is `not_found` (NOT collapsed).
    let unknown = client
        .get_signature(proto::GetSignatureRequest {
            signature_id: vec![0x22; 32],
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);

    let list = client
        .list_signatures(proto::ListSignaturesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(list.signatures.len(), 1);
    assert_eq!(list.signatures[0].signature_id, MOCK_ID.to_vec());
}

#[tokio::test]
async fn register_immutability_conflict_is_failed_precondition() {
    let mut client = spawn(service_with_catalog(MockCatalog { conflict: true })).await;
    let err = client
        .register_signature(proto::RegisterSignatureRequest {
            manifest: b"x".to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::FailedPrecondition);
}

// --- Invoke RPC: the optional recipe-binding seam (R2b) ---------------------

/// A mock [`RecipeBinder`] that binds any request to a single canned Mote.
struct MockBinder;

#[tonic::async_trait]
impl RecipeBinder for MockBinder {
    async fn bind(
        &self,
        _party: &str,
        _handle: &str,
        _args: &[u8],
    ) -> Result<BoundRecipe, BinderError> {
        Ok(BoundRecipe {
            recipe_fingerprint: RECIPE_FP,
            motes: vec![(sample_mote(), sample_warrant())],
            terminal_mote_id: sample_mote().id,
        })
    }
}

#[tokio::test]
async fn invoke_unimplemented_without_a_binder() {
    // The default service wires no binder → Invoke is unimplemented (backward
    // compatible: SubmitRun-only hosts are unaffected).
    let mut client = spawn(service_from(
        build_run(),
        Arc::new(MockSubmitter::default()),
    ))
    .await;
    let err = client
        .invoke(proto::InvokeRequest {
            handle: "ns/coll/name".to_string(),
            args: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unimplemented);
}

#[tokio::test]
async fn invoke_without_a_resolved_party_is_unauthenticated() {
    // Binder wired, but the plain harness injects no CallerParty (no interceptor)
    // → the handler refuses (identity is server-derived; absent ⇒ deny).
    let svc = service_from(build_run(), Arc::new(MockSubmitter::default()))
        .with_recipe_binder(Arc::new(MockBinder));
    let mut client = spawn(svc).await;
    let err = client
        .invoke(proto::InvokeRequest {
            handle: "ns/coll/name".to_string(),
            args: vec![],
        })
        .await
        .unwrap_err();
    assert_eq!(err.code(), Code::Unauthenticated);
}

#[tokio::test]
async fn invoke_dispatches_to_binder_then_proposes() {
    let submitter = Arc::new(MockSubmitter::default());
    let svc = service_from(build_run(), submitter.clone()).with_recipe_binder(Arc::new(MockBinder));
    let mut client = spawn_with_party(svc, "alice@acme").await;

    let resp = client
        .invoke(proto::InvokeRequest {
            handle: "ns/coll/name".to_string(),
            args: vec![],
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(
        resp.terminal_mote_id,
        sample_mote().id.as_bytes().to_vec(),
        "the server-derived terminal Mote is returned (SN-8)"
    );

    // Register-first, then one submit per bound Mote (the propose-proxy order).
    let calls = submitter.calls();
    assert!(calls.first().is_some_and(|c| c.starts_with("register_run")));
    assert_eq!(calls.iter().filter(|c| *c == "submit_mote").count(), 1);
}

// --- UI-2: ListRuns (always available) + the recipe-discovery seam -----------

#[tokio::test]
async fn list_runs_enumerates_the_registered_run_server_derived() {
    // `build_run` records ONE RunRegistered (seq 1) for INSTANCE_ID/RECIPE_FP.
    // ListRuns folds it out of the journal — no seam needed (it uses the reader).
    let mut client = spawn(service_from(build_run(), no_submitter())).await;
    let resp = client
        .list_runs(proto::ListRunsRequest {
            limit: None,
            before_seq: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.runs.len(), 1);
    assert!(!resp.has_more);
    let r = &resp.runs[0];
    assert_eq!(r.instance_id, INSTANCE_ID.to_vec());
    assert_eq!(r.recipe_fingerprint, RECIPE_FP.to_vec());
    assert_eq!(
        r.registered_seq, 1,
        "the RunRegistered fact is the first entry"
    );
}

#[tokio::test]
async fn list_runs_on_an_empty_journal_is_empty_not_an_error() {
    // A journal with no RunRegistered → an empty page (not an oracle/error).
    let mut client = spawn(service_from(build_run(), no_submitter())).await;
    let resp = client
        .list_runs(proto::ListRunsRequest {
            limit: Some(10),
            // No run has seq < 1, so this page is empty.
            before_seq: Some(1),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(resp.runs.is_empty());
    assert!(!resp.has_more);
}

/// A mock [`RecipeCatalog`] exposing one `kx/recipes/demo` handle with a single
/// required `topic` STR field.
struct MockRecipeCatalog;

impl RecipeCatalog for MockRecipeCatalog {
    fn list_recipes(&self) -> Vec<String> {
        vec!["kx/recipes/demo".to_string()]
    }
    fn get_recipe_form(&self, handle: &str) -> Option<Vec<RecipeFormFieldEntry>> {
        (handle == "kx/recipes/demo").then(|| {
            vec![RecipeFormFieldEntry {
                name: "topic".to_string(),
                kind: RecipeParamKind::Str,
                required: true,
                max_len: Some(4096),
                allowed: vec![],
            }]
        })
    }
}

#[tokio::test]
async fn recipe_rpcs_unimplemented_without_a_catalog_seam() {
    // No recipe catalog wired → both recipe RPCs are unimplemented (an old host
    // is unaffected; the SDK degrades to the manual handle+JSON path).
    let mut client = spawn(service_from(build_run(), no_submitter())).await;
    assert_eq!(
        client
            .list_recipes(proto::ListRecipesRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .get_recipe_form(proto::GetRecipeFormRequest {
                handle: "kx/recipes/demo".to_string(),
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
}

#[tokio::test]
async fn recipe_rpcs_dispatch_to_the_catalog_seam() {
    let svc =
        service_from(build_run(), no_submitter()).with_recipe_catalog(Arc::new(MockRecipeCatalog));
    let mut client = spawn(svc).await;

    let list = client
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(list.recipes.len(), 1);
    assert_eq!(list.recipes[0].handle, "kx/recipes/demo");

    let form = client
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: "kx/recipes/demo".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(form.handle, "kx/recipes/demo");
    assert_eq!(form.fields.len(), 1);
    assert_eq!(form.fields[0].name, "topic");
    assert_eq!(form.fields[0].r#type, proto::RecipeParamType::Str as i32);
    assert!(form.fields[0].required);
    assert_eq!(form.fields[0].max_len, Some(4096));

    // A public discovery surface: an unknown handle is `not_found` (NOT the
    // uniform NotAuthorized of the Invoke execution surface).
    let unknown = client
        .get_recipe_form(proto::GetRecipeFormRequest {
            handle: "kx/recipes/nope".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
}

// --- UI-3: the teams (MembershipView) + grants (GrantView) read seams --------

/// A mock [`MembershipView`] exposing one `kx/teams/demo` team with two members
/// (alice = owner-side member; bob = a delegate). `list_members` returns a resolved
/// warrant ONLY when an `asset_ref` is supplied (the resolve-member-warrant toggle).
struct MockMembershipView;

impl MembershipView for MockMembershipView {
    fn list_teams(&self) -> Vec<TeamSummaryEntry> {
        vec![TeamSummaryEntry {
            team_id: "kx/teams/demo".to_string(),
            display_name: "Demo Team".to_string(),
            owner: "kx-gateway".to_string(),
            member_count: 2,
        }]
    }
    fn list_members(&self, team_id: &str, asset_ref: Option<&str>) -> Option<TeamMembersView> {
        if team_id != "kx/teams/demo" {
            return None;
        }
        let warrant = asset_ref.map(|_| WarrantProjection {
            executor_class: "Bwrap".to_string(),
            model_route: "m ×10 (1000/1000 tok)".to_string(),
            net_scope: "None".to_string(),
            fs_scope: String::new(),
            max_calls: 10,
            cpu_milli: 1_000,
            wall_clock_ms: 1_000,
        });
        Some(TeamMembersView {
            owner: "kx-gateway".to_string(),
            members: vec![
                TeamMemberEntry {
                    party: "alice@acme".to_string(),
                    role: "demo-member".to_string(),
                    action_caps: vec!["Read".to_string(), "Use".to_string()],
                    resolved_warrant: warrant.clone(),
                },
                TeamMemberEntry {
                    party: "bob@acme".to_string(),
                    role: "demo-delegate".to_string(),
                    action_caps: vec![
                        "Read".to_string(),
                        "Use".to_string(),
                        "Delegate".to_string(),
                    ],
                    resolved_warrant: warrant,
                },
            ],
        })
    }
}

/// A mock [`GrantView`] exposing one root grant + one revoked delegated grant on
/// `kx/recipes/echo`; any other asset is unknown (`None`).
struct MockGrantView;

impl GrantView for MockGrantView {
    fn list_asset_grants(&self, asset_ref: &str) -> Option<AssetGrantsView> {
        if asset_ref != "kx/recipes/echo" {
            return None;
        }
        Some(AssetGrantsView {
            owner: "kx-gateway".to_string(),
            grants: vec![
                GrantEntry {
                    grantor: "kx-gateway".to_string(),
                    grantee: "alice@acme".to_string(),
                    actions: vec!["Read".to_string(), "Use".to_string()],
                    runtime_scope: "demo".to_string(),
                    is_root: true,
                    revoked: false,
                },
                GrantEntry {
                    grantor: "alice@acme".to_string(),
                    grantee: "bob@acme".to_string(),
                    actions: vec!["Use".to_string()],
                    runtime_scope: "demo".to_string(),
                    is_root: false,
                    revoked: true,
                },
            ],
        })
    }
}

#[tokio::test]
async fn team_and_grant_rpcs_unimplemented_without_seams() {
    // No membership/grant view wired → all three UI-3 RPCs are unimplemented (an old
    // host is unaffected; the SDK degrades to the not-wired empty-state).
    let mut client = spawn(service_from(build_run(), no_submitter())).await;
    assert_eq!(
        client
            .list_teams(proto::ListTeamsRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .list_team_members(proto::ListTeamMembersRequest {
                team_id: "kx/teams/demo".to_string(),
                asset_ref: None,
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .list_asset_grants(proto::ListAssetGrantsRequest {
                asset_ref: "kx/recipes/echo".to_string(),
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
}

#[tokio::test]
async fn dataset_rpcs_unimplemented_without_a_seam() {
    // No DatasetView wired (the `hnsw` feature off / an old host) → all three T3.7
    // RPCs are unimplemented; the SDK/UI degrades to the not-enabled empty-state.
    let mut client = spawn(service_from(build_run(), no_submitter())).await;
    assert_eq!(
        client
            .list_datasets(proto::ListDatasetsRequest {})
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .ingest_documents(proto::IngestDocumentsRequest {
                dataset: "corpus".to_string(),
                documents: vec![proto::IngestDocument {
                    content: b"x".to_vec(),
                    embedding: vec![1.0, 0.0],
                    ..Default::default()
                }],
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
    assert_eq!(
        client
            .query_dataset(proto::QueryDatasetRequest {
                dataset: "corpus".to_string(),
                query_text: String::new(),
                query_embedding: vec![1.0, 0.0],
                k: 1,
            })
            .await
            .unwrap_err()
            .code(),
        Code::Unimplemented,
    );
}

#[tokio::test]
async fn team_rpcs_dispatch_to_the_membership_view() {
    let svc = service_from(build_run(), no_submitter())
        .with_membership_view(Arc::new(MockMembershipView));
    let mut client = spawn(svc).await;

    let teams = client
        .list_teams(proto::ListTeamsRequest {})
        .await
        .unwrap()
        .into_inner();
    assert_eq!(teams.teams.len(), 1);
    assert_eq!(teams.teams[0].team_id, "kx/teams/demo");
    assert_eq!(teams.teams[0].owner, "kx-gateway");
    assert_eq!(teams.teams[0].member_count, 2);

    // Without asset_ref: members + roles, NO resolved warrant.
    let members = client
        .list_team_members(proto::ListTeamMembersRequest {
            team_id: "kx/teams/demo".to_string(),
            asset_ref: None,
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(members.owner, "kx-gateway");
    assert_eq!(members.members.len(), 2);
    let bob = members
        .members
        .iter()
        .find(|m| m.party == "bob@acme")
        .unwrap();
    assert!(
        bob.action_caps.contains(&"Delegate".to_string()),
        "bob is a delegate"
    );
    assert!(
        members.members.iter().all(|m| m.resolved_warrant.is_none()),
        "no asset_ref ⇒ no resolved warrant"
    );

    // With asset_ref: each member carries the resolved-warrant projection.
    let with_asset = client
        .list_team_members(proto::ListTeamMembersRequest {
            team_id: "kx/teams/demo".to_string(),
            asset_ref: Some("kx/recipes/echo".to_string()),
        })
        .await
        .unwrap()
        .into_inner();
    let w = with_asset.members[0].resolved_warrant.as_ref().unwrap();
    assert_eq!(
        w.max_calls, 10,
        "resolve-member-warrant populated with asset_ref"
    );

    // A public viewer surface: an unknown team is `not_found`.
    let unknown = client
        .list_team_members(proto::ListTeamMembersRequest {
            team_id: "kx/teams/nope".to_string(),
            asset_ref: None,
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
}

#[tokio::test]
async fn grant_rpcs_dispatch_to_the_grant_view() {
    let svc = service_from(build_run(), no_submitter()).with_grant_view(Arc::new(MockGrantView));
    let mut client = spawn(svc).await;

    let grants = client
        .list_asset_grants(proto::ListAssetGrantsRequest {
            asset_ref: "kx/recipes/echo".to_string(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(grants.owner, "kx-gateway");
    assert_eq!(grants.grants.len(), 2);
    let root = grants.grants.iter().find(|g| g.is_root).unwrap();
    assert_eq!(root.grantee, "alice@acme");
    assert!(!root.revoked);
    let revoked = grants.grants.iter().find(|g| g.revoked).unwrap();
    assert_eq!(revoked.grantee, "bob@acme");
    assert!(
        !revoked.is_root,
        "the revoked grant is a delegated sub-grant"
    );

    // An unknown asset is `not_found` (a public viewer surface).
    let unknown = client
        .list_asset_grants(proto::ListAssetGrantsRequest {
            asset_ref: "kx/recipes/nope".to_string(),
        })
        .await
        .unwrap_err();
    assert_eq!(unknown.code(), Code::NotFound);
}
