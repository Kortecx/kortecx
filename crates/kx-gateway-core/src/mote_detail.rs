//! `GetMoteDetail` (Batch B): resolve one committed Mote's `mote_def_hash` to
//! a CAPPED, display-only definition summary.
//!
//! Pure mapping lives here (unit-testable without a store or journal); the
//! handler composes the existing `view::fold_through` + ownership check with
//! the [`MoteDefView`] seam. Caps are DEFENSIVE display bounds, not redaction:
//! secrets cannot reach `config_subset` by construction (`CredentialRef` is a
//! transport-setup concern that never serializes into a def — D81), but a
//! display RPC still never echoes unbounded client-shaped bytes.

use kx_mote::{MoteDef, MoteId, NdClass, PROMPT_KEY, REACT_TURN_KEY};
use kx_proto::proto;

use crate::error::{internal, GatewayError};
use crate::mote_def_view::MoteDefView;
use crate::reader::JournalReader;
use crate::view::fold_through;

/// Per-config-value display cap (bytes). Larger values truncate with
/// `truncated = true` + an honest `full_len`.
pub const MAX_CONFIG_VALUE_BYTES: usize = 4096;

/// Config-entry display cap. The submit path already bounds params well below
/// this; the cap exists so the response size is provably bounded.
pub const MAX_CONFIG_ENTRIES: usize = 128;

/// Prompt display cap (bytes of the UTF-8-lossy rendering).
pub const MAX_PROMPT_BYTES: usize = 65536;

/// The display step-kind vocabulary (closed; first match wins). A STRING, not
/// a proto enum — display classification may grow finer without a wire bump
/// (the `ModelSummary.modalities` precedent), and nothing may branch authority
/// on it (SN-8).
pub(crate) fn step_kind(def: &MoteDef) -> &'static str {
    if def.is_topology_shaper {
        return "shaper";
    }
    if def.critic_for.is_some() || def.critic_check.is_some() {
        return "critic";
    }
    if def.config_subset.keys().any(|k| k.0 == REACT_TURN_KEY) {
        return "react-turn";
    }
    let has_prompt = def.config_subset.keys().any(|k| k.0 == PROMPT_KEY);
    if has_prompt && !def.model_id.0.is_empty() {
        return "model";
    }
    if !def.tool_contract.is_empty() || def.nd_class == NdClass::WorldMutating {
        return "exec";
    }
    "pure"
}

/// Map a resolved def (or its honest absence) into the wire `MoteDetail`.
/// `def_hash` is `None` until the Mote commits (the fold collects hashes from
/// `Committed` entries only).
pub(crate) fn to_proto_detail(
    mote_id: MoteId,
    def_hash: Option<[u8; 32]>,
    def: Option<&MoteDef>,
) -> proto::MoteDetail {
    let mut detail = proto::MoteDetail {
        mote_id: mote_id.as_bytes().to_vec(),
        mote_def_hash: def_hash.map_or_else(Vec::new, |h| h.to_vec()),
        def_found: false,
        step_kind: String::new(),
        model_id: String::new(),
        prompt: String::new(),
        prompt_truncated: false,
        config_subset: Vec::new(),
        tool_contract: std::collections::HashMap::new(),
        logic_ref: Vec::new(),
        nd_class: proto::NdClass::Unspecified as i32,
        effect_pattern: proto::EffectPattern::Unspecified as i32,
        critic_for: None,
        is_topology_shaper: false,
        schema_version: 0,
    };
    let Some(def) = def else {
        return detail; // honest empty: uncommitted, pre-Batch-B, or persist miss
    };

    detail.def_found = true;
    detail.step_kind = step_kind(def).to_string();
    detail.model_id.clone_from(&def.model_id.0);

    // The prompt gets a dedicated field (UTF-8 lossy, capped) and is EXCLUDED
    // from the config list — one home per fact, no duplication.
    if let Some(prompt) = def.config_subset.iter().find(|(k, _)| k.0 == PROMPT_KEY) {
        let text = String::from_utf8_lossy(&prompt.1 .0);
        if text.len() > MAX_PROMPT_BYTES {
            // Cut on a char boundary at or below the cap (lossy text is valid UTF-8).
            let mut cut = MAX_PROMPT_BYTES;
            while !text.is_char_boundary(cut) {
                cut -= 1;
            }
            detail.prompt = text[..cut].to_string();
            detail.prompt_truncated = true;
        } else {
            detail.prompt = text.into_owned();
        }
    }

    detail.config_subset = def
        .config_subset
        .iter()
        .filter(|(k, _)| k.0 != PROMPT_KEY)
        .take(MAX_CONFIG_ENTRIES)
        .map(|(k, v)| {
            let full_len = v.0.len() as u64;
            let truncated = v.0.len() > MAX_CONFIG_VALUE_BYTES;
            let value = if truncated {
                v.0[..MAX_CONFIG_VALUE_BYTES].to_vec()
            } else {
                v.0.clone()
            };
            proto::MoteConfigEntry {
                key: k.0.clone(),
                value,
                truncated,
                full_len,
            }
        })
        .collect();

    detail.tool_contract = def
        .tool_contract
        .iter()
        .map(|(name, version)| (name.0.clone(), version.0.clone()))
        .collect();
    detail.logic_ref = def.logic_ref.as_bytes().to_vec();
    detail.nd_class = proto::NdClass::from(def.nd_class) as i32;
    detail.effect_pattern = proto::EffectPattern::from(def.effect_pattern) as i32;
    detail.critic_for = def.critic_for.map(|id| id.as_bytes().to_vec());
    detail.is_topology_shaper = def.is_topology_shaper;
    detail.schema_version = u32::from(def.schema_version);
    detail
}

