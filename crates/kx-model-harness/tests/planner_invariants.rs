//! M6 invariant guards that need both crates in scope.
//!
//! - The planner carries a step's intent under its own `PLAN_PROMPT_KEY`; for a
//!   model executor to USE it as the instruction, that key MUST equal the
//!   harness's `prompt::PROMPT_KEY`. A drift guard pins the two constants equal
//!   (cheap protection against the hand-mirrored-constant hazard, IMP-7).
//! - The thesis dependency-ban (D73): `kx-planner`'s `[dependencies]` must name
//!   none of `kx-scheduler` / `kx-projection` / `kx-executor` / `kx-inference`,
//!   so the planner layer can never couple to the engine it sits above.

#![allow(clippy::unwrap_used, clippy::expect_used)]

#[test]
fn plan_prompt_key_matches_the_harness_prompt_convention() {
    assert_eq!(
        kx_planner::PLAN_PROMPT_KEY,
        kx_model_harness::prompt::PROMPT_KEY,
        "kx_planner::PLAN_PROMPT_KEY must equal kx_model_harness::prompt::PROMPT_KEY — \
         otherwise a planner step's intent would not be read as the model instruction"
    );
}

#[test]
fn kx_planner_does_not_depend_on_the_engine_crates_thesis_ban() {
    // Read kx-planner's manifest from disk and assert the [dependencies] table
    // names none of the banned engine crates (D73 / the P2 thesis test).
    let manifest = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../kx-planner/Cargo.toml"
    ))
    .expect("read kx-planner/Cargo.toml");

    // Isolate the [dependencies] section (up to the next top-level table).
    let deps = manifest
        .split_once("[dependencies]")
        .expect("kx-planner has a [dependencies] table")
        .1;
    let deps = deps.split("\n[").next().unwrap_or(deps);

    // Collect the dependency KEYS only (the token before `=`/`.`), ignoring
    // comments — so a banned name mentioned in a comment never trips the guard.
    let keys: Vec<&str> = deps
        .lines()
        .filter_map(|line| line.split('#').next()) // strip trailing comments
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| l.split(['=', '.', ' ']).next().unwrap_or(l).trim())
        .collect();

    for banned in [
        "kx-scheduler",
        "kx-projection",
        "kx-executor",
        "kx-inference",
    ] {
        assert!(
            !keys.contains(&banned),
            "kx-planner must NOT depend on {banned} (D73 / thesis test) — found it as a \
             [dependencies] key"
        );
    }
}
