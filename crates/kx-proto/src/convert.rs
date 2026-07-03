//! Typed conversions between the generated wire ([`crate::proto`]) types and the
//! canonical Rust domain types (`kx-mote` / `kx-warrant` / `kx-content`).
//!
//! Direction matters:
//! - **`domain -> proto`** ([`From`]) is total — a valid domain value always has
//!   a wire representation.
//! - **`proto -> domain`** ([`TryFrom`]) is the **untrusted boundary**: it
//!   validates 32-byte hash lengths, rejects the `*_UNSPECIFIED` enum sentinel,
//!   requires present message fields, and reconstructs `BTreeMap`/`BTreeSet`
//!   (restoring canonical order regardless of protobuf wire order).
//!
//! ## Identity invariant
//!
//! [`kx_mote::Mote`] is rebuilt via [`kx_mote::Mote::new`], which **re-derives**
//! the [`kx_mote::MoteId`] from `def + input_data_id + graph_position`. The wire
//! `mote_id` is advisory routing only and is never trusted as the identity. A
//! caller that wants `warrant_ref` recomputes it with
//! [`kx_warrant::warrant_ref_of`] over the reconstructed [`kx_warrant::WarrantSpec`].
//! Protobuf bytes are never hashed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use kx_content::ContentRef;
use kx_critic_types::CheckSpec;
use kx_mote::{
    ConfigKey, ConfigVal, EdgeKind, EdgeMeta, EffectPattern, Grammar, GraphPosition,
    InferenceParams, InputDataId, LogicRef, ModelId, Mote, MoteDef, MoteId, NdClass, ParentRef,
    PromptTemplateHash, ToolName, ToolVersion,
};
use kx_warrant::{
    ExecutorClass, FsMode, FsScope, Host, ModelRoute, MoteClass, NetScope, ResourceCeiling,
    SecretRef, SecretScope, ToolGrant, WarrantSpec,
};
use smallvec::SmallVec;

use crate::error::ConvertError;
use crate::proto;

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// Validate that a wire `bytes` field is exactly a 32-byte hash.
fn hash32(bytes: &[u8], field: &'static str) -> Result<[u8; 32], ConvertError> {
    <[u8; 32]>::try_from(bytes).map_err(|_| ConvertError::BadHashLength {
        field,
        len: bytes.len(),
    })
}

