//! The LOCAL secret store: one operator-visible file, plus the `SecretAdmin` write surface.
//!
//! A connector's `credential_ref` resolves by NAME against this store first and the host
//! environment second ([`kx_mcp::EnvSecretStore`]), so a local agent can authenticate real
//! services without exporting credentials into the process environment.
//!
//! ## Why a file rather than the OS keychain
//! The store is a single JSON file under the catalog dir, alongside the other off-journal
//! sidecars. An operator can open it, see exactly which credentials the runtime holds, and
//! paste a new one in — which an OS keychain cannot offer, and which is the point of a
//! local-first runtime. It also keeps the build free of a native per-platform dependency.
//!
//! **This is deliberately not a vault.** The values are plaintext on disk, protected by file
//! permissions alone: any process running as the operator can read them. The mitigations here
//! are that the file is created `0600`, is REFUSED at open if its mode is broader than that,
//! and never leaves the host. The hardened multi-tenant vault (rotation, audit, envelope
//! encryption) lives behind the same [`kx_mcp::SecretStore`] seam (D94); OSS ships the honest
//! local store and makes no "best-cryptography vault" claim.
//!
//! ## Security posture (D81)
//! - Secrets are resolved **by NAME** ([`kx_warrant::SecretRef`]); the value is read
//!   transiently at transport setup, injected into a header / child env, and dropped. It is
//!   NEVER journaled, in a `MoteId`/`StepRecord`, or the model's context. The broker
//!   `secret_scope` precheck (`kx-capability`) remains the sole authorization gate — this
//!   module is the resolve/store MECHANISM only.
//! - Every operation re-reads the file rather than caching it. The admin surface and the
//!   transport resolver are separate callers, so a cache would let a `PutSecret` be invisible
//!   to the very next run; re-reading keeps them coherent by construction. Secrets are few
//!   and reads are rare (transport setup), so the cost is not worth a cache-coherence bug.
//! - A corrupt or over-permissive file is REFUSED at open, never silently recreated empty.
//!   Recreating would destroy the operator's credentials, and an enumeration that silently
//!   empties reports "you have no secrets" — a worse answer than an error. The gateway then
//!   leaves the three secret RPCs `unimplemented` and the file untouched.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use kx_gateway_core::{SecretAdmin, SecretAdminError, SecretNameView};

use crate::error::GatewayError;

/// The store file name, under the catalog dir beside the other sidecars.
const SECRETS_FILE: &str = "secrets.json";

/// Default / maximum `ListSecretNames` page size.
const DEFAULT_LIST_LIMIT: u32 = 200;
const MAX_LIST_LIMIT: u32 = 1000;

/// One stored credential.
///
/// Two shapes are accepted on READ so the file is genuinely hand-editable: the bare form
/// `"NAME": "value"` is what an operator writes when pasting a key, and the full form carries
/// the advisory timestamps `ListSecretNames` reports. Writes always emit the full form, so a
/// hand-added bare entry gains timestamps the next time the runtime rewrites the file.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum SecretEntry {
    /// `"NAME": "value"` — the hand-written short form.
    Bare(String),
    /// `"NAME": { "value": …, "created_unix_ms": …, "updated_unix_ms": … }`.
    Full {
        value: String,
        #[serde(default)]
        created_unix_ms: u64,
        #[serde(default)]
        updated_unix_ms: u64,
    },
}

impl SecretEntry {
    fn value(&self) -> &str {
        match self {
            Self::Bare(v) | Self::Full { value: v, .. } => v,
        }
    }

    fn created(&self) -> u64 {
        match self {
            Self::Bare(_) => 0,
            Self::Full {
                created_unix_ms, ..
            } => *created_unix_ms,
        }
    }

    fn updated(&self) -> u64 {
        match self {
            Self::Bare(_) => 0,
            Self::Full {
                updated_unix_ms, ..
            } => *updated_unix_ms,
        }
    }
}

/// `BTreeMap` so the file is written in a stable NAME order — a hand-edited file stays
/// diffable across runtime rewrites, and `list_names` keyset paging is already name-ordered.
type SecretMap = BTreeMap<String, SecretEntry>;

/// The file-backed secret store: the `SecretAdmin` write surface AND (under `mcp-gateway`)
/// the [`kx_mcp::SecretStore`] resolver arm, over one file so both see the same bytes.
pub(crate) struct SecretFile {
    path: PathBuf,
    tmp: PathBuf,
}

// Manual `Debug` — print ONLY the path, so a future field on this type can never reach a log
// line through a derived impl. The type holds no secret material today and this keeps it so;
// same reasoning as `kx_mcp::CredentialRef`.
impl std::fmt::Debug for SecretFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretFile")
            .field("path", &self.path)
            .finish_non_exhaustive()
    }
}

