//! Per-hosted-app scoped credentials — the authenticated channel from a served page back to
//! the runtime.
//!
//! # Why the operator's bearer can never be the answer
//!
//! A hosted app is a web page. Anything it can read, its user can read, its browser extensions
//! can read, and anyone looking over its shoulder can read. Handing that page the operator's
//! token would hand every visitor the operator's whole authority — the opposite of what an
//! App's declared capabilities are for. So the supervisor mints a credential per running app,
//! scoped to that app, and drops it when the app stops.
//!
//! # What a scoped token may do, and why it is that
//!
//! It may run the Apps its own envelope DECLARES in `references.apps`, and read the runs it
//! started. That is the whole surface.
//!
//! The declaration is the point. A page cannot name a tool, a model, or a credential; it can
//! only ask for work its author already wrote down, and each of those Apps re-resolves its own
//! warrants from the operator's grants when it runs. So the reachable authority is
//! `what the App declared ∩ what the operator can actually do` — bounded twice, and bounded by
//! artifacts a reviewer can read rather than by anything the page sends.
//!
//! An empty `references.apps` is therefore a hosted app whose page can start nothing. That is
//! the honest default: composing is opt-in, and a page that was never given anything to run
//! should not be able to run something.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};

/// The set of loopback origins hosted apps are CURRENTLY served on.
///
/// A hosted app's page runs on its own `http://127.0.0.1:<port>` and calls the gateway
/// cross-origin, so the gateway's CORS must allow that origin — but the port is assigned when
/// the app starts, long after the CORS layer was built. So the layer consults this live set
/// through a predicate instead of a fixed allowlist: the supervisor adds a port when a child
/// starts serving and removes it when the app stops, and only the origins actually being
/// served are ever allowed. A dead app's origin stops being permitted the moment it stops.
#[derive(Debug, Default)]
pub(crate) struct HostedOrigins {
    ports: RwLock<BTreeSet<u16>>,
}

impl HostedOrigins {
    #[must_use]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Mark `port` as live (a hosted app started serving on it).
    #[cfg_attr(not(feature = "hosted-apps"), allow(dead_code))]
    pub(crate) fn insert(&self, port: u16) {
        if let Ok(mut p) = self.ports.write() {
            p.insert(port);
        }
    }

    /// Mark `port` as gone (the app stopped / failed).
    #[cfg_attr(not(feature = "hosted-apps"), allow(dead_code))]
    pub(crate) fn remove(&self, port: u16) {
        if let Ok(mut p) = self.ports.write() {
            p.remove(&port);
        }
    }

    /// Whether `origin` is a loopback origin currently being served.
    ///
    /// Exact scheme + loopback host + a live port. `https` is not accepted: a hosted app is
    /// served over plain `http` on loopback (TLS/public URLs are Cloud), so an `https` origin
    /// on the same port is a different, un-served thing.
    #[must_use]
    pub(crate) fn allows(&self, origin: &str) -> bool {
        let Some(rest) = origin.strip_prefix("http://") else {
            return false;
        };
        let Some((host, port)) = rest.rsplit_once(':') else {
            return false;
        };
        if host != "127.0.0.1" && host != "localhost" {
            return false;
        }
        let Ok(port) = port.parse::<u16>() else {
            return false;
        };
        self.ports.read().is_ok_and(|p| p.contains(&port))
    }
}

/// What one minted token is allowed to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AppScope {
    /// The party the hosted app belongs to. Runs authored through this token bind the SAME
    /// party's grants — the token narrows what may be asked for, never who is asking.
    pub(crate) party: String,
    /// The hosted app's own catalog handle (audit + the log line; not an authority).
    pub(crate) handle: String,
    /// The App handles this page may run — the hosted envelope's `references.apps`, resolved
    /// once at mint. Held by VALUE rather than re-read per request so a live page's reach
    /// cannot silently widen when the envelope is edited underneath it; restarting the app
    /// re-mints and picks the change up, which is a visible act.
    pub(crate) runnable: Vec<String>,
}

/// The live token → scope map. Written by the hosted-app supervisor (mint on start, drop on
/// stop), read by the auth resolver on every request.
#[derive(Debug, Default)]
pub(crate) struct AppTokenStore {
    by_token: RwLock<HashMap<String, AppScope>>,
}

