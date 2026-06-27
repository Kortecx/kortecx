//! `kx secrets set | list | rm` — manage the LOCAL OS-keychain secret store
//! (MM-3 / D110) over the gateway RPCs (`PutSecret` / `ListSecretNames` /
//! `DeleteSecret`). Tri-surface parity with the UI + SDK.
//!
//! A secret is host credential material referenced elsewhere by NAME only (a
//! `kx-warrant` SecretRef — e.g. a connection's `--credential-ref` or a trigger's
//! `--secret-ref`). SN-8/D110: the VALUE is WRITE-ONLY — `set` sends it once and it
//! is NEVER returned by any RPC; `list` yields NAMES + timestamps only. `set` /
//! `rm` are gated loopback-only + an authenticated party server-side.

use kx_proto::proto;

use crate::client::{next_value, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `secrets` subcommand.
#[derive(Debug)]
pub enum SecretsSub {
    /// Store (or overwrite) a secret by NAME. The value is write-only.
    Set {
        /// The SecretRef NAME credentials point at.
        name: String,
        /// The secret value (write-only; never returned).
        value: String,
    },
    /// List the stored secret NAMES + timestamps (never the value).
    List,
    /// Remove a secret by NAME.
    Remove {
        /// The secret NAME.
        name: String,
    },
}

/// Parsed `secrets` arguments.
#[derive(Debug)]
pub struct SecretsArgs {
    /// The subcommand.
    pub sub: SecretsSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `secrets` args (the verb already consumed). The first token selects the
/// subcommand (`set` / `list` / `rm`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<SecretsArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("secrets requires a subcommand: set | list | rm".into()))?;

    let mut name: Option<String> = None;
    let mut value: Option<String> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--name" => name = Some(next_value(&mut args, "--name")?),
            "--value" => value = Some(next_value(&mut args, "--value")?),
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let require_name = |name: Option<String>, verb: &str| -> Result<String, CliError> {
        name.filter(|s| !s.is_empty())
            .ok_or_else(|| CliError::Usage(format!("secrets {verb} requires --name <NAME>")))
    };

    let sub = match kw.as_str() {
        "list" => SecretsSub::List,
        "set" => SecretsSub::Set {
            name: require_name(name, "set")?,
            value: value
                .filter(|s| !s.is_empty())
                .ok_or_else(|| CliError::Usage("secrets set requires --value <VALUE>".into()))?,
        },
        "rm" | "remove" => SecretsSub::Remove {
            name: require_name(name, "rm")?,
        },
        other => {
            return Err(CliError::Usage(format!(
                "unknown secrets subcommand {other:?} (expected set | list | rm)"
            )))
        }
    };
    Ok(SecretsArgs { sub, common })
}

/// Execute `secrets`.
pub async fn execute(args: SecretsArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        SecretsSub::Set { name, value } => {
            let req = proto::PutSecretRequest { name, value };
            let resp = client
                .put_secret(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_put_secret(&resp, json));
        }
        SecretsSub::List => {
            let req = proto::ListSecretNamesRequest {
                limit: 0,
                after_name: String::new(),
            };
            let resp = client
                .list_secret_names(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_secret_names(&resp, json));
        }
        SecretsSub::Remove { name } => {
            let req = proto::DeleteSecretRequest { name };
            let resp = client
                .delete_secret(resolved.request(req)?)
                .await
                .map_err(CliError::from_status)?
                .into_inner();
            println!("{}", format::render_delete_secret(&resp, json));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<SecretsArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_set_with_name_and_value() {
        let a = p(&["set", "--name", "GH_TOKEN", "--value", "s3cr3t"]).unwrap();
        let SecretsSub::Set { name, value } = a.sub else {
            panic!("expected Set");
        };
        assert_eq!(name, "GH_TOKEN");
        assert_eq!(value, "s3cr3t");
    }

    #[test]
    fn parses_list_and_rm() {
        assert!(matches!(p(&["list"]).unwrap().sub, SecretsSub::List));
        assert!(matches!(
            p(&["rm", "--name", "GH_TOKEN"]).unwrap().sub,
            SecretsSub::Remove { .. }
        ));
        // `remove` is an accepted alias for `rm`.
        assert!(matches!(
            p(&["remove", "--name", "GH_TOKEN"]).unwrap().sub,
            SecretsSub::Remove { .. }
        ));
    }

    #[test]
    fn rejects_missing_required_and_unknown() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["set", "--name", "x"]).is_err(), "set needs --value");
        assert!(p(&["set", "--value", "v"]).is_err(), "set needs --name");
        assert!(p(&["rm"]).is_err(), "rm needs --name");
        assert!(p(&["frobnicate"]).is_err(), "unknown subcommand");
        assert!(
            p(&["set", "--name", "x", "--value", "v", "--bogus"]).is_err(),
            "unknown flag"
        );
    }

    #[test]
    fn common_flags_are_consumed() {
        let a = p(&["list", "--endpoint", "http://h:1", "--json"]).unwrap();
        assert_eq!(a.common.endpoint, "http://h:1");
        assert!(a.common.json);
    }
}
