//! `kx projection --instance <hex16> [--at-seq N]` — render a run as a DAG of
//! Mote states (all fields server-derived from the kx-projection fold).

use kx_proto::proto;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// Parsed `projection` arguments.
#[derive(Debug)]
pub struct ProjectionArgs {
    /// The run to render (16B instance id).
    pub instance: [u8; 16],
    /// Fold up to and including this seq (absent = current head).
    pub at_seq: Option<u64>,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `projection` args (the verb already consumed).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<ProjectionArgs, CliError> {
    let mut instance: Option<[u8; 16]> = None;
    let mut at_seq: Option<u64> = None;
    let mut common = ClientCommon::default();

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--at-seq" => {
                let v = next_value(&mut args, "--at-seq")?;
                at_seq = Some(v.parse().map_err(|_| {
                    CliError::Usage(format!("--at-seq expects an integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let instance =
        instance.ok_or_else(|| CliError::Usage("projection requires --instance <hex16>".into()))?;
    Ok(ProjectionArgs {
        instance,
        at_seq,
        common,
    })
}

/// Execute `projection`.
pub async fn execute(args: ProjectionArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;

    let view = client
        .get_projection(resolved.request(proto::GetProjectionRequest {
            instance_id: args.instance.to_vec(),
            at_seq: args.at_seq,
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();

    println!("{}", format::render_projection(&view, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<ProjectionArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_instance_and_at_seq() {
        let a = p(&["--instance", &"ab".repeat(16), "--at-seq", "5"]).unwrap();
        assert_eq!(a.at_seq, Some(5));
        assert_eq!(a.instance, [0xab; 16]);
    }

    #[test]
    fn missing_instance_is_usage() {
        assert!(p(&["--at-seq", "1"]).is_err());
    }

    #[test]
    fn bad_hex_instance_is_usage() {
        assert!(p(&["--instance", "abcd"]).is_err(), "wrong length");
        assert!(p(&["--instance", &"zz".repeat(16)]).is_err(), "non-hex");
    }

    #[test]
    fn bad_at_seq_and_unknown_flag_are_usage() {
        assert!(p(&["--instance", &"ab".repeat(16), "--at-seq", "soon"]).is_err());
        assert!(p(&["--instance", &"ab".repeat(16), "--nope"]).is_err());
    }
}