/// The `GetMoteDetail` read: fold → ownership (uniform denial, no oracle) →
/// the mote must exist in the OWNED run (an owner can already enumerate motes
/// via `GetProjection`, so `NotFound` here is honest, not an oracle) → resolve
/// the committed def hash through the seam (absence at any step degrades to
/// `def_found = false`, never an error).
pub(crate) fn mote_detail(
    reader: &dyn JournalReader,
    defs: &dyn MoteDefView,
    instance_id: [u8; 16],
    mote_id: [u8; 32],
) -> Result<proto::MoteDetail, GatewayError> {
    let head = reader.current_seq().map_err(internal)?;
    let (projection, def_hashes) = fold_through(reader, head)?;
    match projection.run_registration() {
        Some((inst, _)) if inst == instance_id => {}
        _ => return Err(GatewayError::NotAuthorized),
    }
    let mote_id = MoteId::from_bytes(mote_id);
    if !projection.iter_motes().any(|(id, _)| id == mote_id) {
        return Err(GatewayError::NotFound("no such mote in this run"));
    }
    let def_hash = def_hashes.get(&mote_id).copied();
    let def = match def_hash {
        Some(hash) => defs.get_def(&hash)?,
        None => None, // not committed yet — the hash only exists on Committed facts
    };
    Ok(to_proto_detail(mote_id, def_hash, def.as_ref()))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use kx_mote::{
        ConfigKey, ConfigVal, EffectPattern, LogicRef, ModelId, MoteDef, PromptTemplateHash,
        ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
    };

    use super::*;

    fn base_def() -> MoteDef {
        MoteDef {
            critic_check: None,
            logic_ref: LogicRef::from_bytes([0xaa; 32]),
            model_id: ModelId(String::new()),
            prompt_template_hash: PromptTemplateHash::from_bytes([0xbb; 32]),
            tool_contract: BTreeMap::new(),
            nd_class: NdClass::Pure,
            config_subset: BTreeMap::new(),
            effect_pattern: EffectPattern::IdempotentByConstruction,
            critic_for: None,
            is_topology_shaper: false,
            inference_params: kx_mote::InferenceParams::default(),
            schema_version: MOTE_DEF_SCHEMA_VERSION,
        }
    }

    fn with_config(mut def: MoteDef, key: &str, value: &[u8]) -> MoteDef {
        def.config_subset
            .insert(ConfigKey(key.to_string()), ConfigVal(value.to_vec()));
        def
    }

    #[test]
    fn step_kind_covers_the_closed_vocabulary_in_priority_order() {
        // shaper wins over everything.
        let mut shaper = with_config(base_def(), PROMPT_KEY, b"plan");
        shaper.is_topology_shaper = true;
        shaper.model_id = ModelId("m".into());
        assert_eq!(step_kind(&shaper), "shaper");

        // critic: by critic_for OR critic_check.
        let mut critic = base_def();
        critic.critic_for = Some(MoteId::from_bytes([1; 32]));
        assert_eq!(step_kind(&critic), "critic");

        // react-turn: the identity-bearing marker key.
        let react = with_config(
            with_config(base_def(), REACT_TURN_KEY, b"0"),
            PROMPT_KEY,
            b"think",
        );
        assert_eq!(step_kind(&react), "react-turn");

        // model: prompt + a bound model id.
        let mut model = with_config(base_def(), PROMPT_KEY, b"say hi");
        model.model_id = ModelId("qwen3".into());
        assert_eq!(step_kind(&model), "model");

        // exec: a tool contract (or WM) without a model prompt.
        let mut exec = base_def();
        exec.tool_contract
            .insert(ToolName("echo".into()), ToolVersion("1".into()));
        assert_eq!(step_kind(&exec), "exec");
        let mut wm = base_def();
        wm.nd_class = NdClass::WorldMutating;
        assert_eq!(step_kind(&wm), "exec");

        // pure: the residual.
        assert_eq!(step_kind(&base_def()), "pure");
    }

    #[test]
    fn prompt_gets_the_dedicated_field_and_leaves_the_config_list() {
        let def = with_config(
            with_config(base_def(), PROMPT_KEY, b"the instruction"),
            "temperature",
            b"0",
        );
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), Some([9; 32]), Some(&def));
        assert!(detail.def_found);
        assert_eq!(detail.prompt, "the instruction");
        assert!(!detail.prompt_truncated);
        assert_eq!(detail.config_subset.len(), 1, "PROMPT_KEY excluded");
        assert_eq!(detail.config_subset[0].key, "temperature");
    }

    #[test]
    fn oversized_values_truncate_with_honest_lengths() {
        let big = vec![0x61u8; MAX_CONFIG_VALUE_BYTES + 100];
        let def = with_config(base_def(), "blob", &big);
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), Some([9; 32]), Some(&def));
        let entry = &detail.config_subset[0];
        assert!(entry.truncated);
        assert_eq!(entry.value.len(), MAX_CONFIG_VALUE_BYTES);
        assert_eq!(entry.full_len, (MAX_CONFIG_VALUE_BYTES + 100) as u64);

        let huge_prompt = "p".repeat(MAX_PROMPT_BYTES + 5);
        let def = with_config(base_def(), PROMPT_KEY, huge_prompt.as_bytes());
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), Some([9; 32]), Some(&def));
        assert!(detail.prompt_truncated);
        assert_eq!(detail.prompt.len(), MAX_PROMPT_BYTES);
    }

    #[test]
    fn entry_count_caps_at_the_bound() {
        let mut def = base_def();
        for i in 0..(MAX_CONFIG_ENTRIES + 10) {
            def.config_subset
                .insert(ConfigKey(format!("k{i:04}")), ConfigVal(vec![1]));
        }
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), Some([9; 32]), Some(&def));
        assert_eq!(detail.config_subset.len(), MAX_CONFIG_ENTRIES);
    }

    #[test]
    fn absent_def_is_an_honest_empty_not_an_error() {
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), Some([9; 32]), None);
        assert!(!detail.def_found);
        assert_eq!(detail.mote_def_hash, vec![9u8; 32]);
        assert!(detail.prompt.is_empty());

        // Uncommitted: not even a hash yet.
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), None, None);
        assert!(!detail.def_found);
        assert!(detail.mote_def_hash.is_empty());
    }

    #[test]
    fn full_detail_maps_every_field() {
        let mut def = with_config(base_def(), PROMPT_KEY, b"go");
        def.model_id = ModelId("qwen3".into());
        def.tool_contract
            .insert(ToolName("echo".into()), ToolVersion("1".into()));
        def.critic_for = Some(MoteId::from_bytes([3; 32]));
        let hash = *def.hash().as_bytes();
        let detail = to_proto_detail(MoteId::from_bytes([7; 32]), Some(hash), Some(&def));
        assert!(detail.def_found);
        assert_eq!(detail.mote_def_hash, hash.to_vec());
        assert_eq!(detail.model_id, "qwen3");
        assert_eq!(
            detail.tool_contract.get("echo").map(String::as_str),
            Some("1")
        );
        assert_eq!(detail.logic_ref, vec![0xaa; 32]);
        assert_eq!(detail.nd_class, proto::NdClass::Pure as i32);
        assert_eq!(
            detail.effect_pattern,
            proto::EffectPattern::IdempotentByConstruction as i32
        );
        assert_eq!(detail.critic_for, Some(vec![3u8; 32]));
        assert_eq!(detail.schema_version, u32::from(MOTE_DEF_SCHEMA_VERSION));
    }
}
