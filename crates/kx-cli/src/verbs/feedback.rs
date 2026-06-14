//! `kx feedback submit | list` — record + read back 👍/👎 feedback on an answer
//! over the gateway (`SubmitFeedback` / `ListFeedback`, PR-4.1). Tri-surface
//! parity with the UI + SDK. ADVISORY/DISPLAY-ONLY product signal: the rows live
//! in a rebuildable-to-empty `feedback.db` sidecar — never truth, never identity,
//! never a digest input. The caller principal + the `feedback_id` are server-
//! derived (SN-8); re-rating the same answer OVERWRITES. A gateway without the
//! sidecar answers `Unimplemented` (rendered honestly).

use kx_proto::proto;
use tonic::Code;

use crate::client::{next_value, take_fixed, ClientCommon};
use crate::error::CliError;
use crate::format;

/// The `feedback` subcommand.
#[derive(Debug)]
pub enum FeedbackSub {
    /// Record a 👍/👎 rating on an answer.
    Submit(SubmitSpec),
    /// Read back recorded feedback (newest-first, paginated).
    List(ListSpec),
}

/// A `feedback submit` request, assembled from the flags.
#[derive(Debug)]
pub struct SubmitSpec {
    /// The proto rating int (`1 = UP`, `2 = DOWN`).
    pub rating: i32,
    /// The client-local chat message id (the stable per-answer key; required).
    pub message_id: String,
    /// The run backing the answer (16B), if any.
    pub instance: Option<[u8; 16]>,
    /// The terminal mote (32B), advisory join.
    pub mote: Option<[u8; 32]>,
    /// The answer's content ref (32B), advisory join.
    pub content_ref: Option<[u8; 32]>,
    /// Optional free note (server-capped).
    pub comment: String,
    /// Advisory context: the backing blueprint handle.
    pub handle: String,
    /// Advisory context: the model that answered.
    pub model: String,
}

/// A `feedback list` request, assembled from the flags.
#[derive(Debug)]
pub struct ListSpec {
    /// Scope to one run (16B instance id).
    pub instance: Option<[u8; 16]>,
    /// Page size (server clamps 1..=500; absent = 200).
    pub limit: Option<u32>,
    /// Pagination cursor: only rows with `rowid < before_rowid`.
    pub before_rowid: Option<u64>,
}

/// Parsed `feedback` arguments.
#[derive(Debug)]
pub struct FeedbackArgs {
    /// The subcommand.
    pub sub: FeedbackSub,
    /// Common client flags.
    pub common: ClientCommon,
}

/// Parse `feedback` args (the verb already consumed). The first token selects the
/// subcommand (`submit` / `list`).
pub fn parse(mut args: impl Iterator<Item = String>) -> Result<FeedbackArgs, CliError> {
    let kw = args
        .next()
        .ok_or_else(|| CliError::Usage("feedback requires a subcommand: submit | list".into()))?;

    let mut common = ClientCommon::default();
    let mut rating: Option<i32> = None;
    let mut message_id: Option<String> = None;
    let mut instance: Option<[u8; 16]> = None;
    let mut mote: Option<[u8; 32]> = None;
    let mut content_ref: Option<[u8; 32]> = None;
    let mut comment = String::new();
    let mut handle = String::new();
    let mut model = String::new();
    let mut limit: Option<u32> = None;
    let mut before_rowid: Option<u64> = None;

    while let Some(flag) = args.next() {
        if common.try_consume(&flag, &mut args)? {
            continue;
        }
        match flag.as_str() {
            "--rating" => {
                let v = next_value(&mut args, "--rating")?;
                rating = Some(match v.as_str() {
                    "up" => proto::FeedbackRating::Up as i32,
                    "down" => proto::FeedbackRating::Down as i32,
                    other => {
                        return Err(CliError::Usage(format!(
                            "--rating must be up | down, got {other:?}"
                        )))
                    }
                });
            }
            "--message-id" => message_id = Some(next_value(&mut args, "--message-id")?),
            "--instance" => instance = Some(take_fixed::<_, 16>(&mut args, "--instance")?),
            "--mote" => mote = Some(take_fixed::<_, 32>(&mut args, "--mote")?),
            "--content-ref" => content_ref = Some(take_fixed::<_, 32>(&mut args, "--content-ref")?),
            "--comment" => comment = next_value(&mut args, "--comment")?,
            "--handle" => handle = next_value(&mut args, "--handle")?,
            "--model" => model = next_value(&mut args, "--model")?,
            "--limit" => {
                let v = next_value(&mut args, "--limit")?;
                limit = Some(v.parse::<u32>().map_err(|_| {
                    CliError::Usage(format!("--limit must be a positive integer, got {v:?}"))
                })?);
            }
            "--before-rowid" => {
                let v = next_value(&mut args, "--before-rowid")?;
                before_rowid = Some(v.parse::<u64>().map_err(|_| {
                    CliError::Usage(format!("--before-rowid must be an integer, got {v:?}"))
                })?);
            }
            other => return Err(CliError::Usage(format!("unknown flag {other:?}"))),
        }
    }

    let sub = match kw.as_str() {
        "submit" => {
            let rating = rating.ok_or_else(|| {
                CliError::Usage("feedback submit requires --rating up | down".into())
            })?;
            let message_id = message_id.filter(|s| !s.is_empty()).ok_or_else(|| {
                CliError::Usage("feedback submit requires --message-id <id>".into())
            })?;
            FeedbackSub::Submit(SubmitSpec {
                rating,
                message_id,
                instance,
                mote,
                content_ref,
                comment,
                handle,
                model,
            })
        }
        "list" => FeedbackSub::List(ListSpec {
            instance,
            limit,
            before_rowid,
        }),
        other => {
            return Err(CliError::Usage(format!(
                "unknown feedback subcommand {other:?} (expected submit | list)"
            )))
        }
    };
    Ok(FeedbackArgs { sub, common })
}

