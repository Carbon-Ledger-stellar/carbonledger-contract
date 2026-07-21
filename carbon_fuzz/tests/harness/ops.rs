//! Operation generator for the full cross-contract lifecycle.
//!
//! A *sequence* is a coherent lifecycle — register → verify → mint → list →
//! purchase → retire — drawn from small id pools so collisions (duplicate ids,
//! overlapping serials, over-retirement) are common rather than astronomically
//! rare. The generator is **precondition-aware**: it tracks what it has already
//! created and biases toward operations whose preconditions hold, while still
//! injecting invalid operations at a low rate so every rejection path is
//! exercised too.
//!
//! Ids are namespaced by sequence index (`p3_1`, `b3_0`, serial block `3e6…`) so
//! that many independent sequences can run against one shared set of contracts
//! without interfering — see `exec::World` for why that matters for throughput.

use crate::rng::Rng;

// ── Actor roles ───────────────────────────────────────────────────────────────
// A fixed cast. 0/1/2 hold the privileged roles; 3..N_ACTORS are ordinary
// traders funded with USDC. The generator mostly calls role-gated operations
// with the right actor, but sometimes with the wrong one to exercise the gate.
pub const N_ACTORS: usize = 6;
pub const ADMIN: usize = 0;
pub const VERIFIER: usize = 1;
pub const ORACLE: usize = 2;
pub const FIRST_TRADER: usize = 3;

/// Pool sizes per sequence. Small on purpose — collisions are the point.
pub const N_PROJECTS: usize = 3;
pub const N_BATCHES: usize = 4;

/// Retirement amounts are bounded because `retire_credits` builds a `Vec<u64>`
/// with one entry per credit, so a large amount is an out-of-budget trap rather
/// than a returned error — worth probing deliberately, but not in the main loop.
pub const MAX_AMOUNT: i128 = 200;

/// Serial numbers for sequence `seq` live in `[seq*SERIAL_BLOCK, …)`, keeping
/// distinct sequences' ranges disjoint while allowing dense overlaps within one.
pub const SERIAL_BLOCK: u64 = 1_000_000;

const METHODOLOGIES: [&str; 3] = ["VCS", "GS", "CDM"];

#[derive(Clone, Debug)]
pub enum Op {
    RegisterProject {
        seq: u32,
        actor: usize,
        project: usize,
        vintage_year: u32,
    },
    VerifyProject {
        seq: u32,
        actor: usize,
        project: usize,
    },
    RejectProject {
        seq: u32,
        actor: usize,
        project: usize,
    },
    SuspendProject {
        seq: u32,
        actor: usize,
        project: usize,
    },
    SubmitMonitoring {
        seq: u32,
        actor: usize,
        project: usize,
        period: u32,
        tonnes: i128,
        score: u32,
    },
    UpdatePrice {
        actor: usize,
        methodology: usize,
        vintage_year: u32,
        price: i128,
    },
    FlagProject {
        seq: u32,
        actor: usize,
        project: usize,
    },
    Mint {
        seq: u32,
        actor: usize,
        project: usize,
        batch: usize,
        amount: i128,
        vintage_year: u32,
        serial_start: u64,
        serial_end: u64,
    },
    Retire {
        seq: u32,
        actor: usize,
        batch: usize,
        amount: i128,
        retirement: u32,
    },
    Transfer {
        seq: u32,
        from: usize,
        to: usize,
        batch: usize,
        amount: i128,
    },
    ListCredits {
        seq: u32,
        seller: usize,
        listing: u32,
        batch: usize,
        project: usize,
        amount: i128,
        price: i128,
        vintage_year: u32,
        methodology: usize,
    },
    DelistCredits {
        actor: usize,
        listing: u32,
    },
    Purchase {
        buyer: usize,
        listing: u32,
        amount: i128,
    },
}

// ── Id helpers ────────────────────────────────────────────────────────────────

pub fn project_id(seq: u32, i: usize) -> String {
    format!("p{seq}_{i}")
}
pub fn batch_id(seq: u32, i: usize) -> String {
    format!("b{seq}_{i}")
}
pub fn period_id(seq: u32, p: u32) -> String {
    format!("per{seq}_{p}")
}
/// Listings and retirements use a world-global monotonic id so they never
/// collide — the contracts have no duplicate-id guard for either, and a silent
/// overwrite would only muddy the model.
pub fn listing_id(n: u32) -> String {
    format!("list{n}")
}
pub fn retirement_id(n: u32) -> String {
    format!("ret{n}")
}
pub fn methodology(i: usize) -> &'static str {
    METHODOLOGIES[i % METHODOLOGIES.len()]
}

// ── Generator ─────────────────────────────────────────────────────────────────