impl SecretFile {
    /// Open the store under `dir`, validating an existing file's mode and syntax.
    ///
    /// A missing file is the normal empty state and is NOT created here — it appears on the
    /// first `put`, with the right mode from the start.
    ///
    /// # Errors
    /// [`GatewayError::Catalog`] if the file exists but is group/other-accessible, or does not
    /// parse. Both are refused rather than repaired: the first would silently accept a
    /// credential leak, the second would silently discard credentials.
    pub(crate) fn open(dir: &Path) -> Result<Self, GatewayError> {
        let path = dir.join(SECRETS_FILE);
        let me = Self {
            tmp: dir.join(format!("{SECRETS_FILE}.tmp")),
            path,
        };
        if me.path.exists() {
            me.check_mode()?;
            // Parse once at open so a corrupt file is an honest startup failure rather than a
            // per-call surprise on the transport path.
            me.load()
                .map_err(|e| GatewayError::Catalog(format!("{}: {e}", me.path.display())))?;
        }
        Ok(me)
    }

    /// Refuse a file any other local user can read. Permissions are the ONLY protection these
    /// values have, so a broadened mode is a real exposure and silently accepting it would
    /// make the guarantee in this module's header false.
    #[cfg(unix)]
    fn check_mode(&self) -> Result<(), GatewayError> {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(&self.path)
            .map_err(|e| GatewayError::Catalog(format!("{}: {e}", self.path.display())))?
            .permissions()
            .mode()
            & 0o777;
        if mode & 0o077 != 0 {
            return Err(GatewayError::Catalog(format!(
                "{} is mode {mode:04o}: it must not be readable by group or others. \
                 Run: chmod 600 {}",
                self.path.display(),
                self.path.display()
            )));
        }
        Ok(())
    }

    #[cfg(not(unix))]
    fn check_mode(&self) -> Result<(), GatewayError> {
        Ok(())
    }

    /// Read the current contents. A missing file is an empty store, not an error.
    fn load(&self) -> Result<SecretMap, SecretAdminError> {
        let bytes = match std::fs::read(&self.path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(SecretMap::new()),
            Err(e) => return Err(SecretAdminError::Storage(format!("secrets read: {e}"))),
        };
        if bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(SecretMap::new());
        }
        serde_json::from_slice(&bytes)
            .map_err(|e| SecretAdminError::Storage(format!("secrets parse: {e}")))
    }

    /// Replace the file atomically: write a fresh `0600` temp beside it, then rename over the
    /// target. A crash therefore leaves either the old file or the new one, never a truncated
    /// store — losing a credential to a half-written file is unrecoverable for the operator.
    fn store(&self, map: &SecretMap) -> Result<(), SecretAdminError> {
        let json = serde_json::to_vec_pretty(map)
            .map_err(|e| SecretAdminError::Storage(format!("secrets encode: {e}")))?;
        // Remove any stale temp first: `mode()` below applies only on CREATE, so reusing a
        // leftover file would inherit its permissions instead of 0600.
        let _ = std::fs::remove_file(&self.tmp);
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            opts.mode(0o600);
        }
        let mut f = opts
            .open(&self.tmp)
            .map_err(|e| SecretAdminError::Storage(format!("secrets temp: {e}")))?;
        f.write_all(&json)
            .and_then(|()| f.sync_all())
            .map_err(|e| SecretAdminError::Storage(format!("secrets write: {e}")))?;
        drop(f);
        std::fs::rename(&self.tmp, &self.path)
            .map_err(|e| SecretAdminError::Storage(format!("secrets rename: {e}")))
    }

    /// Resolve a secret VALUE by NAME. `None` ⇒ absent or unreadable — the runtime then never
    /// fabricates a credential and the far end fails its own auth.
    pub(crate) fn resolve(&self, name: &str) -> Option<String> {
        self.load().ok()?.get(name).map(|e| e.value().to_string())
    }
}

/// The store arm of the host `ChainedSecretStore { file → env }`: a name in the file wins; a
/// name present only in the environment still resolves (back-compat). Gated on `mcp-gateway`
/// because that is the feature under which `kx-mcp` (and the seam it provides) is in the build.
#[cfg(feature = "mcp-gateway")]
impl kx_mcp::SecretStore for SecretFile {
    fn resolve(&self, secret_ref: &kx_warrant::SecretRef) -> Option<String> {
        Self::resolve(self, &secret_ref.0)
    }
}

