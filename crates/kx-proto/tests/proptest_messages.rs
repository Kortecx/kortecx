//! Property tests: the proto<->domain mapping is the identity for the
//! identity-bearing types (and preserves `MoteId` / `mote_def_hash` /
//! `warrant_ref`), and the prost wire encoding round-trips, across 64 random
//! cases per property.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_content::ContentRef;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeMeta, EffectPattern, Grammar, GraphPosition, InferenceParams,
    InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef, PromptTemplateHash,
    ToolName, ToolVersion, MOTE_DEF_SCHEMA_VERSION,
};
use kx_proto::proto;
use kx_warrant::{
    warrant_ref_of, ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope,
    ResourceCeiling, ToolGrant, WarrantSpec,
};
use proptest::prelude::*;
use prost::Message;
use smallvec::SmallVec;
use std::path::PathBuf;

// --- leaf strategies --------------------------------------------------------

fn arb_hash() -> impl Strategy<Value = [u8; 32]> {
    proptest::array::uniform32(any::<u8>())
}

fn arb_string() -> impl Strategy<Value = String> {
    proptest::string::string_regex("[a-zA-Z0-9 /._-]{0,12}").unwrap()
}

fn arb_nd() -> impl Strategy<Value = NdClass> {
    prop_oneof![
        Just(NdClass::Pure),
        Just(NdClass::ReadOnlyNondet),
        Just(NdClass::WorldMutating),
    ]
}

fn arb_effect() -> impl Strategy<Value = EffectPattern> {
    prop_oneof![
        Just(EffectPattern::IdempotentByConstruction),
        Just(EffectPattern::StageThenCommit),
        Just(EffectPattern::ValidateThenCommit),
    ]
}

fn arb_mote_class() -> impl Strategy<Value = MoteClass> {
    prop_oneof![
        Just(MoteClass::Pure),
        Just(MoteClass::ReadOnlyNondet),
        Just(MoteClass::WorldMutating),
    ]
}

fn arb_fs_mode() -> impl Strategy<Value = FsMode> {
    prop_oneof![
        Just(FsMode::ReadOnly),
        Just(FsMode::ReadWrite),
        Just(FsMode::ExecOnly),
    ]
}

fn arb_executor() -> impl Strategy<Value = ExecutorClass> {
    prop_oneof![
        Just(ExecutorClass::Bwrap),
        Just(ExecutorClass::OciDaemon),
        Just(ExecutorClass::CloudMicroVm),
        Just(ExecutorClass::MacOsSandbox),
    ]
}

fn arb_edge_meta() -> impl Strategy<Value = EdgeMeta> {
    prop_oneof![
        Just(EdgeMeta::data()),
        Just(EdgeMeta::control()),
        Just(EdgeMeta::control_non_cascading()),
    ]
}

// --- composite strategies ---------------------------------------------------

fn arb_inference_params() -> impl Strategy<Value = InferenceParams> {
    (
        any::<u32>(),
        any::<u32>(),
        any::<u32>(),
        any::<u32>(),
        any::<u32>(),
        prop::collection::vec(arb_string(), 0..4),
        prop::option::of(arb_string()),
    )
        .prop_map(|(mo, t, tp, tk, s, stops, gram)| InferenceParams {
            max_output_tokens: mo,
            temperature_bps: t,
            top_p_bps: tp,
            top_k: tk,
            seed: s,
            stop_tokens: stops.into_iter().collect(),
            grammar: gram.map(Grammar::new),
        })
}

fn arb_mote_def() -> impl Strategy<Value = MoteDef> {
    (
        (arb_hash(), arb_hash(), prop::option::of(arb_hash())),
        arb_string(),
        prop::collection::btree_map(arb_string(), arb_string(), 0..3),
        arb_nd(),
        prop::collection::btree_map(arb_string(), prop::collection::vec(any::<u8>(), 0..4), 0..3),
        arb_effect(),
        any::<bool>(),
        arb_inference_params(),
    )
        .prop_map(
            |((logic, pth, critic), model, tools, nd, cfg, eff, shaper, inf)| MoteDef {
                critic_check: None,
                logic_ref: LogicRef::from_bytes(logic),
                model_id: ModelId(model),
                prompt_template_hash: PromptTemplateHash::from_bytes(pth),
                tool_contract: tools
                    .into_iter()
                    .map(|(k, v)| (ToolName(k), ToolVersion(v)))
                    .collect(),
                nd_class: nd,
                config_subset: cfg
                    .into_iter()
                    .map(|(k, v)| (ConfigKey(k), ConfigVal(v)))
                    .collect(),
                effect_pattern: eff,
                critic_for: critic.map(MoteId::from_bytes),
                is_topology_shaper: shaper,
                inference_params: inf,
                schema_version: MOTE_DEF_SCHEMA_VERSION,
            },
        )
}

fn arb_parents() -> impl Strategy<Value = SmallVec<[ParentRef; 4]>> {
    prop::collection::vec(
        (arb_hash(), arb_edge_meta()).prop_map(|(pid, edge)| ParentRef {
            parent_id: MoteId::from_bytes(pid),
            edge,
        }),
        0..4,
    )
    .prop_map(|v| v.into_iter().collect())
}

