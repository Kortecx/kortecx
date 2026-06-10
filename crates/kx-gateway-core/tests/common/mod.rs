//! Shared fixtures for the gateway-core integration tests: a journal builder, a
//! recording `RunSubmitter` mock, sample domain values, and an in-process tonic
//! server harness hosting `KxGateway`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::pedantic,
    // The `spawn_with_party` interceptor closure returns `Result<_, tonic::Status>`
    // — the type is dictated by tonic's `Interceptor` contract, not chosen here
    // (same justification as `kx-gateway/src/auth.rs`).
    clippy::result_large_err,
    dead_code,
    unreachable_pub
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use kx_content::{ContentRef, ContentStore, InMemoryContentStore};
use kx_gateway_core::{
    CallerParty, ContentReader, GatewayService, JournalReader, ReadOnly, RunSubmitter,
    SubmitMoteOutcome, SubmitStatus, SubmitterError,
};
use kx_journal::{InMemoryJournal, Journal, JournalEntry, ParentEntry};
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, GraphPosition, InferenceParams, InputDataId,
    LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash,
    MOTE_DEF_SCHEMA_VERSION,
};
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use kx_proto::proto::kx_gateway_server::KxGatewayServer;
use kx_warrant::{
    ExecutorClass, FsScope, ModelRoute, MoteClass, NetScope, ResourceCeiling, WarrantSpec,
};
use tonic::transport::{Channel, Server};

/// A fixed 16-byte run instance id for tests.
pub const INSTANCE_ID: [u8; 16] = [0x11; 16];
/// A fixed 32-byte recipe fingerprint for tests.
pub const RECIPE_FP: [u8; 32] = [0x22; 32];

/// A recording [`RunSubmitter`] mock: records the order of `register_run` /
/// `submit_mote` calls and returns canned, well-formed outcomes.
#[derive(Clone, Default)]
pub struct MockSubmitter {
    pub calls: Arc<Mutex<Vec<String>>>,
}

impl MockSubmitter {
    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
}

#[tonic::async_trait]
impl RunSubmitter for MockSubmitter {
    async fn register_run(&self, recipe_fingerprint: [u8; 32]) -> Result<[u8; 16], SubmitterError> {
        self.calls
            .lock()
            .unwrap()
            .push(format!("register_run({})", recipe_fingerprint[0]));
        Ok(INSTANCE_ID)
    }

    async fn submit_mote(
        &self,
        mote: Mote,
        _warrant: WarrantSpec,
        _accept_at_least_once: bool,
        react_seed: bool,
    ) -> Result<SubmitMoteOutcome, SubmitterError> {
        // Record the react flag so the PR-2d-2 Invoke-forwards-react_seed test
        // can assert it without a second mock.
        self.calls.lock().unwrap().push(if react_seed {
            "submit_mote(react_seed)".to_string()
        } else {
            "submit_mote".to_string()
        });
        Ok(SubmitMoteOutcome {
            // The re-derived identity (the wire advisory id was discarded at
            // TryFrom) — the SN-8 property at the submit boundary.
            mote_id: *mote.id.as_bytes(),
            instance_id: INSTANCE_ID,
            status: SubmitStatus::Accepted,
        })
    }
}

/// Build a [`MoteDef`] keyed by `tag` (so distinct tags hash distinctly).
#[must_use]
pub fn mote_def(tag: u8, nd: NdClass) -> MoteDef {
    let mut config_subset = BTreeMap::new();
    config_subset.insert(ConfigKey("tag".into()), ConfigVal(vec![tag]));
    MoteDef {
        critic_check: None,
        logic_ref: LogicRef::from_bytes([7u8; 32]),
        model_id: ModelId("test-model".into()),
        prompt_template_hash: PromptTemplateHash::from_bytes([9u8; 32]),
        tool_contract: BTreeMap::new(),
        nd_class: nd,
        config_subset,
        effect_pattern: EffectPattern::IdempotentByConstruction,
        critic_for: None,
        is_topology_shaper: false,
        inference_params: InferenceParams::default(),
        schema_version: MOTE_DEF_SCHEMA_VERSION,
    }
}

/// A representative valid warrant (used for the SubmitRun proxy test).
#[must_use]
pub fn sample_warrant() -> WarrantSpec {
    WarrantSpec {
        mote_class: MoteClass::Pure,
        nd_class: MoteClass::Pure,
        fs_scope: FsScope::default(),
        net_scope: NetScope::None,
        syscall_profile_ref: ContentRef::from_bytes([4u8; 32]),
        tool_grants: std::collections::BTreeSet::new(),
        model_route: ModelRoute {
            model_id: ModelId("test-model".into()),
            max_input_tokens: 4_096,
            max_output_tokens: 512,
            max_calls: 3,
        },
        resource_ceiling: ResourceCeiling {
            cpu_milli: 1_000,
            mem_bytes: 1 << 30,
            wall_clock_ms: 30_000,
            fd_count: 64,
            disk_bytes: 1 << 28,
        },
        environment_ref: None,
        executor_class: ExecutorClass::Bwrap,
        ..Default::default()
    }
}

