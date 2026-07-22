// SPDX-License-Identifier: LicenseRef-Kortecx-Sustainable-Use-1.0
//! G1a durable membership backend (D94): the membership ledger survives a process
//! restart. Mirrors `kx-catalog/tests/durable_backends.rs` — `run_with_each_backend`
//! holds the SQLite impl to the SAME contract as the in-memory one; a write → drop →
//! reopen sweep proves the FOLD (not just raw facts) survives — including the
//! time-ordered removal / re-admit semantics that depend on the persisted `seq`
//! order — plus atomicity-under-panic and a loud schema-version-mismatch refusal.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::pedantic)]

use kx_catalog::{CatalogAction, CatalogActionSet, PartyId};
use kx_fleet::{
    Admit, Disband, InMemoryMembershipLedger, MembershipLedger, MembershipLedgerError,
    MembershipOutcome, Removal, SqliteMembershipLedger, Team,
};
use kx_mote::ModelId;
use kx_warrant::{ModelRoute, Role, WarrantSpec};

// --- fixtures --------------------------------------------------------------

fn warrant(max_calls: u32) -> WarrantSpec {
    WarrantSpec {
        model_route: ModelRoute {
            model_id: ModelId("m".into()),
            max_input_tokens: 1_000,
            max_output_tokens: 1_000,
            max_calls,
        },
        ..Default::default()
    }
}
fn role(name: &str, max_calls: u32) -> Role {
    Role {
        name: name.into(),
        version: 1,
        spec: warrant(max_calls),
        description: String::new(),
    }
}
fn team_p() -> PartyId {
    PartyId::new("team:sre@acme")
}
fn owner() -> PartyId {
    PartyId::new("admin@acme")
}
fn admit(member: &str, r: &str, caps: impl IntoIterator<Item = CatalogAction>) -> Admit {
    Admit::new(
        team_p(),
        PartyId::new(member),
        owner(),
        role(r, 10),
        CatalogActionSet::allow(caps),
    )
}

/// Seed a ledger with a team + three admits (alice/bob/carol), then remove bob — so
/// the FOLD (effective set, not raw facts) is exercised across a restart.
fn seed_team(ledger: &dyn MembershipLedger) {
    ledger
        .append_founding(Team::found(team_p(), owner(), "SRE"))
        .unwrap();
    ledger
        .append_admit(admit("alice@acme", "oncall", [CatalogAction::Use]))
        .unwrap();
    ledger
        .append_admit(admit(
            "bob@acme",
            "lead",
            [CatalogAction::Use, CatalogAction::Delegate],
        ))
        .unwrap();
    ledger
        .append_admit(admit("carol@acme", "oncall", [CatalogAction::Use]))
        .unwrap();
    // Remove bob (revoke-by-new-fact) — owner is authorized.
    ledger
        .append_remove(Removal::new(team_p(), PartyId::new("bob@acme"), owner()))
        .unwrap();
}

// --- write → drop → reopen → the FOLD is intact ----------------------------

#[test]
fn membership_and_fold_survive_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteMembershipLedger::open(&path).unwrap();
        seed_team(&ledger);
    }
    // Reopen: the FOLD (effective members, owner, roles) — not just facts — is intact.
    let ledger = SqliteMembershipLedger::open(&path).unwrap();
    assert_eq!(ledger.owner_of_team(&team_p()), Some(owner()));
    assert!(ledger.is_member(&PartyId::new("alice@acme"), &team_p()));
    assert!(ledger.is_member(&PartyId::new("carol@acme"), &team_p()));
    assert!(
        !ledger.is_member(&PartyId::new("bob@acme"), &team_p()),
        "the removal survived the restart (revoke-by-new-fact)"
    );
    let members = ledger.effective_members(&team_p());
    let names: Vec<String> = members.iter().map(|m| m.member().to_string()).collect();
    assert_eq!(
        names,
        vec!["alice@acme".to_string(), "carol@acme".to_string()]
    );
}

#[test]
fn reseed_is_idempotent_durable() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteMembershipLedger::open(&path).unwrap();
        seed_team(&ledger);
    }
    let ledger = SqliteMembershipLedger::open(&path).unwrap();
    let before = ledger.len();
    // Re-append the byte-identical founding + admit after a restart → AlreadyPresent,
    // no growth (exactly the bootstrap-demo-team re-seed on every `kx serve` start).
    assert!(matches!(
        ledger.append_founding(Team::found(team_p(), owner(), "SRE")),
        Ok(MembershipOutcome::AlreadyPresent(_))
    ));
    assert!(matches!(
        ledger.append_admit(admit("alice@acme", "oncall", [CatalogAction::Use])),
        Ok(MembershipOutcome::AlreadyPresent(_))
    ));
    assert_eq!(
        ledger.len(),
        before,
        "idempotent re-seed never double-appends"
    );
}

#[test]
fn removal_then_readmit_survives_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteMembershipLedger::open(&path).unwrap();
        ledger
            .append_founding(Team::found(team_p(), owner(), "SRE"))
            .unwrap();
        ledger
            .append_admit(admit("dana@acme", "oncall", [CatalogAction::Use]))
            .unwrap();
        ledger
            .append_remove(Removal::new(team_p(), PartyId::new("dana@acme"), owner()))
            .unwrap();
        // A FRESH re-admit appended AFTER the removal restores access. This is
        // position-ordered, so it only survives a restart if `seq` preserves order.
        ledger
            .append_admit(admit("dana@acme", "oncall2", [CatalogAction::Use]))
            .unwrap();
    }
    let ledger = SqliteMembershipLedger::open(&path).unwrap();
    assert!(
        ledger.is_member(&PartyId::new("dana@acme"), &team_p()),
        "re-admit-after-removal restores access across a restart (seq order preserved)"
    );
}

