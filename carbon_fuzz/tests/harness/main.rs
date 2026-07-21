//! Entry point for the stateful, cross-contract fuzzing harness.
//!
//! Cargo only auto-discovers integration tests at `tests/*.rs` and
//! `tests/*/main.rs`. Everything in this directory hangs off this file; a module
//! not declared here is silently never compiled.
//!
//! The harness drives all four CarbonLedger contracts — registry, credit,
//! marketplace, oracle — through randomly generated lifecycle sequences
//! (register → verify → mint → list → purchase → retire), checking after every
//! operation that each contract agrees with an independent shadow model and that
//! a set of safety invariants still hold. On failure it shrinks the sequence to
//! a minimal reproducer. See `FUZZING.md` for how to run and reproduce.

mod exec;
mod invariants;
mod model;
mod ops;
mod rng;
mod shrink;

use std::panic::{self, AssertUnwindSafe};

use exec::{World, WORLD_BATCH};
use ops::Generator;

/// Operations per sequence. Long enough to reach FullyRetired batches, drained
/// listings, and serial-range collisions; short enough that a 50k-sequence
/// campaign stays inside the CI time budget.
const SEQUENCE_LEN: usize = 12;

/// Default sequence count for `cargo test` on every push. The 50k+ acceptance
/// target is reached by the release soak — see `FUZZING.md` — which overrides
/// this via `FUZZ_SEQS`.
const DEFAULT_SEQS: u64 = 300;

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

/// Runs a whole campaign as a pure function of `seed`: same seed, same stream of
/// sequences, same operations. `deep` toggles the per-world contract-state
/// reconciliation, which is thorough but costs invocations — left off for the
/// high-volume soak.
fn run_campaign(seed: u64, seqs: u64, deep: bool) {
    let mut generator = Generator::new(seed);
    let mut world = World::new();

    for s in 0..seqs as u32 {
        // Rebuild the world every WORLD_BATCH sequences to bound per-Env
        // invocation growth and the contracts' global-list serialization cost.
        if s > 0 && s % WORLD_BATCH == 0 {
            if deep {
                world.deep_reconcile();
            }
            world = World::new();
        }

        let sequence = generator.sequence(s, SEQUENCE_LEN);
        let outcome = panic::catch_unwind(AssertUnwindSafe(|| {
            for op in &sequence {
                world.step(op);
                // Checking after every step — not just at the end — is what makes
                // a failure cheap to diagnose: the panic lands on the culprit op.
                invariants::check_fast(&world.model);
            }
        }));

        if let Err(payload) = outcome {
            report_failure(seed, s, &sequence, &payload);
        }
    }

    if deep {
        world.deep_reconcile();
    }
}

/// Prints the failing seed/sequence, shrinks the sequence to a minimal
/// reproducer, prints it, and then fails the test.
fn report_failure(seed: u64, s: u32, sequence: &[ops::Op], payload: &Box<dyn std::any::Any + Send>) {
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "<non-string panic>".to_string());

    eprintln!("\n──────────────── fuzz failure ────────────────");
    eprintln!("seed={seed}  sequence={s}");
    eprintln!("panic: {msg}");
    eprintln!("raw sequence: {} ops", sequence.len());

    let minimal = shrink::shrink(sequence);
    eprintln!("minimal reproducer: {} ops", minimal.len());
    for (i, op) in minimal.iter().enumerate() {
        eprintln!("  [{i:>2}] {op:?}");
    }
    eprintln!(
        "\nreplay this campaign with:\n  FUZZ_SEED={seed} cargo test --release -p carbon_fuzz --test harness replay -- --nocapture"
    );
    eprintln!("───────────────────────────────────────────────\n");

    panic!("fuzz campaign failed at seed={seed} sequence={s}: {msg}");
}

/// The default campaign. Kept small enough to run in CI on every push; the
/// release soak widens it via `FUZZ_SEQS` (see `FUZZING.md`). `FUZZ_DEEP=0`
/// disables per-world contract reconciliation for maximum throughput.
#[test]
fn campaign() {
    let seqs = env_u64("FUZZ_SEQS", DEFAULT_SEQS);
    let seed = env_u64("FUZZ_SEED", 0);
    let deep = std::env::var("FUZZ_DEEP").map(|v| v != "0").unwrap_or(true);
    run_campaign(seed, seqs, deep);
}

