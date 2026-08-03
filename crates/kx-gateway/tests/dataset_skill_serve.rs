//! Live DATASETS + SKILLS witnesses — the two holes the RPC-level suites cannot reach.
//!
//! The dataset and skill RPC surfaces are already covered model-free (`datasets_e2e.rs`,
//! `kx-gateway-core/tests/skills_batch.rs`) and the skill-BIND is covered at the authoring
//! level (`app_run.rs`). What no test anywhere asserts is what those capabilities are FOR:
//!
//!   1. **GROUNDEDNESS.** `react_rag_serve.rs` proves the model FIRED `retrieve` and that
//!      the chain answered. It never proves the answer USED what came back. A run that
//!      searches, ignores the result and confabulates satisfies every assertion there.
//!      Here the target passage carries a nonce that exists nowhere else — not in the
//!      question, not in the sibling documents, not in any pretraining corpus — so the
//!      nonce reaching the ANSWER turn is only possible by way of the retrieval.
//!   2. **STEERING.** Nothing proves a skill changes what a live model DOES. The skill
//!      here carries an org-private rule the model cannot otherwise know, and the two arms
//!      differ by exactly one thing: whether the App declares the skill.
//!
//! ⚠ Every retrieval number in this file is reported beside the EMBEDDER that produced it.
//! With `KX_SERVE_EMBED_MODEL` unset the serve silently falls back to the chat primary,
//! which is a decoder; a retrieval result read without that context is unattributable.
//!
//! ⚠ FEATURE GATE. `serve-engine` + `hnsw`, deliberately NOT `inference`.
//! `inference = ["serve-engine", ...]` is one-directional, so an `inference`-gated file
//! compiles to an EMPTY harness under `console,serve-engine,hnsw,hosted-apps,observability`
//! — the exact set the live proofs build.
//!
//! ```text
//! # Ollama (the RC-shaped, FFI-free build) — serve a DEDICATED embedder:
//! KX_SERVE_OLLAMA=on KX_SERVE_OLLAMA_MODELS=gemma4:12b,embeddinggemma:latest \
//!   KX_SERVE_EMBED_MODEL=embeddinggemma:latest \
//!   cargo test -p kx-gateway --features serve-engine,hnsw --test dataset_skill_serve \
//!     -- --ignored --nocapture --test-threads=1
//! # llama.cpp (needs the FFI):
//! KX_SERVE_MODEL_GGUF=.../gemma-4-12b-it-q4_k_m.gguf \
//!   cargo test -p kx-gateway --features inference,hnsw --test dataset_skill_serve \
//!     -- --ignored --nocapture --test-threads=1
//! ```

#![cfg(all(feature = "serve-engine", feature = "hnsw"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

mod common;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use kx_gateway::{start, REACT_RAG_RECIPE_HANDLE};
use kx_proto::proto;
use kx_proto::proto::kx_gateway_client::KxGatewayClient;
use tonic::transport::Channel;

/// The nonce the grounded answer must carry. It appears in exactly ONE ingested document
/// and nowhere else in this file, so a model cannot produce it from the question, from a
/// sibling passage, or from anything it was trained on.
const NONCE: &str = "ZEPHYRINE-7734";

fn serve_model() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("KX_SERVE_MODEL_GGUF") {
        let p = PathBuf::from(p);
        return p.is_file().then_some(p);
    }
    let standin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/models/qwen3-0.6b-q4_k_m.gguf");
    standin.is_file().then_some(standin)
}

fn ollama_opted_in() -> bool {
    matches!(
        std::env::var("KX_SERVE_OLLAMA")
            .unwrap_or_default()
            .trim()
            .to_ascii_lowercase()
            .as_str(),
        "1" | "on" | "true" | "yes"
    )
}

fn engine() -> &'static str {
    if ollama_opted_in() {
        "ollama"
    } else {
        "llamacpp"
    }
}

/// Which embedder produced the vectors. An unset `KX_SERVE_EMBED_MODEL` falls back to the
/// CHAT primary — a decoder — and the retrieval numbers mean something different then.
fn embedder() -> String {
    std::env::var("KX_SERVE_EMBED_MODEL").unwrap_or_else(|_| "(unset — the chat primary)".into())
}

