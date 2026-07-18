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
    AppAuthor, AppCatalog, AppRunError, BinderError, RecipeBinder, RegisteredToolsView,
    RunSubmitter, TriggerAdmin, TriggerAdminError, TriggerFireOutcome, TriggerRegistration,
    TriggerView,
};

use crate::triggers_store::{trigger_id_of, TriggerRow, TriggersDb};

/// The host trigger admin: the durable store + the run-submission seams.
pub(crate) struct HostTriggerAdmin {
    triggers: Arc<TriggersDb>,
    binder: Arc<dyn RecipeBinder>,
    submitter: Arc<dyn RunSubmitter>,
    /// Whether this serve can drive a live ReAct chain (the Invoke react backstop).
    react_supported: bool,
    /// T-APP-TRIGGER-TARGET: the App-run resolver (the RunApp seam). `None` ⇒ App-target
    /// triggers fail closed (the serve was built without the App-run path — no
    /// mcp-gateway / connections.db). Recipe triggers never touch it.
    app_author: Option<Arc<dyn AppAuthor>>,
    /// The live broker-fireable view for the ported RunApp fireable-grant backstop
    /// (an App blueprint is client-authored, so a grant the broker never registered must
    /// fail closed here). `Some` exactly when `app_author` is `Some`.
    fireable: Option<Arc<dyn RegisteredToolsView>>,
    /// The App catalog, for the register-time hosted-app guard (D213): a hosted
    /// (Experience) App has no blueprint and can never be run via `RunApp`, so it must
    /// not be accepted as a trigger target. `None` ⇒ the guard is skipped (the fire path
    /// still fails closed via `author_app`).
    app_catalog: Option<Arc<dyn AppCatalog>>,
}