/// Execute `feedback`.
pub async fn execute(args: FeedbackArgs) -> Result<(), CliError> {
    let resolved = args.common.resolve()?;
    let mut client = resolved.connect().await?;
    let json = args.common.json;

    match args.sub {
        FeedbackSub::Submit(spec) => {
            let req = proto::SubmitFeedbackRequest {
                rating: spec.rating,
                message_id: spec.message_id,
                instance_id: spec.instance.map(|b| b.to_vec()),
                mote_id: spec.mote.map(|b| b.to_vec()),
                content_ref: spec.content_ref.map(|b| b.to_vec()),
                comment: spec.comment,
                recipe_handle: spec.handle,
                model_id: spec.model,
            };
            let resp = client
                .submit_feedback(resolved.request(req)?)
                .await
                .map_err(degrade)?
                .into_inner();
            println!("{}", format::render_feedback_submit(&resp, json));
        }
        FeedbackSub::List(spec) => {
            let req = proto::ListFeedbackRequest {
                limit: spec.limit,
                instance_id: spec.instance.map(|b| b.to_vec()),
                before_rowid: spec.before_rowid,
            };
            let resp = client
                .list_feedback(resolved.request(req)?)
                .await
                .map_err(degrade)?
                .into_inner();
            println!("{}", format::render_feedback_list(&resp, json));
        }
    }
    Ok(())
}

/// Forward-compat degrade: a gateway without the feedback sidecar answers
/// `Unimplemented` — say so honestly (the telemetry verb precedent).
fn degrade(status: tonic::Status) -> CliError {
    if status.code() == Code::Unimplemented {
        CliError::Rpc {
            code: Code::Unimplemented,
            message: "feedback is not wired on this gateway (upgrade the serve)".into(),
            refusal_code: None,
        }
    } else {
        CliError::from_status(status)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(parts: &[&str]) -> Result<FeedbackArgs, CliError> {
        parse(parts.iter().map(|s| (*s).to_string()))
    }

    #[test]
    fn parses_submit_with_all_fields() {
        let a = p(&[
            "submit",
            "--rating",
            "up",
            "--message-id",
            "answer-9",
            "--instance",
            &"ab".repeat(16),
            "--mote",
            &"cd".repeat(32),
            "--content-ref",
            &"ef".repeat(32),
            "--comment",
            "great",
            "--handle",
            "kx/recipes/chat",
            "--model",
            "qwen3",
            "--json",
        ])
        .unwrap();
        let FeedbackSub::Submit(s) = a.sub else {
            panic!("expected Submit");
        };
        assert_eq!(s.rating, proto::FeedbackRating::Up as i32);
        assert_eq!(s.message_id, "answer-9");
        assert_eq!(s.instance, Some([0xab; 16]));
        assert_eq!(s.mote, Some([0xcd; 32]));
        assert_eq!(s.content_ref, Some([0xef; 32]));
        assert_eq!(s.comment, "great");
        assert!(a.common.json);
    }

    #[test]
    fn parses_list_with_pagination() {
        let a = p(&[
            "list",
            "--instance",
            &"ab".repeat(16),
            "--limit",
            "50",
            "--before-rowid",
            "9",
        ])
        .unwrap();
        let FeedbackSub::List(s) = a.sub else {
            panic!("expected List");
        };
        assert_eq!(s.instance, Some([0xab; 16]));
        assert_eq!(s.limit, Some(50));
        assert_eq!(s.before_rowid, Some(9));
    }

    #[test]
    fn submit_requires_rating_and_message_id() {
        assert!(p(&["submit", "--message-id", "m"]).is_err(), "no rating");
        assert!(p(&["submit", "--rating", "up"]).is_err(), "no message-id");
        assert!(
            p(&["submit", "--rating", "meh", "--message-id", "m"]).is_err(),
            "bad rating"
        );
    }

    #[test]
    fn bad_inputs_are_usage_errors() {
        assert!(p(&[]).is_err(), "no subcommand");
        assert!(p(&["history"]).is_err(), "unknown subcommand");
        assert!(p(&["list", "--limit", "many"]).is_err());
        assert!(
            p(&["list", "--instance", &"ab".repeat(32)]).is_err(),
            "wrong id length"
        );
        assert!(p(&["list", "--bogus"]).is_err());
    }
}
