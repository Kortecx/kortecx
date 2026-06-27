//! `kx triggers add | list | test | fire | rm` — govern the D113 event-ingress
//! triggers over the gateway RPCs (`RegisterTrigger` / `ListTriggers` /
//! `TestTrigger` / `SubmitTrigger` / `DeregisterTrigger`). Tri-surface parity
//! with the UI + SDK.
//!
//! A trigger binds an inbound source (`webhook` | `cron` | `grpc`) to a recipe
//! handle; on an event the gateway starts a FRESH registered run via the Invoke
//! path. SN-8: `trigger_id` is server-derived; the run binds under the
//! REGISTRANT's party; the auth secret is referenced by NAME only (never the
//! value, D81). `test` dry-runs the binding WITHOUT firing; `fire` is the inbound
//! `grpc` event verb (idempotency-keyed dedup).

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `triggers` subcommand.
#[derive(Debug)]
pub enum TriggersSub {
    /// Register a trigger binding (`webhook` | `cron` | `grpc` → recipe handle).
    Add(AddSpec),
    /// List the registered triggers + their bindings.
    List,
    /// Dry-run a trigger binding (validate the handle + payload) WITHOUT firing.
    Test {
        /// The trigger name.
        name: String,
        /// JSON event payload (default `{}`).
        payload_json: String,
    },
    /// Fire a trigger via `SubmitTrigger` (the inbound `grpc` event verb).
    Fire {
        /// The trigger name.
        name: String,
        /// Event-level dedup key (empty ⇒ server-derived from the payload).
        idempotency_key: String,
        /// JSON event payload (default `{}`).
        payload_json: String,
    },
    /// Remove a trigger by name.
    Remove {
        /// The trigger name.
        name: String,
    },
}

/// A `triggers add` request, assembled from the flags.
#[derive(Debug)]
pub struct AddSpec {
    /// The unique operator handle (derives `trigger_id`).
    pub name: String,
    /// The proto [`proto::TriggerKind`] discriminant.
    pub kind: i32,
    /// The `kx/recipes/...` handle the event Invokes.
    pub recipe_handle: String,
    /// The proto [`proto::TriggerAuth`] discriminant (default `none`).
    pub auth: i32,
    /// SecretRef NAME of the HMAC/bearer secret (never the value).
    pub auth_secret_ref: String,
    /// cron: interval seconds (e.g. "300"); empty otherwise.
    pub schedule_spec: String,
    /// Whether the trigger is enabled on registration.
    pub enabled: bool,
}