impl HostTriggerAdmin {
    pub(crate) fn new(
        triggers: Arc<TriggersDb>,
        binder: Arc<dyn RecipeBinder>,
        submitter: Arc<dyn RunSubmitter>,
        react_supported: bool,
        app_author: Option<Arc<dyn AppAuthor>>,
        fireable: Option<Arc<dyn RegisteredToolsView>>,
        app_catalog: Option<Arc<dyn AppCatalog>>,
    ) -> Self {
        Self {
            triggers,
            binder,
            submitter,
            react_supported,
            app_author,
            fireable,
            app_catalog,
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

/// Map an [`AppRunError`] (the RunApp seam) to a [`TriggerAdminError`], mirroring the
/// `run_app` RPC handler's mapping so an App-target trigger fails the SAME way RunApp does.
fn app_err(e: AppRunError) -> TriggerAdminError {
    match e {
        AppRunError::NotAuthorized => TriggerAdminError::NotAuthorized,
        AppRunError::InvalidArgs(d) => TriggerAdminError::InvalidArgument(d),
        AppRunError::MissingIntegration(name) => TriggerAdminError::Unsupported(format!(
            "missing integration: {name} (register it with `kx connections add`)"
        )),
        AppRunError::UnservedModelRoute(route) => TriggerAdminError::Unsupported(format!(
            "model route {route:?} is not served here (start `kx serve` with a matching model)"
        )),
        AppRunError::Internal(d) => TriggerAdminError::Storage(d),
    }
}

#[tonic::async_trait]
impl TriggerAdmin for HostTriggerAdmin {
    async fn register(&self, reg: TriggerRegistration) -> Result<[u8; 16], TriggerAdminError> {
        // T-APP-TRIGGER-TARGET: EXACTLY ONE of recipe_handle | app_handle. Both-or-neither
        // is a clear authoring error (never a silent recipe fallback).
        let is_app = !reg.app_handle.trim().is_empty();
        let is_recipe = !reg.recipe_handle.trim().is_empty();
        if is_app == is_recipe {
            return Err(TriggerAdminError::InvalidArgument(
                "exactly one of recipe_handle | app_handle is required".into(),
            ));
        }
        // An App-target trigger needs the App-run seam (mcp-gateway + connections.db).
        // Fail FAST at register so a serve without it says so immediately, rather than
        // accept a trigger that dead-letters every fire.
        if is_app && self.app_author.is_none() {
            return Err(TriggerAdminError::Unsupported(
                "App-target triggers require the App-run seam \
                 (build --features mcp-gateway with a connections.db)"
                    .into(),
            ));
        }
        // D213: a hosted (Experience) App carries no blueprint and is served by the
        // hosted-app supervisor, never scheduled — refuse it as a trigger target at
        // REGISTER (fail fast) rather than dead-lettering every fire. Skipped when the
        // catalog is unwired; a not-found app is left to the fire path.
        if is_app {
            if let Some(catalog) = self.app_catalog.as_ref() {
                if let Ok(Some((record, _))) = catalog.get(&reg.owner_party, reg.app_handle.trim())
                {
                    if record.kind == "experience" {
                        return Err(TriggerAdminError::InvalidArgument(
                            "hosted (experience) apps are not schedulable; scheduling requires \
                             a functional app"
                                .into(),
                        ));
                    }
                }
            }
        }
        // Validate auth posture: an HMAC/bearer webhook needs a secret ref to verify against.
        if matches!(reg.auth.as_str(), "hmac_sha256" | "bearer") && reg.auth_secret_ref.is_empty() {
            return Err(TriggerAdminError::InvalidArgument(format!(
                "auth '{}' requires an auth_secret_ref (the SecretRef NAME of the verify key)",
                reg.auth
            )));
        }
        // Cron: validate the schedule (legacy interval-seconds OR a 5-field crontab expr
        // in `timezone`) + seed the first fire watermark. Validating HERE makes a bad
        // expression / unknown timezone a synchronous `invalid_argument`, never a silent
        // never-firing trigger.
        let now = now_unix_ms();
        let next_fire_unix_ms = if reg.kind == "cron" {
            crate::schedule::next_fire(&reg.schedule_spec, &reg.timezone, now)
                .map_err(|e| TriggerAdminError::InvalidArgument(e.to_string()))?
        } else {
            0
        };
        let trigger_id = trigger_id_of(&reg.name);
        let row = TriggerRow {
            trigger_id,
            name: reg.name,
            kind: reg.kind,
            recipe_handle: reg.recipe_handle,
            app_handle: reg.app_handle,
            args_template_json: String::new(),
            auth: reg.auth,
            auth_secret_ref: reg.auth_secret_ref,
            schedule_spec: reg.schedule_spec,
            timezone: reg.timezone,
            owner_party: reg.owner_party,
            require_approval: reg.require_approval,
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
                app_handle: r.app_handle,
                auth: r.auth,
                auth_secret_present: !r.auth_secret_ref.is_empty(),
                schedule_spec: r.schedule_spec,
                timezone: r.timezone,
                enabled: r.enabled,
                require_approval: r.require_approval,
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
        // Bind under the trigger's OWNER party (D102.2) with the event payload as the args
        // (passthrough). An empty payload is the empty object.
        let args = if payload_json.trim().is_empty() {
            b"{}".to_vec()
        } else {
            payload_json.as_bytes().to_vec()
        };
        // T-APP-TRIGGER-TARGET: route by target. An App target runs through the SAME
        // `author_app` the RunApp RPC uses — connections + secret_scope + the context/RAG
        // rail resolved, HITL posture stamped — so a credentialed App fires unattended.
        // A recipe target binds as before. Both yield a BoundRecipe fed to the identical
        // register_run + submit_mote tail below (the coordinator stays the sole writer).
        let bound = if cfg.app_handle.is_empty() {
            self.binder
                .bind(&cfg.owner_party, &cfg.recipe_handle, &args, &[], &[])
                .await
                .map_err(bind_err)?
        } else {
            let author = self.app_author.as_ref().ok_or_else(|| {
                TriggerAdminError::Unsupported(
                    "App-target trigger requires the App-run seam (no connections.db on this serve)"
                        .into(),
                )
            })?;
            let bound = author
                .author_app(
                    &cfg.owner_party,
                    &cfg.app_handle,
                    &args,
                    cfg.require_approval,
                )
                .await
                .map_err(app_err)?;
            // Port the RunApp fireable-grant backstop (service.rs run_app): an App
            // blueprint is client-authored, so a tool grant the broker never registered
            // must fail closed HERE — never submitted as a warrant the recipe path would
            // never produce (SN-8 / provisioning-drift guard).
            if let Some(fireable) = self.fireable.as_ref() {
                let registered = fireable.registered_grants();
                for (_, warrant) in &bound.motes {
                    if let Some(g) = warrant.tool_grants.iter().find(|g| {
                        !registered.contains(&(g.tool_id.0.clone(), g.tool_version.0.clone()))
                    }) {
                        return Err(TriggerAdminError::Unsupported(format!(
                            "app step grants tool {}@{} but this serve registered no such capability",
                            g.tool_id.0, g.tool_version.0
                        )));
                    }
                }
            }
            bound
        };
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
        // A dry-run authors/binds but NEVER submits; a resolution failure is a non-fatal
        // (ok=false) detail, an internal store error is fatal.
        if cfg.app_handle.is_empty() {
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
                Err(BinderError::NotAuthorized) => {
                    Ok((false, "not authorized for the recipe".into()))
                }
                Err(BinderError::InvalidArgs(d)) => {
                    Ok((false, format!("payload does not bind: {d}")))
                }
                Err(BinderError::Internal(d)) => Err(TriggerAdminError::Storage(d)),
            }
        } else {
            // T-APP-TRIGGER-TARGET: an App-target dry-run authors the App (no submit, no
            // fatal fireable-backstop — report an over-broad grant would be caught at fire).
            let Some(author) = self.app_author.as_ref() else {
                return Ok((false, "App-run seam not wired on this serve".into()));
            };
            match author
                .author_app(
                    &cfg.owner_party,
                    &cfg.app_handle,
                    &args,
                    cfg.require_approval,
                )
                .await
            {
                Ok(bound) => Ok((
                    true,
                    format!(
                        "authors '{}' ({} mote{})",
                        cfg.app_handle,
                        bound.motes.len(),
                        if bound.motes.len() == 1 { "" } else { "s" }
                    ),
                )),
                Err(AppRunError::NotAuthorized) => Ok((false, "not authorized for the app".into())),
                Err(AppRunError::InvalidArgs(d)) => {
                    Ok((false, format!("payload does not bind: {d}")))
                }
                Err(AppRunError::MissingIntegration(n)) => {
                    Ok((false, format!("missing integration: {n}")))
                }
                Err(AppRunError::UnservedModelRoute(route)) => {
                    Ok((false, format!("model route {route:?} is not served here")))
                }
                Err(AppRunError::Internal(d)) => Err(TriggerAdminError::Storage(d)),
            }
        }
    }
}