fn arb_mote() -> impl Strategy<Value = Mote> {
    (
        arb_mote_def(),
        arb_hash(),
        prop::collection::vec(any::<u8>(), 0..6),
        arb_parents(),
    )
        .prop_map(|(def, idid, gp, parents)| {
            Mote::new(
                def,
                InputDataId::from_bytes(idid),
                GraphPosition(gp),
                parents,
            )
        })
}

fn arb_model_route() -> impl Strategy<Value = ModelRoute> {
    (arb_string(), any::<u32>(), any::<u32>(), any::<u32>()).prop_map(|(m, i, o, c)| ModelRoute {
        model_id: ModelId(m),
        max_input_tokens: i,
        max_output_tokens: o,
        max_calls: c,
    })
}

fn arb_resource_ceiling() -> impl Strategy<Value = ResourceCeiling> {
    (
        any::<u32>(),
        any::<u64>(),
        any::<u64>(),
        any::<u32>(),
        any::<u64>(),
    )
        .prop_map(|(cpu, mem, wall, fd, disk)| ResourceCeiling {
            cpu_milli: cpu,
            mem_bytes: mem,
            wall_clock_ms: wall,
            fd_count: fd,
            disk_bytes: disk,
        })
}

fn arb_fs_scope() -> impl Strategy<Value = FsScope> {
    prop::collection::btree_map(arb_string().prop_map(PathBuf::from), arb_fs_mode(), 0..4)
        .prop_map(|mounts| FsScope { mounts })
}

fn arb_net_scope() -> impl Strategy<Value = NetScope> {
    prop_oneof![
        Just(NetScope::None),
        prop::collection::btree_set(arb_string().prop_map(Host), 0..4)
            .prop_map(NetScope::EgressAllowlist),
    ]
}

fn arb_tool_grants() -> impl Strategy<Value = std::collections::BTreeSet<ToolGrant>> {
    prop::collection::btree_set(
        (arb_string(), arb_string()).prop_map(|(a, b)| ToolGrant {
            tool_id: ToolName(a),
            tool_version: ToolVersion(b),
        }),
        0..4,
    )
}

fn arb_warrant() -> impl Strategy<Value = WarrantSpec> {
    (
        (
            arb_mote_class(),
            arb_mote_class(),
            arb_hash(),
            prop::option::of(arb_hash()),
            arb_executor(),
        ),
        arb_fs_scope(),
        arb_net_scope(),
        arb_tool_grants(),
        arb_model_route(),
        arb_resource_ceiling(),
    )
        .prop_map(
            |((mc, nd, syscall, env, exec), fs, net, tools, mr, rc)| WarrantSpec {
                mote_class: mc,
                nd_class: nd,
                fs_scope: fs,
                net_scope: net,
                syscall_profile_ref: ContentRef::from_bytes(syscall),
                tool_grants: tools,
                model_route: mr,
                resource_ceiling: rc,
                environment_ref: env.map(ContentRef::from_bytes),
                executor_class: exec,
                ..Default::default()
            },
        )
}

// --- properties -------------------------------------------------------------

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    #[test]
    fn prop_inference_params_round_trips(p in arb_inference_params()) {
        let wire: proto::InferenceParams = p.clone().into();
        let back: InferenceParams = wire.try_into().expect("convert");
        prop_assert_eq!(p, back);
    }

    #[test]
    fn prop_mote_def_round_trips(def in arb_mote_def()) {
        let wire: proto::MoteDef = def.clone().into();
        let back: MoteDef = wire.try_into().expect("convert");
        prop_assert_eq!(&def, &back);
        prop_assert_eq!(def.hash(), back.hash());
    }

    #[test]
    fn prop_mote_round_trips(mote in arb_mote()) {
        let wire: proto::Mote = mote.clone().into();
        let back: Mote = wire.try_into().expect("convert");
        prop_assert_eq!(&mote, &back);
        prop_assert_eq!(mote.id, back.id);
    }

    #[test]
    fn prop_warrant_round_trips(w in arb_warrant()) {
        let wire: proto::WarrantSpec = w.clone().into();
        let back: WarrantSpec = wire.try_into().expect("convert");
        prop_assert_eq!(&w, &back);
        prop_assert_eq!(warrant_ref_of(&w), warrant_ref_of(&back));
    }

    #[test]
    fn prop_mote_def_wire_round_trips(def in arb_mote_def()) {
        let wire: proto::MoteDef = def.into();
        let bytes = wire.encode_to_vec();
        let decoded = proto::MoteDef::decode(&bytes[..]).expect("decode");
        prop_assert_eq!(wire, decoded);
    }

    #[test]
    fn prop_warrant_wire_round_trips(w in arb_warrant()) {
        let wire: proto::WarrantSpec = w.into();
        let bytes = wire.encode_to_vec();
        let decoded = proto::WarrantSpec::decode(&bytes[..]).expect("decode");
        prop_assert_eq!(wire, decoded);
    }
}
