//! `kx mote show <instance-hex16> <mote-hex32>` — per-mote definition
//! inspection over the gateway (`GetMoteDetail`, Batch B). Tri-surface parity
//! with the console node inspector + the SDK `getMoteDetail` / `get_mote_detail`.
//! DISPLAY-ONLY (SN-8): the capped def summary never authorizes anything; an
//! uncommitted or pre-Batch-B mote answers `def_found: false` honestly.

use kx_proto::proto;

use crate::client::ClientCommon;
use crate::error::CliError;
use crate::format;
use crate::hex;

/// Parsed `mote` arguments.
#[derive(Debug)]
pub struct MoteArgs {
    /// Common client flags.
    pub common: ClientCommon,
    /// The run's 16-byte instance id (the ownership ticket).
    pub instance: [u8; 16],
    /// The 32-byte Mote id to inspect.
    pub mote: [u8; 32],
}

/// Parse `mote` args (the verb already consumed): `show <instance> <mote>`
/// positionally, then client flags.
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<MoteArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("mote requires a subcommand: show".into()))?;
    if kw != "show" {
        return Err(CliError::Usage(format!(
            "unknown mote subcommand {kw:?} (expected: show)"
        )));
    }
    let instance_hex = args
        .next()
        .ok_or_else(|| CliError::Usage("mote show requires <instance-hex16>".into()))?;
    let mote_hex = args
        .next()
        .ok_or_else(|| CliError::Usage("mote show requires <mote-hex32>".into()))?;
    let instance = hex::decode_fixed::<16>(&instance_hex)?;
    let mote = hex::decode_fixed::<32>(&mote_hex)?;
    let mut common = ClientCommon::default();
    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        return Err(CliError::Usage(format!("unknown flag {flag:?}")));
    }
    Ok(MoteArgs {
        common,
        instance,
        mote,
    })
}

/// Execute `mote show`.
pub async fn execute(args: MoteArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let detail = client
        .get_mote_detail(resolved.request(proto::GetMoteDetailRequest {
            instance_id: args.instance.to_vec(),
            mote_id: args.mote.to_vec(),
        })?)
        .await
        .map_err(CliError::from_status)?
        .into_inner();
    println!("{}", format::render_mote_detail(&detail, args.common.json));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<MoteArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    const I16: &str = "11111111111111111111111111111111"; // 16 bytes
    const M32: &str = "2222222222222222222222222222222222222222222222222222222222222222"; // 32 bytes

    #[test]
    fn show_parses_positional_ids() {
        let a = p(&["show", I16, M32]).unwrap();
        assert_eq!(a.instance, [0x11; 16]);
        assert_eq!(a.mote, [0x22; 32]);
        let a = p(&["show", I16, M32, "--json"]).unwrap();
        assert!(a.common.json);
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err());
        assert!(p(&["inspect", I16, M32]).is_err());
        assert!(p(&["show"]).is_err());
        assert!(p(&["show", I16]).is_err());
        // Swapped lengths fail hex validation before any RPC.
        assert!(p(&["show", M32, I16]).is_err());
        assert!(p(&["show", "zz", M32]).is_err());
        assert!(p(&["show", I16, M32, "--bogus"]).is_err());
    }
}
