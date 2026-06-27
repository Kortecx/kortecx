//! D113 host wiring: [`HostTriggerAdmin`] — the [`TriggerAdmin`] seam impl over the
//! `triggers.db` sidecar + the SAME [`RecipeBinder`] + [`RunSubmitter`] the Invoke
//! path uses. An inbound event (gRPC `SubmitTrigger`, or the host webhook/cron
//! listeners calling [`HostTriggerAdmin::submit`] directly) starts a FRESH registered
//! run through the existing propose-proxy:
//!
//! ```text
//! event → dedup(idempotency_key) → bind(owner_party, recipe_handle, payload)
//!       → register_run → submit_mote* → record the run origin
//! ```
//!
//! The coordinator stays the sole journal writer (no journal-writer dep here); the
//! frozen trio is untouched; this is the Invoke flow minus the context-bundle /
//! run-inputs / react-salt extras (D113 minimal-local). The payload is passed to the
//! recipe args VERBATIM (passthrough) — shape the event to the recipe's parameters.

use std::sync::Arc;

use kx_content::ContentRef;
use kx_gateway_core::{
    BinderError, RecipeBinder, RunSubmitter, TriggerAdmin, TriggerAdminError, TriggerFireOutcome,
    TriggerRegistration, TriggerView,
};

use crate::triggers_store::{trigger_id_of, TriggerRow, TriggersDb};

/// The host trigger admin: the durable store + the run-submission seams.
pub(crate) struct HostTriggerAdmin {
    triggers: Arc<TriggersDb>,
    binder: Arc<dyn RecipeBinder>,
    submitter: Arc<dyn RunSubmitter>,
    /// Whether this serve can drive a live ReAct chain (the Invoke react backstop).
    react_supported: bool,
}

impl HostTriggerAdmin {
    pub(crate) fn new(
        triggers: Arc<TriggersDb>,
        binder: Arc<dyn RecipeBinder>,
        submitter: Arc<dyn RunSubmitter>,
        react_supported: bool,
    ) -> Self {
        Self {
            triggers,
            binder,
            submitter,
            react_supported,
        }
    }

    /// Derive the dedup key for a payload when the caller supplied none:
    /// `hex(blake3("kx-trigger-evt\0" ‖ trigger_id ‖ payload))`.
    fn derived_key(trigger_id: &[u8; 16], payload: &str) -> String {
        use std::fmt::Write as _;
        let mut keyed = Vec::with_capacity(16 + 16 + payload.len());
        keyed.extend_from_slice(b"kx-trigger-evt\0");
        keyed.extend_from_slice(trigger_id);
        keyed.extend_from_slice(payload.as_bytes());
        let mut hex = String::with_capacity(64);
        for b in ContentRef::of(&keyed).0 {
            let _ = write!(hex, "{b:02x}");
        }
        hex
    }
}

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

fn bind_err(e: BinderError) -> TriggerAdminError {
    match e {
        BinderError::NotAuthorized => TriggerAdminError::NotAuthorized,
        BinderError::InvalidArgs(d) => TriggerAdminError::InvalidArgument(d),
        BinderError::Internal(d) => TriggerAdminError::Storage(d),
    }
}

#[tonic::async_trait]
impl TriggerAdmin for HostTriggerAdmin {
    async fn register(&self, reg: TriggerRegistration) -> Result<[u8; 16], TriggerAdminError> {
        // Validate auth posture: an HMAC/bearer webhook needs a secret ref to verify against.
        if matches!(reg.auth.as_str(), "hmac_sha256" | "bearer") && reg.auth_secret_ref.is_empty() {
            return Err(TriggerAdminError::InvalidArgument(format!(
                "auth '{}' requires an auth_secret_ref (the SecretRef NAME of the verify key)",
                reg.auth
            )));
        }
        // Cron: validate the interval + seed the first fire watermark.
        let now = now_unix_ms();
        let next_fire_unix_ms = if reg.kind == "cron" {
            let secs: u64 = reg.schedule_spec.trim().parse().map_err(|_| {
                TriggerAdminError::InvalidArgument(
                    "cron schedule_spec must be a positive integer (interval seconds)".into(),
                )
            })?;
            if secs == 0 {
                return Err(TriggerAdminError::InvalidArgument(
                    "cron interval must be > 0 seconds".into(),
                ));
            }
            now.saturating_add(secs.saturating_mul(1000))
        } else {
            0
        };
        let trigger_id = trigger_id_of(&reg.name);
        let row = TriggerRow {
            trigger_id,
            name: reg.name,
            kind: reg.kind,
            recipe_handle: reg.recipe_handle,
            args_template_json: String::new(),
            auth: reg.auth,
            auth_secret_ref: reg.auth_secret_ref,
            schedule_spec: reg.schedule_spec,
            owner_party: reg.owner_party,
            enabled: reg.enabled,
            next_fire_unix_ms,
            created_unix_ms: now,
            last_fire_unix_ms: 0,
        };
        self.triggers
            .upsert(&row)
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?;
        Ok(trigger_id)
    }