/// Resolve a secret VALUE by NAME through the file-then-environment chain — the same
/// precedence the connector transport resolver uses. Used by the D113 webhook listener to
/// fetch a trigger's HMAC/bearer verify key. `None` ⇒ unresolvable (the webhook then fails
/// closed; never a fabricated credential). Takes the store explicitly rather than reaching for
/// a global, so a serve with no readable store degrades to the environment arm alone.
pub(crate) fn resolve_secret_value(store: Option<&SecretFile>, name: &str) -> Option<String> {
    store
        .and_then(|s| s.resolve(name))
        .or_else(|| std::env::var(name).ok())
}

/// Wall-clock ms since epoch (off-digest; advisory timestamps only).
fn now_unix_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

impl SecretAdmin for SecretFile {
    fn put(&self, name: &str, value: &str) -> Result<(), SecretAdminError> {
        let mut map = self.load()?;
        let now = now_unix_ms();
        // Preserve `created_unix_ms` across an overwrite: the operator's question is "when did
        // I first store this credential", which a rotation does not reset.
        let created = map.get(name).map_or(now, |e| match e.created() {
            0 => now,
            c => c,
        });
        map.insert(
            name.to_string(),
            SecretEntry::Full {
                value: value.to_string(),
                created_unix_ms: created,
                updated_unix_ms: now,
            },
        );
        self.store(&map)
    }

    fn list_names(
        &self,
        limit: u32,
        after_name: &str,
    ) -> Result<(Vec<SecretNameView>, bool), SecretAdminError> {
        let lim = match limit {
            0 => DEFAULT_LIST_LIMIT,
            n => n.min(MAX_LIST_LIMIT),
        } as usize;
        let map = self.load()?;
        // Keyset page: names strictly after the cursor, in name order (the map is already
        // sorted). Take lim+1 to detect `has_more` without a second pass.
        let mut rows: Vec<SecretNameView> = map
            .range(after_name.to_string()..)
            .filter(|(n, _)| n.as_str() > after_name)
            .take(lim + 1)
            .map(|(n, e)| SecretNameView {
                name: n.clone(),
                created_unix_ms: e.created(),
                updated_unix_ms: e.updated(),
            })
            .collect();
        let has_more = rows.len() > lim;
        rows.truncate(lim);
        Ok((rows, has_more))
    }

    fn delete(&self, name: &str) -> Result<bool, SecretAdminError> {
        let mut map = self.load()?;
        if map.remove(name).is_none() {
            return Ok(false);
        }
        self.store(&map)?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn admin(dir: &Path) -> SecretFile {
        SecretFile::open(dir).unwrap()
    }

    fn secrets_path(dir: &Path) -> PathBuf {
        dir.join(SECRETS_FILE)
    }

    /// Hand-write the store the way an operator must: content, then `chmod 600`. Plain
    /// `fs::write` lands at 0644 under the usual umask, which `open` refuses by design — so a
    /// test that skipped the chmod would fail on the MODE while claiming to test something
    /// else.
    fn write_store(dir: &Path, content: &[u8]) {
        let p = secrets_path(dir);
        std::fs::write(&p, content).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        }
    }

