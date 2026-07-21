//! Invariants.
//!
//! `check_fast` runs after **every** operation and reads only the shadow model,
//! which `exec` has already reconciled against the contracts on this step. It
//! costs no contract invocations, so it is affordable in the high-volume
//! campaign and still pins a failure to the exact culprit operation. Properties
//! that must re-read contract state live in `exec::World::deep_reconcile`
//! instead, and run once per world.
//!
//! `report_gaps` is different in kind: it surfaces the cross-contract
//! consistency *gaps* the system genuinely has — a listing for more credits
//! than were ever minted, a mint against an unverified project — as reported
//! findings rather than assertions, because the contracts make no cross-contract
//! calls and so do not uphold these properties today. See `FUZZING.md`.

use crate::model::{LedgerModel, ListingStatus};

/// Model-only invariants, checked after every operation.
pub fn check_fast(model: &LedgerModel) {
    credit_no_overlapping_ranges(model);
    credit_retired_within_minted(model);
    credit_retired_serials_partition(model);
    credit_certs_sum_to_retired(model);
    market_amounts_are_sane(model);
    usdc_is_conserved(model);
}

// ── Credit ────────────────────────────────────────────────────────────────────

/// No two registered serial ranges overlap — the double-counting defence, and
/// the single most important property in the system.
fn credit_no_overlapping_ranges(model: &LedgerModel) {
    let ranges = &model.credit.ranges;
    for (i, &(a_lo, a_hi)) in ranges.iter().enumerate() {
        for &(b_lo, b_hi) in &ranges[i + 1..] {
            assert!(
                !(a_lo <= b_hi && b_lo <= a_hi),
                "serial ranges [{a_lo},{a_hi}] and [{b_lo},{b_hi}] overlap — double counting"
            );
        }
    }
}

/// Retired credits never exceed the minted supply, and active balance is never
/// negative.
fn credit_retired_within_minted(model: &LedgerModel) {
    for (bid, batch) in &model.credit.batches {
        assert!(
            batch.retired <= batch.amount,
            "batch {bid}: retired {} exceeds minted {}",
            batch.retired,
            batch.amount
        );
        assert!(batch.active() >= 0, "batch {bid}: negative active balance");
    }
}

/// Within a batch, retirement certificates carve serials into disjoint,
/// contiguous spans covering exactly `[serial_start, serial_start+retired-1]`.
/// A serial retired twice is a credit sold twice.
fn credit_retired_serials_partition(model: &LedgerModel) {
    for (bid, batch) in &model.credit.batches {
        let mut spans: Vec<(u64, u64)> = model
            .credit
            .certs
            .values()
            .filter(|c| &c.batch_id == bid)
            .map(|c| (c.serial_lo, c.serial_hi))
            .collect();
        spans.sort_unstable();

        for pair in spans.windows(2) {
            assert!(
                pair[0].1 < pair[1].0,
                "batch {bid}: retirement serial spans overlap at {}/{}",
                pair[0].1,
                pair[1].0
            );
        }
        if let (Some(&(first_lo, _)), Some(&(_, last_hi))) = (spans.first(), spans.last()) {
            assert_eq!(first_lo, batch.serial_start, "batch {bid}: retired serials do not start at serial_start");
            assert_eq!(
                last_hi,
                batch.serial_start + batch.retired as u64 - 1,
                "batch {bid}: retired serial span does not cover the retired amount"
            );
        }
    }
}

/// Certificate amounts account for exactly the retired balance — no more (a
/// certificate for credits never retired), no less (credits retired without a
/// certificate).
fn credit_certs_sum_to_retired(model: &LedgerModel) {
    for (bid, batch) in &model.credit.batches {
        let certified: i128 = model
            .credit
            .certs
            .values()
            .filter(|c| &c.batch_id == bid)
            .map(|c| c.amount)
            .sum();
        assert_eq!(
            certified, batch.retired,
            "batch {bid}: certificates account for {certified} but {} retired",
            batch.retired
        );
    }
}

// ── Marketplace ───────────────────────────────────────────────────────────────

/// A listing's available amount stays within `[0, original]`, and status stays
/// consistent with it: fully drained iff `Sold`.
fn market_amounts_are_sane(model: &LedgerModel) {
    for (lid, l) in &model.market.listings {
        assert!(
            l.amount_available >= 0 && l.amount_available <= l.original_amount,
            "listing {lid}: available {} out of [0,{}]",
            l.amount_available,
            l.original_amount
        );
        if l.status == ListingStatus::Sold {
            assert_eq!(l.amount_available, 0, "listing {lid}: Sold but has liquidity");
        }
        if l.amount_available == 0
            && l.status != ListingStatus::Delisted
            && l.status != ListingStatus::Active
        {
            assert_eq!(l.status, ListingStatus::Sold, "listing {lid}: drained but not Sold");
        }
    }
}

// ── USDC ──────────────────────────────────────────────────────────────────────

/// Settlement only moves USDC between accounts, so the total is invariant and no
/// balance goes negative.
fn usdc_is_conserved(model: &LedgerModel) {
    let total: i128 = model.usdc.values().sum();
    assert_eq!(
        total, model.usdc_initial_total,
        "USDC total changed: settlement is not conservative"
    );
    for (&actor, &bal) in &model.usdc {
        assert!(bal >= 0, "actor {actor}: negative USDC balance {bal}");
    }
}

// ── Cross-contract gap reporting ──────────────────────────────────────────────

/// A cross-contract consistency gap the system exhibits today. These are real
/// defects reachable because the contracts do not call one another, reported
/// rather than asserted.
#[derive(Debug, Default)]
pub struct Gaps {
    /// Batches whose retirement serials escape the declared serial range.
    pub serials_escape_range: Vec<String>,
    /// Listings offering more credits than the referenced batch was ever minted
    /// (or for a batch that was never minted at all).
    pub listings_exceed_supply: usize,
    /// Batches minted against a project that is not `Verified` in the registry.
    pub mint_without_verified_project: usize,
}

impl Gaps {
    pub fn any(&self) -> bool {
        !self.serials_escape_range.is_empty()
            || self.listings_exceed_supply > 0
            || self.mint_without_verified_project > 0
    }
}

/// Inspects the model for the known cross-contract gaps. Used by a documented
/// test to demonstrate they are reachable, and available to a campaign for
/// diagnostics.
pub fn report_gaps(model: &LedgerModel) -> Gaps {
    let mut gaps = Gaps::default();
    gaps.serials_escape_range = model.credit.serials_escape_declared_range();

    gaps.mint_without_verified_project = model
        .credit
        .batches
        .values()
        .filter(|b| {
            model.registry.projects.get(&b.project_id)
                != Some(&crate::model::ProjectStatus::Verified)
        })
        .count();

    // A listing offers more than its referenced batch was minted (or names a
    // batch that was never minted) — the marketplace never checks either.
    gaps.listings_exceed_supply = model
        .market
        .listings
        .values()
        .filter(|l| match model.credit.batches.get(&l.batch_id) {
            None => true,
            Some(b) => l.original_amount > b.amount,
        })
        .count();

    gaps
}
