//! Every entity catalog save reserves its handle against the others.
//!
//! ## The failure being prevented is an ABSENCE
//!
//! Branches, locks and branch HISTORY are keyed `(principal, handle)` with no
//! entity axis, so two entities sharing a handle share one project branch, one
//! lock and one history. The refusal that stops this shipped on `SaveWorkflow`
//! ALONE, with a comment saying "SaveApp stays untouched" — which left the
//! collision fully reachable from the other side. Save the App second and
//! nothing objects.
//!
//! No behavioural test of the saves that DO reserve can see the one that does
//! not, which is why the structural half below is a SOURCE SCAN — the same shape
//! `sidecar_policy.rs` uses, and for the same reason. A new handle-owning
//! catalog added without a reservation is an omission, and an omission is
//! invisible to tests of what is present.
//!
//! The behavioural half proves the reservation actually refuses, in BOTH
//! directions, so the source scan cannot be satisfied by a call that does
//! nothing.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn service_source() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/service.rs");
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Strip `#[cfg(test)] mod tests { ... }` so a test fixture cannot satisfy — or
/// trip — a production-code rule.
fn production(src: &str) -> &str {
    match src.find("\nmod tests {") {
        Some(i) => &src[..i],
        None => src,
    }
}

/// Extract a handler body by name, up to the next `    async fn ` at the same
/// indentation.
fn handler<'a>(src: &'a str, name: &str) -> &'a str {
    let needle = format!("    async fn {name}(");
    let start = src
        .find(&needle)
        .unwrap_or_else(|| panic!("service.rs declares {name}"));
    let rest = &src[start + needle.len()..];
    let end = rest.find("\n    async fn ").unwrap_or(rest.len());
    &rest[..end]
}

/// Every handler that writes into an ENTITY catalog calls `reserve_handle`.
///
/// The list is explicit rather than derived: deriving "which handlers write an
/// entity" from the source would be a heuristic, and a heuristic that silently
/// stopped matching would take the guard with it. A fourth entity catalog is a
/// deliberate edit here — which is the review trigger.
#[test]
fn every_entity_catalog_save_reserves_its_handle() {
    const ENTITY_SAVES: &[&str] = &["save_app", "save_workflow", "put_context_bundle"];

    let src = service_source();
    let src = production(&src);
    let missing: Vec<&str> = ENTITY_SAVES
        .iter()
        .copied()
        .filter(|name| !handler(src, name).contains("self.reserve_handle("))
        .collect();

    assert!(
        missing.is_empty(),
        "these entity-catalog saves do not reserve their handle: {missing:?}\n\n\
         Branches, locks and branch history are keyed (principal, handle) with no \
         entity axis, so two entities sharing a handle share one project branch, one \
         lock and one point-in-time history. A reservation on only SOME of the saves \
         makes the refusal depend on which save happens first, which is a race the \
         user loses silently rather than a rule."
    );
}

/// The reservation covers every OTHER entity, not just the first one thought of.
///
/// This is the assertion that would have caught the original defect: the shipped
/// check named `self.apps` and nothing else, so it was structurally incapable of
/// noticing a workflow/bundle collision.
#[test]
fn the_reservation_consults_every_entity_catalog() {
    let src = service_source();
    let src = production(&src);
    let start = src
        .find("fn reserve_handle(")
        .expect("service.rs declares reserve_handle");
    let rest = &src[start..];
    let end = rest
        .find("\n    /// Refuse")
        .or_else(|| rest.find("\n    #[allow"))
        .unwrap_or_else(|| rest.len().min(4000));
    let body = &rest[..end];

    for catalog in ["self.apps", "self.workflows", "self.bundles"] {
        assert!(
            body.contains(catalog),
            "reserve_handle never consults {catalog} — the handle space it protects \
             includes that catalog, so a collision with it would go unnoticed"
        );
    }
}

/// Anti-vacuity for both scans above: the helpers actually read something.
///
/// A `handler()` that silently returned an empty slice would make
/// `every_entity_catalog_save_reserves_its_handle` fail loudly, but a
/// `service_source()` that returned "" would make
/// `the_reservation_consults_every_entity_catalog` panic on the `.expect` —
/// so the risk is the opposite direction: a scan that matches too much.
#[test]
fn the_scan_discriminates() {
    let src = service_source();
    assert!(
        src.len() > 100_000,
        "service.rs read as {} bytes — the scan is not reading the real file",
        src.len()
    );
    let src = production(&src);

    // A handler that is NOT an entity save must not contain the call, or the
    // scan above would pass for the wrong reason.
    assert!(
        !handler(src, "list_workflows").contains("self.reserve_handle("),
        "list_workflows is a read and must not reserve — if it does, the scan \
         above cannot distinguish a real reservation from an incidental match"
    );
    // And the extraction really does isolate one handler.
    let save_app = handler(src, "save_app");
    assert!(
        save_app.contains("SaveApp"),
        "handler() found the right body"
    );
    assert!(
        !save_app.contains("async fn save_workflow"),
        "handler() must stop at the next handler, got {} bytes",
        save_app.len()
    );
    let _ = Path::new("");
    let _: BTreeSet<&str> = BTreeSet::new();
}