/// Select the engine. FAILS rather than skips — a live oracle that returns green when no
/// model was served is the exact defect this wave exists to close.
fn configure_engine() {
    if ollama_opted_in() {
        return;
    }
    let gguf = serve_model().expect(
        "PRECONDITION: no serve model. Set KX_SERVE_MODEL_GGUF to a GGUF, or \
         KX_SERVE_OLLAMA=on with KX_SERVE_OLLAMA_MODELS. It fails instead of skipping \
         because a skip is indistinguishable from a pass.",
    );
    std::env::set_var("KX_SERVE_MODEL_GGUF", &gguf);
}

async fn client(addr: SocketAddr) -> KxGatewayClient<Channel> {
    common::connect_client(addr).await
}

fn doc(content: &[u8]) -> proto::IngestDocument {
    proto::IngestDocument {
        content: content.to_vec(),
        embedding: Vec::new(),
        ..Default::default()
    }
}

async fn assert_recipe_provisioned(c: &mut KxGatewayClient<Channel>, handle: &str) {
    let recipes = c
        .list_recipes(proto::ListRecipesRequest {})
        .await
        .unwrap()
        .into_inner();
    assert!(
        recipes.recipes.iter().any(|r| r.handle == handle),
        "PRECONDITION: {handle} is not provisioned on [{}] — it needs a served model and \
         hnsw. Provisioned: {:?}",
        engine(),
        recipes
            .recipes
            .iter()
            .map(|r| &r.handle)
            .collect::<Vec<_>>()
    );
}

