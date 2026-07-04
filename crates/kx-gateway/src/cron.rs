//! D113: the local cron ticker — fires due interval-scheduled triggers.
//!
//! A `CRON` trigger carries an interval (`schedule_spec` = seconds) and a `next_fire`
//! watermark in `triggers.db`. This host task polls for due triggers, advances each
//! watermark FIRST (so a slow submit or the next tick never re-picks it), then fires
//! via the SAME [`TriggerAdmin::submit`] path the webhook + gRPC use — with a derived
//! idempotency key bound to the scheduled tick, so a retried/raced tick never
//! double-fires. The watermark is off-journal; a process restart re-reads it + resumes.
//!
//! Host-owned (gateway-core stays tokio-free); aborted on gateway shutdown like the
//! other auxiliary tasks. Only spawned when the trigger seam is wired.

use std::sync::Arc;
use std::time::Duration;

use kx_gateway_core::TriggerAdmin;

use crate::triggers_store::TriggersDb;

/// How often to scan for due cron triggers. A trigger fires within this granularity of
/// its scheduled time — fine for a local interval scheduler (cron-at-scale is CLOUD).
const POLL_CADENCE: Duration = Duration::from_secs(5);

fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Run the cron ticker until aborted. Each tick: read due triggers, advance their
/// watermark, then fire each with a tick-derived idempotency key.
pub(crate) async fn serve_cron(triggers: Arc<TriggersDb>, admin: Arc<dyn TriggerAdmin>) {
    let mut interval = tokio::time::interval(POLL_CADENCE);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        interval.tick().await;
        let now = now_unix_ms();
        let due = match triggers.due_cron(now) {
            Ok(d) => d,
            Err(error) => {
                tracing::warn!(%error, "cron: due-scan failed; continuing");
                continue;
            }
        };
        for cfg in due {
            // The idempotency key binds this fire to the scheduled tick (the watermark
            // that made it due) — a retried/raced tick with the same watermark dedups.
            let key = format!("cron:{}:{}", cfg.name, cfg.next_fire_unix_ms);
            // Advance the watermark FIRST so neither a slow submit nor the next tick
            // re-picks this trigger. A failed submit = one missed fire (next interval
            // fires); the local cron makes no exactly-once delivery claim (that is CLOUD).
            // `next_fire` resolves BOTH a legacy interval-seconds spec and a 5-field
            // crontab expression (in the trigger's timezone, DST-correct). Register-time
            // validation makes a runtime error unexpected — the defensive path backs off
            // an hour rather than hot-loop the 5s poll on a corrupt row.
            let next = match crate::schedule::next_fire(&cfg.schedule_spec, &cfg.timezone, now) {
                Ok(n) => n,
                Err(error) => {
                    tracing::warn!(%error, trigger = %cfg.name, "cron: schedule invalid at fire; backing off 1h");
                    now.saturating_add(3_600_000)
                }
            };
            if let Err(error) = triggers.set_next_fire(&cfg.name, next) {
                tracing::warn!(%error, trigger = %cfg.name, "cron: watermark advance failed; skipping");
                continue;
            }
            match admin.submit(&cfg.name, &key, "{}").await {
                Ok(out) => tracing::info!(
                    trigger = %cfg.name,
                    deduped = out.deduped,
                    "cron: fired"
                ),
                Err(error) => {
                    tracing::warn!(%error, trigger = %cfg.name, "cron: fire failed");
                }
            }
        }
    }
}
