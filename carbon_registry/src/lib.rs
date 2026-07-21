#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, contracterror,
    Address, Env, String, Vec, Map,
    symbol_short, vec,
};

// ── Error Enum ────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CarbonError {
    ProjectNotFound       = 1,
    ProjectNotVerified    = 2,
    ProjectSuspended      = 3,
    InsufficientCredits   = 4,
    AlreadyRetired        = 5,
    SerialNumberConflict  = 6,
    UnauthorizedVerifier  = 7,
    UnauthorizedOracle    = 8,
    InvalidVintageYear    = 9,
    ListingNotFound       = 10,
    InsufficientLiquidity = 11,
    PriceNotSet           = 12,
    MonitoringDataStale   = 13,
    DoubleCountingDetected = 14,
    RetirementIrreversible = 15,
    ZeroAmountNotAllowed  = 16,
    ProjectAlreadyExists  = 17,
    InvalidSerialRange    = 18,
    AlreadyInitialized    = 19,
    ReentrancyGuard       = 20,
    ArithmeticOverflow    = 21,
}

// ── Checked-arithmetic helper ─────────────────────────────────────────────────
//
// Arithmetic safety (Issue 4): every add/sub/mul in this contract uses the
// `checked_*` family and surfaces overflow/underflow as
// [`CarbonError::ArithmeticOverflow`] instead of trapping the transaction.
//
// # Input-range assumptions
// - `total_credits_issued` accumulates positive i128 amounts; assumed
//   `< 1e15` per project (≈1 billion tonnes at 1e6 precision) with ample i128
//   headroom, but the running total is still checked on every increment.
// - Timestamps and time-lock delays are `u64`; `timestamp + delay` overflow is
//   unreachable in practice but still checked.
// - On overflow/underflow every guarded operation returns
//   `CarbonError::ArithmeticOverflow`; none can wrap or panic-trap in release wasm.
macro_rules! checked {
    ($env:expr, $opt:expr) => {
        match $opt {
            Some(v) => v,
            None => {
                Self::release_lock($env);
                return Err(CarbonError::ArithmeticOverflow);
            }
        }
    };
}

// ── Storage Keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Project(String),
    Verifiers,
    OracleAddress,
    RegistryAdmin,
    Locked,
    // Time-lock keys (Issue 3)
    TimelockOp(String),      // op_id → PendingOp
    TimelockContest(String), // op_id → ContestRecord
    TimelockDelay,           // u64 seconds (default 172_800 = 48 h)
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProjectStatus {
    Pending,
    Verified,
    Rejected,
    Suspended,
    Completed,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct CarbonProject {
    pub project_id:            String,
    pub name:                  String,
    pub methodology:           String,
    pub country:               String,
    pub project_type:          String,
    pub verifier_address:      Address,
    pub metadata_cid:          String,
    pub total_credits_issued:  i128,
    pub total_credits_retired: i128,
    pub status:                ProjectStatus,
    pub vintage_year:          u32,
    pub created_at:            u64,
}

// ── Time-lock types (Issue 3) ─────────────────────────────────────────────────

/// Default delay: 48 hours in seconds.
const TIMELOCK_DEFAULT_DELAY_SECS: u64 = 172_800;

/// Governance operations subject to time-lock.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GovAction {
    SuspendProject,
    UnsuspendProject,
    ChangeTimelockDelay,
}

/// A pending governance operation waiting out its delay period.
#[contracttype]
#[derive(Clone, Debug)]
pub struct PendingOp {
    pub op_id:        String,
    pub action:       GovAction,
    pub target:       String,    // project_id or parameter name
    pub initiated_by: Address,
    pub eta:          u64,       // earliest execution timestamp (seconds)
    pub payload:      String,    // human-readable reason / new value
}

/// Record of a contest raised against a pending operation.
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
pub struct CarbonRegistryContract;

#[contractimpl]
impl CarbonRegistryContract {