/// Poll a react chain to a terminal branch and return every turn together with the RAW
/// text each turn emitted (`GetProjection` → `result_ref` → `GetContent`, the shape
/// `args_grammar_serve` uses). `ListReactTurns` alone carries no output text, so an
/// assertion about WHAT the model said cannot be made from it.
async fn settle_and_read(
    c: &mut KxGatewayClient<Channel>,
    instance_id: &[u8],
    step_salt: Option<Vec<u8>>,
) -> Vec<(proto::ReactTurnSummary, String)> {
    let mut settled = None;
    for _ in 0..1800 {
        let t = c
            .list_react_turns(proto::ListReactTurnsRequest {
                limit: None,
                instance_id: Some(instance_id.to_vec()),
                step_salt: step_salt.clone(),
            })
            .await
            .unwrap()
            .into_inner();
        if t.turns
            .iter()
            .any(|x| x.branch == "answer" || x.branch == "dead_lettered")
        {
            settled = Some(t);
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let turns = settled
        .unwrap_or_else(|| {
            panic!(
                "the chain never settled a terminal branch on [{}] within 180s",
                engine()
            )
        })
        .turns;

    let view = c
        .get_projection(proto::GetProjectionRequest {
            instance_id: instance_id.to_vec(),
            at_seq: None,
        })
        .await
        .unwrap()
        .into_inner();

    let mut out = Vec::new();
    for t in turns {
        let text = match view
            .motes
            .iter()
            .find(|m| m.mote_id == t.turn_mote_id)
            .and_then(|m| m.result_ref.clone())
        {
            Some(rref) => c
                .get_content(proto::GetContentRequest {
                    content_ref: rref,
                    instance_id: instance_id.to_vec(),
                })
                .await
                .map(|r| String::from_utf8_lossy(&r.into_inner().payload).into_owned())
                .unwrap_or_default(),
            None => String::new(),
        };
        out.push((t, text));
    }
    out
}

/// The concatenated text of every ANSWER turn — what the run actually told its caller.
fn answer_text(turns: &[(proto::ReactTurnSummary, String)]) -> String {
    turns
        .iter()
        .filter(|(t, _)| t.branch == "answer")
        .map(|(_, txt)| txt.as_str())
        .collect::<Vec<_>>()
        .join("\n")
}

// ---------------------------------------------------------------------------------
// 1 · GROUNDEDNESS — the answer must carry what the retrieval returned
// ---------------------------------------------------------------------------------

/// THE GROUNDEDNESS ORACLE. Firing `retrieve` is not grounding: a chain that searches,
/// discards the passages and answers from priors fires the tool, settles an Answer, and
/// passes every assertion the existing live-RAG witness makes.
///
/// The target document carries [`NONCE`] — a token that appears in no question, no sibling
/// passage and no pretraining corpus. Its presence in the ANSWER turn is therefore only
/// reachable through the retrieval, and its ABSENCE is a real, nameable defect rather than
/// model noise.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference + dataset embedding; needs a served model; opt in with --ignored"]
async fn rag_answer_is_grounded_in_the_passage_it_retrieved() {
    configure_engine();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;
    assert_recipe_provisioned(&mut c, REACT_RAG_RECIPE_HANDLE).await;

    // The corpus. Only the FIRST document can answer the question, and it is the only
    // place the nonce exists.
    let question = "What is the internal project code name for the Q3 storage migration?";
    let target = format!(
        "Internal reference: the Q3 storage migration is tracked under the project code \
         name {NONCE}. All migration tickets must cite it."
    );
    assert!(
        !question.contains(NONCE),
        "the question must not contain the nonce — otherwise the oracle passes on an echo"
    );

    let ingested = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: "internal-docs".to_string(),
            documents: vec![
                doc(target.as_bytes()),
                doc(
                    b"The cafeteria menu rotates every two weeks and vegetarian options are \
                      always available at the salad bar.",
                ),
                doc(
                    b"Expense reports are submitted monthly and require a manager signature \
                      before the fifteenth.",
                ),
            ],
        })
        .await
        .unwrap_or_else(|e| {
            panic!(
                "PRECONDITION: IngestDocuments failed on [{}] with embedder {} — {e}",
                engine(),
                embedder()
            )
        })
        .into_inner();
    assert_eq!(
        ingested.doc_count,
        3,
        "three documents were indexed [{}] (embedder {}, dim {})",
        engine(),
        embedder(),
        ingested.dim
    );

    // A direct dataset query first: if RETRIEVAL itself cannot surface the target, a
    // failed groundedness assertion below would be blamed on the model when the defect is
    // in the index. Assert the precondition so the two are never confused.
    let direct = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "internal-docs".to_string(),
            query_text: question.to_string(),
            query_embedding: Vec::new(),
            k: 3,
            retrieval_mode: 0,
            rerank: None,
        })
        .await
        .expect("QueryDataset")
        .into_inner();
    let retrievable = direct
        .hits
        .iter()
        .any(|h| String::from_utf8_lossy(&h.content).contains(NONCE));
    assert!(
        retrievable,
        "PRECONDITION: the target passage is not retrievable at all on [{}] with embedder \
         {} — that is an INDEX defect, not a grounding one. Hits: {:?}",
        engine(),
        embedder(),
        direct
            .hits
            .iter()
            .map(|h| String::from_utf8_lossy(&h.content)
                .chars()
                .take(60)
                .collect::<String>())
            .collect::<Vec<_>>()
    );

    let args = serde_json::json!({
        "instruction": format!("{question} Search the dataset and answer using what you find."),
        "dataset": "internal-docs",
        "max_turns": 4,
        "max_tool_calls": 3,
    });
    let resp = c
        .invoke(proto::InvokeRequest {
            handle: REACT_RAG_RECIPE_HANDLE.to_string(),
            args: serde_json::to_vec(&args).unwrap(),
            context_bundles: vec![],
            context_refs: vec![],
        })
        .await
        .expect("invoke react-rag")
        .into_inner();
    let salt = (!resp.react_chain_salt.is_empty()).then(|| resp.react_chain_salt.clone());
    let turns = settle_and_read(&mut c, &resp.instance_id, salt).await;

    for (t, text) in &turns {
        eprintln!(
            "  [{}] turn={} branch={} tool={} raw={:?}",
            engine(),
            t.turn,
            t.branch,
            t.tool_id,
            text.chars().take(160).collect::<String>()
        );
    }

    assert!(
        turns
            .iter()
            .any(|(t, _)| t.branch == "tool" && t.tool_id == "retrieve"),
        "the chain must fire `retrieve` [{}] (embedder {})",
        engine(),
        embedder()
    );
    let answer = answer_text(&turns);
    assert!(
        !answer.trim().is_empty(),
        "the chain must settle a non-empty Answer [{}]",
        engine()
    );

    // THE ASSERTION THIS FILE EXISTS FOR.
    assert!(
        answer.contains(NONCE),
        "the answer must be GROUNDED in the retrieved passage on [{}] (embedder {}): the \
         nonce {NONCE} appears in exactly one indexed document and nowhere else, and the \
         direct query above proved that document is retrievable. The chain fired \
         `retrieve` and then answered without it. Answer was: {answer:?}",
        engine(),
        embedder()
    );

    eprintln!(
        "✓ grounded RAG [{}] embedder={}: the answer carries {NONCE} from the retrieved passage",
        engine(),
        embedder()
    );
    running.shutdown().await.unwrap();
}