/// A representative `Mote` (a leaf Pure mote) for the SubmitRun proxy test.
#[must_use]
pub fn sample_mote() -> Mote {
    Mote::new(
        mote_def(0xAB, NdClass::Pure),
        InputDataId::from_bytes([5u8; 32]),
        GraphPosition(vec![0]),
        smallvec::SmallVec::<[ParentRef; 4]>::new(),
    )
}

/// A small committed run: RunRegistered, then A (committed, leaf), B (committed,
/// data-child of A), C (proposed → Scheduled). Returns the journal, a content
/// store holding A's + B's result bytes, and the mote ids + A's result ref.
pub struct BuiltRun {
    pub journal: InMemoryJournal,
    pub content: InMemoryContentStore,
    pub a: MoteId,
    pub b: MoteId,
    pub c: MoteId,
    pub a_ref: ContentRef,
}

#[must_use]
pub fn build_run() -> BuiltRun {
    let journal = InMemoryJournal::new();
    let content = InMemoryContentStore::new();

    let a = MoteId::from_bytes([0xA1; 32]);
    let b = MoteId::from_bytes([0xB2; 32]);
    let c = MoteId::from_bytes([0xC3; 32]);
    let a_ref = content.put(b"result-of-A").unwrap();
    let b_ref = content.put(b"result-of-B").unwrap();

    // seq 1 — run registration.
    journal
        .append(JournalEntry::RunRegistered {
            instance_id: INSTANCE_ID,
            recipe_fingerprint: RECIPE_FP,
            ts: 0,
            seq: 0,
        })
        .unwrap();
    // seq 2 — A committed (leaf).
    journal
        .append(JournalEntry::Committed {
            mote_id: a,
            idempotency_key: [0xA1; 32],
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref: a_ref,
            parents: smallvec::SmallVec::new(),
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: mote_def(0x01, NdClass::Pure).hash(),
        })
        .unwrap();
    // seq 3 — B committed (data-child of A).
    let mut b_parents = smallvec::SmallVec::<[ParentEntry; 4]>::new();
    b_parents.push(ParentEntry::from_parent_ref(&ParentRef {
        parent_id: a,
        edge: EdgeMeta::data(),
    }));
    journal
        .append(JournalEntry::Committed {
            mote_id: b,
            idempotency_key: [0xB2; 32],
            seq: 0,
            nondeterminism: NdClass::Pure,
            result_ref: b_ref,
            parents: b_parents,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
            mote_def_hash: mote_def(0x02, NdClass::Pure).hash(),
        })
        .unwrap();
    // seq 4 — C proposed (Scheduled).
    journal
        .append(JournalEntry::Proposed {
            mote_id: c,
            idempotency_key: [0xC3; 32],
            seq: 0,
            nondeterminism: NdClass::Pure,
            placement_hint: 0,
            warrant_ref: ContentRef::from_bytes([0xaa; 32]),
        })
        .unwrap();

    BuiltRun {
        journal,
        content,
        a,
        b,
        c,
        a_ref,
    }
}

/// Wrap a built run + a submitter into a [`GatewayService`].
#[must_use]
pub fn service_from(run: BuiltRun, submitter: Arc<dyn RunSubmitter>) -> GatewayService {
    let reader: Arc<dyn JournalReader> = Arc::new(ReadOnly::new(run.journal));
    let content: Arc<dyn ContentReader> = Arc::new(run.content);
    GatewayService::new(reader, submitter, content)
}

/// Spawn `service` on an ephemeral local port and return a connected client.
pub async fn spawn(service: GatewayService) -> KxGatewayClient<Channel> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    tokio::spawn(async move {
        Server::builder()
            .add_service(KxGatewayServer::new(service))
            .serve(addr)
            .await
            .unwrap();
    });

    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(client) = KxGatewayClient::connect(endpoint.clone()).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client failed to connect to the gateway server");
}

/// Spawn `service` behind an interceptor that stamps every request with a
/// server-derived [`CallerParty`] — the test stand-in for the host's auth
/// interceptor — so the `Invoke` handler can resolve identity.
pub async fn spawn_with_party(service: GatewayService, party: &str) -> KxGatewayClient<Channel> {
    let party = party.to_string();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    drop(listener);

    tokio::spawn(async move {
        let svc = KxGatewayServer::with_interceptor(service, move |mut req: tonic::Request<()>| {
            req.extensions_mut().insert(CallerParty(party.clone()));
            Ok(req)
        });
        Server::builder()
            .add_service(svc)
            .serve(addr)
            .await
            .unwrap();
    });

    let endpoint = format!("http://{addr}");
    for _ in 0..100 {
        if let Ok(client) = KxGatewayClient::connect(endpoint.clone()).await {
            return client;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("client failed to connect to the gateway server");
}
