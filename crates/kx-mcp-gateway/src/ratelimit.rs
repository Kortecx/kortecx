//! Per-server token-bucket rate limiting for the live egress surface.
//!
//! A hostile or runaway server interaction is bounded by a per-`server` token
//! bucket: each dial (`register` / `discover` / `test`, and — in PR-6b-2 — each
//! tool fire) must acquire a token. Integer math only (no floats — SN-8
//! discipline); refill is wall-clock-based, which is sound because rate-limiting
//! is OFF the digest/journal path (a pure egress guard, never an identity input).

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Instant;

/// A per-server token bucket. `tokens` are integers; `refill_per_sec` tokens are
/// added back over wall-clock time up to `capacity`.
struct Bucket {
    tokens: u32,
    last_refill: Instant,
}

/// A per-server-name token-bucket rate limiter (deny-on-empty, fail-closed).
pub struct RateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
    capacity: u32,
    refill_per_sec: u32,
}

impl std::fmt::Debug for RateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RateLimiter")
            .field("capacity", &self.capacity)
            .field("refill_per_sec", &self.refill_per_sec)
            .finish_non_exhaustive()
    }
}

impl RateLimiter {
    /// Build a limiter with `capacity` burst tokens per server, refilling
    /// `refill_per_sec` tokens/second. Both are clamped to at least 1.
    #[must_use]
    pub fn new(capacity: u32, refill_per_sec: u32) -> Self {
        Self {
            buckets: Mutex::new(HashMap::new()),
            capacity: capacity.max(1),
            refill_per_sec: refill_per_sec.max(1),
        }
    }

    /// Try to acquire one token for `server`. `true` ⇒ permitted (token consumed);
    /// `false` ⇒ over budget (refuse the dial fail-closed).
    ///
    /// Uses [`Instant::now`] for refill. A poisoned mutex fails OPEN (permits) —
    /// a rate limiter must never wedge the whole gateway on an internal panic.
    #[must_use]
    pub fn try_acquire(&self, server: &str) -> bool {
        self.try_acquire_at(server, Instant::now())
    }

    /// Testable core: the same logic with an injected `now`.
    fn try_acquire_at(&self, server: &str, now: Instant) -> bool {
        let Ok(mut buckets) = self.buckets.lock() else {
            return true; // fail-open on a poisoned lock (never wedge the gateway)
        };
        let bucket = buckets.entry(server.to_string()).or_insert(Bucket {
            tokens: self.capacity,
            last_refill: now,
        });
        // Integer refill: floor(elapsed_ms * rate / 1000), capped at `capacity`.
        let elapsed_ms = now
            .saturating_duration_since(bucket.last_refill)
            .as_millis();
        let capped = (elapsed_ms.saturating_mul(u128::from(self.refill_per_sec)) / 1000)
            .min(u128::from(self.capacity));
        // `capped <= capacity` (a u32) ⇒ the conversion never truncates.
        let refill = u32::try_from(capped).unwrap_or(self.capacity);
        if refill > 0 {
            bucket.tokens = bucket.tokens.saturating_add(refill).min(self.capacity);
            bucket.last_refill = now;
        }
        if bucket.tokens > 0 {
            bucket.tokens -= 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn bursts_up_to_capacity_then_refuses() {
        let rl = RateLimiter::new(3, 1);
        let t0 = Instant::now();
        assert!(rl.try_acquire_at("s", t0));
        assert!(rl.try_acquire_at("s", t0));
        assert!(rl.try_acquire_at("s", t0));
        assert!(
            !rl.try_acquire_at("s", t0),
            "4th in the same instant is refused"
        );
    }

    #[test]
    fn refills_over_time() {
        let rl = RateLimiter::new(2, 4); // 4 tokens/sec
        let t0 = Instant::now();
        assert!(rl.try_acquire_at("s", t0));
        assert!(rl.try_acquire_at("s", t0));
        assert!(!rl.try_acquire_at("s", t0));
        // 500ms later: 4*0.5 = 2 tokens refilled (capped at capacity 2).
        let t1 = t0 + Duration::from_millis(500);
        assert!(rl.try_acquire_at("s", t1));
        assert!(rl.try_acquire_at("s", t1));
        assert!(!rl.try_acquire_at("s", t1));
    }

    #[test]
    fn per_server_isolation() {
        let rl = RateLimiter::new(1, 1);
        let t0 = Instant::now();
        assert!(rl.try_acquire_at("a", t0));
        assert!(!rl.try_acquire_at("a", t0));
        // A different server has its own fresh bucket.
        assert!(rl.try_acquire_at("b", t0));
    }
}