/// Decode an `i32` wire enum value into its domain enum: first into the proto
/// enum (rejecting unknown values), then into the domain enum (rejecting the
/// `*_UNSPECIFIED` sentinel).
fn decode_enum<P, D>(value: i32, enum_name: &'static str) -> Result<D, ConvertError>
where
    P: TryFrom<i32>,
    D: TryFrom<P, Error = ConvertError>,
{
    let p = P::try_from(value).map_err(|_| ConvertError::UnknownEnum { enum_name, value })?;
    D::try_from(p)
}

// ---------------------------------------------------------------------------
// enums
// ---------------------------------------------------------------------------

impl From<NdClass> for proto::NdClass {
    fn from(d: NdClass) -> Self {
        match d {
            NdClass::Pure => Self::Pure,
            NdClass::ReadOnlyNondet => Self::ReadOnlyNondet,
            NdClass::WorldMutating => Self::WorldMutating,
        }
    }
}

impl TryFrom<proto::NdClass> for NdClass {
    type Error = ConvertError;
    fn try_from(p: proto::NdClass) -> Result<Self, Self::Error> {
        match p {
            proto::NdClass::Unspecified => Err(ConvertError::UnspecifiedEnum {
                enum_name: "NdClass",
            }),
            proto::NdClass::Pure => Ok(Self::Pure),
            proto::NdClass::ReadOnlyNondet => Ok(Self::ReadOnlyNondet),
            proto::NdClass::WorldMutating => Ok(Self::WorldMutating),
        }
    }
}

impl From<EffectPattern> for proto::EffectPattern {
    fn from(d: EffectPattern) -> Self {
        match d {
            EffectPattern::IdempotentByConstruction => Self::IdempotentByConstruction,
            EffectPattern::StageThenCommit => Self::StageThenCommit,
            EffectPattern::ValidateThenCommit => Self::ValidateThenCommit,
        }
    }
}

impl TryFrom<proto::EffectPattern> for EffectPattern {
    type Error = ConvertError;
    fn try_from(p: proto::EffectPattern) -> Result<Self, Self::Error> {
        match p {
            proto::EffectPattern::Unspecified => Err(ConvertError::UnspecifiedEnum {
                enum_name: "EffectPattern",
            }),
            proto::EffectPattern::IdempotentByConstruction => Ok(Self::IdempotentByConstruction),
            proto::EffectPattern::StageThenCommit => Ok(Self::StageThenCommit),
            proto::EffectPattern::ValidateThenCommit => Ok(Self::ValidateThenCommit),
        }
    }
}

impl From<EdgeKind> for proto::EdgeKind {
    fn from(d: EdgeKind) -> Self {
        match d {
            EdgeKind::Data => Self::Data,
            EdgeKind::Control => Self::Control,
        }
    }
}

impl TryFrom<proto::EdgeKind> for EdgeKind {
    type Error = ConvertError;
    fn try_from(p: proto::EdgeKind) -> Result<Self, Self::Error> {
        match p {
            proto::EdgeKind::Unspecified => Err(ConvertError::UnspecifiedEnum {
                enum_name: "EdgeKind",
            }),
            proto::EdgeKind::Data => Ok(Self::Data),
            proto::EdgeKind::Control => Ok(Self::Control),
        }
    }
}

impl From<MoteClass> for proto::MoteClass {
    fn from(d: MoteClass) -> Self {
        match d {
            MoteClass::Pure => Self::Pure,
            MoteClass::ReadOnlyNondet => Self::ReadOnlyNondet,
            MoteClass::WorldMutating => Self::WorldMutating,
        }
    }
}

impl TryFrom<proto::MoteClass> for MoteClass {
    type Error = ConvertError;
    fn try_from(p: proto::MoteClass) -> Result<Self, Self::Error> {
        match p {
            proto::MoteClass::Unspecified => Err(ConvertError::UnspecifiedEnum {
                enum_name: "MoteClass",
            }),
            proto::MoteClass::Pure => Ok(Self::Pure),
            proto::MoteClass::ReadOnlyNondet => Ok(Self::ReadOnlyNondet),
            proto::MoteClass::WorldMutating => Ok(Self::WorldMutating),
        }
    }
}

impl From<FsMode> for proto::FsMode {
    fn from(d: FsMode) -> Self {
        match d {
            FsMode::ReadOnly => Self::ReadOnly,
            FsMode::ReadWrite => Self::ReadWrite,
            FsMode::ExecOnly => Self::ExecOnly,
        }
    }
}

impl TryFrom<proto::FsMode> for FsMode {
    type Error = ConvertError;
    fn try_from(p: proto::FsMode) -> Result<Self, Self::Error> {
        match p {
            proto::FsMode::Unspecified => Err(ConvertError::UnspecifiedEnum {
                enum_name: "FsMode",
            }),
            proto::FsMode::ReadOnly => Ok(Self::ReadOnly),
            proto::FsMode::ReadWrite => Ok(Self::ReadWrite),
            proto::FsMode::ExecOnly => Ok(Self::ExecOnly),
        }
    }
}

impl From<ExecutorClass> for proto::ExecutorClass {
    fn from(d: ExecutorClass) -> Self {
        match d {
            ExecutorClass::Bwrap => Self::Bwrap,
            ExecutorClass::OciDaemon => Self::OciDaemon,
            ExecutorClass::CloudMicroVm => Self::CloudMicroVm,
            ExecutorClass::MacOsSandbox => Self::MacosSandbox,
        }
    }
}

impl TryFrom<proto::ExecutorClass> for ExecutorClass {
    type Error = ConvertError;
    fn try_from(p: proto::ExecutorClass) -> Result<Self, Self::Error> {
        match p {
            proto::ExecutorClass::Unspecified => Err(ConvertError::UnspecifiedEnum {
                enum_name: "ExecutorClass",
            }),
            proto::ExecutorClass::Bwrap => Ok(Self::Bwrap),
            proto::ExecutorClass::OciDaemon => Ok(Self::OciDaemon),
            proto::ExecutorClass::CloudMicroVm => Ok(Self::CloudMicroVm),
            proto::ExecutorClass::MacosSandbox => Ok(Self::MacOsSandbox),
        }
    }
}

// ---------------------------------------------------------------------------
// mote value messages
// ---------------------------------------------------------------------------

impl From<InferenceParams> for proto::InferenceParams {
    fn from(d: InferenceParams) -> Self {
        Self {
            max_output_tokens: d.max_output_tokens,
            temperature_bps: d.temperature_bps,
            top_p_bps: d.top_p_bps,
            top_k: d.top_k,
            seed: d.seed,
            stop_tokens: d.stop_tokens.into_iter().collect(),
            grammar: d.grammar.map(|g| g.raw),
        }
    }
}

impl TryFrom<proto::InferenceParams> for InferenceParams {
    type Error = ConvertError;
    fn try_from(p: proto::InferenceParams) -> Result<Self, Self::Error> {
        Ok(Self {
            max_output_tokens: p.max_output_tokens,
            temperature_bps: p.temperature_bps,
            top_p_bps: p.top_p_bps,
            top_k: p.top_k,
            seed: p.seed,
            stop_tokens: p.stop_tokens.into_iter().collect(),
            grammar: p.grammar.map(Grammar::new),
        })
    }
}

impl From<MoteDef> for proto::MoteDef {
    fn from(d: MoteDef) -> Self {
        Self {
            logic_ref: d.logic_ref.as_bytes().to_vec(),
            model_id: d.model_id.0,
            prompt_template_hash: d.prompt_template_hash.as_bytes().to_vec(),
            tool_contract: d
                .tool_contract
                .into_iter()
                .map(|(k, v)| (k.0, v.0))
                .collect(),
            nd_class: proto::NdClass::from(d.nd_class) as i32,
            config_subset: d
                .config_subset
                .into_iter()
                .map(|(k, v)| (k.0, v.0))
                .collect(),
            effect_pattern: proto::EffectPattern::from(d.effect_pattern) as i32,
            critic_for: d.critic_for.map(|m| m.as_bytes().to_vec()),
            is_topology_shaper: d.is_topology_shaper,
            inference_params: Some(d.inference_params.into()),
            schema_version: u32::from(d.schema_version),
            critic_check: d.critic_check.as_ref().map(encode_check_spec),
        }
    }
}

/// Encode a [`CheckSpec`] to canonical bincode bytes for the wire. Byte-identical
/// to the embedding used in `kx_mote::MoteDef::hash`, so a round-tripped critic
/// Mote re-derives the same `MoteId` (SN-8). Infallible: `CheckSpec` is
/// integer-only with no non-encodable types.
// SAFETY (expect): CheckSpec is integer-only with no non-encodable types, so
// canonical bincode encoding is infallible — mirrors the documented-infallible
// `kx_mote::MoteDef::hash` / `kx_critic_types::CriticVerdict::encode` sites.
#[allow(clippy::expect_used)]
fn encode_check_spec(spec: &CheckSpec) -> Vec<u8> {
    bincode::serde::encode_to_vec(spec, kx_critic_types::canonical_config())
        .expect("CheckSpec canonical encoding is infallible (no floats, no non-encodable types)")
}

/// Decode a [`CheckSpec`] from canonical bincode bytes received on the (untrusted)
/// wire. Rejects malformed or trailing-garbage payloads — never silently drops.
fn decode_check_spec(bytes: &[u8]) -> Result<CheckSpec, ConvertError> {
    let (spec, consumed) = bincode::serde::decode_from_slice::<CheckSpec, _>(
        bytes,
        kx_critic_types::canonical_config(),
    )
    .map_err(|_| ConvertError::MalformedPayload {
        field: "MoteDef.critic_check",
    })?;
    if consumed != bytes.len() {
        return Err(ConvertError::MalformedPayload {
            field: "MoteDef.critic_check",
        });
    }
    Ok(spec)
}

impl TryFrom<proto::MoteDef> for MoteDef {
    type Error = ConvertError;
    fn try_from(p: proto::MoteDef) -> Result<Self, Self::Error> {
        let schema_version =
            u16::try_from(p.schema_version).map_err(|_| ConvertError::OutOfRange {
                field: "MoteDef.schema_version",
                value: u64::from(p.schema_version),
            })?;
        let critic_for = match p.critic_for {
            Some(b) => Some(MoteId::from_bytes(hash32(&b, "MoteDef.critic_for")?)),
            None => None,
        };
        let inference_params = p
            .inference_params
            .ok_or(ConvertError::MissingField {
                field: "MoteDef.inference_params",
            })?
            .try_into()?;
        Ok(Self {
            logic_ref: LogicRef::from_bytes(hash32(&p.logic_ref, "MoteDef.logic_ref")?),
            model_id: ModelId(p.model_id),
            prompt_template_hash: PromptTemplateHash::from_bytes(hash32(
                &p.prompt_template_hash,
                "MoteDef.prompt_template_hash",
            )?),
            tool_contract: p
                .tool_contract
                .into_iter()
                .map(|(k, v)| (ToolName(k), ToolVersion(v)))
                .collect(),
            nd_class: decode_enum::<proto::NdClass, NdClass>(p.nd_class, "NdClass")?,
            config_subset: p
                .config_subset
                .into_iter()
                .map(|(k, v)| (ConfigKey(k), ConfigVal(v)))
                .collect(),
            effect_pattern: decode_enum::<proto::EffectPattern, EffectPattern>(
                p.effect_pattern,
                "EffectPattern",
            )?,
            critic_for,
            is_topology_shaper: p.is_topology_shaper,
            inference_params,
            schema_version,
            critic_check: p
                .critic_check
                .as_deref()
                .map(decode_check_spec)
                .transpose()?,
        })
    }
}

impl From<ParentRef> for proto::ParentRef {
    fn from(d: ParentRef) -> Self {
        Self {
            parent_id: d.parent_id.as_bytes().to_vec(),
            edge_kind: proto::EdgeKind::from(d.edge.kind) as i32,
            non_cascade: d.edge.non_cascade,
        }
    }
}

impl TryFrom<proto::ParentRef> for ParentRef {
    type Error = ConvertError;
    fn try_from(p: proto::ParentRef) -> Result<Self, Self::Error> {
        Ok(Self {
            parent_id: MoteId::from_bytes(hash32(&p.parent_id, "ParentRef.parent_id")?),
            edge: EdgeMeta {
                kind: decode_enum::<proto::EdgeKind, EdgeKind>(p.edge_kind, "EdgeKind")?,
                non_cascade: p.non_cascade,
            },
        })
    }
}

impl From<Mote> for proto::Mote {
    fn from(d: Mote) -> Self {
        Self {
            mote_id: d.id.as_bytes().to_vec(),
            def: Some(d.def.into()),
            input_data_id: d.input_data_id.as_bytes().to_vec(),
            graph_position: d.graph_position.0,
            parents: d.parents.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<proto::Mote> for Mote {
    type Error = ConvertError;
    fn try_from(p: proto::Mote) -> Result<Self, Self::Error> {
        let def = p
            .def
            .ok_or(ConvertError::MissingField { field: "Mote.def" })?
            .try_into()?;
        let input_data_id =
            InputDataId::from_bytes(hash32(&p.input_data_id, "Mote.input_data_id")?);
        let graph_position = GraphPosition(p.graph_position);
        let parents: SmallVec<[ParentRef; 4]> = p
            .parents
            .into_iter()
            .map(ParentRef::try_from)
            .collect::<Result<_, _>>()?;
        // IDENTITY INVARIANT: re-derive MoteId Rust-side. The wire `mote_id` is
        // advisory routing only and is intentionally not trusted here.
        Ok(Mote::new(def, input_data_id, graph_position, parents))
    }
}

// ---------------------------------------------------------------------------
// warrant messages
// ---------------------------------------------------------------------------

impl From<ToolGrant> for proto::ToolGrant {
    fn from(d: ToolGrant) -> Self {
        Self {
            tool_id: d.tool_id.0,
            tool_version: d.tool_version.0,
        }
    }
}

impl From<proto::ToolGrant> for ToolGrant {
    fn from(p: proto::ToolGrant) -> Self {
        Self {
            tool_id: ToolName(p.tool_id),
            tool_version: ToolVersion(p.tool_version),
        }
    }
}

impl From<ModelRoute> for proto::ModelRoute {
    fn from(d: ModelRoute) -> Self {
        Self {
            model_id: d.model_id.0,
            max_input_tokens: d.max_input_tokens,
            max_output_tokens: d.max_output_tokens,
            max_calls: d.max_calls,
        }
    }
}

impl From<proto::ModelRoute> for ModelRoute {
    fn from(p: proto::ModelRoute) -> Self {
        Self {
            model_id: ModelId(p.model_id),
            max_input_tokens: p.max_input_tokens,
            max_output_tokens: p.max_output_tokens,
            max_calls: p.max_calls,
        }
    }
}

impl From<ResourceCeiling> for proto::ResourceCeiling {
    fn from(d: ResourceCeiling) -> Self {
        Self {
            cpu_milli: d.cpu_milli,
            mem_bytes: d.mem_bytes,
            wall_clock_ms: d.wall_clock_ms,
            fd_count: d.fd_count,
            disk_bytes: d.disk_bytes,
        }
    }
}

impl From<proto::ResourceCeiling> for ResourceCeiling {
    fn from(p: proto::ResourceCeiling) -> Self {
        Self {
            cpu_milli: p.cpu_milli,
            mem_bytes: p.mem_bytes,
            wall_clock_ms: p.wall_clock_ms,
            fd_count: p.fd_count,
            disk_bytes: p.disk_bytes,
        }
    }
}

impl From<FsScope> for proto::FsScope {
    fn from(d: FsScope) -> Self {
        Self {
            mounts: d
                .mounts
                .into_iter()
                .map(|(path, mode)| proto::FsMount {
                    path: path.to_string_lossy().into_owned(),
                    mode: proto::FsMode::from(mode) as i32,
                })
                .collect(),
        }
    }
}

impl TryFrom<proto::FsScope> for FsScope {
    type Error = ConvertError;
    fn try_from(p: proto::FsScope) -> Result<Self, Self::Error> {
        let mut mounts = BTreeMap::new();
        for m in p.mounts {
            mounts.insert(
                PathBuf::from(m.path),
                decode_enum::<proto::FsMode, FsMode>(m.mode, "FsMode")?,
            );
        }
        Ok(Self { mounts })
    }
}

impl From<NetScope> for proto::NetScope {
    fn from(d: NetScope) -> Self {
        let scope = match d {
            NetScope::None => proto::net_scope::Scope::None(proto::NetScopeNone {}),
            NetScope::EgressAllowlist(hosts) => {
                proto::net_scope::Scope::Allowlist(proto::HostAllowlist {
                    hosts: hosts.into_iter().map(|h| h.0).collect(),
                })
            }
        };
        Self { scope: Some(scope) }
    }
}

impl TryFrom<proto::NetScope> for NetScope {
    type Error = ConvertError;
    fn try_from(p: proto::NetScope) -> Result<Self, Self::Error> {
        match p.scope.ok_or(ConvertError::MissingField {
            field: "NetScope.scope",
        })? {
            proto::net_scope::Scope::None(_) => Ok(Self::None),
            proto::net_scope::Scope::Allowlist(a) => Ok(Self::EgressAllowlist(
                a.hosts.into_iter().map(Host).collect(),
            )),
        }
    }
}

impl From<SecretScope> for proto::SecretScope {
    fn from(d: SecretScope) -> Self {
        let scope = match d {
            SecretScope::None => proto::secret_scope::Scope::None(proto::SecretScopeNone {}),
            SecretScope::AllowList(refs) => {
                proto::secret_scope::Scope::Allowlist(proto::SecretRefAllowlist {
                    names: refs.into_iter().map(|r| r.0).collect(),
                })
            }
        };
        Self { scope: Some(scope) }
    }
}

impl TryFrom<proto::SecretScope> for SecretScope {
    type Error = ConvertError;
    fn try_from(p: proto::SecretScope) -> Result<Self, Self::Error> {
        match p.scope.ok_or(ConvertError::MissingField {
            field: "SecretScope.scope",
        })? {
            proto::secret_scope::Scope::None(_) => Ok(Self::None),
            proto::secret_scope::Scope::Allowlist(a) => Ok(Self::AllowList(
                a.names.into_iter().map(SecretRef).collect(),
            )),
        }
    }
}

impl From<WarrantSpec> for proto::WarrantSpec {
    fn from(d: WarrantSpec) -> Self {
        Self {
            mote_class: proto::MoteClass::from(d.mote_class) as i32,
            nd_class: proto::MoteClass::from(d.nd_class) as i32,
            fs_scope: Some(d.fs_scope.into()),
            net_scope: Some(d.net_scope.into()),
            syscall_profile_ref: d.syscall_profile_ref.as_bytes().to_vec(),
            tool_grants: d.tool_grants.into_iter().map(Into::into).collect(),
            model_route: Some(d.model_route.into()),
            resource_ceiling: Some(d.resource_ceiling.into()),
            environment_ref: d.environment_ref.map(|r| r.as_bytes().to_vec()),
            executor_class: proto::ExecutorClass::from(d.executor_class) as i32,
            secret_scope: Some(d.secret_scope.into()),
            // `cost_ceiling` / `tls_required` are intentionally not written yet — the
            // proto omits them (see the WarrantSpec message doc): their live
            // enforcement + digest handling land with PR-8/cost-expansion.
        }
    }
}

impl TryFrom<proto::WarrantSpec> for WarrantSpec {
    type Error = ConvertError;
    fn try_from(p: proto::WarrantSpec) -> Result<Self, Self::Error> {
        let environment_ref = match p.environment_ref {
            Some(b) => Some(ContentRef::from_bytes(hash32(
                &b,
                "WarrantSpec.environment_ref",
            )?)),
            None => None,
        };
        let tool_grants: BTreeSet<ToolGrant> = p.tool_grants.into_iter().map(Into::into).collect();
        Ok(Self {
            mote_class: decode_enum::<proto::MoteClass, MoteClass>(p.mote_class, "MoteClass")?,
            nd_class: decode_enum::<proto::MoteClass, MoteClass>(p.nd_class, "MoteClass")?,
            fs_scope: p
                .fs_scope
                .ok_or(ConvertError::MissingField {
                    field: "WarrantSpec.fs_scope",
                })?
                .try_into()?,
            net_scope: p
                .net_scope
                .ok_or(ConvertError::MissingField {
                    field: "WarrantSpec.net_scope",
                })?
                .try_into()?,
            syscall_profile_ref: ContentRef::from_bytes(hash32(
                &p.syscall_profile_ref,
                "WarrantSpec.syscall_profile_ref",
            )?),
            tool_grants,
            model_route: p
                .model_route
                .ok_or(ConvertError::MissingField {
                    field: "WarrantSpec.model_route",
                })?
                .into(),
            resource_ceiling: p
                .resource_ceiling
                .ok_or(ConvertError::MissingField {
                    field: "WarrantSpec.resource_ceiling",
                })?
                .into(),
            environment_ref,
            executor_class: decode_enum::<proto::ExecutorClass, ExecutorClass>(
                p.executor_class,
                "ExecutorClass",
            )?,
            // secret_scope (D110.3) is carried on the wire (field 11). An absent
            // field — a pre-fix peer that never encoded it — decodes to the
            // fail-closed `SecretScope::None`, preserving back-compat. NOTE: the
            // embedded single-node coordinator DOES round-trip through this proto
            // (via TonicCoordinatorSubmitter over loopback), so this axis MUST
            // survive here for a RunApp-stamped AllowList to reach the react
            // OBSERVATION dispatch (T-RUNAPP-SECRET-SCOPE-OBSERVATION).
            secret_scope: match p.secret_scope {
                Some(s) => s.try_into()?,
                None => SecretScope::None,
            },
            // `cost_ceiling` / `tls_required` remain intentionally wire-absent and
            // decode to their fail-closed defaults via `..Default::default()`
            // (tracked for PR-8/cost-expansion).
            ..Default::default()
        })
    }
}
