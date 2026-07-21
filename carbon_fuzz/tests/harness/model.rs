//! Shadow model of the whole CarbonLedger system.
//!
//! One sub-model per contract — registry, credit, marketplace, oracle — plus a
//! tiny USDC balance ledger for the token the marketplace settles against. The
//! executor runs every operation against both the real contracts and this
//! model and asserts they agree on the outcome; a divergence is either a
//! contract bug or a model bug, and both are worth knowing about.
//!
//! The model is written from each contract's *documented behaviour and check
//! order*, deliberately not by calling into the contract. Two rules matter when
//! extending it:
//!
//! * **Check ordering is load-bearing.** Every mutating entry point validates in
//!   a fixed sequence and returns the *first* failure. The model must reject in
//!   the same order or it will predict the right outcome with the wrong error.
//! * **The four contracts make no cross-contract calls to each other.** Minting
//!   never consults the registry; a listing is never validated against a real
//!   batch; a purchase moves USDC but not credit ownership. The model mirrors
//!   that independence faithfully — the resulting consistency gaps are reported
//!   by `invariants::report_gaps`, not asserted away.

use std::collections::{HashMap, HashSet};

/// The subset of `CarbonError` reachable from the operations we generate. Every
/// contract ships the *same* error enum, so one `Fault` vocabulary covers all
/// four. A variant the generator cannot reach is deliberately absent so that the
/// executor panics loudly if the contract ever returns it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    ZeroAmountNotAllowed,
    InvalidSerialRange,
    InvalidVintageYear,
    SerialNumberConflict,
    DoubleCountingDetected,
    AlreadyRetired,
    InsufficientCredits,
    InsufficientLiquidity,
    ListingNotFound,
    ProjectNotFound,
    ProjectAlreadyExists,
    UnauthorizedVerifier,
    UnauthorizedOracle,
}

pub type Predicted = Result<(), Fault>;

// ── Registry ────────────────────────────────────────────────────────────────

/// Mirrors `carbon_registry::ProjectStatus`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectStatus {
    Pending,
    Verified,
    Rejected,
    Suspended,
    // Reached only via the oracle's `update_project_status`, which the generator
    // does not drive today; kept so the mirror of the contract enum is complete.
    #[allow(dead_code)]
    Completed,
}

#[derive(Default)]
pub struct RegistryModel {
    pub projects: HashMap<String, ProjectStatus>,
}

impl RegistryModel {
    /// `register_project`: admin gate, then duplicate id, then vintage range.
    pub fn predict_register(&self, is_admin: bool, project_id: &str, vintage: u32) -> Predicted {
        if !is_admin {
            return Err(Fault::UnauthorizedVerifier);
        }
        if self.projects.contains_key(project_id) {
            return Err(Fault::ProjectAlreadyExists);
        }
        if !(2000..=2100).contains(&vintage) {
            return Err(Fault::InvalidVintageYear);
        }
        Ok(())
    }

    pub fn apply_register(&mut self, project_id: &str) {
        self.projects
            .insert(project_id.to_string(), ProjectStatus::Pending);
    }

    /// `verify_project`/`reject_project`: verifier gate, then project exists.
    pub fn predict_verifier_action(&self, is_verifier: bool, project_id: &str) -> Predicted {
        if !is_verifier {
            return Err(Fault::UnauthorizedVerifier);
        }
        if !self.projects.contains_key(project_id) {
            return Err(Fault::ProjectNotFound);
        }
        Ok(())
    }

    /// `suspend_project`: admin gate, then project exists.
    pub fn predict_admin_action(&self, is_admin: bool, project_id: &str) -> Predicted {
        if !is_admin {
            return Err(Fault::UnauthorizedVerifier);
        }
        if !self.projects.contains_key(project_id) {
            return Err(Fault::ProjectNotFound);
        }
        Ok(())
    }

    pub fn set_status(&mut self, project_id: &str, status: ProjectStatus) {
        self.projects.insert(project_id.to_string(), status);
    }
}

// ── Credit ──────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreditStatus {
    Active,
    PartiallyRetired,
    FullyRetired,
}

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
    pub fn status(&self) -> CreditStatus {
        if self.retired == 0 {
            CreditStatus::Active
        } else if self.retired >= self.amount {
            CreditStatus::FullyRetired
        } else {
            CreditStatus::PartiallyRetired
        }
    }

    /// Credits not yet retired. Mirrors `active_amount`, which short-circuits to
    /// 0 once the batch is fully retired.
    pub fn active(&self) -> i128 {
        if self.status() == CreditStatus::FullyRetired {
            0
        } else {
            self.amount - self.retired
        }
    }

    /// Width of the declared serial range — *not* tied to `amount` anywhere in
    /// the contract. See `serials_escape_declared_range`.
    pub fn declared_width(&self) -> u128 {
        (self.serial_end - self.serial_start) as u128 + 1
    }
}