/// Parsed `triggers` arguments.
#[derive(Debug)]
pub struct TriggersArgs {
    /// The subcommand.
    pub sub: TriggersSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Map the `--kind` string flag to a [`proto::TriggerKind`] discriminant.
fn parse_kind(kind: &str) -> Result<i32, CliError> {
    Ok(match kind {
        "webhook" => proto::TriggerKind::Webhook as i32,
        "cron" => proto::TriggerKind::Cron as i32,
        "grpc" => proto::TriggerKind::Grpc as i32,
        other => {
            return Err(CliError::Usage(format!(
                "--kind must be webhook | cron | grpc, got {other:?}"
            )))
        }
    })
}

/// Map the `--auth` string flag to a [`proto::TriggerAuth`] discriminant.
fn parse_auth(auth: &str) -> Result<i32, CliError> {
    Ok(match auth {
        "none" => proto::TriggerAuth::None as i32,
        "hmac_sha256" => proto::TriggerAuth::HmacSha256 as i32,
        "bearer" => proto::TriggerAuth::Bearer as i32,
        other => {
            return Err(CliError::Usage(format!(
                "--auth must be none | hmac_sha256 | bearer, got {other:?}"
            )))
        }
    })
}

/// Parse `triggers` args (the verb already consumed). The first token selects the
/// subcommand (`add` / `list` / `test` / `fire` / `rm`).
#[allow(clippy::too_many_lines)] // a flat flag-parsing dispatcher (the verbs' convention)
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<TriggersArgs, CliError> {
    let kw = args.next().ok_or_else(|| {
        CliError::Usage("triggers requires a subcommand: add | list | test | fire | rm".into())
    })?;

    let mut name: Option<String> = None;
    let mut kind: Option<String> = None;
    let mut recipe: Option<String> = None;
    let mut auth: Option<String> = None;
    let mut secret_ref = String::new();
    let mut schedule = String::new();
    let mut enabled = false;
    let mut idempotency_key = String::new();
    let mut payload: Option<String> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--name" => name = Some(next_value(&mut args, "--name")?),
            "--kind" => kind = Some(next_value(&mut args, "--kind")?),
            "--recipe" => recipe = Some(next_value(&mut args, "--recipe")?),
            "--auth" => auth = Some(next_value(&mut args, "--auth")?),
            "--secret-ref" => secret_ref = next_value(&mut args, "--secret-ref")?,
            "--schedule" => schedule = next_value(&mut args, "--schedule")?,
            "--enabled" => enabled = true,
            "--idempotency-key" => idempotency_key = next_value(&mut args, "--idempotency-key")?,
            "--payload" => payload = Some(next_value(&mut args, "--payload")?),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let require_name = |name: Option<String>, verb: &str| -> Result<String, CliError> {
        name.filter(|s| !s.is_empty())
            .ok_or_else(|| CliError::Usage(format!("triggers {verb} requires --name <NAME>")))
    };

    let sub = match kw.as_str() {
        "list" => TriggersSub::List,
        "add" => {
            let name = require_name(name, "add")?;
            let kind_str = kind.filter(|s| !s.is_empty()).ok_or_else(|| {
                CliError::Usage("triggers add requires --kind <webhook|cron|grpc>".into())
            })?;
            let kind = parse_kind(&kind_str)?;
            let recipe_handle = recipe
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("triggers add requires --recipe <handle>".into()))?;
            // Default to `none` (validated only on a loopback webhook bind server-side).
            let auth = match auth {
                Some(a) => parse_auth(&a)?,
                None => proto::TriggerAuth::None as i32,
            };
            TriggersSub::Add(AddSpec {
                name,
                kind,
                recipe_handle,
                auth,
                auth_secret_ref: secret_ref,
                schedule_spec: schedule,
                enabled,
            })
        }
        "test" => TriggersSub::Test {
            name: require_name(name, "test")?,
            // Default to the empty object; never null/garbage.
            payload_json: payload
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "{}".to_string()),
        },
        "fire" => TriggersSub::Fire {
            name: require_name(name, "fire")?,
            idempotency_key,
            payload_json: payload
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| "{}".to_string()),
        },
        "rm" | "remove" => TriggersSub::Remove {
            name: require_name(name, "rm")?,
        },
        other => {
            return Err(CliError::Usage(format!(
                "unknown triggers subcommand {other:?} (expected add | list | test | fire | rm)"
            )))
        }
    };
    Ok(TriggersArgs { sub, common })
}

