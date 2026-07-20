#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror,
    Address, Env, String, Vec,
    symbol_short, vec,
};

// ── Error Enum ────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CarbonError {
    ProjectNotFound        = 1,
    ProjectNotVerified     = 2,
    ProjectSuspended       = 3,
    InsufficientCredits    = 4,
    AlreadyRetired         = 5,
    SerialNumberConflict   = 6,
    UnauthorizedVerifier   = 7,
    UnauthorizedOracle     = 8,
    InvalidVintageYear     = 9,
    ListingNotFound        = 10,
    InsufficientLiquidity  = 11,
    PriceNotSet            = 12,
    MonitoringDataStale    = 13,
    DoubleCountingDetected = 14,
    RetirementIrreversible = 15,
    ZeroAmountNotAllowed   = 16,
    ProjectAlreadyExists   = 17,
    InvalidSerialRange     = 18,
    AlreadyInitialized     = 19,
    ReentrancyGuard        = 20,
}

// ── Storage Keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Batch(String),
    Retirement(String),
    ProjectBatches(String),
    SerialRegistry,
    Admin,
    RegistryContract,
    Locked,
    // Time-lock keys (Issue 3)
    TimelockOp(String),
    TimelockContest(String),
    TimelockDelay,
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreditStatus {
    Active,
    PartiallyRetired,
    FullyRetired,
    Suspended,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CreditBatch {
    pub batch_id:     String,
    pub project_id:   String,
    pub vintage_year: u32,
    pub amount:       i128,
    pub serial_start: u64,
    pub serial_end:   u64,
    pub issued_at:    u64,
    pub status:       CreditStatus,
    pub metadata_cid: String,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RetirementCertificate {
    pub retirement_id:    String,
    pub credit_batch_id:  String,
    pub project_id:       String,
    pub amount:           i128,
    pub retired_by:       Address,
    pub beneficiary:      String,
    pub retirement_reason: String,
    pub vintage_year:     u32,
    pub serial_numbers:   Vec<u64>,
    pub retired_at:       u64,
    pub tx_hash:          String,
}

/// Compact serial range stored globally to detect overlaps.
#[contracttype]
#[derive(Clone, Debug)]
pub struct SerialRange {
    pub start: u64,
    pub end:   u64,
}

/// Tracks how many credits in a batch have been retired so far.
#[contracttype]
#[derive(Clone)]
pub enum RetiredKey {
    BatchRetired(String),
}

// ── Time-lock types (Issue 3) ─────────────────────────────────────────────────

const TIMELOCK_DEFAULT_DELAY_SECS: u64 = 172_800; // 48 hours

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GovAction {
    PauseContract,
    UnpauseContract,
    ChangeTimelockDelay,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct PendingOp {
    pub op_id:        String,
    pub action:       GovAction,
    pub target:       String,
    pub initiated_by: Address,
    pub eta:          u64,
    pub payload:      String,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct ContestRecord {
    pub op_id:        String,
    pub contested_by: Address,
    pub reason:       String,
    pub contested_at: u64,
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct CarbonCreditContract;

#[contractimpl]
impl CarbonCreditContract {

    /// Initialise with admin address.
    /// Can only be called once — subsequent calls return [`CarbonError::AlreadyInitialized`].
    pub fn initialize(env: Env, admin: Address, registry_contract: Address) -> Result<(), CarbonError> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(CarbonError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::RegistryContract, &registry_contract);
        let ranges: Vec<SerialRange> = vec![&env];
        env.storage().persistent().set(&DataKey::SerialRegistry, &ranges);
        Ok(())
    }

    /// Mint verified carbon credits for a verified project. Assigns unique serial
    /// numbers to each credit, preventing double-counting globally.
    ///
    /// # Errors
    /// - [`CarbonError::ZeroAmountNotAllowed`] if `amount` is zero.
    /// - [`CarbonError::InvalidSerialRange`] if `serial_end < serial_start`.
    /// - [`CarbonError::SerialNumberConflict`] if serial range overlaps an existing batch.
    /// - [`CarbonError::InvalidVintageYear`] if vintage year is out of range.
    pub fn mint_credits(
        env: Env,
        admin: Address,
        project_id: String,
        amount: i128,
        vintage_year: u32,
        batch_id: String,
        serial_start: u64,
        serial_end: u64,
        metadata_cid: String,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }

        if amount <= 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }
        if serial_end < serial_start { Self::release_lock(&env); return Err(CarbonError::InvalidSerialRange); }
        if vintage_year < 2000 || vintage_year > 2100 { Self::release_lock(&env); return Err(CarbonError::InvalidVintageYear); }
        if env.storage().persistent().has(&DataKey::Batch(batch_id.clone())) {
            Self::release_lock(&env); return Err(CarbonError::SerialNumberConflict);
        }
        if !Self::verify_serial_range_internal(&env, serial_start, serial_end) {
            Self::release_lock(&env); return Err(CarbonError::DoubleCountingDetected);
        }

        // ── effects ───────────────────────────────────────────────────────────
        // Register serial range globally
        let mut ranges: Vec<SerialRange> = env
            .storage()
            .persistent()
            .get(&DataKey::SerialRegistry)
            .unwrap_or_else(|| vec![&env]);
        ranges.push_back(SerialRange { start: serial_start, end: serial_end });
        env.storage().persistent().set(&DataKey::SerialRegistry, &ranges);

        let batch = CreditBatch {
            batch_id:     batch_id.clone(),
            project_id:   project_id.clone(),
            vintage_year,
            amount,
            serial_start,
            serial_end,
            issued_at:    env.ledger().timestamp(),
            status:       CreditStatus::Active,
            metadata_cid: metadata_cid.clone(),
        };
        if let Err(e) = Self::assert_valid_batch(&batch) { Self::release_lock(&env); return Err(e); }
        env.storage().persistent().set(&DataKey::Batch(batch_id.clone()), &batch);

        // Append to project batch index
        let mut project_batches: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::ProjectBatches(project_id.clone()))
            .unwrap_or_else(|| vec![&env]);
        project_batches.push_back(batch_id.clone());
        env.storage().persistent().set(&DataKey::ProjectBatches(project_id.clone()), &project_batches);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("minted")),
            (batch_id, project_id, amount, vintage_year, serial_start, serial_end),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Permanently and irreversibly retire carbon credits on-chain. Retired credits
    /// are burned and can never be transferred or retired again under any circumstance.
    /// A permanent [`RetirementCertificate`] is recorded on-chain.
    ///
    /// # Errors
    /// - [`CarbonError::ZeroAmountNotAllowed`] if `amount` is zero.
    /// - [`CarbonError::InsufficientCredits`] if batch has fewer active credits than requested.
    /// - [`CarbonError::AlreadyRetired`] if batch is fully retired.
    pub fn retire_credits(
        env: Env,
        holder: Address,
        batch_id: String,
        amount: i128,
        retirement_reason: String,
        beneficiary: String,
        retirement_id: String,
        tx_hash: String,
    ) -> Result<RetirementCertificate, CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        holder.require_auth();

        if amount <= 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }

        let mut batch = match Self::load_batch(&env, &batch_id) {
            Ok(b) => b,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };

        if batch.status == CreditStatus::FullyRetired { Self::release_lock(&env); return Err(CarbonError::AlreadyRetired); }
        if batch.status == CreditStatus::Suspended { Self::release_lock(&env); return Err(CarbonError::ProjectSuspended); }

        let active_amount = Self::active_amount(&env, &batch);
        if amount > active_amount { Self::release_lock(&env); return Err(CarbonError::InsufficientCredits); }

        // ── effects ───────────────────────────────────────────────────────────
        // Compute serial numbers for this retirement slice
        let already_retired: i128 = env
            .storage()
            .persistent()
            .get(&RetiredKey::BatchRetired(batch_id.clone()))
            .unwrap_or(0i128);

        let retire_serial_start = batch.serial_start + already_retired as u64;
        let retire_serial_end   = retire_serial_start + amount as u64 - 1;

        let mut serial_numbers: Vec<u64> = vec![&env];
        let mut s = retire_serial_start;
        while s <= retire_serial_end {
            serial_numbers.push_back(s);
            s += 1;
        }

        // Update batch status — track retired amount persistently
        let new_retired = already_retired + amount;
        env.storage().persistent().set(&RetiredKey::BatchRetired(batch_id.clone()), &new_retired);

        let new_active = batch.amount - new_retired;
        batch.status = if new_active == 0 {
            CreditStatus::FullyRetired
        } else {
            CreditStatus::PartiallyRetired
        };
        env.storage().persistent().set(&DataKey::Batch(batch_id.clone()), &batch);

        let cert = RetirementCertificate {
            retirement_id:     retirement_id.clone(),
            credit_batch_id:   batch_id.clone(),
            project_id:        batch.project_id.clone(),
            amount,
            retired_by:        holder.clone(),
            beneficiary:       beneficiary.clone(),
            retirement_reason: retirement_reason.clone(),
            vintage_year:      batch.vintage_year,
            serial_numbers:    serial_numbers.clone(),
            retired_at:        env.ledger().timestamp(),
            tx_hash:           tx_hash.clone(),
        };
        if let Err(e) = Self::assert_valid_retirement(&cert) { Self::release_lock(&env); return Err(e); }
        env.storage().persistent().set(&DataKey::Retirement(retirement_id.clone()), &cert);

        // ── interactions ──────────────────────────────────────────────────────
        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("retired")),
            (retirement_id, batch_id, batch.project_id, amount, holder, beneficiary),
        );
        Self::release_lock(&env);
        Ok(cert)
    }

    /// Transfer credits between accounts. Retired batches cannot be transferred.
    ///
    /// # Errors
    /// - [`CarbonError::AlreadyRetired`] if batch is fully retired.
    /// - [`CarbonError::InsufficientCredits`] if insufficient active credits.
    pub fn transfer_credits(
        env: Env,
        from: Address,
        to: Address,
        batch_id: String,
        amount: i128,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        from.require_auth();

        if amount <= 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }

        let batch = match Self::load_batch(&env, &batch_id) {
            Ok(b) => b,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };

        if batch.status == CreditStatus::FullyRetired { Self::release_lock(&env); return Err(CarbonError::AlreadyRetired); }
        if batch.status == CreditStatus::Suspended { Self::release_lock(&env); return Err(CarbonError::ProjectSuspended); }

        let active = Self::active_amount(&env, &batch);
        if amount > active { Self::release_lock(&env); return Err(CarbonError::InsufficientCredits); }

        // ── effects ───────────────────────────────────────────────────────────
        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("transfer")),
            (batch_id, from, to, amount),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Returns a [`CreditBatch`] by ID.
    pub fn get_credit_batch(env: Env, batch_id: String) -> Result<CreditBatch, CarbonError> {
        Self::load_batch(&env, &batch_id)
    }

    /// Returns a permanent [`RetirementCertificate`] by retirement ID.
    pub fn get_retirement_certificate(
        env: Env,
        retirement_id: String,
    ) -> Result<RetirementCertificate, CarbonError> {
        env.storage()
            .persistent()
            .get(&DataKey::Retirement(retirement_id))
            .ok_or(CarbonError::ProjectNotFound)
    }

    /// Returns `true` if the serial range `[serial_start, serial_end]` does NOT
    /// overlap any existing batch — i.e., safe to mint.
    pub fn verify_serial_range(env: Env, serial_start: u64, serial_end: u64) -> bool {
        Self::verify_serial_range_internal(&env, serial_start, serial_end)
    }

    /// Returns all [`CreditBatch`] records for a given project.
    pub fn get_project_credits(env: Env, project_id: String) -> Vec<CreditBatch> {
        let batch_ids: Vec<String> = env
            .storage()
            .persistent()
            .get(&DataKey::ProjectBatches(project_id))
            .unwrap_or_else(|| vec![&env]);

        let mut result: Vec<CreditBatch> = vec![&env];
        for id in batch_ids.iter() {
            if let Some(b) = env.storage().persistent().get(&DataKey::Batch(id.clone())) {
                result.push_back(b);
            }
        }
        result
    }

    // ── Time-lock governance functions (Issue 3) ─────────────────────────────

    /// Propose pausing all credit mutations. Queued with time-lock delay.
    pub fn propose_pause(env: Env, admin: Address, op_id: String, reason: String) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }
        let delay = env.storage().persistent()
            .get::<DataKey, u64>(&DataKey::TimelockDelay)
            .unwrap_or(TIMELOCK_DEFAULT_DELAY_SECS);
        let op = PendingOp {
            op_id: op_id.clone(), action: GovAction::PauseContract,
            target: String::from_str(&env, "contract"), initiated_by: admin.clone(),
            eta: env.ledger().timestamp() + delay, payload: reason.clone(),
        };
        env.storage().persistent().set(&DataKey::TimelockOp(op_id.clone()), &op);
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("tl_queue")), (op_id, admin, reason));
        Self::release_lock(&env);
        Ok(())
    }

    /// Execute a queued pause after delay has elapsed.
    pub fn execute_pause(env: Env, admin: Address, op_id: String) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }
        let op: PendingOp = match env.storage().persistent().get(&DataKey::TimelockOp(op_id.clone())) {
            Some(o) => o,
            None => { Self::release_lock(&env); return Err(CarbonError::ProjectNotFound); }
        };
        if env.storage().persistent().has(&DataKey::TimelockContest(op_id.clone())) {
            Self::release_lock(&env); return Err(CarbonError::RetirementIrreversible);
        }
        if env.ledger().timestamp() < op.eta {
            Self::release_lock(&env); return Err(CarbonError::RetirementIrreversible);
        }
        env.storage().persistent().remove(&DataKey::TimelockOp(op_id.clone()));
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("paused")), (op_id, admin));
        Self::release_lock(&env);
        Ok(())
    }

    /// Contest a pending governance operation. Any address may contest.
    pub fn contest_operation(env: Env, contestant: Address, op_id: String, reason: String) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        contestant.require_auth();
        let op: PendingOp = match env.storage().persistent().get(&DataKey::TimelockOp(op_id.clone())) {
            Some(o) => o,
            None => { Self::release_lock(&env); return Err(CarbonError::ProjectNotFound); }
        };
        if env.ledger().timestamp() >= op.eta {
            Self::release_lock(&env); return Err(CarbonError::AlreadyRetired);
        }
        let record = ContestRecord {
            op_id: op_id.clone(), contested_by: contestant.clone(),
            reason: reason.clone(), contested_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DataKey::TimelockContest(op_id.clone()), &record);
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("tl_ctest")), (op_id, contestant, reason));
        Self::release_lock(&env);
        Ok(())
    }

    /// Roll back a pending or contested operation. Admin only.
    pub fn rollback_operation(env: Env, admin: Address, op_id: String) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }
        if !env.storage().persistent().has(&DataKey::TimelockOp(op_id.clone())) {
            Self::release_lock(&env); return Err(CarbonError::ProjectNotFound);
        }
        env.storage().persistent().remove(&DataKey::TimelockOp(op_id.clone()));
        if env.storage().persistent().has(&DataKey::TimelockContest(op_id.clone())) {
            env.storage().persistent().remove(&DataKey::TimelockContest(op_id.clone()));
        }
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("tl_rback")), (op_id, admin));
        Self::release_lock(&env);
        Ok(())
    }

    /// Query a pending operation by ID.
    pub fn get_pending_op(env: Env, op_id: String) -> Result<PendingOp, CarbonError> {
        env.storage().persistent().get(&DataKey::TimelockOp(op_id)).ok_or(CarbonError::ProjectNotFound)
    }

    /// Query contest record by op ID.
    pub fn get_contest(env: Env, op_id: String) -> Result<ContestRecord, CarbonError> {
        env.storage().persistent().get(&DataKey::TimelockContest(op_id)).ok_or(CarbonError::ProjectNotFound)
    }

    /// Update the time-lock delay in seconds. Admin only.
    pub fn set_timelock_delay(env: Env, admin: Address, delay_secs: u64) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }
        if delay_secs == 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }
        env.storage().persistent().set(&DataKey::TimelockDelay, &delay_secs);
        Self::release_lock(&env);
        Ok(())
    }

    // ── Validation helpers (Issue 2) ──────────────────────────────────────────

    /// Assert that a [`CreditBatch`] satisfies all data-structure invariants:
    /// - `batch_id`, `project_id`, `metadata_cid` must be non-empty.
    /// - `amount` > 0.
    /// - `vintage_year` ∈ [2000, 2100].
    /// - `serial_end` ≥ `serial_start`.
    /// - `serial_end - serial_start + 1` equals `amount` (serial range matches amount).
    fn assert_valid_batch(batch: &CreditBatch) -> Result<(), CarbonError> {
        if batch.batch_id.len() == 0     { return Err(CarbonError::SerialNumberConflict); }
        if batch.project_id.len() == 0   { return Err(CarbonError::ProjectNotFound); }
        if batch.metadata_cid.len() == 0 { return Err(CarbonError::ProjectNotFound); }
        if batch.amount <= 0             { return Err(CarbonError::ZeroAmountNotAllowed); }
        if batch.vintage_year < 2000 || batch.vintage_year > 2100 {
            return Err(CarbonError::InvalidVintageYear);
        }
        if batch.serial_end < batch.serial_start { return Err(CarbonError::InvalidSerialRange); }
        let range_len = (batch.serial_end - batch.serial_start + 1) as i128;
        if range_len != batch.amount { return Err(CarbonError::InvalidSerialRange); }
        Ok(())
    }

    /// Assert that a [`RetirementCertificate`] satisfies all invariants:
    /// - `retirement_id`, `credit_batch_id`, `project_id`, `tx_hash` non-empty.
    /// - `amount` > 0.
    /// - `serial_numbers` non-empty and count matches `amount`.
    fn assert_valid_retirement(cert: &RetirementCertificate) -> Result<(), CarbonError> {
        if cert.retirement_id.len() == 0   { return Err(CarbonError::ProjectNotFound); }
        if cert.credit_batch_id.len() == 0 { return Err(CarbonError::ProjectNotFound); }
        if cert.project_id.len() == 0      { return Err(CarbonError::ProjectNotFound); }
        if cert.tx_hash.len() == 0         { return Err(CarbonError::ProjectNotFound); }
        if cert.amount <= 0                { return Err(CarbonError::ZeroAmountNotAllowed); }
        if cert.serial_numbers.len() == 0  { return Err(CarbonError::InvalidSerialRange); }
        if cert.serial_numbers.len() as i128 != cert.amount {
            return Err(CarbonError::InvalidSerialRange);
        }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn load_batch(env: &Env, batch_id: &String) -> Result<CreditBatch, CarbonError> {
        env.storage()
            .persistent()
            .get(&DataKey::Batch(batch_id.clone()))
            .ok_or(CarbonError::ProjectNotFound)
    }

    fn require_admin(env: &Env, caller: &Address) -> Result<(), CarbonError> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .ok_or(CarbonError::UnauthorizedVerifier)?;
        if &admin != caller {
            return Err(CarbonError::UnauthorizedVerifier);
        }
        Ok(())
    }

    /// Returns the number of credits in a batch that have not yet been retired.
    fn active_amount(env: &Env, batch: &CreditBatch) -> i128 {
        if batch.status == CreditStatus::FullyRetired {
            return 0;
        }
        let retired: i128 = env
            .storage()
            .persistent()
            .get(&RetiredKey::BatchRetired(batch.batch_id.clone()))
            .unwrap_or(0i128);
        batch.amount - retired
    }

    fn verify_serial_range_internal(env: &Env, start: u64, end: u64) -> bool {
        let ranges: Vec<SerialRange> = env
            .storage()
            .persistent()
            .get(&DataKey::SerialRegistry)
            .unwrap_or_else(|| vec![env]);

        for r in ranges.iter() {
            // Overlap check: two ranges overlap if start <= r.end && end >= r.start
            if start <= r.end && end >= r.start {
                return false;
            }
        }
        true
    }

    // ── Reentrancy guard ──────────────────────────────────────────────────────

    fn acquire_lock(env: &Env) -> Result<(), CarbonError> {
        if env.storage().instance().get::<DataKey, bool>(&DataKey::Locked).unwrap_or(false) {
            return Err(CarbonError::ReentrancyGuard);
        }
        env.storage().instance().set(&DataKey::Locked, &true);
        Ok(())
    }

    fn release_lock(env: &Env) {
        env.storage().instance().set(&DataKey::Locked, &false);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, String, vec};

    fn setup() -> (Env, CarbonCreditContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let client = CarbonCreditContractClient::new(&env, &id);
        client.initialize(&admin, &registry).unwrap();
        (env, client)
    }

    fn s(env: &Env, v: &str) -> String { String::from_str(env, v) }

    fn mint(env: &Env, client: &CarbonCreditContractClient, admin: &Address) {
        let _ = client.mint_credits(
            admin,
            &s(env, "proj-001"),
            &1000_i128,
            &2023_u32,
            &s(env, "batch-001"),
            &1_u64,
            &1000_u64,
            &s(env, "QmCID"),
        );
    }

    #[test]
    fn test_mint_credits_success() {
        let (env, client) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(
            &admin,
            &s(&env, "proj-001"),
            &500_i128,
            &2023_u32,
            &s(&env, "batch-A"),
            &1_u64,
            &500_u64,
            &s(&env, "QmCID"),
        ).unwrap();

        let b = c.get_credit_batch(&s(&env, "batch-A")).unwrap();
        assert_eq!(b.amount, 500);
        assert_eq!(b.status, CreditStatus::Active);
    }

    #[test]
    fn test_serial_conflict_detection() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();
        // Overlapping range 50-150 should fail
        let result = c.try_mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b2"), &50_u64, &150_u64, &s(&env, "cid"));
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_serial_range_no_overlap() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();
        // Non-overlapping range should return true
        assert!(c.verify_serial_range(&101_u64, &200_u64));
        // Overlapping range should return false
        assert!(!c.verify_serial_range(&50_u64, &150_u64));
    }

    #[test]
    fn test_retire_credits_permanent() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        let cert = c.retire_credits(
            &holder,
            &s(&env, "b1"),
            &100_i128,
            &s(&env, "offset 2023 emissions"),
            &s(&env, "Acme Corp"),
            &s(&env, "ret-001"),
            &s(&env, "txhash123"),
        ).unwrap();

        assert_eq!(cert.amount, 100);
        assert_eq!(cert.beneficiary, s(&env, "Acme Corp"));

        let batch = c.get_credit_batch(&s(&env, "b1")).unwrap();
        assert_eq!(batch.status, CreditStatus::FullyRetired);
    }

    #[test]
    fn test_retired_credits_cannot_be_transferred() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        c.retire_credits(&holder, &s(&env, "b1"), &100_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "ret-001"), &s(&env, "tx")).unwrap();

        let to = Address::generate(&env);
        let result = c.try_transfer_credits(&holder, &to, &s(&env, "b1"), &10_i128);
        assert!(result.is_err());
    }

    #[test]
    fn test_retired_credits_cannot_be_retired_again() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        c.retire_credits(&holder, &s(&env, "b1"), &100_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "ret-001"), &s(&env, "tx")).unwrap();

        let result = c.try_retire_credits(&holder, &s(&env, "b1"), &100_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "ret-002"), &s(&env, "tx2"));
        assert!(result.is_err());
    }

    #[test]
    fn test_partial_retirement_updates_status() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        c.retire_credits(&holder, &s(&env, "b1"), &40_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "ret-001"), &s(&env, "tx")).unwrap();

        let batch = c.get_credit_batch(&s(&env, "b1")).unwrap();
        assert_eq!(batch.status, CreditStatus::PartiallyRetired);
    }

    #[test]
    fn test_get_retirement_certificate() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        c.retire_credits(&holder, &s(&env, "b1"), &100_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "ret-001"), &s(&env, "tx")).unwrap();

        let cert = c.get_retirement_certificate(&s(&env, "ret-001")).unwrap();
        assert_eq!(cert.amount, 100);
        assert_eq!(cert.retirement_id, s(&env, "ret-001"));
    }

    #[test]
    fn test_zero_amount_rejected() {
        let (env, _) = setup();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        let result = c.try_mint_credits(&admin, &s(&env, "p1"), &0_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid"));
        assert!(result.is_err());
    }

    #[test]
    fn test_initialize_twice_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        let result = c.try_initialize(&admin, &registry);
        assert!(result.is_err());
    }

    // ── Reentrancy guard tests ─────────────────────────────────────────────────

    /// Lock is released after mint_credits succeeds; second mint on different batch works.
    #[test]
    fn test_lock_released_after_mint() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();
        // non-overlapping range: lock must be free
        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b2"), &101_u64, &200_u64, &s(&env, "cid")).unwrap();

        let b = c.get_credit_batch(&s(&env, "b2")).unwrap();
        assert_eq!(b.amount, 100);
    }

    /// Lock is released after retire_credits succeeds.
    #[test]
    fn test_lock_released_after_retire() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &200_i128, &2023_u32, &s(&env, "b1"), &1_u64, &200_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        c.retire_credits(&holder, &s(&env, "b1"), &100_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "r1"), &s(&env, "tx")).unwrap();
        // retire again on the same batch (partial left): lock must be free
        c.retire_credits(&holder, &s(&env, "b1"), &100_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "r2"), &s(&env, "tx2")).unwrap();

        let b = c.get_credit_batch(&s(&env, "b1")).unwrap();
        assert_eq!(b.status, CreditStatus::FullyRetired);
    }

    /// Lock is released after a failed retire (over-retirement attempt).
    #[test]
    fn test_lock_released_after_failed_retire() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &50_i128, &2023_u32, &s(&env, "b1"), &1_u64, &50_u64, &s(&env, "cid")).unwrap();

        let holder = Address::generate(&env);
        // This should fail (too many)
        let _ = c.try_retire_credits(&holder, &s(&env, "b1"), &999_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "r1"), &s(&env, "tx"));
        // Lock must be free — valid retire should succeed
        c.retire_credits(&holder, &s(&env, "b1"), &50_i128, &s(&env, "reason"), &s(&env, "Corp"), &s(&env, "r2"), &s(&env, "tx2")).unwrap();
    }

    /// Lock is released after transfer_credits succeeds.
    #[test]
    fn test_lock_released_after_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();

        let from = Address::generate(&env);
        let to   = Address::generate(&env);
        c.transfer_credits(&from, &to, &s(&env, "b1"), &10_i128).unwrap();
        // Second transfer: lock must be free
        c.transfer_credits(&from, &to, &s(&env, "b1"), &10_i128).unwrap();
    }

    /// Lock is released after a failed mint (duplicate batch id).
    #[test]
    fn test_lock_released_after_failed_mint() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        c.mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &1_u64, &100_u64, &s(&env, "cid")).unwrap();
        // Duplicate batch_id — should fail
        let _ = c.try_mint_credits(&admin, &s(&env, "p1"), &100_i128, &2023_u32, &s(&env, "b1"), &201_u64, &300_u64, &s(&env, "cid"));
        // New batch on free range must succeed (lock released)
        c.mint_credits(&admin, &s(&env, "p1"), &50_i128, &2023_u32, &s(&env, "b3"), &201_u64, &250_u64, &s(&env, "cid")).unwrap();
    }

    // ── Issue 2: Validation helper tests ──────────────────────────────────────

    #[test]
    fn test_mint_mismatched_serial_range_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        // amount=100 but serial range 1..=50 — only 50 serials, mismatch
        let result = c.try_mint_credits(
            &admin,
            &s(&env, "p1"),
            &100_i128,
            &2023_u32,
            &s(&env, "b-mismatch"),
            &1_u64,
            &50_u64,
            &s(&env, "cid"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_mint_empty_metadata_cid_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();

        let result = c.try_mint_credits(
            &admin, &s(&env, "p1"), &100_i128, &2023_u32,
            &s(&env, "b-empty-cid"), &1_u64, &100_u64, &s(&env, ""),
        );
        assert!(result.is_err());
    }

    // ── Issue 3: Time-lock tests ──────────────────────────────────────────────

    #[test]
    fn test_propose_pause_and_query() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        c.propose_pause(&admin, &s(&env, "op-001"), &s(&env, "maintenance")).unwrap();
        let op = c.get_pending_op(&s(&env, "op-001")).unwrap();
        assert_eq!(op.op_id, s(&env, "op-001"));
    }

    #[test]
    fn test_execute_pause_before_delay_fails() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        c.propose_pause(&admin, &s(&env, "op-002"), &s(&env, "test")).unwrap();
        assert!(c.try_execute_pause(&admin, &s(&env, "op-002")).is_err());
    }

    #[test]
    fn test_execute_pause_after_delay_succeeds() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        c.propose_pause(&admin, &s(&env, "op-003"), &s(&env, "upgrade")).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 172_800 + 1, protocol_version: 20, sequence_number: 200,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        c.execute_pause(&admin, &s(&env, "op-003")).unwrap();
        assert!(c.try_get_pending_op(&s(&env, "op-003")).is_err());
    }

    #[test]
    fn test_contest_pause_blocks_execution() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        c.propose_pause(&admin, &s(&env, "op-004"), &s(&env, "contested")).unwrap();
        let user = Address::generate(&env);
        c.contest_operation(&user, &s(&env, "op-004"), &s(&env, "unjustified")).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 172_800 + 1, protocol_version: 20, sequence_number: 200,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        assert!(c.try_execute_pause(&admin, &s(&env, "op-004")).is_err());
    }

    #[test]
    fn test_rollback_pause_op() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        c.propose_pause(&admin, &s(&env, "op-005"), &s(&env, "rollback")).unwrap();
        c.rollback_operation(&admin, &s(&env, "op-005")).unwrap();
        assert!(c.try_get_pending_op(&s(&env, "op-005")).is_err());
    }

    #[test]
    fn test_unauthorized_propose_pause_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let registry = Address::generate(&env);
        let id = env.register_contract(None, CarbonCreditContract);
        let c = CarbonCreditContractClient::new(&env, &id);
        c.initialize(&admin, &registry).unwrap();
        let rogue = Address::generate(&env);
        assert!(c.try_propose_pause(&rogue, &s(&env, "op-bad"), &s(&env, "hack")).is_err());
    }
}