// ---------------------------------------------------------------------------
// The behavioural half.
//
// The source scan above proves a reservation is CALLED. It cannot prove the
// call refuses anything — a `reserve_handle` that returned `Ok(())`
// unconditionally would satisfy every assertion so far. These tests drive the
// real RPCs over a real transport and assert the refusal fires in BOTH
// directions, which is the property the one-directional original lacked.
// ---------------------------------------------------------------------------

mod common;

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use common::{build_run, spawn_with_party, MockSubmitter};
use kx_gateway_core::{
    AppCatalog, AppRecord, BundleItemRecord, BundleManifest, BundleStore, GatewayError,
    GatewayService, JournalReader, ReadOnly, RunSubmitter, WorkflowCatalog, WorkflowRecord,
};
use kx_proto::proto;

const PARTY: &str = "alice@acme";

/// An in-memory App catalog. Only `get` matters to the reservation; the rest is
/// the minimum that satisfies the trait honestly.
#[derive(Default)]
struct MemApps(Mutex<BTreeMap<(String, String), Vec<u8>>>);

impl AppCatalog for MemApps {
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
        _source_digest: Option<&[u8]>,
    ) -> Result<(AppRecord, bool), GatewayError> {
        self.0
            .lock()
            .unwrap()
            .insert((principal.into(), handle.into()), envelope_json.to_vec());
        Ok((rec_app(handle), false))
    }
    fn list(
        &self,
        _p: &str,
        _l: usize,
        _a: Option<&str>,
    ) -> Result<(Vec<AppRecord>, bool), GatewayError> {
        Ok((Vec::new(), false))
    }
    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(AppRecord, Vec<u8>)>, GatewayError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(principal.into(), handle.into()))
            .map(|b| (rec_app(handle), b.clone())))
    }
    fn delete(&self, principal: &str, handle: &str) -> Result<bool, GatewayError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .remove(&(principal.into(), handle.into()))
            .is_some())
    }
    fn set_lifecycle(&self, _p: &str, _h: &str, _l: &str) -> Result<bool, GatewayError> {
        Ok(false)
    }
}

fn rec_app(handle: &str) -> AppRecord {
    AppRecord {
        app_ref: [1u8; 16],
        handle: handle.into(),
        name: "a".into(),
        version: "1".into(),
        description: String::new(),
        delivers: String::new(),
        tags: Vec::new(),
        step_count: 0,
        source_digest: None,
        kind: String::new(),
        mode: String::new(),
        lifecycle: String::new(),
    }
}

#[derive(Default)]
struct MemWorkflows(Mutex<BTreeMap<(String, String), Vec<u8>>>);

impl WorkflowCatalog for MemWorkflows {
    fn save(
        &self,
        principal: &str,
        handle: &str,
        envelope_json: &[u8],
        _source_digest: Option<&[u8]>,
        _lifecycle: &str,
    ) -> Result<(WorkflowRecord, bool), GatewayError> {
        self.0
            .lock()
            .unwrap()
            .insert((principal.into(), handle.into()), envelope_json.to_vec());
        Ok((rec_wf(handle), false))
    }
    fn list(
        &self,
        _p: &str,
        _l: usize,
        _a: Option<&str>,
    ) -> Result<(Vec<WorkflowRecord>, bool), GatewayError> {
        Ok((Vec::new(), false))
    }
    fn get(
        &self,
        principal: &str,
        handle: &str,
    ) -> Result<Option<(WorkflowRecord, Vec<u8>)>, GatewayError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(principal.into(), handle.into()))
            .map(|b| (rec_wf(handle), b.clone())))
    }
    fn delete(&self, principal: &str, handle: &str) -> Result<bool, GatewayError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .remove(&(principal.into(), handle.into()))
            .is_some())
    }
    fn set_lifecycle(&self, _p: &str, _h: &str, _l: &str) -> Result<bool, GatewayError> {
        Ok(false)
    }
}

fn rec_wf(handle: &str) -> WorkflowRecord {
    WorkflowRecord {
        workflow_ref: [2u8; 16],
        handle: handle.into(),
        name: "w".into(),
        version: "1".into(),
        description: String::new(),
        delivers: String::new(),
        tags: Vec::new(),
        step_count: 0,
        source_digest: None,
        lifecycle: String::new(),
    }
}

#[derive(Default)]
struct MemBundles(Mutex<BTreeMap<(String, String), BundleManifest>>);