pub struct Generator {
    rng: Rng,
    // World-global monotonic counters (unique ids across the whole campaign).
    listing_ctr: u32,
    retirement_ctr: u32,
    // Per-sequence known state, reset by `begin_sequence`.
    seq: u32,
    known_projects: Vec<usize>,
    known_batches: Vec<usize>,
    known_listings: Vec<u32>,
    period_ctr: u32,
}

impl Generator {
    pub fn new(seed: u64) -> Self {
        Generator {
            rng: Rng::new(seed),
            listing_ctr: 0,
            retirement_ctr: 0,
            seq: 0,
            known_projects: Vec::new(),
            known_batches: Vec::new(),
            known_listings: Vec::new(),
            period_ctr: 0,
        }
    }

    /// Generates the operations for one logical sequence in namespace `seq`.
    pub fn sequence(&mut self, seq: u32, len: usize) -> Vec<Op> {
        self.seq = seq;
        self.known_projects.clear();
        self.known_batches.clear();
        self.known_listings.clear();
        self.period_ctr = 0;
        (0..len).map(|_| self.next_op()).collect()
    }

    fn next_op(&mut self) -> Op {
        // Weighted toward the operations that make lifecycle progress. Registry
        // and mint are heavy early; retire/transfer/purchase become productive
        // only once batches and listings exist, which the pickers handle.
        match self.rng.weighted(&[
            18, // register
            10, // verify
            3,  // reject
            3,  // suspend
            6,  // monitoring
            5,  // price
            3,  // flag
            18, // mint
            12, // retire
            6,  // transfer
            12, // list
            3,  // delist
            12, // purchase
        ]) {
            0 => self.gen_register(),
            1 => self.gen_verifier_action(false),
            2 => self.gen_verifier_action(true),
            3 => self.gen_suspend(),
            4 => self.gen_monitoring(),
            5 => self.gen_price(),
            6 => self.gen_flag(),
            7 => self.gen_mint(),
            8 => self.gen_retire(),
            9 => self.gen_transfer(),
            10 => self.gen_list(),
            11 => self.gen_delist(),
            _ => self.gen_purchase(),
        }
    }

    /// Mostly the intended role, occasionally a wrong actor to test the gate.
    fn actor_for(&mut self, role: usize) -> usize {
        if self.rng.chance(85) {
            role
        } else {
            self.rng.below(N_ACTORS)
        }
    }

    fn trader(&mut self) -> usize {
        FIRST_TRADER + self.rng.below(N_ACTORS - FIRST_TRADER)
    }

    fn vintage(&mut self) -> u32 {
        if self.rng.chance(10) {
            // Out-of-band vintages exercise InvalidVintageYear on both sides.
            if self.rng.chance(50) {
                self.rng.range_u32(1900, 1999)
            } else {
                self.rng.range_u32(2101, 2200)
            }
        } else {
            self.rng.range_u32(2000, 2100)
        }
    }

    fn amount(&mut self) -> i128 {
        if self.rng.chance(8) {
            self.rng.range_i128(-3, 0)
        } else {
            self.rng.range_i128(1, MAX_AMOUNT)
        }
    }

    fn gen_register(&mut self) -> Op {
        let project = self.rng.below(N_PROJECTS);
        if !self.known_projects.contains(&project) {
            self.known_projects.push(project);
        }
        Op::RegisterProject {
            seq: self.seq,
            actor: self.actor_for(ADMIN),
            project,
            vintage_year: self.vintage(),
        }
    }

    fn gen_verifier_action(&mut self, reject: bool) -> Op {
        let project = self.pick_project();
        let actor = self.actor_for(VERIFIER);
        if reject {
            Op::RejectProject { seq: self.seq, actor, project }
        } else {
            Op::VerifyProject { seq: self.seq, actor, project }
        }
    }

    fn gen_suspend(&mut self) -> Op {
        Op::SuspendProject {
            seq: self.seq,
            actor: self.actor_for(ADMIN),
            project: self.pick_project(),
        }
    }

    fn gen_monitoring(&mut self) -> Op {
        self.period_ctr += 1;
        Op::SubmitMonitoring {
            seq: self.seq,
            actor: self.actor_for(ORACLE),
            project: self.pick_project(),
            period: self.period_ctr,
            tonnes: self.amount(),
            score: self.rng.range_u32(0, 100),
        }
    }

    fn gen_price(&mut self) -> Op {
        Op::UpdatePrice {
            actor: self.actor_for(ORACLE),
            methodology: self.rng.below(METHODOLOGIES.len()),
            vintage_year: self.rng.range_u32(2000, 2100),
            price: if self.rng.chance(8) {
                self.rng.range_i128(-3, 0)
            } else {
                self.rng.range_i128(1, 100_0000000)
            },
        }
    }

