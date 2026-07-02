//! RC-SW1 skill-catalog RPCs over a real tonic transport: `AddSkill` (the
//! content-write seam mints the instructions ref — SN-8), `ListSkills` (paging),
//! `GetSkillForm` (the ADVISORY `registered` wish bit + preview), `RemoveSkill`,
//! and the honest degrades (`unimplemented` without the seam / without the
//! content-write seam when a body rides the request).
//!
//! The load-bearing assertions: the caps fail closed BEFORE the catalog or the
//! content store is touched, the `registered` bit is display-only truth from the
//! same fireable set the admission backstops use, and not-found is UNIFORM.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::BTreeMap;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc, Mutex,
};

use common::{build_run, service_from, spawn_with_party, MockSubmitter};
use kx_content::{ContentStore, InMemoryContentStore};
use kx_gateway_core::{
    AddedInstructions, ContentWriter, GatewayError, SkillCatalog, SkillRecord,
    MAX_SKILL_INSTRUCTIONS_BODY_BYTES, MAX_SKILL_MANIFEST_BYTES, SKILL_PREVIEW_CAP_BYTES,
};
use kx_proto::proto;
use tonic::Code;

/// An in-memory [`SkillCatalog`] fake (the host's `skills.db` stand-in). It
/// derives the record from the raw manifest JSON just enough for the handler
/// mapping to be observable; call counting pins "caps fire BEFORE the store".
#[derive(Default)]
struct MemSkills {
    rows: Mutex<BTreeMap<(String, String), SkillRecord>>,
    add_calls: AtomicUsize,
}

impl SkillCatalog for MemSkills {
    fn add(
        &self,
        principal: &str,
        manifest_json: &[u8],
        instructions: Option<AddedInstructions>,
    ) -> Result<(SkillRecord, bool), GatewayError> {
        self.add_calls.fetch_add(1, Ordering::SeqCst);
        let v: serde_json::Value = serde_json::from_slice(manifest_json)
            .map_err(|_| GatewayError::InvalidArgument("bad manifest json"))?;
        let name = v["name"].as_str().unwrap_or_default().to_string();
        if name.is_empty() {
            return Err(GatewayError::InvalidArgument("manifest carries no name"));
        }
        let (instructions_ref, preview, truncated) = match instructions {
            Some(a) => (hex(&a.content_ref), a.preview, a.truncated),
            None => {
                let r = v["instructions_ref"].as_str().unwrap_or_default();
                if r.len() != 64 {
                    return Err(GatewayError::InvalidArgument(
                        "manifest names no instructions_ref and no body was stored",
                    ));
                }
                (r.to_string(), String::new(), false)
            }
        };
        let mut tools: BTreeMap<String, String> = BTreeMap::new();
        if let Some(o) = v["tools"].as_object() {
            for (k, val) in o {
                tools.insert(k.clone(), val.as_str().unwrap_or_default().to_string());
            }
        }
        let record = SkillRecord {
            skill_ref: [0xab; 16],
            name: name.clone(),
            version: v["version"].as_str().unwrap_or("1").to_string(),
            description: v["description"].as_str().unwrap_or_default().to_string(),
            tags: Vec::new(),
            instructions_ref,
            tools,
            instructions_preview: preview,
            preview_truncated: truncated,
        };
        let mut rows = self.rows.lock().unwrap();
        let key = (principal.to_string(), name);
        let dedup = rows.get(&key) == Some(&record);
        rows.insert(key, record.clone());
        Ok((record, dedup))
    }

    fn list(
        &self,
        principal: &str,
        limit: usize,
        after_name: Option<&str>,
    ) -> Result<(Vec<SkillRecord>, bool), GatewayError> {
        let rows = self.rows.lock().unwrap();
        let mut all: Vec<SkillRecord> = rows
            .iter()
            .filter(|((p, name), _)| {
                p == principal && after_name.is_none_or(|after| name.as_str() > after)
            })
            .map(|(_, r)| r.clone())
            .collect();
        all.sort_by(|a, b| a.name.cmp(&b.name));
        let has_more = all.len() > limit;
        all.truncate(limit);
        Ok((all, has_more))
    }