#[derive(Clone, Debug)]
pub struct Cert {
    pub batch_id: String,
    pub amount: i128,
    pub serial_lo: u64,
    pub serial_hi: u64,
}

#[derive(Default)]
pub struct CreditModel {
    pub batches: HashMap<String, Batch>,
    /// Global append-only serial registry. Mirrors `DataKey::SerialRegistry`.
    pub ranges: Vec<(u64, u64)>,
    pub certs: HashMap<String, Cert>,
    /// Batch ids ever observed `FullyRetired`, to assert status never rolls back.
    pub ever_fully_retired: HashSet<String>,
}

impl CreditModel {
    fn overlaps(&self, start: u64, end: u64) -> bool {
        self.ranges.iter().any(|&(s, e)| start <= e && end >= s)
    }

    /// `mint_credits`: admin, amount, serial range, vintage, duplicate id, overlap.
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
        // `assert_valid_batch` runs last (in the effects section) and requires the
        // declared serial range to be exactly `amount` wide. This is the check
        // that closed the old "serials escape the declared range" gap.
        if (serial_end - serial_start + 1) as i128 != amount {
            return Err(Fault::InvalidSerialRange);
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
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

    /// `retire_credits`: amount, batch exists, already retired, active balance.
    /// There is no ownership check in the contract — any authenticated address
    /// may retire any batch — so no actor argument.
    pub fn predict_retire(&self, batch_id: &str, amount: i128) -> Predicted {
        if amount <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        let Some(batch) = self.batches.get(batch_id) else {
            return Err(Fault::ProjectNotFound);
        };
        if batch.status() == CreditStatus::FullyRetired {
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
        if batch.status() == CreditStatus::FullyRetired {
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

    /// `transfer_credits`: same guards as retire; writes no storage on success.
    pub fn predict_transfer(&self, batch_id: &str, amount: i128) -> Predicted {
        if amount <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        let Some(batch) = self.batches.get(batch_id) else {
            return Err(Fault::ProjectNotFound);
        };
        if batch.status() == CreditStatus::FullyRetired {
            return Err(Fault::AlreadyRetired);
        }
        if amount > batch.active() {
            return Err(Fault::InsufficientCredits);
        }
        Ok(())
    }

    /// Batches whose retired serials run past the declared `serial_end` — the
    /// documented double-counting gap. Reported, not asserted.
    pub fn serials_escape_declared_range(&self) -> Vec<String> {
        self.batches
            .iter()
            .filter(|(_, b)| b.retired > 0 && (b.retired as u128) > b.declared_width())
            .map(|(id, _)| id.clone())
            .collect()
    }
}

// ── Marketplace ───────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListingStatus {
    Active,
    Sold,
    PartiallyFilled,
    Delisted,
}

#[derive(Clone, Debug)]
pub struct Listing {
    /// Actor index of the seller, so the model can predict the delist owner gate.
    pub seller: usize,
    /// Batch the listing claims to sell. The marketplace never checks it against
    /// the credit contract, so it may name a batch that was never minted — which
    /// is exactly the gap `report_gaps` surfaces.
    pub batch_id: String,
    pub original_amount: i128,
    pub amount_available: i128,
    pub price_per_credit: i128,
    pub status: ListingStatus,
}

#[derive(Default)]
pub struct MarketModel {
    pub listings: HashMap<String, Listing>,
}

impl MarketModel {
    /// `list_credits`: amount and price must both be positive. The contract does
    /// *not* check for a duplicate listing id (it would overwrite); the
    /// generator hands out unique ids so that path is never exercised here.
    pub fn predict_list(&self, amount: i128, price: i128) -> Predicted {
        if amount <= 0 || price <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        Ok(())
    }

    pub fn apply_list(
        &mut self,
        listing_id: &str,
        seller: usize,
        batch_id: &str,
        amount: i128,
        price: i128,
    ) {
        self.listings.insert(
            listing_id.to_string(),
            Listing {
                seller,
                batch_id: batch_id.to_string(),
                original_amount: amount,
                amount_available: amount,
                price_per_credit: price,
                status: ListingStatus::Active,
            },
        );
    }

    /// `delist_credits`: listing exists, then caller is the seller. Note there is
    /// no status guard — even a Sold listing can be re-flagged Delisted.
    pub fn predict_delist(&self, listing_id: &str, caller: usize) -> Predicted {
        let Some(listing) = self.listings.get(listing_id) else {
            return Err(Fault::ListingNotFound);
        };
        if listing.seller != caller {
            return Err(Fault::UnauthorizedVerifier);
        }
        Ok(())
    }

    pub fn apply_delist(&mut self, listing_id: &str) {
        if let Some(l) = self.listings.get_mut(listing_id) {
            l.status = ListingStatus::Delisted;
        }
    }

    /// `purchase_credits`: amount positive, listing exists, listing not
    /// Delisted/Sold, sufficient liquidity.
    pub fn predict_purchase(&self, listing_id: &str, amount: i128) -> Predicted {
        if amount <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        let Some(listing) = self.listings.get(listing_id) else {
            return Err(Fault::ListingNotFound);
        };
        if listing.status == ListingStatus::Delisted || listing.status == ListingStatus::Sold {
            // A dead listing reports ListingNotFound, reusing that code.
            return Err(Fault::ListingNotFound);
        }
        if amount > listing.amount_available {
            return Err(Fault::InsufficientLiquidity);
        }
        Ok(())
    }

    /// Applies a purchase and returns `(seller, proceeds, protocol_fee)` so the
    /// executor can update the USDC ledger. Fee is 1% by integer division.
    pub fn apply_purchase(&mut self, listing_id: &str, amount: i128) -> (usize, i128, i128) {
        let l = self
            .listings
            .get_mut(listing_id)
            .expect("apply_purchase on a listing the model rejected");
        let total = l.price_per_credit * amount;
        let fee = total / 100;
        let proceeds = total - fee;
        l.amount_available -= amount;
        l.status = if l.amount_available == 0 {
            ListingStatus::Sold
        } else {
            ListingStatus::PartiallyFilled
        };
        (l.seller, proceeds, fee)
    }
}

// ── Oracle ────────────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct OracleModel {
    /// project_id → whether monitoring data has ever been submitted. Timestamps
    /// are not advanced in the harness, so submitted data is always "current".
    pub monitored: HashSet<String>,
    /// (methodology, vintage) → whether a benchmark price has been set.
    pub priced: HashSet<(String, u32)>,
}

impl OracleModel {
    /// `submit_monitoring_data`: oracle gate, then positive tonnes.
    pub fn predict_submit(&self, is_oracle: bool, tonnes: i128) -> Predicted {
        if !is_oracle {
            return Err(Fault::UnauthorizedOracle);
        }
        if tonnes <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        Ok(())
    }

    /// `update_credit_price`: oracle gate, then positive price.
    pub fn predict_price(&self, is_oracle: bool, price: i128) -> Predicted {
        if !is_oracle {
            return Err(Fault::UnauthorizedOracle);
        }
        if price <= 0 {
            return Err(Fault::ZeroAmountNotAllowed);
        }
        Ok(())
    }

    /// `flag_project`: oracle gate only — no other validation.
    pub fn predict_flag(&self, is_oracle: bool) -> Predicted {
        if !is_oracle {
            return Err(Fault::UnauthorizedOracle);
        }
        Ok(())
    }
}

// ── Aggregate ───────────────────────────────────────────────────────────────

pub struct LedgerModel {
    pub registry: RegistryModel,
    pub credit: CreditModel,
    pub market: MarketModel,
    pub oracle: OracleModel,
    /// Actor index → expected USDC balance. Only settlement moves this, so it is
    /// an exact mirror the executor can reconcile against the token contract.
    pub usdc: HashMap<usize, i128>,
    /// Total USDC in existence, fixed at construction — settlement must conserve it.
    pub usdc_initial_total: i128,
}

impl LedgerModel {
    /// `funded` lists the actors seeded with a starting USDC balance and its size.
    pub fn new(funded: &[usize], starting: i128) -> Self {
        let mut usdc = HashMap::new();
        for &a in funded {
            usdc.insert(a, starting);
        }
        LedgerModel {
            registry: RegistryModel::default(),
            credit: CreditModel::default(),
            market: MarketModel::default(),
            oracle: OracleModel::default(),
            usdc,
            usdc_initial_total: starting * funded.len() as i128,
        }
    }

    pub fn usdc_of(&self, actor: usize) -> i128 {
        *self.usdc.get(&actor).unwrap_or(&0)
    }

    pub fn credit_usdc(&mut self, actor: usize, delta: i128) {
        *self.usdc.entry(actor).or_insert(0) += delta;
    }
}