    /// Initialise the registry with an admin, oracle address, and initial verifier set.
    /// Can only be called once — subsequent calls return [`CarbonError::AlreadyInitialized`].
    pub fn initialize(
        env: Env,
        admin: Address,
        oracle_address: Address,
        verifiers: Vec<Address>,
    ) -> Result<(), CarbonError> {
        if env.storage().persistent().has(&DataKey::RegistryAdmin) {
            return Err(CarbonError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::RegistryAdmin, &admin);
        env.storage().persistent().set(&DataKey::OracleAddress, &oracle_address);
        env.storage().persistent().set(&DataKey::Verifiers, &verifiers);
        Ok(())
    }

    /// Register a new carbon offset project. Status is set to `Pending` until a
    /// verifier calls [`verify_project`].
    ///
    /// # Errors
    /// - [`CarbonError::ProjectAlreadyExists`] if `project_id` is already registered.
    /// - [`CarbonError::InvalidVintageYear`] if `vintage_year` is before 2000 or after 2100.
    pub fn register_project(
        env: Env,
        admin: Address,
        project_id: String,
        name: String,
        metadata_cid: String,
        verifier_address: Address,
        methodology: String,
        country: String,
        project_type: String,
        vintage_year: u32,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }

        if env.storage().persistent().has(&DataKey::Project(project_id.clone())) {
            Self::release_lock(&env);
            return Err(CarbonError::ProjectAlreadyExists);
        }
        if vintage_year < 2000 || vintage_year > 2100 {
            Self::release_lock(&env);
            return Err(CarbonError::InvalidVintageYear);
        }

        // ── effects ───────────────────────────────────────────────────────────
        let project = CarbonProject {
            project_id:            project_id.clone(),
            name:                  name.clone(),
            methodology:           methodology.clone(),
            country:               country.clone(),
            project_type:          project_type.clone(),
            verifier_address:      verifier_address.clone(),
            metadata_cid:          metadata_cid.clone(),
            total_credits_issued:  0,
            total_credits_retired: 0,
            status:                ProjectStatus::Pending,
            vintage_year,
            created_at:            env.ledger().timestamp(),
        };
        if let Err(e) = Self::assert_valid_project(&project) {
            Self::release_lock(&env);
            return Err(e);
        }
        env.storage().persistent().set(&DataKey::Project(project_id.clone()), &project);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("reg_proj")),
            (project_id, methodology, country, vintage_year),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Approve a pending project for credit issuance. Caller must be an
    /// accredited verifier stored in `VERIFIED_VERIFIERS`.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedVerifier`] if caller is not a registered verifier.
    /// - [`CarbonError::ProjectNotFound`] if `project_id` does not exist.
    pub fn verify_project(
        env: Env,
        verifier_address: Address,
        project_id: String,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        verifier_address.require_auth();
        if let Err(e) = Self::require_verifier(&env, &verifier_address) { Self::release_lock(&env); return Err(e); }

        let mut project = match Self::load_project(&env, &project_id) {
            Ok(p) => p,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };

        // ── effects ───────────────────────────────────────────────────────────
        project.status = ProjectStatus::Verified;
        env.storage().persistent().set(&DataKey::Project(project_id.clone()), &project);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("verified")),
            (project_id, verifier_address),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Permanently reject a fraudulent project. Rejection is irreversible.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedVerifier`] if caller is not a registered verifier.
    /// - [`CarbonError::ProjectNotFound`] if `project_id` does not exist.
    pub fn reject_project(
        env: Env,
        verifier_address: Address,
        project_id: String,
        reason: String,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        verifier_address.require_auth();
        if let Err(e) = Self::require_verifier(&env, &verifier_address) { Self::release_lock(&env); return Err(e); }

        let mut project = match Self::load_project(&env, &project_id) {
            Ok(p) => p,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };

        // ── effects ───────────────────────────────────────────────────────────
        project.status = ProjectStatus::Rejected;
        env.storage().persistent().set(&DataKey::Project(project_id.clone()), &project);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("rejected")),
            (project_id, verifier_address, reason),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Oracle pushes updated monitoring status for a project.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedOracle`] if caller is not the registered oracle.
    /// - [`CarbonError::ProjectNotFound`] if `project_id` does not exist.
    pub fn update_project_status(
        env: Env,
        oracle_address: Address,
        project_id: String,
        status: ProjectStatus,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        oracle_address.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_address) { Self::release_lock(&env); return Err(e); }

        let mut project = match Self::load_project(&env, &project_id) {
            Ok(p) => p,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };

        // ── effects ───────────────────────────────────────────────────────────
        project.status = status.clone();
        env.storage().persistent().set(&DataKey::Project(project_id.clone()), &project);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("st_update")),
            (project_id, oracle_address),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Propose a suspension of a project under investigation.
    /// The operation is queued with a time-lock delay (default 48 h).
    /// Execute via [`execute_suspend_project`] after the delay has elapsed.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedVerifier`] if caller is not the admin.
    /// - [`CarbonError::ProjectNotFound`] if `project_id` does not exist.
    pub fn propose_suspend_project(
        env: Env,
        admin: Address,
        op_id: String,
        project_id: String,
        reason: String,
    ) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }
        if let Err(e) = Self::load_project(&env, &project_id) { Self::release_lock(&env); return Err(e); }

        let delay = env.storage().persistent()
            .get::<DataKey, u64>(&DataKey::TimelockDelay)
            .unwrap_or(TIMELOCK_DEFAULT_DELAY_SECS);
        let eta = checked!(&env, env.ledger().timestamp().checked_add(delay));

        let op = PendingOp {
            op_id: op_id.clone(),
            action: GovAction::SuspendProject,
            target: project_id.clone(),
            initiated_by: admin.clone(),
            eta,
            payload: reason.clone(),
        };
        env.storage().persistent().set(&DataKey::TimelockOp(op_id.clone()), &op);
        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("tl_queue")),
            (op_id, admin, project_id, eta),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Execute a previously queued suspend operation after its delay has elapsed.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedVerifier`] if caller is not the admin.
    /// - [`CarbonError::ProjectNotFound`] if op or project does not exist.
    /// - [`CarbonError::RetirementIrreversible`] if delay not elapsed or op contested.
    pub fn execute_suspend_project(
        env: Env,
        admin: Address,
        op_id: String,
    ) -> Result<(), CarbonError> {
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

        let mut project = match Self::load_project(&env, &op.target) {
            Ok(p) => p,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };
        project.status = ProjectStatus::Suspended;
        env.storage().persistent().set(&DataKey::Project(op.target.clone()), &project);
        env.storage().persistent().remove(&DataKey::TimelockOp(op_id.clone()));
        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("suspended")),
            (op.target, admin, op.payload),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Contest a pending governance operation during its delay window.
    /// Any address may contest. A valid contest prevents execution.
    ///
    /// # Errors
    /// - [`CarbonError::ProjectNotFound`] if the op does not exist.
    /// - [`CarbonError::AlreadyRetired`] if the delay window has already closed.
    pub fn contest_operation(
        env: Env,
        contestant: Address,
        op_id: String,
        reason: String,
    ) -> Result<(), CarbonError> {
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
            op_id: op_id.clone(),
            contested_by: contestant.clone(),
            reason: reason.clone(),
            contested_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DataKey::TimelockContest(op_id.clone()), &record);
        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("tl_ctest")),
            (op_id, contestant, reason),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Roll back (cancel) a pending or contested operation. Admin only.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedVerifier`] if caller is not the admin.
    /// - [`CarbonError::ProjectNotFound`] if the op does not exist.
    pub fn rollback_operation(
        env: Env,
        admin: Address,
        op_id: String,
    ) -> Result<(), CarbonError> {
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
        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("tl_rback")),
            (op_id, admin),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Query a pending operation by ID.
    pub fn get_pending_op(env: Env, op_id: String) -> Result<PendingOp, CarbonError> {
        env.storage().persistent()
            .get(&DataKey::TimelockOp(op_id))
            .ok_or(CarbonError::ProjectNotFound)
    }

    /// Query the contest record for an op, if any.
    pub fn get_contest(env: Env, op_id: String) -> Result<ContestRecord, CarbonError> {
        env.storage().persistent()
            .get(&DataKey::TimelockContest(op_id))
            .ok_or(CarbonError::ProjectNotFound)
    }

    /// Update the time-lock delay in seconds. Admin only. Default is 172_800 (48 h).
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedVerifier`] if caller is not the admin.
    /// - [`CarbonError::ZeroAmountNotAllowed`] if delay is zero.
    pub fn set_timelock_delay(
        env: Env,
        admin: Address,
        delay_secs: u64,
    ) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }
        if delay_secs == 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }
        env.storage().persistent().set(&DataKey::TimelockDelay, &delay_secs);
        Self::release_lock(&env);
        Ok(())
    }

    /// Admin suspends a project directly (retained for backward compatibility).
    /// Prefer the time-locked propose/execute flow for production use.
    ///
    /// # Errors
    /// - [`CarbonError::ProjectNotFound`] if `project_id` does not exist.
    pub fn suspend_project(
        env: Env,
        admin: Address,
        project_id: String,
        reason: String,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        admin.require_auth();
        if let Err(e) = Self::require_admin(&env, &admin) { Self::release_lock(&env); return Err(e); }

        let mut project = match Self::load_project(&env, &project_id) {
            Ok(p) => p,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };

        // ── effects ───────────────────────────────────────────────────────────
        project.status = ProjectStatus::Suspended;
        env.storage().persistent().set(&DataKey::Project(project_id.clone()), &project);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("suspended")),
            (project_id, admin, reason),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Returns the full [`CarbonProject`] record.
    ///
    /// # Errors
    /// - [`CarbonError::ProjectNotFound`] if `project_id` does not exist.
    pub fn get_project(env: Env, project_id: String) -> Result<CarbonProject, CarbonError> {
        Self::load_project(&env, &project_id)
    }

    /// Increment the issued credit counter for a project (called by carbon_credit contract).
    pub fn increment_issued(
        env: Env,
        oracle_address: Address,
        project_id: String,
        amount: i128,
    ) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        oracle_address.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_address) { Self::release_lock(&env); return Err(e); }
        let mut project = match Self::load_project(&env, &project_id) {
            Ok(p) => p,
            Err(e) => { Self::release_lock(&env); return Err(e); }
        };
        project.total_credits_issued =
            checked!(&env, project.total_credits_issued.checked_add(amount));
        if let Err(e) = Self::assert_valid_project(&project) {
            Self::release_lock(&env);
            return Err(e);
        }
        env.storage().persistent().set(&DataKey::Project(project_id), &project);
        Self::release_lock(&env);
        Ok(())
    }

    // ── Validation helpers (Issue 2) ──────────────────────────────────────────

    /// Assert that a [`CarbonProject`] satisfies all data-structure invariants:
    /// - `project_id`, `name`, `methodology`, `country`, `project_type`,
    ///   `metadata_cid` must be non-empty (len > 0).
    /// - `vintage_year` ∈ [2000, 2100].
    /// - `total_credits_issued` and `total_credits_retired` must be ≥ 0.
    /// - `total_credits_retired` must be ≤ `total_credits_issued`.
    fn assert_valid_project(project: &CarbonProject) -> Result<(), CarbonError> {
        if project.project_id.len() == 0   { return Err(CarbonError::ProjectNotFound); }
        if project.name.len() == 0         { return Err(CarbonError::ProjectNotFound); }
        if project.methodology.len() == 0  { return Err(CarbonError::ProjectNotFound); }
        if project.country.len() == 0      { return Err(CarbonError::ProjectNotFound); }
        if project.project_type.len() == 0 { return Err(CarbonError::ProjectNotFound); }
        if project.metadata_cid.len() == 0 { return Err(CarbonError::ProjectNotFound); }
        if project.vintage_year < 2000 || project.vintage_year > 2100 {
            return Err(CarbonError::InvalidVintageYear);
        }
        if project.total_credits_issued < 0  { return Err(CarbonError::ZeroAmountNotAllowed); }
        if project.total_credits_retired < 0 { return Err(CarbonError::ZeroAmountNotAllowed); }
        if project.total_credits_retired > project.total_credits_issued {
            return Err(CarbonError::InsufficientCredits);
        }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

    fn load_project(env: &Env, project_id: &String) -> Result<CarbonProject, CarbonError> {
        env.storage()
            .persistent()
            .get(&DataKey::Project(project_id.clone()))
            .ok_or(CarbonError::ProjectNotFound)
    }

    fn require_admin(env: &Env, caller: &Address) -> Result<(), CarbonError> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::RegistryAdmin)
            .ok_or(CarbonError::UnauthorizedVerifier)?;
        if &admin != caller {
            return Err(CarbonError::UnauthorizedVerifier);
        }
        Ok(())
    }

    fn require_verifier(env: &Env, caller: &Address) -> Result<(), CarbonError> {
        let verifiers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Verifiers)
            .unwrap_or_else(|| vec![env]);
        if !verifiers.contains(caller) {
            return Err(CarbonError::UnauthorizedVerifier);
        }
        Ok(())
    }

    fn require_oracle(env: &Env, caller: &Address) -> Result<(), CarbonError> {
        let oracle: Address = env
            .storage()
            .persistent()
            .get(&DataKey::OracleAddress)
            .ok_or(CarbonError::UnauthorizedOracle)?;
        if &oracle != caller {
            return Err(CarbonError::UnauthorizedOracle);
        }
        Ok(())
    }

    // ── Reentrancy guard ──────────────────────────────────────────────────────

    /// Acquire the reentrancy lock. Returns [`CarbonError::ReentrancyGuard`] if
    /// the contract is already executing a state-mutating function.
    fn acquire_lock(env: &Env) -> Result<(), CarbonError> {
        if env.storage().instance().get::<DataKey, bool>(&DataKey::Locked).unwrap_or(false) {
            return Err(CarbonError::ReentrancyGuard);
        }
        env.storage().instance().set(&DataKey::Locked, &true);
        Ok(())
    }

    /// Release the reentrancy lock. Must be called at the end of every
    /// state-mutating function that called `acquire_lock`.
    fn release_lock(env: &Env) {
        env.storage().instance().set(&DataKey::Locked, &false);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger}, vec, Env, String};

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let oracle   = Address::generate(&env);
        let verifier = Address::generate(&env);
        let client = CarbonRegistryContractClient::new(&env, &env.register_contract(None, CarbonRegistryContract));
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        (env, admin, oracle, verifier)
    }

    fn make_str(env: &Env, s: &str) -> String {
        String::from_str(env, s)
    }

    fn register(env: &Env, client: &CarbonRegistryContractClient, admin: &Address) {
        client.register_project(
            admin,
            &make_str(env, "proj-001"),
            &make_str(env, "Amazon Reforestation"),
            &make_str(env, "QmCID123"),
            &Address::generate(env),
            &make_str(env, "VCS"),
            &make_str(env, "Brazil"),
            &make_str(env, "forestry"),
            &2023_u32,
        );
    }

    #[test]
    fn test_register_project_valid() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Pending);
        assert_eq!(p.vintage_year, 2023);
    }

    #[test]
    fn test_register_duplicate_fails() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        let result = client.try_register_project(
            &admin,
            &make_str(&env, "proj-001"),
            &make_str(&env, "Dup"),
            &make_str(&env, "cid"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2023_u32,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_verifier_approves_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.verify_project(&verifier, &make_str(&env, "proj-001"));
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Verified);
    }

    #[test]
    fn test_unauthorized_verifier_rejected() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        let rogue = Address::generate(&env);
        let result = client.try_verify_project(&rogue, &make_str(&env, "proj-001"));
        assert!(result.is_err());
    }

    #[test]
    fn test_verifier_rejects_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.reject_project(&verifier, &make_str(&env, "proj-001"), &make_str(&env, "fraud"));
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Rejected);
    }

    #[test]
    fn test_oracle_updates_status() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.update_project_status(&oracle, &make_str(&env, "proj-001"), &ProjectStatus::Completed);
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Completed);
    }

    #[test]
    fn test_admin_suspends_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.suspend_project(&admin, &make_str(&env, "proj-001"), &make_str(&env, "investigation"));
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Suspended);
    }

    #[test]
    fn test_get_project_returns_correct_data() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.project_id, make_str(&env, "proj-001"));
        assert_eq!(p.country, make_str(&env, "Brazil"));
        assert_eq!(p.total_credits_issued, 0);
    }

    #[test]
    fn test_initialize_twice_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin    = Address::generate(&env);
        let oracle   = Address::generate(&env);
        let verifier = Address::generate(&env);
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        let result = client.try_initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        assert!(result.is_err());
    }

    // ── Reentrancy guard tests ─────────────────────────────────────────────────

    /// After a successful call the lock must be released (next call succeeds).
    #[test]
    fn test_lock_released_after_register_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        // First call succeeds
        client.register_project(
            &admin,
            &make_str(&env, "p1"),
            &make_str(&env, "Proj1"),
            &make_str(&env, "cid1"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2023_u32,
        );

        // Second call on a different project_id also succeeds (lock was released)
        client.register_project(
            &admin,
            &make_str(&env, "p2"),
            &make_str(&env, "Proj2"),
            &make_str(&env, "cid2"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2024_u32,
        );

        let p = client.get_project(&make_str(&env, "p2"));
        assert_eq!(p.status, ProjectStatus::Pending);
    }

    /// Lock is released even when register_project returns an error (duplicate).
    #[test]
    fn test_lock_released_after_failed_register() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        // Duplicate — should fail but release the lock
        let _ = client.try_register_project(
            &admin,
            &make_str(&env, "proj-001"),
            &make_str(&env, "Dup"),
            &make_str(&env, "cid"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2023_u32,
        );

        // A fresh project_id must succeed (lock released)
        client.register_project(
            &admin,
            &make_str(&env, "proj-002"),
            &make_str(&env, "Another"),
            &make_str(&env, "cid2"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2024_u32,
        );
    }

    /// Lock is released after verify_project succeeds.
    #[test]
    fn test_lock_released_after_verify_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.verify_project(&verifier, &make_str(&env, "proj-001"));

        // Suspend must succeed, proving the lock was released by verify_project
        client.suspend_project(&admin, &make_str(&env, "proj-001"), &make_str(&env, "audit"));
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Suspended);
    }

    /// Lock is released after suspend_project succeeds.
    #[test]
    fn test_lock_released_after_suspend_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.suspend_project(&admin, &make_str(&env, "proj-001"), &make_str(&env, "fraud"));

        // update_project_status must succeed (lock released)
        client.update_project_status(&oracle, &make_str(&env, "proj-001"), &ProjectStatus::Completed);
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Completed);
    }

    /// Lock is released after reject_project succeeds.
    #[test]
    fn test_lock_released_after_reject_project() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        register(&env, &client, &admin);
        client.reject_project(&verifier, &make_str(&env, "proj-001"), &make_str(&env, "fraud"));

        // Register a new project — lock must be free
        client.register_project(
            &admin,
            &make_str(&env, "proj-new"),
            &make_str(&env, "New"),
            &make_str(&env, "cid"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2025_u32,
        );
    }

    // ── Issue 2: Validation helper tests ──────────────────────────────────────

    #[test]
    fn test_register_project_empty_name_fails() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        let result = client.try_register_project(
            &admin,
            &make_str(&env, "proj-x"),
            &make_str(&env, ""),  // empty name — invalid
            &make_str(&env, "cid"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &2023_u32,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_register_project_invalid_vintage_fails() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);

        let result = client.try_register_project(
            &admin,
            &make_str(&env, "proj-y"),
            &make_str(&env, "Proj Y"),
            &make_str(&env, "cid"),
            &Address::generate(&env),
            &make_str(&env, "VCS"),
            &make_str(&env, "Brazil"),
            &make_str(&env, "forestry"),
            &1999_u32, // out of range
        );
        assert!(result.is_err());
    }

    // ── Issue 3: Time-lock tests ──────────────────────────────────────────────

    #[test]
    fn test_propose_and_query_pending_op() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        register(&env, &client, &admin);

        client.propose_suspend_project(
            &admin, &make_str(&env, "op-001"), &make_str(&env, "proj-001"), &make_str(&env, "investigation"),
        );
        let op = client.get_pending_op(&make_str(&env, "op-001"));
        assert_eq!(op.op_id, make_str(&env, "op-001"));
        assert_eq!(op.target, make_str(&env, "proj-001"));
    }

    #[test]
    fn test_execute_before_delay_fails() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        register(&env, &client, &admin);

        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.propose_suspend_project(
            &admin, &make_str(&env, "op-002"), &make_str(&env, "proj-001"), &make_str(&env, "test"),
        );
        let result = client.try_execute_suspend_project(&admin, &make_str(&env, "op-002"));
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_after_delay_succeeds() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        register(&env, &client, &admin);

        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.propose_suspend_project(
            &admin, &make_str(&env, "op-003"), &make_str(&env, "proj-001"), &make_str(&env, "audit"),
        );
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 172_800 + 1, protocol_version: 20, sequence_number: 200,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.execute_suspend_project(&admin, &make_str(&env, "op-003"));
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.status, ProjectStatus::Suspended);
    }

    #[test]
    fn test_contest_blocks_execution() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        register(&env, &client, &admin);

        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.propose_suspend_project(
            &admin, &make_str(&env, "op-004"), &make_str(&env, "proj-001"), &make_str(&env, "contested"),
        );
        let user = Address::generate(&env);
        client.contest_operation(&user, &make_str(&env, "op-004"), &make_str(&env, "unjustified"));
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 172_800 + 1, protocol_version: 20, sequence_number: 200,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        let result = client.try_execute_suspend_project(&admin, &make_str(&env, "op-004"));
        assert!(result.is_err());
    }

    #[test]
    fn test_rollback_removes_op() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        register(&env, &client, &admin);

        client.propose_suspend_project(
            &admin, &make_str(&env, "op-005"), &make_str(&env, "proj-001"), &make_str(&env, "rollback"),
        );
        client.rollback_operation(&admin, &make_str(&env, "op-005"));
        assert!(client.try_get_pending_op(&make_str(&env, "op-005")).is_err());
    }

    #[test]
    fn test_set_timelock_delay() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        client.set_timelock_delay(&admin, &3600_u64);
        assert!(client.try_set_timelock_delay(&admin, &0_u64).is_err());
    }

    #[test]
    fn test_unauthorized_propose_fails() {
        let (env, admin, oracle, verifier) = setup();
        let contract_id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]);
        register(&env, &client, &admin);
        let rogue = Address::generate(&env);
        assert!(client.try_propose_suspend_project(
            &rogue, &make_str(&env, "op-bad"), &make_str(&env, "proj-001"), &make_str(&env, "hack"),
        ).is_err());
    }

    // ── Issue 4: Arithmetic safety / boundary tests ───────────────────────────

    fn fresh(env: &Env) -> (CarbonRegistryContractClient<'static>, Address, Address) {
        let admin    = Address::generate(env);
        let oracle   = Address::generate(env);
        let verifier = Address::generate(env);
        let id = env.register_contract(None, CarbonRegistryContract);
        let client = CarbonRegistryContractClient::new(env, &id);
        client.initialize(&admin, &oracle, &vec![env, verifier.clone()]);
        (client, admin, oracle)
    }

    /// `total_credits_issued += amount` accumulating past i128::MAX must return
    /// `ArithmeticOverflow` rather than trapping.
    #[test]
    fn test_increment_issued_overflow() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, oracle) = fresh(&env);
        register(&env, &client, &admin);
        // First increment to the maximum is fine.
        client.increment_issued(&oracle, &make_str(&env, "proj-001"), &i128::MAX);
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.total_credits_issued, i128::MAX);
        // One more credit overflows the accumulator.
        let res = client.try_increment_issued(&oracle, &make_str(&env, "proj-001"), &1_i128);
        assert_eq!(res, Err(Ok(CarbonError::ArithmeticOverflow)));
    }

    /// Typical increments accumulate correctly through the checked add path.
    #[test]
    fn test_increment_issued_typical() {
        let env = Env::default();
        env.mock_all_auths();
        let (client, admin, oracle) = fresh(&env);
        register(&env, &client, &admin);
        client.increment_issued(&oracle, &make_str(&env, "proj-001"), &1_000_i128);
        client.increment_issued(&oracle, &make_str(&env, "proj-001"), &2_500_i128);
        let p = client.get_project(&make_str(&env, "proj-001"));
        assert_eq!(p.total_credits_issued, 3_500);
    }
}
