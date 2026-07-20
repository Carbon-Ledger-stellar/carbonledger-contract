//! Global invariants, checked against contract state after every operation.
//!
//! These are deliberately read back from the *contract*, not the model. The
//! executor already proves model/contract agreement step by step; this module
//! asserts properties that should hold of the contract on its own terms, so a
//! shared misconception between model and contract still gets caught.

use soroban_sdk::String as SString;

use crate::exec::World;
use crate::model::Status;
use crate::ops::{batch_id, N_BATCHES};

/// Invariants cheap enough to check after every single operation. These read
/// only the model, which the executor has already reconciled against the
/// contract on this step, so they still pin down the exact failing operation.
pub fn check_fast(world: &World) {
    no_overlapping_serial_ranges(world);
    retired_never_exceeds_minted(world);
    retired_serials_are_unique(world);
    cert_amounts_sum_to_retired(world);
}

/// Invariants that re-read contract state. Each getter is a full contract
/// invocation, so running these per-step costs more than the operations
/// themselves; they run once per sequence instead.
pub fn check_deep(world: &World) {
    serial_registry_agrees_with_contract(world);
    retirement_is_terminal(world);
    status_matches_retired_amount(world);
    project_index_is_complete(world);
}

/// No two registered serial ranges may overlap — this is the double-counting
/// defence, and the single most important property in the system.
fn no_overlapping_serial_ranges(world: &World) {
    let ranges = &world.model.ranges;
    for (i, &(a_lo, a_hi)) in ranges.iter().enumerate() {
        for &(b_lo, b_hi) in &ranges[i + 1..] {
            assert!(
                !(a_lo <= b_hi && b_lo <= a_hi),
                "serial ranges [{a_lo},{a_hi}] and [{b_lo},{b_hi}] overlap — double counting"
            );
        }
    }
}

/// Cross-check the model's range list against the contract's own view: a range
/// that is already registered must be reported as unavailable.
fn serial_registry_agrees_with_contract(world: &World) {
    for &(lo, hi) in &world.model.ranges {
        assert!(
            !world.client.verify_serial_range(&lo, &hi),
            "contract claims registered range [{lo},{hi}] is still free"
        );
    }
}

/// Retired credits can never exceed the minted supply of a batch.
fn retired_never_exceeds_minted(world: &World) {
    for (bid, batch) in &world.model.batches {
        assert!(
            batch.retired <= batch.amount,
            "batch {bid}: retired {} exceeds minted {}",
            batch.retired,
            batch.amount
        );
        assert!(
            batch.active() >= 0,
            "batch {bid}: negative active balance {}",
            batch.active()
        );
    }
}

/// Retirement is irreversible: once a batch is fully retired it stays that way,
/// and both retire and transfer must reject it.
fn retirement_is_terminal(world: &World) {
    for bid in &world.model.ever_fully_retired {
        let batch = world
            .model
            .batches
            .get(bid)
            .expect("a fully-retired batch vanished from the model");
        assert_eq!(
            batch.status(),
            Status::FullyRetired,
            "batch {bid} left the FullyRetired state — retirement is not irreversible"
        );

        let on_chain = world
            .client
            .get_credit_batch(&SString::from_str(&world.env, bid));
        assert_eq!(
            on_chain.status,
            carbon_credit::CreditStatus::FullyRetired,
            "batch {bid} is FullyRetired in the model but not on chain"
        );
    }
}

/// Status is a pure function of the retired amount. A batch must never be
/// `Active` with credits already retired.
fn status_matches_retired_amount(world: &World) {
    for i in 0..N_BATCHES {
        let bid = batch_id(i);
        let Some(batch) = world.model.batches.get(&bid) else {
            continue;
        };
        let on_chain = world
            .client
            .get_credit_batch(&SString::from_str(&world.env, &bid));

        let expected = match batch.status() {
            Status::Active => carbon_credit::CreditStatus::Active,
            Status::PartiallyRetired => carbon_credit::CreditStatus::PartiallyRetired,
            Status::FullyRetired => carbon_credit::CreditStatus::FullyRetired,
        };
        assert_eq!(
            on_chain.status, expected,
            "batch {bid}: status is not a function of retired={}/{}",
            batch.retired, batch.amount
        );
    }
}

/// Certificates are the audit trail for retirement. Their amounts must account
/// for exactly the retired balance — no more, no less. A shortfall means
/// credits were destroyed without a certificate; an excess means a certificate
/// was issued for credits that were never retired.
fn cert_amounts_sum_to_retired(world: &World) {
    for (bid, batch) in &world.model.batches {
        let certified: i128 = world
            .model
            .certs
            .values()
            .filter(|c| &c.batch_id == bid)
            .map(|c| c.amount)
            .sum();
        assert_eq!(
            certified, batch.retired,
            "batch {bid}: certificates account for {certified} but {} credits are retired",
            batch.retired
        );
    }
}

/// Every minted batch must be reachable through its project index. A batch that
/// exists but is not indexed is invisible to `get_project_credits`, which is how
/// any downstream consumer enumerates a project's supply.
fn project_index_is_complete(world: &World) {
    use std::collections::HashMap;

    let mut by_project: HashMap<&str, Vec<&String>> = HashMap::new();
    for (bid, batch) in &world.model.batches {
        by_project
            .entry(batch.project_id.as_str())
            .or_default()
            .push(bid);
    }

    for (pid, expected) in by_project {
        let indexed = world
            .client
            .get_project_credits(&SString::from_str(&world.env, pid));
        assert_eq!(
            indexed.len() as usize,
            expected.len(),
            "project {pid}: index holds {} batches, model has {}",
            indexed.len(),
            expected.len()
        );
    }
}

/// Within a batch, retirement certificates must carve up serials into disjoint,
/// contiguous spans. A serial retired twice is a credit sold twice.
fn retired_serials_are_unique(world: &World) {
    for (bid, batch) in &world.model.batches {
        let mut spans: Vec<(u64, u64)> = world
            .model
            .certs
            .values()
            .filter(|c| &c.batch_id == bid)
            .map(|c| (c.serial_lo, c.serial_hi))
            .collect();
        spans.sort_unstable();

        for pair in spans.windows(2) {
            let (_, prev_hi) = pair[0];
            let (next_lo, _) = pair[1];
            assert!(
                prev_hi < next_lo,
                "batch {bid}: retirement serial spans overlap at {prev_hi}/{next_lo}"
            );
        }

        // The union must be exactly [serial_start, serial_start + retired - 1].
        if let (Some(&(first_lo, _)), Some(&(_, last_hi))) = (spans.first(), spans.last()) {
            assert_eq!(
                first_lo, batch.serial_start,
                "batch {bid}: retired serials do not start at serial_start"
            );
            assert_eq!(
                last_hi,
                batch.serial_start + batch.retired as u64 - 1,
                "batch {bid}: retired serial span does not cover the retired amount"
            );
        }
    }
}