    fn gen_flag(&mut self) -> Op {
        Op::FlagProject {
            seq: self.seq,
            actor: self.actor_for(ORACLE),
            project: self.pick_project(),
        }
    }

    fn gen_mint(&mut self) -> Op {
        let batch = self.rng.below(N_BATCHES);
        if !self.known_batches.contains(&batch) {
            self.known_batches.push(batch);
        }
        let amount = self.amount();
        // The contract now requires `serial_end - serial_start + 1 == amount`, so
        // a valid mint must size the range to the amount. `serial_start` is still
        // drawn from a cramped span so distinct batches' ranges collide often
        // (exercising overlap rejection). Two variants deliberately break the
        // rules: an inverted range (rejected early) and a width that does not
        // match the amount (rejected by the new reconciliation check).
        let base = self.seq as u64 * SERIAL_BLOCK;
        let serial_start = base + self.rng.range_u64(1, 2000);
        let serial_end = if self.rng.chance(8) {
            // Inverted range → InvalidSerialRange (before the overlap check).
            serial_start.saturating_sub(self.rng.range_u64(1, 10))
        } else if amount <= 0 {
            // Amount is rejected first; any well-ordered range will do.
            serial_start
        } else if self.rng.chance(10) {
            // Width deliberately off by one → InvalidSerialRange (after overlap).
            serial_start + amount as u64
        } else {
            // Matched range, the only shape the contract accepts.
            serial_start + amount as u64 - 1
        };
        Op::Mint {
            seq: self.seq,
            actor: self.actor_for(ADMIN),
            project: self.pick_project(),
            batch,
            amount,
            vintage_year: self.vintage(),
            serial_start,
            serial_end,
        }
    }

    fn gen_retire(&mut self) -> Op {
        self.retirement_ctr += 1;
        Op::Retire {
            seq: self.seq,
            actor: self.trader(),
            batch: self.pick_batch(),
            amount: self.amount(),
            retirement: self.retirement_ctr,
        }
    }

    fn gen_transfer(&mut self) -> Op {
        Op::Transfer {
            seq: self.seq,
            from: self.trader(),
            to: self.trader(),
            batch: self.pick_batch(),
            amount: self.amount(),
        }
    }

    fn gen_list(&mut self) -> Op {
        self.listing_ctr += 1;
        let listing = self.listing_ctr;
        self.known_listings.push(listing);
        let batch = self.pick_batch();
        Op::ListCredits {
            seq: self.seq,
            seller: self.trader(),
            listing,
            batch,
            project: self.pick_project(),
            amount: if self.rng.chance(8) {
                self.rng.range_i128(-3, 0)
            } else {
                self.rng.range_i128(1, MAX_AMOUNT)
            },
            price: if self.rng.chance(8) {
                self.rng.range_i128(-3, 0)
            } else {
                self.rng.range_i128(1, 50_0000000)
            },
            vintage_year: self.rng.range_u32(2000, 2100),
            methodology: self.rng.below(METHODOLOGIES.len()),
        }
    }

    fn gen_delist(&mut self) -> Op {
        Op::DelistCredits {
            actor: self.trader(),
            listing: self.pick_listing(),
        }
    }

    fn gen_purchase(&mut self) -> Op {
        Op::Purchase {
            buyer: self.trader(),
            listing: self.pick_listing(),
            amount: if self.rng.chance(8) {
                self.rng.range_i128(-3, 0)
            } else {
                self.rng.range_i128(1, MAX_AMOUNT)
            },
        }
    }

    // Pickers prefer a known target (precondition satisfied) but occasionally
    // reach for an out-of-pool id to exercise the not-found rejection path.

    fn pick_project(&mut self) -> usize {
        if self.known_projects.is_empty() || self.rng.chance(15) {
            self.rng.below(N_PROJECTS)
        } else {
            let i = self.rng.below(self.known_projects.len());
            self.known_projects[i]
        }
    }

    fn pick_batch(&mut self) -> usize {
        if self.known_batches.is_empty() || self.rng.chance(15) {
            self.rng.below(N_BATCHES)
        } else {
            let i = self.rng.below(self.known_batches.len());
            self.known_batches[i]
        }
    }

    fn pick_listing(&mut self) -> u32 {
        if self.known_listings.is_empty() || self.rng.chance(15) {
            // Reach for a plausible-but-maybe-absent id.
            self.rng.range_u64(1, (self.listing_ctr + 1) as u64) as u32
        } else {
            let i = self.rng.below(self.known_listings.len());
            self.known_listings[i]
        }
    }
}