#[test]
fn disband_is_terminal_after_reopen() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteMembershipLedger::open(&path).unwrap();
        seed_team(&ledger);
        ledger
            .append_disband(Disband::new(team_p(), owner()))
            .unwrap();
    }
    let ledger = SqliteMembershipLedger::open(&path).unwrap();
    assert!(
        ledger.effective_members(&team_p()).is_empty(),
        "an owner disband is terminal across a restart"
    );
    assert!(!ledger.is_member(&PartyId::new("alice@acme"), &team_p()));
}

// --- run_with_each_backend: the Sqlite impl is held to the SAME contract ----

#[test]
fn membership_obligations_hold_on_both_backends() {
    fn obligations(ledger: &dyn MembershipLedger) {
        seed_team(ledger);
        // Idempotency: re-founding by the SAME owner is AlreadyPresent.
        assert!(matches!(
            ledger.append_founding(Team::found(team_p(), owner(), "SRE-renamed")),
            Ok(MembershipOutcome::AlreadyPresent(_))
        ));
        // Owner conflict: a different owner is refused.
        assert!(matches!(
            ledger.append_founding(Team::found(team_p(), PartyId::new("evil@x"), "x")),
            Err(MembershipLedgerError::OwnerConflict(_))
        ));
        // The fold: alice/carol are members, bob (removed) is not.
        assert!(ledger.is_member(&PartyId::new("alice@acme"), &team_p()));
        assert!(!ledger.is_member(&PartyId::new("bob@acme"), &team_p()));
        // member_role: alice's merged cap conveys Use, not Delegate.
        let r = ledger
            .member_role(&PartyId::new("alice@acme"), &team_p())
            .expect("alice is a member");
        assert!(r.action_cap().contains(CatalogAction::Use));
        assert!(!r.action_cap().contains(CatalogAction::Delegate));
        // teams_of: alice belongs to exactly the one team.
        assert_eq!(ledger.teams_of(&PartyId::new("alice@acme")), vec![team_p()]);
    }
    obligations(&InMemoryMembershipLedger::new());
    obligations(&SqliteMembershipLedger::open_in_memory().unwrap());
}

#[test]
fn empty_ledger_is_inert_on_both_backends() {
    fn obligations(ledger: &dyn MembershipLedger) {
        assert!(ledger.is_empty());
        assert!(ledger.owner_of_team(&team_p()).is_none());
        assert!(ledger.effective_members(&team_p()).is_empty());
        assert!(ledger
            .member_role(&PartyId::new("nobody"), &team_p())
            .is_none());
    }
    obligations(&InMemoryMembershipLedger::new());
    obligations(&SqliteMembershipLedger::open_in_memory().unwrap());
}

// --- atomicity-under-panic + schema-version-mismatch -----------------------

#[test]
fn forged_mid_txn_insert_rolls_back() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteMembershipLedger::open(&path).unwrap();
        seed_team(&ledger);
    }
    let before = SqliteMembershipLedger::open(&path).unwrap().len();
    let result = std::panic::catch_unwind(|| {
        let mut conn = rusqlite::Connection::open(&path).unwrap();
        let txn = conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .unwrap();
        txn.execute(
            "INSERT INTO facts (seq, fact_id, kind, fact_bytes) VALUES (999, ?1, 1, ?2)",
            rusqlite::params![&[0u8; 32][..], &[0u8; 8][..]],
        )
        .unwrap();
        panic!("simulated mid-txn crash"); // Drop rolls the txn back.
    });
    assert!(result.is_err(), "the panic propagates");
    let ledger = SqliteMembershipLedger::open(&path).unwrap();
    assert_eq!(
        ledger.len(),
        before,
        "rolled-back forged insert must not persist"
    );
    // A subsequent normal append still works.
    assert!(ledger
        .append_admit(admit("eve@acme", "oncall", [CatalogAction::Use]))
        .is_ok());
}

#[test]
fn schema_version_mismatch_is_refused_loudly() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let _ = SqliteMembershipLedger::open(&path).unwrap();
    }
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        let bogus: [u8; 2] = 999u16.to_le_bytes();
        conn.execute(
            "UPDATE metadata SET value = ?1 WHERE key = 'schema_version'",
            rusqlite::params![&bogus[..]],
        )
        .unwrap();
    }
    assert!(
        matches!(
            SqliteMembershipLedger::open(&path),
            Err(MembershipLedgerError::Storage(_))
        ),
        "a schema-version mismatch on reopen must refuse loudly"
    );
}

#[test]
fn corrupt_fact_row_is_refused_loudly() {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_owned();
    {
        let ledger = SqliteMembershipLedger::open(&path).unwrap();
        seed_team(&ledger);
    }
    // Forge a structurally-undecodable fact_bytes row (a garbage BLOB).
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute(
            "INSERT INTO facts (seq, fact_id, kind, fact_bytes) VALUES (10000, ?1, 1, ?2)",
            rusqlite::params![&[7u8; 32][..], &[0xFFu8; 4][..]],
        )
        .unwrap();
    }
    assert!(
        matches!(
            SqliteMembershipLedger::open(&path),
            Err(MembershipLedgerError::Storage(_))
        ),
        "an undecodable fact row must refuse loudly on rebuild, never silently mis-read"
    );
}
