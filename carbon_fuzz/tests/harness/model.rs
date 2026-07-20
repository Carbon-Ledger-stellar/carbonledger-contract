//! Shadow model of `carbon_credit`.
//!
//! The model is a plain-Rust reimplementation of what the contract *should*
//! do, kept deliberately independent of the contract source: it is written
//! from the documented behaviour, not by calling into the contract. The
//! executor runs both and compares. A divergence is either a contract bug or a
//! model bug, and both are worth knowing about.
//!
//! Check ordering matters. `mint_credits` validates in a fixed sequence and
//! returns the *first* failure, so the model must reject in the same order or
//! the predicted error code will not match.

use std::collections::{HashMap, HashSet};

/// Mirrors `carbon_credit::CreditStatus`, minus `Suspended` — nothing in the
/// contract can currently reach that state, so the model would never predict it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Active,
    PartiallyRetired,
    FullyRetired,
}

/// The subset of `CarbonError` reachable from the operations we generate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    ZeroAmountNotAllowed,
    InvalidSerialRange,
    InvalidVintageYear,
    SerialNumberConflict,
    DoubleCountingDetected,
    AlreadyRetired,
    InsufficientCredits,
    ProjectNotFound,
    UnauthorizedVerifier,
}

pub type Predicted = Result<(), Fault>;

#[derive(Clone, Debug)]
pub struct Batch {
    pub project_id: String,
    pub amount: i128,
    pub vintage_year: u32,
    pub serial_start: u64,
    pub serial_end: u64,
    pub retired: i128,
}

impl Batch {
    pub fn status(&self) -> Status {
        if self.retired == 0 {
            Status::Active
        } else if self.retired >= self.amount {
            Status::FullyRetired
        } else {
            Status::PartiallyRetired
        }
    }

    /// Credits not yet retired. Mirrors `Self::active_amount`, which short
    /// circuits to 0 once the batch is fully retired.
    pub fn active(&self) -> i128 {
        if self.status() == Status::FullyRetired {
            0
        } else {
            self.amount - self.retired
        }
    }

    /// Width of the declared serial range. Note this is *not* tied to `amount`
    /// anywhere in the contract — see `serials_escape_declared_range`.
    pub fn declared_width(&self) -> u128 {
        (self.serial_end - self.serial_start) as u128 + 1
    }
}

#[derive(Clone, Debug)]
pub struct Cert {
    pub batch_id: String,
    pub amount: i128,
    /// Inclusive serial span allocated to this retirement.
    pub serial_lo: u64,
    pub serial_hi: u64,
}

#[derive(Default)]
pub struct CreditModel {
    pub batches: HashMap<String, Batch>,
    /// Global append-only serial registry. Mirrors `DataKey::SerialRegistry`.
    pub ranges: Vec<(u64, u64)>,
    pub certs: HashMap<String, Cert>,
    /// Batch ids that have ever been observed `FullyRetired`, so we can assert
    /// the status never rolls back.
    pub ever_fully_retired: HashSet<String>,
}

impl CreditModel {
    pub fn new() -> Self {
        Self::default()
    }

    fn overlaps(&self, start: u64, end: u64) -> bool {
        self.ranges.iter().any(|&(s, e)| start <= e && end >= s)
    }

    /// Predicts `mint_credits`, replicating the contract's check order:
    /// admin, amount, serial range, vintage, duplicate batch id, overlap.
    pub fn predict_mint(
        &self,
        is_admin: bool,
        batch_id: &str,
        amount: i128,
        vintage_year: u32,
        serial_start: u64,
        serial_end: u64,
    ) -> Predicted {
        if !is_admin {
            return Err(Fault::UnauthorizedVerifier);
        }
        if amount <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        if serial_end < serial_start {
            return Err(Fault::InvalidSerialRange);
        }
        if !(2000..=2100).contains(&vintage_year) {
            return Err(Fault::InvalidVintageYear);
        }
        // A duplicate batch id reports SerialNumberConflict, not a dedicated
        // "already exists" code. Surprising, but it is the contract's contract.
        if self.batches.contains_key(batch_id) {
            return Err(Fault::SerialNumberConflict);
        }
        if self.overlaps(serial_start, serial_end) {
            return Err(Fault::DoubleCountingDetected);
        }
        Ok(())
    }

    pub fn apply_mint(
        &mut self,
        batch_id: &str,
        project_id: &str,
        amount: i128,
        vintage_year: u32,
        serial_start: u64,
        serial_end: u64,
    ) {
        self.ranges.push((serial_start, serial_end));
        self.batches.insert(
            batch_id.to_string(),
            Batch {
                project_id: project_id.to_string(),
                amount,
                vintage_year,
                serial_start,
                serial_end,
                retired: 0,
            },
        );
    }

    /// Predicts `retire_credits`: amount, batch exists, already retired,
    /// sufficient active balance.
    ///
    /// Note there is no ownership check in the contract — any authenticated
    /// address may retire any batch — so the model takes no actor argument.
    pub fn predict_retire(&self, batch_id: &str, amount: i128) -> Predicted {
        if amount <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        let Some(batch) = self.batches.get(batch_id) else {
            // Missing batches surface as ProjectNotFound, reused as a generic
            // "not found" code.
            return Err(Fault::ProjectNotFound);
        };
        if batch.status() == Status::FullyRetired {
            return Err(Fault::AlreadyRetired);
        }
        if amount > batch.active() {
            return Err(Fault::InsufficientCredits);
        }
        Ok(())
    }

    /// Returns the serial span the contract will allocate for this retirement.
    pub fn apply_retire(&mut self, batch_id: &str, retirement_id: &str, amount: i128) -> (u64, u64) {
        let batch = self
            .batches
            .get_mut(batch_id)
            .expect("apply_retire on a batch the model rejected");

        // Allocation is sequential from the batch start, by cumulative retired
        // count — independent of the declared serial_end.
        let lo = batch.serial_start + batch.retired as u64;
        let hi = lo + amount as u64 - 1;
        batch.retired += amount;

        if batch.status() == Status::FullyRetired {
            self.ever_fully_retired.insert(batch_id.to_string());
        }
        self.certs.insert(
            retirement_id.to_string(),
            Cert {
                batch_id: batch_id.to_string(),
                amount,
                serial_lo: lo,
                serial_hi: hi,
            },
        );
        (lo, hi)
    }

    /// Predicts `transfer_credits`. The contract validates against the batch's
    /// active amount and then writes no storage at all — there is no per-address
    /// balance ledger — so a successful transfer leaves the model unchanged.
    pub fn predict_transfer(&self, batch_id: &str, amount: i128) -> Predicted {
        if amount <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        let Some(batch) = self.batches.get(batch_id) else {
            return Err(Fault::ProjectNotFound);
        };
        if batch.status() == Status::FullyRetired {
            return Err(Fault::AlreadyRetired);
        }
        if amount > batch.active() {
            return Err(Fault::InsufficientCredits);
        }
        Ok(())
    }

    /// Batches whose retired serials run past the declared `serial_end`.
    ///
    /// The contract never reconciles `amount` against the serial range width,
    /// so minting `amount = 100` over serials `1..=5` lets retirement allocate
    /// serials 1..=100 — 95 of which were never registered in the global
    /// registry and may already belong to another batch. Reported rather than
    /// asserted, because the current contract genuinely behaves this way.
    pub fn serials_escape_declared_range(&self) -> Vec<String> {
        self.batches
            .iter()
            .filter(|(_, b)| b.retired > 0 && (b.retired as u128) > b.declared_width())
            .map(|(id, _)| id.clone())
            .collect()
    }
}