    fn get(&self, principal: &str, name: &str) -> Result<Option<SkillRecord>, GatewayError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .get(&(principal.to_string(), name.to_string()))
            .cloned())
    }

    fn remove(&self, principal: &str, name: &str) -> Result<bool, GatewayError> {
        Ok(self
            .rows
            .lock()
            .unwrap()
            .remove(&(principal.to_string(), name.to_string()))
            .is_some())
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn manifest(name: &str) -> Vec<u8> {
    format!(
        r#"{{"schema":"kortecx.skill/v1","name":"{name}","version":"1","description":"d","tools":{{"mcp-echo/echo":"1","gmail/search":"1"}}}}"#
    )
    .into_bytes()
}

struct Rig {
    client: kx_proto::proto::kx_gateway_client::KxGatewayClient<tonic::transport::Channel>,
    skills: Arc<MemSkills>,
    store: Arc<InMemoryContentStore>,
}

async fn rig() -> Rig {
    let run = build_run();
    let skills = Arc::new(MemSkills::default());
    let store = Arc::new(InMemoryContentStore::new());
    let service = service_from(run, Arc::new(MockSubmitter::default()))
        .with_skill_catalog(skills.clone())
        .with_content_writer(store.clone() as Arc<dyn ContentWriter>)
        .with_registered_tools(
            [("mcp-echo/echo".to_string(), "1".to_string())]
                .into_iter()
                .collect(),
        );
    let client = spawn_with_party(service, "alice").await;
    Rig {
        client,
        skills,
        store,
    }
}

#[tokio::test]
async fn without_the_seam_all_four_rpcs_are_unimplemented() {
    let run = build_run();
    let service = service_from(run, Arc::new(MockSubmitter::default()));
    let mut client = spawn_with_party(service, "alice").await;
    let add = client
        .add_skill(proto::AddSkillRequest {
            manifest_json: manifest("s"),
            instructions_body: b"body".to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(add.code(), Code::Unimplemented);
    let list = client
        .list_skills(proto::ListSkillsRequest::default())
        .await
        .unwrap_err();
    assert_eq!(list.code(), Code::Unimplemented);
    let form = client
        .get_skill_form(proto::GetSkillFormRequest { name: "s".into() })
        .await
        .unwrap_err();
    assert_eq!(form.code(), Code::Unimplemented);
    let rm = client
        .remove_skill(proto::RemoveSkillRequest { name: "s".into() })
        .await
        .unwrap_err();
    assert_eq!(rm.code(), Code::Unimplemented);
}

#[tokio::test]
async fn add_with_a_body_mints_the_ref_via_the_content_write_seam() {
    let mut r = rig().await;
    let resp = r
        .client
        .add_skill(proto::AddSkillRequest {
            manifest_json: manifest("email-triage"),
            instructions_body: b"# Triage\nSearch first.".to_vec(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(resp.name, "email-triage");
    assert_eq!(resp.skill_ref.len(), 16);
    assert_eq!(resp.instructions_ref.len(), 64);
    assert!(!resp.deduplicated);
    // SN-8: the ref is server-derived over the exact body bytes.
    let expected = kx_content::ContentRef::of(b"# Triage\nSearch first.");
    assert_eq!(resp.instructions_ref, hex(&expected.0));
    assert!(r.store.contains(&expected));
}

#[tokio::test]
async fn caps_fail_closed_before_the_store_or_catalog_is_touched() {
    let mut r = rig().await;
    let huge_manifest = vec![b'x'; MAX_SKILL_MANIFEST_BYTES + 1];
    let e = r
        .client
        .add_skill(proto::AddSkillRequest {
            manifest_json: huge_manifest,
            instructions_body: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let huge_body = vec![b'y'; MAX_SKILL_INSTRUCTIONS_BODY_BYTES + 1];
    let e = r
        .client
        .add_skill(proto::AddSkillRequest {
            manifest_json: manifest("s"),
            instructions_body: huge_body,
        })
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    let e = r
        .client
        .add_skill(proto::AddSkillRequest {
            manifest_json: Vec::new(),
            instructions_body: Vec::new(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::InvalidArgument);
    // Neither the catalog nor the store was ever touched.
    assert_eq!(r.skills.add_calls.load(Ordering::SeqCst), 0);
    let oversized_ref =
        kx_content::ContentRef::of(&vec![b'y'; MAX_SKILL_INSTRUCTIONS_BODY_BYTES + 1]);
    assert!(!r.store.contains(&oversized_ref));
}

#[tokio::test]
async fn a_body_without_the_content_write_seam_is_unimplemented() {
    let run = build_run();
    let skills = Arc::new(MemSkills::default());
    let service =
        service_from(run, Arc::new(MockSubmitter::default())).with_skill_catalog(skills.clone());
    let mut client = spawn_with_party(service, "alice").await;
    let e = client
        .add_skill(proto::AddSkillRequest {
            manifest_json: manifest("s"),
            instructions_body: b"body".to_vec(),
        })
        .await
        .unwrap_err();
    assert_eq!(e.code(), Code::Unimplemented);
    // A ref-only add (STORED form) still works without the write seam.
    let stored = format!(
        r#"{{"schema":"kortecx.skill/v1","name":"s","instructions_ref":"{}"}}"#,
        "a".repeat(64)
    );
    let ok = client
        .add_skill(proto::AddSkillRequest {
            manifest_json: stored.into_bytes(),
            instructions_body: Vec::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(ok.instructions_ref, "a".repeat(64));
}

#[tokio::test]
async fn list_pages_deterministically_and_form_enriches_the_registered_bit() {
    let mut r = rig().await;
    for name in ["alpha", "beta", "gamma"] {
        r.client
            .add_skill(proto::AddSkillRequest {
                manifest_json: manifest(name),
                instructions_body: format!("# {name}").into_bytes(),
            })
            .await
            .unwrap();
    }
    let page = r
        .client
        .list_skills(proto::ListSkillsRequest {
            limit: 2,
            after_name: String::new(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(
        page.skills
            .iter()
            .map(|s| s.name.as_str())
            .collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert!(page.has_more);
    let rest = r
        .client
        .list_skills(proto::ListSkillsRequest {
            limit: 2,
            after_name: "beta".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert_eq!(rest.skills.len(), 1);
    assert!(!rest.has_more);

    // The registered bit is ADVISORY truth from the fireable set: mcp-echo/echo
    // is registered on this rig, gmail/search is not.
    let form = r
        .client
        .get_skill_form(proto::GetSkillFormRequest {
            name: "alpha".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(form.found);
    let bits: BTreeMap<String, bool> = form
        .wishes
        .iter()
        .map(|w| (w.tool_id.clone(), w.registered))
        .collect();
    assert!(bits["mcp-echo/echo"]);
    assert!(!bits["gmail/search"]);
    assert_eq!(form.instructions_preview, "# alpha");
    assert!(!form.preview_truncated);
}

#[tokio::test]
async fn form_preview_truncates_at_the_cap_and_not_found_is_uniform() {
    let mut r = rig().await;
    let long_body = "z".repeat(SKILL_PREVIEW_CAP_BYTES + 100);
    r.client
        .add_skill(proto::AddSkillRequest {
            manifest_json: manifest("long"),
            instructions_body: long_body.into_bytes(),
        })
        .await
        .unwrap();
    let form = r
        .client
        .get_skill_form(proto::GetSkillFormRequest {
            name: "long".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(form.preview_truncated);
    assert_eq!(form.instructions_preview.len(), SKILL_PREVIEW_CAP_BYTES);

    // Uniform not-found: absent and (via a second party) not-owned look identical.
    let absent = r
        .client
        .get_skill_form(proto::GetSkillFormRequest {
            name: "no-such".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!absent.found);
    assert!(absent.summary.is_none());
    let removed = r
        .client
        .remove_skill(proto::RemoveSkillRequest {
            name: "no-such".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!removed.removed);
}

#[tokio::test]
async fn skills_are_caller_scoped_across_parties() {
    // Two parties over the SAME catalog: bob never sees alice's skill.
    let run = build_run();
    let skills = Arc::new(MemSkills::default());
    let store = Arc::new(InMemoryContentStore::new());
    let service = service_from(run, Arc::new(MockSubmitter::default()))
        .with_skill_catalog(skills.clone())
        .with_content_writer(store as Arc<dyn ContentWriter>);
    let mut alice = spawn_with_party(service, "alice").await;
    alice
        .add_skill(proto::AddSkillRequest {
            manifest_json: manifest("private-skill"),
            instructions_body: b"secret sauce".to_vec(),
        })
        .await
        .unwrap();

    let run2 = build_run();
    let service_bob =
        service_from(run2, Arc::new(MockSubmitter::default())).with_skill_catalog(skills.clone());
    let mut bob = spawn_with_party(service_bob, "bob").await;
    let form = bob
        .get_skill_form(proto::GetSkillFormRequest {
            name: "private-skill".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!form.found, "cross-party read must be uniform not-found");
    let removed = bob
        .remove_skill(proto::RemoveSkillRequest {
            name: "private-skill".into(),
        })
        .await
        .unwrap()
        .into_inner();
    assert!(!removed.removed, "cross-party remove must be a uniform no");
}
