//! Operation generator.
//!
//! Generates sequences biased toward *interesting* states rather than uniformly
//! random ones. Purely random arguments almost never hit a valid path: a random
//! u64 serial range essentially never overlaps, and a random batch id
//! essentially never collides. So the generator draws ids from a small pool and
//! deliberately reuses serial ranges, which is what makes duplicate-id and
//! overlap rejection actually get exercised.

use crate::rng::Rng;

/// Pool sizes. Small on purpose — collisions are the point.
pub const N_BATCHES: usize = 6;
pub const N_PROJECTS: usize = 3;
pub const N_ACTORS: usize = 3;

/// Retirement amounts are bounded because `retire_credits` builds a `Vec<u64>`
/// with one entry per credit. A large amount is not just slow, it is an
/// out-of-budget panic — worth probing deliberately, but not in the main loop.
pub const MAX_AMOUNT: i128 = 200;

#[derive(Clone, Debug)]
pub enum Op {
    Mint {
        actor: usize,
        batch: usize,
        project: usize,
        amount: i128,
        vintage_year: u32,
        serial_start: u64,
        serial_end: u64,
    },
    Retire {
        actor: usize,
        batch: usize,
        amount: i128,
        retirement_seq: u32,
    },
    Transfer {
        from: usize,
        to: usize,
        batch: usize,
        amount: i128,
    },
}

pub struct Generator {
    rng: Rng,
    /// Monotonic counter so every retirement gets a distinct id. Reusing a
    /// retirement id would silently overwrite the certificate.
    retirement_seq: u32,
}

impl Generator {
    pub fn new(seed: u64) -> Self {
        Generator {
            rng: Rng::new(seed),
            retirement_seq: 0,
        }
    }

    pub fn sequence(&mut self, len: usize) -> Vec<Op> {
        (0..len).map(|_| self.next_op()).collect()
    }

    fn next_op(&mut self) -> Op {
        // Mint-heavy: without batches in existence the other two ops are all
        // trivial ProjectNotFound rejections and the campaign learns nothing.
        match self.rng.weighted(&[40, 40, 20]) {
            0 => self.gen_mint(),
            1 => self.gen_retire(),
            _ => self.gen_transfer(),
        }
    }

    fn gen_mint(&mut self) -> Op {
        let batch = self.rng.below(N_BATCHES);

        // Serial ranges are drawn from a deliberately cramped space so that
        // overlaps between distinct batches are common rather than astronomical.
        let serial_start = self.rng.range_u64(1, 500);
        let width = if self.rng.chance(10) {
            // Occasionally emit an inverted range to exercise InvalidSerialRange.
            0
        } else {
            self.rng.range_u64(1, 100)
        };
        let serial_end = if width == 0 {
            serial_start.saturating_sub(self.rng.range_u64(1, 10))
        } else {
            serial_start + width - 1
        };

        let amount = if self.rng.chance(8) {
            // Zero/negative amounts exercise ZeroAmountNotAllowed.
            self.rng.range_i128(-5, 0)
        } else {
            self.rng.range_i128(1, MAX_AMOUNT)
        };

        let vintage_year = if self.rng.chance(8) {
            // Out-of-band vintages exercise InvalidVintageYear on both sides.
            if self.rng.chance(50) {
                self.rng.range_u32(1900, 1999)
            } else {
                self.rng.range_u32(2101, 2200)
            }
        } else {
            self.rng.range_u32(2000, 2100)
        };

        Op::Mint {
            // Non-admin actors exercise the admin gate.
            actor: if self.rng.chance(85) { 0 } else { self.rng.below(N_ACTORS) },
            batch,
            project: self.rng.below(N_PROJECTS),
            amount,
            vintage_year,
            serial_start,
            serial_end,
        }
    }

    fn gen_retire(&mut self) -> Op {
        self.retirement_seq += 1;
        Op::Retire {
            actor: self.rng.below(N_ACTORS),
            batch: self.rng.below(N_BATCHES),
            amount: if self.rng.chance(8) {
                self.rng.range_i128(-5, 0)
            } else {
                // Overshooting the batch amount exercises InsufficientCredits.
                self.rng.range_i128(1, MAX_AMOUNT)
            },
            retirement_seq: self.retirement_seq,
        }
    }

    fn gen_transfer(&mut self) -> Op {
        Op::Transfer {
            from: self.rng.below(N_ACTORS),
            to: self.rng.below(N_ACTORS),
            batch: self.rng.below(N_BATCHES),
            amount: if self.rng.chance(8) {
                self.rng.range_i128(-5, 0)
            } else {
                self.rng.range_i128(1, MAX_AMOUNT)
            },
        }
    }
}

pub fn batch_id(i: usize) -> String {
    format!("batch-{i}")
}

pub fn project_id(i: usize) -> String {
    format!("proj-{i}")
}

pub fn retirement_id(seq: u32) -> String {
    format!("ret-{seq}")
}
