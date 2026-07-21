//! Entry point for the stateful fuzzing harness.
//!
//! Cargo only auto-discovers integration tests at `tests/*.rs` and
//! `tests/*/main.rs`. Everything in this directory hangs off this file; a
//! module that is not declared here is silently never compiled.
//!
//! See `FUZZING.md` at the repository root for how to run a campaign and how
//! to reproduce a reported failure.

mod exec;
mod invariants;
mod model;
mod ops;
mod rng;

use exec::World;
use ops::Generator;

/// Operations per seed. Long enough for batches to reach FullyRetired and for
/// serial ranges to start colliding.
const SEQUENCE_LEN: usize = 60;

/// Runs one seed to completion, checking invariants after every step.
fn run_seed(seed: u64) {
    let mut world = World::new();
    let ops = Generator::new(seed).sequence(SEQUENCE_LEN);

    for op in &ops {
        world.step(op);
        // Checking after *every* step, not just at the end, is what makes a
        // failure cheap to diagnose: the panic lands on the culprit operation
        // rather than somewhere downstream of it.
        invariants::check_fast(&world);
    }
    invariants::check_deep(&world);
}

/// The default campaign. Kept small enough to run in CI on every push; use
/// `FUZZ_SEEDS` to widen it locally.
#[test]
fn campaign() {
    let seeds: u64 = std::env::var("FUZZ_SEEDS")
        .ok()
        .and_then(|v| v.parse().ok())
        // 24 seeds runs in roughly half a minute, which is affordable on every
        // push. Nightly and pre-release runs should widen it substantially.
        .unwrap_or(24);

    for seed in 0..seeds {
        run_seed(seed);
    }
}

/// Documents a real gap the harness surfaced, rather than asserting it away.
///
/// `mint_credits` never reconciles `amount` against the width of the declared
/// serial range, so a batch may be minted with `amount = 100` over serials
/// `1..=5`. Retirement then allocates serials sequentially by *count*, walking
/// straight past `serial_end` into numbers that were never registered in the
/// global serial registry — and which may already belong to another batch.
///
/// The global registry's overlap check protects range *declarations*, not the
/// serials retirement actually hands out, so double-counting is reachable
/// despite that defence.
///
/// This test asserts the gap is still reachable. When the contract is fixed to
/// require `amount == serial_end - serial_start + 1` (or to bound retirement at
/// `serial_end`), this test will fail and should be inverted into a regression
/// test proving the gap is closed.
#[test]
fn known_gap_amount_not_reconciled_with_serial_range() {
    let mut world = World::new();

    // amount deliberately exceeds the 5-wide declared range.
    world.mint_raw("gap-batch", "gap-proj", 100, 2020, 1, 5);
    world.retire_raw("gap-batch", "gap-ret", 100);

    let escaped = world.model.serials_escape_declared_range();
    assert!(
        escaped.contains(&"gap-batch".to_string()),
        "expected retirement serials to escape the declared range; \
         if this now fails the contract may have been fixed — see the doc comment"
    );

    let cert = world.certificate("gap-ret");
    assert_eq!(
        cert.serial_numbers.last().unwrap(),
        100,
        "retirement allocated serials up to 100 despite serial_end = 5"
    );
}

/// Replays a single seed. Point this at the seed printed by a campaign failure
/// via `FUZZ_SEED=<n> cargo test -p carbon_fuzz replay -- --nocapture`.
#[test]
fn replay() {
    let Some(seed) = std::env::var("FUZZ_SEED").ok().and_then(|v| v.parse().ok()) else {
        return;
    };
    run_seed(seed);
}