/// Replays a campaign deterministically. Point it at the seed printed by a
/// failure via `FUZZ_SEED=<n> cargo test -p carbon_fuzz --test harness replay -- --nocapture`.
#[test]
fn replay() {
    let Some(seed) = std::env::var("FUZZ_SEED").ok().and_then(|v| v.parse().ok()) else {
        return;
    };
    let seqs = env_u64("FUZZ_SEQS", DEFAULT_SEQS);
    run_campaign(seed, seqs, true);
}

// ── Documented gaps ───────────────────────────────────────────────────────────

/// Regression test for a gap this harness previously *documented* as open: that
/// `mint_credits` did not reconcile `amount` against the declared serial range,
/// letting a batch be minted with `amount = 100` over serials `1..=5` and
/// retirement then walk serials past `serial_end` into numbers belonging to
/// other batches. Upstream closed it by requiring `serial_end - serial_start + 1
/// == amount` at mint; this test pins that shut.
#[test]
fn regression_mint_requires_serial_range_matches_amount() {
    use ops::{ADMIN, FIRST_TRADER};
    use soroban_sdk::String as SString;
    let mut w = World::new();
    let admin = w.actors[ADMIN].clone();
    let env = w.env.clone();
    let s = |v: &str| SString::from_str(&env, v);

    // amount=100 over a 5-wide range is now rejected outright.
    let rejected = w.credit.try_mint_credits(
        &admin,
        &s("p0"),
        &100_i128,
        &2020_u32,
        &s("bx"),
        &1_u64,
        &5_u64,
        &s("cid"),
    );
    assert!(
        matches!(rejected, Err(Ok(carbon_credit::CarbonError::InvalidSerialRange))),
        "mint should reject a serial range whose width does not equal amount, got {rejected:?}"
    );

    // A matched range mints, and retirement stays strictly within the declared
    // range — so no serials can escape it.
    w.step(&ops::Op::Mint {
        seq: 0,
        actor: ADMIN,
        project: 0,
        batch: 0,
        amount: 10,
        vintage_year: 2020,
        serial_start: 1,
        serial_end: 10,
    });
    w.step(&ops::Op::Retire {
        seq: 0,
        actor: FIRST_TRADER,
        batch: 0,
        amount: 10,
        retirement: 1,
    });

    assert!(
        invariants::report_gaps(&w.model).serials_escape_range.is_empty(),
        "serial-range gap should be closed: no retirement may escape its declared range"
    );
    let cert = w.credit.get_retirement_certificate(&s("ret1"));
    assert_eq!(
        cert.serial_numbers.last().unwrap(),
        10,
        "retirement serials must stay within the declared range [1,10]"
    );
}

/// Documents that the contracts uphold no cross-contract consistency, because
/// they make no cross-contract calls to one another. A batch can be minted for a
/// project the registry never verified, and the marketplace will list credits
/// for a batch that was never minted at all. Reported by `report_gaps`, not
/// asserted away — see `FUZZING.md`.
#[test]
fn known_gap_no_cross_contract_consistency() {
    use ops::ADMIN;
    let mut w = World::new();

    // Mint a batch for a project that was never registered or verified.
    w.step(&ops::Op::Mint {
        seq: 0,
        actor: ADMIN,
        project: 0,
        batch: 0,
        amount: 10,
        vintage_year: 2020,
        serial_start: 1,
        serial_end: 10,
    });
    // List 999 credits of batch b0_1, which was never minted.
    w.step(&ops::Op::ListCredits {
        seq: 0,
        seller: ops::FIRST_TRADER,
        listing: 1,
        batch: 1,
        project: 0,
        amount: 999,
        price: 1,
        vintage_year: 2020,
        methodology: 0,
    });

    let gaps = invariants::report_gaps(&w.model);
    assert!(gaps.any(), "expected the cross-contract gaps to be reachable");
    assert!(
        gaps.mint_without_verified_project >= 1,
        "minting does not require a verified project — expected the gap to be reachable"
    );
    assert!(
        gaps.listings_exceed_supply >= 1,
        "listing does not validate against minted supply — expected the gap to be reachable"
    );
}