/// The dataset FAILURE paths, each asserting the REASON rather than merely that something
/// failed. A negative test that accepts any error also accepts the feature being switched
/// off — these name the code AND the text the operator has to act on.
///
/// Model-free by construction (client vectors / no query at all), so it is the cheap
/// companion to the grounded oracle above rather than a second inference run.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a served embedder for the ingest leg; opt in with --ignored"]
async fn dataset_failure_paths_name_their_reason() {
    configure_engine();
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    // A real dataset to contrast against — every refusal below is one variable away from
    // this accepting control.
    let ok = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: "real-set".to_string(),
            documents: vec![doc(b"a stored document about quarterly planning")],
        })
        .await
        .unwrap_or_else(|e| {
            panic!(
                "PRECONDITION: the accepting control failed to ingest on [{}] with embedder \
                 {} — {e}",
                engine(),
                embedder()
            )
        })
        .into_inner();
    let dim = ok.dim as usize;
    assert!(dim > 0, "the dataset reports its embedding dimension");

    // ── ACCEPTING CONTROL: a correctly-sized client vector is answered.
    let good = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "real-set".to_string(),
            query_text: String::new(),
            query_embedding: vec![0.1; dim],
            k: 1,
            retrieval_mode: 0,
            rerank: None,
        })
        .await;
    assert!(
        good.is_ok(),
        "a correctly-dimensioned client vector must be ACCEPTED [{}] — without this the \
         refusals below prove nothing: {:?}",
        engine(),
        good.err()
    );

    // ── ONE VARIABLE: the same query with the wrong dimension.
    let mismatched = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "real-set".to_string(),
            query_text: String::new(),
            query_embedding: vec![0.1; dim + 1],
            k: 1,
            retrieval_mode: 0,
            rerank: None,
        })
        .await
        .expect_err("a dimension-mismatched query vector must be refused");
    let msg = mismatched.message().to_lowercase();
    assert!(
        msg.contains("dim") && msg.contains(&dim.to_string()),
        "the refusal must NAME the dimension it expected [{}] — a bare error is \
         indistinguishable from the feature being off. code={:?} message={:?}",
        engine(),
        mismatched.code(),
        mismatched.message()
    );

    // ── An unknown dataset is NOT_FOUND and names what was asked for.
    let unknown = c
        .query_dataset(proto::QueryDatasetRequest {
            dataset: "no-such-dataset".to_string(),
            query_text: String::new(),
            query_embedding: vec![0.1; dim],
            k: 1,
            retrieval_mode: 0,
            rerank: None,
        })
        .await
        .expect_err("an unknown dataset must be refused");
    assert_eq!(
        unknown.code(),
        tonic::Code::NotFound,
        "an unknown dataset is NOT_FOUND, not a generic error [{}] — message={:?}",
        engine(),
        unknown.message()
    );

    // ── An empty ingest is refused rather than silently creating a phantom dataset that
    // then answers every query with nothing.
    let empty = c
        .ingest_documents(proto::IngestDocumentsRequest {
            dataset: "empty-set".to_string(),
            documents: vec![],
        })
        .await;
    match empty {
        Err(status) => assert_eq!(
            status.code(),
            tonic::Code::InvalidArgument,
            "an empty ingest is INVALID_ARGUMENT [{}] — message={:?}",
            engine(),
            status.message()
        ),
        Ok(r) => {
            // If it is accepted, it must NOT have manufactured a queryable dataset.
            let r = r.into_inner();
            assert_eq!(
                r.doc_count,
                0,
                "an empty ingest must not manufacture documents [{}]",
                engine()
            );
            let listed = c
                .list_datasets(proto::ListDatasetsRequest {})
                .await
                .expect("ListDatasets")
                .into_inner();
            eprintln!(
                "  note [{}]: an empty ingest was ACCEPTED; datasets now {:?}",
                engine(),
                listed
                    .datasets
                    .iter()
                    .map(|d| &d.dataset_id)
                    .collect::<Vec<_>>()
            );
        }
    }

    eprintln!(
        "✓ dataset failure paths [{}] embedder={}: dim mismatch names dim={dim}, unknown is \
         NOT_FOUND, and the accepting control passes",
        engine(),
        embedder()
    );
    running.shutdown().await.unwrap();
}

