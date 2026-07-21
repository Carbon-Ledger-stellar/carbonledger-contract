//! Shrinking a failing sequence to a minimal reproducer.
//!
//! When a sequence trips an assertion the raw op list is usually far larger than
//! the handful of operations that actually matter. `shrink` runs delta debugging
//! (the classic ddmin) over the op vector: it repeatedly drops chunks and keeps
//! any reduction that *still fails*, converging on a locally minimal sequence
//! that reproduces the same class of failure. Because every id is namespaced by
//! sequence index, a single sequence's ops replay faithfully on their own, so a
//! failure found inside a shared, reused world reproduces in a fresh one.

use std::panic::{self, AssertUnwindSafe};

use crate::exec::World;
use crate::invariants;
use crate::ops::Op;

/// Replays an explicit op list in a fresh world, checking fast invariants after
/// every step — the same per-op checking the campaign does. Panics on the first
/// divergence, exactly as the campaign would.
pub fn run_ops(ops: &[Op]) {
    let mut world = World::new();
    for op in ops {
        world.step(op);
        invariants::check_fast(&world.model);
    }
}

/// True if replaying `ops` in isolation reproduces a failure (any panic).
fn fails(ops: &[Op]) -> bool {
    panic::catch_unwind(AssertUnwindSafe(|| run_ops(ops))).is_err()
}

/// Delta-debugging: returns a locally minimal subsequence of `ops` that still
/// fails. If the failure does not reproduce in isolation (e.g. it depended on
/// deep per-world reconciliation rather than a per-op check), the input is
/// returned unchanged.
pub fn shrink(ops: &[Op]) -> Vec<Op> {
    // Silence panic output while we deliberately trigger many panics; restore
    // the previous hook afterwards so a genuine later failure still prints.
    let prev = panic::take_hook();
    panic::set_hook(Box::new(|_| {}));
    let result = shrink_inner(ops);
    panic::set_hook(prev);
    result
}

fn shrink_inner(ops: &[Op]) -> Vec<Op> {
    if !fails(ops) {
        return ops.to_vec();
    }
    let mut current = ops.to_vec();
    let mut granularity = 2;
    while current.len() >= 2 {
        let chunk = current.len().div_ceil(granularity);
        let mut reduced = false;
        // Try dropping each chunk (the "complement" is everything else).
        let mut start = 0;
        while start < current.len() {
            let end = (start + chunk).min(current.len());
            let mut candidate = Vec::with_capacity(current.len() - (end - start));
            candidate.extend_from_slice(&current[..start]);
            candidate.extend_from_slice(&current[end..]);
            if !candidate.is_empty() && fails(&candidate) {
                current = candidate;
                reduced = true;
                // Restart at coarse granularity on the smaller input.
                granularity = 2.max(granularity - 1);
                break;
            }
            start += chunk;
        }
        if !reduced {
            if granularity >= current.len() {
                break;
            }
            granularity = (granularity * 2).min(current.len());
        }
    }
    current
}