/// Execute `triggers`.
pub async fn execute(args: TriggersArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        TriggersSub::Add(spec) => {
            let req = proto::RegisterTriggerRequest {
                name: spec.name,
                kind: spec.kind,
                recipe_handle: spec.recipe_handle,
                auth: spec.auth,
                auth_secret_ref: spec.auth_secret_ref,
                schedule_spec: spec.schedule_spec,
                enabled: spec.enabled,
            };
            let resp = client
                .register_trigger(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_register_trigger(&resp, json));
        }
        TriggersSub::List => {
            let req = proto::ListTriggersRequest {
                limit: 0,
                after_name: String::new(),
            };
            let resp = client
                .list_triggers(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_triggers_list(&resp, json));
        }
        TriggersSub::Test { name, payload_json } => {
            let req = proto::TestTriggerRequest { name, payload_json };
            let resp = client
                .test_trigger(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_test_trigger(&resp, json));
        }
        TriggersSub::Fire {
            name,
            idempotency_key,
            payload_json,
        } => {
            let req = proto::SubmitTriggerRequest {
                name,
                idempotency_key,
                payload_json,
            };
            let resp = client
                .submit_trigger(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_submit_trigger(&resp, json));
        }
        TriggersSub::Remove { name } => {
            let req = proto::DeregisterTriggerRequest { name };
            let resp = client
                .deregister_trigger(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_deregister_trigger(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<TriggersArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_add_webhook_with_auth_and_secret() {
        let a = p(&[
            "add",
            "--name",
            "gh-push",
            "--kind",
            "webhook",
            "--recipe",
            "kx/recipes/echo",
            "--auth",
            "hmac_sha256",
            "--secret-ref",
            "GH_HMAC",
            "--enabled",
        ])
        .unwrap();
        let TriggersSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.name, "gh-push");
        assert_eq!(spec.kind, proto::TriggerKind::Webhook as i32);
        assert_eq!(spec.recipe_handle, "kx/recipes/echo");
        assert_eq!(spec.auth, proto::TriggerAuth::HmacSha256 as i32);
        assert_eq!(spec.auth_secret_ref, "GH_HMAC");
        assert!(spec.enabled);
        assert!(spec.schedule_spec.is_empty());
    }

    #[test]
    fn parses_add_cron_with_schedule_defaults_auth_none() {
        let a = p(&[
            "add",
            "--name",
            "nightly",
            "--kind",
            "cron",
            "--recipe",
            "kx/recipes/echo",
            "--schedule",
            "300",
        ])
        .unwrap();
        let TriggersSub::Add(spec) = a.sub else {
            panic!("expected Add");
        };
        assert_eq!(spec.kind, proto::TriggerKind::Cron as i32);
        assert_eq!(spec.schedule_spec, "300");
        // `--auth` absent ⇒ none.
        assert_eq!(spec.auth, proto::TriggerAuth::None as i32);
        assert!(!spec.enabled, "disabled unless --enabled");
    }

    #[test]
    fn parses_test_and_fire_default_payload_to_empty_object() {
        let a = p(&["test", "--name", "gh-push"]).unwrap();
        let TriggersSub::Test { payload_json, .. } = a.sub else {
            panic!("expected Test");
        };
        assert_eq!(payload_json, "{}");

        let a = p(&["fire", "--name", "gh-push", "--idempotency-key", "evt-1"]).unwrap();
        let TriggersSub::Fire {
            idempotency_key,
            payload_json,
            ..
        } = a.sub
        else {
            panic!("expected Fire");
        };
        assert_eq!(idempotency_key, "evt-1");
        assert_eq!(payload_json, "{}");

        let a = p(&["fire", "--name", "gh-push", "--payload", r#"{"x":1}"#]).unwrap();
        let TriggersSub::Fire { payload_json, .. } = a.sub else {
            panic!("expected Fire");
        };
        assert_eq!(payload_json, r#"{"x":1}"#);
    }

    #[test]
    fn parses_list_and_rm() {
        assert!(matches!(p(&["list"]).unwrap().sub, TriggersSub::List));
        assert!(matches!(
            p(&["rm", "--name", "gh-push"]).unwrap().sub,
            TriggersSub::Remove { .. }
        ));
        assert!(matches!(
            p(&["remove", "--name", "gh-push"]).unwrap().sub,
            TriggersSub::Remove { .. }
        ));
    }

    #[test]
    fn rejects_missing_required_and_bad_enums() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(
            p(&["add", "--name", "x", "--recipe", "r"]).is_err(),
            "add needs --kind"
        );
        assert!(
            p(&["add", "--name", "x", "--kind", "webhook"]).is_err(),
            "add needs --recipe"
        );
        assert!(
            p(&["add", "--kind", "webhook", "--recipe", "r"]).is_err(),
            "add needs --name"
        );
        assert!(
            p(&["add", "--name", "x", "--kind", "ftp", "--recipe", "r"]).is_err(),
            "bad kind"
        );
        assert!(
            p(&["add", "--name", "x", "--kind", "webhook", "--recipe", "r", "--auth", "weird"])
                .is_err(),
            "bad auth"
        );
        assert!(p(&["test"]).is_err(), "test needs --name");
        assert!(p(&["fire"]).is_err(), "fire needs --name");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
    }
}