// ---------------------------------------------------------------------------------
// 2 · A SKILL THAT STEERS
// ---------------------------------------------------------------------------------

/// The org-private rule. A model cannot know this scheme: it is invented here, and the
/// mapping is deliberately NOT the conventional one (a full outage would normally be the
/// most severe code, and here it is not) so a lucky guess is not the same as being steered.
const SKILL_INSTRUCTIONS: &str = "\
# Acme incident triage

Acme uses a NON-STANDARD severity scale. Apply it exactly and never substitute the
conventional meaning of these codes.

- A database or storage outage is always severity `SEV-4`.
- A login or authentication failure is always severity `SEV-2`.
- A cosmetic or typographical defect is always severity `SEV-1`.

Answer with the severity code and nothing else.
";

/// The App under test, optionally declaring the skill. Everything except
/// `references.skills` is byte-identical between the two arms.
fn triage_app_envelope(skill: Option<(&str, &str)>) -> Vec<u8> {
    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": "Incident report: the primary database cluster is unreachable and \
                       all storage reads are failing. What is the severity?",
            "params": { "max_turns": "2", "max_tool_calls": "0" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Acme Triage", blueprint);
    env.description = "classify an incident against the org severity scale".to_string();
    if let Some((name, instructions_ref)) = skill {
        env.references.skills.push(kx_app::SkillRef {
            name: name.to_string(),
            instructions_ref: instructions_ref.to_string(),
            tools: std::collections::BTreeMap::new(),
        });
    }
    env.to_canonical_json().unwrap()
}

/// Read the OUTPUT of a non-agentic App run.
///
/// ⚠ A plain model step is a single transform Mote, NOT a ReAct chain — it produces no
/// `ListReactTurns` rows at all, so polling those for an `answer` branch waits forever.
/// (It did: the first version of this harness copied the react poll from the agentic
/// tests and both arms sat until the 180s timeout.) The output lives on the committed
/// Mote's `result_ref` in the run's projection.
///
/// The last-committed Mote is the run's output. No count is asserted — a count over a
/// projection is not the test's to make.
async fn settle_app_output(c: &mut KxGatewayClient<Channel>, instance_id: &[u8]) -> String {
    for _ in 0..900 {
        let view = c
            .get_projection(proto::GetProjectionRequest {
                instance_id: instance_id.to_vec(),
                at_seq: None,
            })
            .await
            .expect("GetProjection")
            .into_inner();
        let terminal = view
            .motes
            .iter()
            .filter(|m| m.result_ref.is_some())
            .max_by_key(|m| m.committed_seq.unwrap_or(0));
        if let Some(m) = terminal {
            let rref = m.result_ref.clone().unwrap();
            if let Ok(content) = c
                .get_content(proto::GetContentRequest {
                    content_ref: rref,
                    instance_id: instance_id.to_vec(),
                })
                .await
            {
                return String::from_utf8_lossy(&content.into_inner().payload).into_owned();
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!(
        "the App run never committed a result on [{}] within 180s",
        engine()
    )
}

/// Run ONE triage arm in its OWN gateway.
///
/// ⚠ Each arm gets a fresh gateway and a fresh temp dir. A live `kx serve` shares one
/// journal across submissions, so two arms in one gateway would have to be separated by
/// sequence arithmetic to be read apart — and an arm that accidentally read its sibling's
/// output would make the two arms compare EQUAL, i.e. would look exactly like the skill
/// having no effect. Isolation is structural here rather than asserted.
async fn run_triage_arm(skill_body: Option<&str>) -> String {
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let skill_ref = match skill_body {
        Some(body) => {
            let manifest = serde_json::json!({
                "schema": "kortecx.skill/v1",
                "name": "acme-triage",
                "description": "the Acme incident severity scale",
            });
            let added = c
                .add_skill(proto::AddSkillRequest {
                    manifest_json: serde_json::to_vec(&manifest).unwrap(),
                    instructions_body: body.as_bytes().to_vec(),
                })
                .await
                .expect("AddSkill")
                .into_inner();
            assert_eq!(
                added.instructions_ref.len(),
                64,
                "AddSkill mints a 64-hex instructions ref [{}]",
                engine()
            );
            Some((added.name, added.instructions_ref))
        }
        None => None,
    };

    let handle = "apps/local/acme-triage";
    c.save_app(proto::SaveAppRequest {
        handle: handle.to_string(),
        envelope_json: triage_app_envelope(
            skill_ref.as_ref().map(|(n, r)| (n.as_str(), r.as_str())),
        ),
        source_digest: Vec::new(),
    })
    .await
    .expect("SaveApp")
    .into_inner();
    let run = c
        .run_app(proto::RunAppRequest {
            handle: handle.to_string(),
            args: Vec::new(),
            require_approval: false,
        })
        .await
        .expect("RunApp")
        .into_inner();
    let out = settle_app_output(&mut c, &run.instance_id).await;
    running.shutdown().await.unwrap();
    out
}

/// THE STEERING ORACLE. A skill's instructions must change what the live model does.
///
/// One variable: whether the App declares the skill. The severity scheme is org-private
/// and deliberately inverted, so the skilled arm can only produce `SEV-4` by reading the
/// folded instructions — and the unskilled arm has no way to reach it.
///
/// ⚠ The arms are asserted to DIFFER before either is asserted on (Rule 54). Identical
/// arms mean the variable did nothing — a fold that never happened — which is a different
/// defect from a rule the model read and ignored, and the two must not be conflated.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a served model; opt in with --ignored"]
async fn a_skill_steers_a_live_model_and_the_arms_must_differ() {
    configure_engine();

    // ── ARM A: with the skill.  ARM B: byte-identical App, skill removed. Each arm gets
    // its own gateway, so nothing but the declaration differs.
    let with_skill = run_triage_arm(Some(SKILL_INSTRUCTIONS)).await;
    let without_skill = run_triage_arm(None).await;

    eprintln!(
        "  [{}] WITH skill    -> {:?}",
        engine(),
        with_skill.chars().take(200).collect::<String>()
    );
    eprintln!(
        "  [{}] WITHOUT skill -> {:?}",
        engine(),
        without_skill.chars().take(200).collect::<String>()
    );

    assert!(
        !with_skill.trim().is_empty() && !without_skill.trim().is_empty(),
        "both arms must answer [{}] — with={with_skill:?} without={without_skill:?}",
        engine()
    );

    // FIRST: the arms must differ. If they do not, the skill changed nothing and every
    // assertion below would be reporting on the model's priors, not on the fold.
    assert_ne!(
        with_skill.trim(),
        without_skill.trim(),
        "the two arms are IDENTICAL on [{}] — the skill fold did not reach the model. \
         That is a wiring defect, not a model one: assert it before asserting the rule.",
        engine()
    );

    // THEN: the skilled arm carries the org-private rule.
    assert!(
        with_skill.contains("SEV-4"),
        "the skilled arm must apply the org severity scale on [{}] — a storage outage is \
         SEV-4 under the folded instructions. Answer was: {with_skill:?}",
        engine()
    );
    // And the control could not have reached it: the scheme exists nowhere else.
    assert!(
        !without_skill.contains("SEV-4"),
        "the UNSKILLED arm produced SEV-4 on [{}] without the instructions — the scheme is \
         invented and inverted, so this means the arms are not isolated (a leaked context \
         item, or the same App ran twice). Answer was: {without_skill:?}",
        engine()
    );

    eprintln!(
        "✓ skill steering [{}]: with-skill applies the org scale, without-skill cannot",
        engine()
    );
}

/// The LIVE half of the fail-soft wish. `app_run.rs::author_app_with_an_unfireable_skill_wish_proceeds_toolless`
/// proves the AUTHORING drops an unfulfillable wish; nothing proved the run that follows
/// is genuinely toolless on a real model rather than quietly granted at dispatch.
///
/// A skill is instructions plus a WISH, and a wish is not authority: the union is
/// intersected with the party's real grants, so a skill naming a tool the caller may not
/// fire must leave the step unable to fire it — while still folding its instructions.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "real LLM inference; needs a served model; opt in with --ignored"]
async fn a_skill_wishing_an_ungranted_tool_runs_toolless() {
    configure_engine();
    // Deliberately NO autogrant: the wish must find no authority to intersect with.
    std::env::remove_var("KX_SERVE_AUTOGRANT");
    let dir = tempfile::TempDir::new().unwrap();
    let running = start(common::gateway_config(&dir, true, HashMap::new()))
        .await
        .unwrap();
    let mut c = client(running.local_addr()).await;

    let manifest = serde_json::json!({
        "schema": "kortecx.skill/v1",
        "name": "acme-triage-greedy",
        "description": "the Acme scale, plus a tool wish the caller cannot fire",
    });
    let added = c
        .add_skill(proto::AddSkillRequest {
            manifest_json: serde_json::to_vec(&manifest).unwrap(),
            instructions_body: SKILL_INSTRUCTIONS.as_bytes().to_vec(),
        })
        .await
        .expect("AddSkill")
        .into_inner();

    let blueprint = serde_json::json!({
        "seed": 0,
        "steps": [{
            "kind": "model",
            "prompt": "Incident report: the primary database cluster is unreachable and \
                       all storage reads are failing. What is the severity?",
            "params": { "max_turns": "2", "max_tool_calls": "1" }
        }]
    });
    let mut env = kx_app::AppEnvelope::new("Acme Triage (greedy)", blueprint);
    env.references.skills.push(kx_app::SkillRef {
        name: added.name.clone(),
        instructions_ref: added.instructions_ref.clone(),
        // A tool that is not registered and is granted to nobody.
        tools: [("acme/delete_everything".to_string(), "1".to_string())]
            .into_iter()
            .collect(),
    });
    c.save_app(proto::SaveAppRequest {
        handle: "apps/local/triage-greedy".to_string(),
        envelope_json: env.to_canonical_json().unwrap(),
        source_digest: Vec::new(),
    })
    .await
    .expect("SaveApp (an unfulfillable wish must not fail the save — a wish is fail-soft)")
    .into_inner();

    let run = c
        .run_app(proto::RunAppRequest {
            handle: "apps/local/triage-greedy".to_string(),
            args: Vec::new(),
            require_approval: false,
        })
        .await
        .expect("RunApp")
        .into_inner();

    // The output first: the run must COMPLETE. Fail-SOFT means the App works minus the
    // wish, not that one unresolvable name bricks it.
    let answer = settle_app_output(&mut c, &run.instance_id).await;

    // Then the tool ledger. With the wish dropped the step carries no tool contract at
    // all, so the honest expectation is that `acme/delete_everything` appears in NO turn —
    // including the case where there are no turns because the step never became agentic.
    let turns = c
        .list_react_turns(proto::ListReactTurnsRequest {
            limit: None,
            instance_id: Some(run.instance_id.clone()),
            step_salt: None,
        })
        .await
        .expect("ListReactTurns")
        .into_inner()
        .turns;
    let fired: Vec<&str> = turns
        .iter()
        .filter(|t| t.branch == "tool")
        .map(|t| t.tool_id.as_str())
        .collect();
    eprintln!(
        "  [{}] greedy-wish run: fired={fired:?} branches={:?} answer={:?}",
        engine(),
        turns.iter().map(|t| &t.branch).collect::<Vec<_>>(),
        answer.chars().take(160).collect::<String>()
    );

    // The wish never became authority.
    assert!(
        !fired.contains(&"acme/delete_everything"),
        "a skill's tool WISH is not authority — `acme/delete_everything` is registered to \
         nobody and granted to nobody, yet it fired on [{}]: {fired:?}",
        engine()
    );
    assert!(
        !answer.trim().is_empty(),
        "an unfulfillable wish must be dropped, not fatal — the App still answers on [{}]",
        engine()
    );
    // The INSTRUCTIONS still folded even though the wish was dropped: the two halves of a
    // skill are independent, and dropping the wish must not drop the steer.
    assert!(
        answer.contains("SEV-4"),
        "the instructions must still steer after the wish is dropped on [{}] — the two \
         halves of a skill are independent. Answer was: {answer:?}",
        engine()
    );

    eprintln!(
        "✓ fail-soft skill wish [{}]: the ungranted tool never fired, the instructions still steered",
        engine()
    );
    running.shutdown().await.unwrap();
}