    async fn list(
        &self,
        limit: u32,
        after_name: &str,
    ) -> Result<(Vec<TriggerView>, bool), TriggerAdminError> {
        let (rows, has_more) = self
            .triggers
            .list(limit, after_name)
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?;
        let views = rows
            .into_iter()
            .map(|r| TriggerView {
                trigger_id: r.trigger_id,
                name: r.name,
                kind: r.kind,
                recipe_handle: r.recipe_handle,
                auth: r.auth,
                auth_secret_present: !r.auth_secret_ref.is_empty(),
                schedule_spec: r.schedule_spec,
                enabled: r.enabled,
                last_fire_unix_ms: r.last_fire_unix_ms,
            })
            .collect();
        Ok((views, has_more))
    }

    async fn deregister(&self, name: &str) -> Result<bool, TriggerAdminError> {
        self.triggers
            .remove(name)
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))
    }

    async fn submit(
        &self,
        name: &str,
        idempotency_key: &str,
        payload_json: &str,
    ) -> Result<TriggerFireOutcome, TriggerAdminError> {
        let cfg = self
            .triggers
            .get(name)
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?
            .ok_or_else(|| TriggerAdminError::NotFound(name.to_string()))?;
        if !cfg.enabled {
            return Err(TriggerAdminError::Unsupported(format!(
                "trigger '{name}' is disabled"
            )));
        }
        let key = if idempotency_key.trim().is_empty() {
            Self::derived_key(&cfg.trigger_id, payload_json)
        } else {
            idempotency_key.to_string()
        };
        // Pre-check dedup: a replayed event returns the prior run and fires nothing.
        if let Some(prior) = self
            .triggers
            .fired(&key)
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?
        {
            return Ok(TriggerFireOutcome {
                instance_id: prior,
                deduped: true,
            });
        }
        // Bind the recipe under the trigger's OWNER party (D102.2) with the event
        // payload as the args (passthrough). An empty payload is the empty object.
        let args = if payload_json.trim().is_empty() {
            b"{}".to_vec()
        } else {
            payload_json.as_bytes().to_vec()
        };
        let bound = self
            .binder
            .bind(&cfg.owner_party, &cfg.recipe_handle, &args, &[], &[])
            .await
            .map_err(bind_err)?;
        if bound.react_seed && !self.react_supported {
            return Err(TriggerAdminError::Unsupported(
                "this serve cannot drive a live ReAct chain (no inference executor)".into(),
            ));
        }
        // The SAME propose-proxy as Invoke: register (returns only after the journaled
        // instance_id), then submit each bound Mote.
        let instance_id = self
            .submitter
            .register_run(bound.recipe_fingerprint)
            .await
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?;
        let react_seed = bound.react_seed;
        for (mote, warrant) in bound.motes {
            self.submitter
                .submit_mote(mote, warrant, false, react_seed)
                .await
                .map_err(|e| TriggerAdminError::Storage(e.to_string()))?;
        }
        // Record the run origin under the dedup key. A concurrent identical event that
        // beat us here keeps its own run (this one is an inert extra — a rare local race;
        // the hardened cross-event dedup is CLOUD). Either way return a deduped flag.
        let (recorded, deduped) = self
            .triggers
            .record_fire(&key, &cfg.trigger_id, &instance_id, now_unix_ms())
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?;
        let _ = self.triggers.set_last_fire(name, now_unix_ms());
        Ok(TriggerFireOutcome {
            instance_id: if deduped { recorded } else { instance_id },
            deduped,
        })
    }

    async fn test(
        &self,
        name: &str,
        payload_json: &str,
    ) -> Result<(bool, String), TriggerAdminError> {
        let cfg = self
            .triggers
            .get(name)
            .map_err(|e| TriggerAdminError::Storage(e.to_string()))?
            .ok_or_else(|| TriggerAdminError::NotFound(name.to_string()))?;
        let args = if payload_json.trim().is_empty() {
            b"{}".to_vec()
        } else {
            payload_json.as_bytes().to_vec()
        };
        match self
            .binder
            .bind(&cfg.owner_party, &cfg.recipe_handle, &args, &[], &[])
            .await
        {
            Ok(bound) => Ok((
                true,
                format!(
                    "binds '{}' ({} mote{})",
                    cfg.recipe_handle,
                    bound.motes.len(),
                    if bound.motes.len() == 1 { "" } else { "s" }
                ),
            )),
            // A dry-run never fires; report the bind failure as a non-fatal detail.
            Err(BinderError::NotAuthorized) => Ok((false, "not authorized for the recipe".into())),
            Err(BinderError::InvalidArgs(d)) => Ok((false, format!("payload does not bind: {d}"))),
            Err(BinderError::Internal(d)) => Err(TriggerAdminError::Storage(d)),
        }
    }
}