impl BundleStore for MemBundles {
    fn upsert(
        &self,
        principal: &str,
        handle: &str,
        description: &str,
        items: &[BundleItemRecord],
    ) -> Result<([u8; 16], bool), GatewayError> {
        self.0.lock().unwrap().insert(
            (principal.into(), handle.into()),
            BundleManifest {
                bundle_ref: [3u8; 16],
                handle: handle.into(),
                description: description.into(),
                items: items.to_vec(),
            },
        );
        Ok(([3u8; 16], false))
    }
    fn get(&self, principal: &str, handle: &str) -> Result<Option<BundleManifest>, GatewayError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(principal.into(), handle.into()))
            .cloned())
    }
    fn list(
        &self,
        _p: &str,
        _l: usize,
        _a: Option<&str>,
    ) -> Result<(Vec<BundleManifest>, bool), GatewayError> {
        Ok((Vec::new(), false))
    }
    fn delete(&self, principal: &str, handle: &str) -> Result<bool, GatewayError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .remove(&(principal.into(), handle.into()))
            .is_some())
    }
}

fn all_three() -> GatewayService {
    let run = build_run();
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let submitter: Arc<dyn RunSubmitter> = Arc::new(MockSubmitter::default());
    GatewayService::new(reader, submitter, Arc::new(run.content))
        .with_apps_catalog(Arc::new(MemApps::default()))
        .with_workflow_catalog(Arc::new(MemWorkflows::default()))
        .with_bundles_store(Arc::new(MemBundles::default()))
}

/// A minimal valid `kortecx.app/v1` envelope is not needed: the reservation runs
/// BEFORE envelope validation, so a save that collides is refused for the handle
/// rather than for its bytes. That ordering is itself the assertion — a caller
/// gets the actionable message, not a schema complaint.
fn app_req(handle: &str) -> proto::SaveAppRequest {
    proto::SaveAppRequest {
        handle: handle.into(),
        envelope_json: br#"{"schema":"kortecx.app/v1"}"#.to_vec(),
        ..Default::default()
    }
}

fn wf_req(handle: &str) -> proto::SaveWorkflowRequest {
    proto::SaveWorkflowRequest {
        handle: handle.into(),
        envelope_json: br#"{"schema":"kortecx.workflow/v1"}"#.to_vec(),
        ..Default::default()
    }
}

/// THE regression: saving the App SECOND must be refused too.
///
/// The original check lived only on `SaveWorkflow`, so this exact order — the
/// one a user hits by creating a workflow first — went through silently and the
/// two entities began sharing a branch, a lock and a history.
#[tokio::test]
async fn an_app_cannot_take_a_handle_a_workflow_already_holds() {
    let mut c = spawn_with_party(all_three(), PARTY).await;
    c.save_workflow(wf_req("ns/coll/shared")).await.unwrap();

    let err = c.save_app(app_req("ns/coll/shared")).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("names a workflow"),
        "the refusal must name what already holds the handle, got {:?}",
        err.message()
    );
}

/// The direction that already worked keeps working.
#[tokio::test]
async fn a_workflow_cannot_take_a_handle_an_app_already_holds() {
    let mut c = spawn_with_party(all_three(), PARTY).await;
    c.save_app(app_req("ns/coll/shared")).await.unwrap();

    let err = c.save_workflow(wf_req("ns/coll/shared")).await.unwrap_err();
    assert_eq!(err.code(), tonic::Code::InvalidArgument);
    assert!(
        err.message().contains("names an App"),
        "{:?}",
        err.message()
    );
}

/// The third catalog is really in the space, both ways.
#[tokio::test]
async fn a_bundle_and_an_app_cannot_share_a_handle() {
    let mut c = spawn_with_party(all_three(), PARTY).await;
    c.save_app(app_req("ns/coll/b")).await.unwrap();
    let err = c
        .put_context_bundle(proto::PutContextBundleRequest {
            handle: "ns/coll/b".into(),
            description: "d".into(),
            items: vec![proto::ContextItem {
                content_ref: vec![9u8; 32],
                ..Default::default()
            }],
        })
        .await
        .unwrap_err();
    assert!(
        err.message().contains("names an App"),
        "{:?}",
        err.message()
    );
}

/// Re-saving the SAME entity at its own handle is not a collision.
///
/// Without the self-exclusion the reservation would refuse every update, which
/// is the way a guard like this most easily becomes a bug.
#[tokio::test]
async fn re_saving_an_entity_at_its_own_handle_is_allowed() {
    let mut c = spawn_with_party(all_three(), PARTY).await;
    c.save_app(app_req("ns/coll/mine")).await.unwrap();
    c.save_app(app_req("ns/coll/mine"))
        .await
        .expect("an App may be re-saved at its own handle");
    c.save_workflow(wf_req("ns/coll/other")).await.unwrap();
    c.save_workflow(wf_req("ns/coll/other"))
        .await
        .expect("a workflow may be re-saved at its own handle");
}

/// Distinct handles are unaffected — the anti-vacuity floor for every refusal
/// above.
#[tokio::test]
async fn distinct_handles_do_not_collide() {
    let mut c = spawn_with_party(all_three(), PARTY).await;
    c.save_app(app_req("ns/coll/one")).await.unwrap();
    c.save_workflow(wf_req("ns/coll/two"))
        .await
        .expect("a different handle is not a collision");
}