    #[test]
    fn put_indexes_name_and_stores_value() {
        let dir = tempfile::tempdir().unwrap();
        let a = admin(dir.path());
        a.put("GITHUB_TOKEN", "ghp_secret").unwrap();
        assert_eq!(a.resolve("GITHUB_TOKEN").as_deref(), Some("ghp_secret"));
        let (names, has_more) = a.list_names(0, "").unwrap();
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].name, "GITHUB_TOKEN");
        assert!(!has_more);
    }

    /// The value lives in exactly ONE file. This is the inverse of the assertion the keychain
    /// store carried (that no value reached the NAME index): now that names and values share a
    /// file, what must be shown is that nothing ELSE in the catalog dir holds the value.
    #[test]
    fn the_value_is_in_the_secrets_file_and_no_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = admin(dir.path());
        a.put("API_KEY", "TOP-SECRET-VALUE-12345").unwrap();
        let needle = b"TOP-SECRET-VALUE-12345";
        let mut found_in_secrets = false;
        for entry in std::fs::read_dir(dir.path()).unwrap() {
            let p = entry.unwrap().path();
            let Ok(bytes) = std::fs::read(&p) else {
                continue;
            };
            let hit = bytes.windows(needle.len()).any(|w| w == needle);
            if p == secrets_path(dir.path()) {
                found_in_secrets = hit;
            } else {
                assert!(!hit, "secret value leaked into {p:?}");
            }
        }
        assert!(
            found_in_secrets,
            "the store must actually hold the value, or the leak scan above proves nothing"
        );
    }

    #[test]
    fn list_names_keyset_pages_and_reports_has_more() {
        let dir = tempfile::tempdir().unwrap();
        let a = admin(dir.path());
        for n in ["a", "b", "c", "d"] {
            a.put(n, "v").unwrap();
        }
        let (page1, has_more) = a.list_names(2, "").unwrap();
        assert_eq!(
            page1.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        assert!(has_more);
        let (page2, has_more2) = a.list_names(2, "b").unwrap();
        assert_eq!(
            page2.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            ["c", "d"]
        );
        assert!(!has_more2);
    }

    #[test]
    fn delete_removes_the_value_and_the_name() {
        let dir = tempfile::tempdir().unwrap();
        let a = admin(dir.path());
        a.put("TOK", "v").unwrap();
        assert!(a.delete("TOK").unwrap(), "existing secret reports removed");
        assert!(a.resolve("TOK").is_none());
        assert!(a.list_names(0, "").unwrap().0.is_empty());
        assert!(
            !a.delete("TOK").unwrap(),
            "absent secret reports not-removed"
        );
    }

    #[test]
    fn put_overwrites_value_keeps_created_ms() {
        let dir = tempfile::tempdir().unwrap();
        let a = admin(dir.path());
        a.put("K", "v1").unwrap();
        let created = a.list_names(0, "").unwrap().0[0].created_unix_ms;
        a.put("K", "v2").unwrap();
        assert_eq!(a.resolve("K").as_deref(), Some("v2"), "value overwritten");
        let row = a.list_names(0, "").unwrap().0;
        assert_eq!(row.len(), 1, "still one NAME");
        assert_eq!(
            row[0].created_unix_ms, created,
            "created_ms preserved on overwrite"
        );
    }

    /// A corrupt store is REFUSED, not recreated. Recreating would silently destroy every
    /// credential the operator had stored.
    #[test]
    fn a_corrupt_file_is_refused_rather_than_recreated() {
        let dir = tempfile::tempdir().unwrap();
        write_store(dir.path(), b"not json at all");
        let err = SecretFile::open(dir.path()).expect_err("a corrupt store is refused");
        let msg = format!("{err}");
        assert!(
            msg.contains("secrets.json") && msg.contains("parse"),
            "the refusal names the file AND says it is a parse failure, so the operator is not \
             sent chasing a permissions problem; got {msg}"
        );
        // The ACCEPTING control, one variable changed: valid content at the same path, same
        // mode, opens. Without it this assertion would pass on any open failure at all.
        write_store(dir.path(), br#"{"K":"v"}"#);
        let a = SecretFile::open(dir.path()).expect("valid content opens");
        assert_eq!(a.resolve("K").as_deref(), Some("v"));
    }

    /// The hand-edit path: an operator pastes `"NAME": "value"` and the runtime reads it,
    /// then rewrites it into the timestamped form on the next write.
    #[test]
    fn a_hand_written_bare_entry_resolves_and_is_upgraded_on_write() {
        let dir = tempfile::tempdir().unwrap();
        write_store(dir.path(), br#"{"PASTED":"abc123"}"#);
        let a = admin(dir.path());
        assert_eq!(a.resolve("PASTED").as_deref(), Some("abc123"));
        let (names, _) = a.list_names(0, "").unwrap();
        assert_eq!(names.len(), 1, "a bare entry is enumerable");
        // Writing another name rewrites the file; the pasted one keeps its value.
        a.put("OTHER", "z").unwrap();
        let b = admin(dir.path());
        assert_eq!(b.resolve("PASTED").as_deref(), Some("abc123"));
        assert_eq!(b.resolve("OTHER").as_deref(), Some("z"));
    }

    #[cfg(unix)]
    #[test]
    fn a_group_or_world_readable_store_is_refused_and_names_the_fix() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let p = secrets_path(dir.path());
        std::fs::write(&p, br#"{"K":"v"}"#).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o644)).unwrap();
        let err = SecretFile::open(dir.path()).expect_err("a world-readable store is refused");
        let msg = format!("{err}");
        assert!(
            msg.contains("0644") && msg.contains("chmod 600"),
            "the refusal states the offending mode AND the fix; got {msg}"
        );
        // The ACCEPTING control: the SAME file at 0600 opens. Without it this assertion would
        // pass on any open failure at all — a missing dir, a parse error, a bad path.
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
        SecretFile::open(dir.path()).expect("the same file at 0600 opens");
    }

    #[cfg(unix)]
    #[test]
    fn a_store_the_runtime_creates_is_0600() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let a = admin(dir.path());
        a.put("K", "v").unwrap();
        let mode = std::fs::metadata(secrets_path(dir.path()))
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "the runtime creates the store owner-only");
    }
}