impl AppTokenStore {
    #[must_use]
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Mint a fresh token for a hosted app, replacing any it already had.
    ///
    /// Replacing rather than reusing means a restart invalidates the previous page's
    /// credential: a token that outlived the app it was minted for would be a credential with
    /// no owner to revoke it.
    ///
    /// 32 bytes of OS randomness, hex-encoded. Not derived from the handle or the party —
    /// a guessable credential is not a credential, and both of those are discoverable.
    #[cfg_attr(not(feature = "hosted-apps"), allow(dead_code))]
    pub(crate) fn mint(&self, party: &str, handle: &str, runnable: Vec<String>) -> String {
        let mut raw = [0u8; 32];
        // A failure here means the OS entropy source is broken; falling back to anything
        // predictable would hand out a guessable credential, so refuse to invent one. The
        // panic is the correct fail-closed: a runtime that cannot generate a secret must not
        // pretend it did.
        #[allow(clippy::expect_used)]
        {
            getrandom::fill(&mut raw).expect("OS entropy is available for a scoped app token");
        }
        let token = raw.iter().fold(String::with_capacity(64), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        });
        let scope = AppScope {
            party: party.to_string(),
            handle: handle.to_string(),
            runnable,
        };
        if let Ok(mut map) = self.by_token.write() {
            map.retain(|_, s| !(s.party == party && s.handle == handle));
            map.insert(token.clone(), scope);
        }
        token
    }

    /// Drop every token for one hosted app (called when it stops).
    #[cfg_attr(not(feature = "hosted-apps"), allow(dead_code))]
    pub(crate) fn revoke(&self, party: &str, handle: &str) {
        if let Ok(mut map) = self.by_token.write() {
            map.retain(|_, s| !(s.party == party && s.handle == handle));
        }
    }

    /// Resolve a bearer token to its scope, or `None` when it is not one of ours.
    ///
    /// A poisoned lock resolves to `None` — an auth store that cannot be read must deny, never
    /// admit.
    pub(crate) fn resolve(&self, token: &str) -> Option<AppScope> {
        self.by_token.read().ok()?.get(token).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hosted_origins_allow_only_live_loopback_ports() {
        let o = HostedOrigins::new();
        assert!(!o.allows("http://127.0.0.1:5173"), "nothing live yet");
        o.insert(5173);
        assert!(o.allows("http://127.0.0.1:5173"));
        assert!(o.allows("http://localhost:5173"));
        assert!(!o.allows("https://127.0.0.1:5173"), "https is not served");
        assert!(!o.allows("http://127.0.0.1:5174"), "a different port");
        assert!(!o.allows("http://evil.example:5173"), "not loopback");
        o.remove(5173);
        assert!(!o.allows("http://127.0.0.1:5173"), "gone when the app stops");
    }

    #[test]
    fn a_minted_token_resolves_to_its_own_scope_and_nothing_else() {
        let store = AppTokenStore::new();
        let t = store.mint("alice@acme", "apps/local/site", vec!["apps/local/research".into()]);
        let scope = store.resolve(&t).expect("the minted token resolves");
        assert_eq!(scope.party, "alice@acme");
        assert!(scope.runnable.contains(&"apps/local/research".to_string()));
        assert!(
            !scope.runnable.contains(&"apps/local/payroll".to_string()),
            "undeclared App refused"
        );
        assert!(store.resolve("not-a-token").is_none());
    }

    /// A restart must invalidate the previous page's credential. A token that outlived the app
    /// it was minted for would be a credential nobody can revoke.
    #[test]
    fn re_minting_invalidates_the_previous_token() {
        let store = AppTokenStore::new();
        let first = store.mint("alice@acme", "apps/local/site", vec![]);
        let second = store.mint("alice@acme", "apps/local/site", vec![]);
        assert_ne!(first, second);
        assert!(store.resolve(&first).is_none(), "the old token is dead");
        assert!(store.resolve(&second).is_some());
    }

    #[test]
    fn revoke_drops_only_that_apps_tokens() {
        let store = AppTokenStore::new();
        let mine = store.mint("alice@acme", "apps/local/site", vec![]);
        let other = store.mint("alice@acme", "apps/local/other", vec![]);
        store.revoke("alice@acme", "apps/local/site");
        assert!(store.resolve(&mine).is_none());
        assert!(store.resolve(&other).is_some(), "a sibling app is untouched");
    }

    /// Two mints must not collide, and a token must not be derivable from what a visitor can
    /// already see (the handle and the party are both discoverable).
    #[test]
    fn tokens_are_unguessable_and_distinct() {
        let store = AppTokenStore::new();
        let a = store.mint("alice@acme", "apps/local/site", vec![]);
        let b = store.mint("alice@acme", "apps/local/other", vec![]);
        assert_ne!(a, b);
        assert_eq!(a.len(), 64, "32 bytes hex-encoded");
        assert!(!a.contains("site") && !a.contains("alice"));
    }
}
