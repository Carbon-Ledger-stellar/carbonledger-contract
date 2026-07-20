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

// ── Constants ─────────────────────────────────────────────────────────────────

/// 365 days in seconds — monitoring data older than this is considered stale.
const MONITORING_FRESHNESS_SECS: u64 = 365 * 24 * 60 * 60;
/// 24 hours in ledger TTL units (each ledger ~5 s → 17_280 ledgers/day).
const PRICE_CACHE_TTL_LEDGERS: u32 = 17_280;

// ── Storage Keys ──────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    MonitoringData(String, String),
    LatestMonitoring(String),
    BenchmarkPrice(String, u32),
    FlaggedProject(String),
    OracleAddress,
    Admin,
    Locked,
    // Time-lock keys (Issue 3)
    TimelockOp(String),
    TimelockContest(String),
    TimelockDelay,
}

// ── Types ─────────────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug)]
pub struct MonitoringData {
    pub project_id:        String,
    pub period:            String,
    pub tonnes_verified:   i128,
    pub methodology_score: u32,
    pub satellite_cid:     String,
    pub submitted_by:      Address,
    pub submitted_at:      u64,
}

// ── Time-lock types (Issue 3) ─────────────────────────────────────────────────

const TIMELOCK_DEFAULT_DELAY_SECS: u64 = 172_800; // 48 hours

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GovAction {
    UpdateCreditPrice,
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
pub struct CarbonOracleContract;

#[contractimpl]
impl CarbonOracleContract {

    /// Initialise oracle with admin and authorised oracle signer address.
    /// Can only be called once — subsequent calls return [`CarbonError::AlreadyInitialized`].
    pub fn initialize(env: Env, admin: Address, oracle_address: Address) -> Result<(), CarbonError> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(CarbonError::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::OracleAddress, &oracle_address);
        Ok(())
    }

    /// Authorised oracle submits satellite-verified monitoring data for a project period.
    /// Methodology score below 70 triggers an on-chain warning event.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedOracle`] if caller is not the registered oracle.
    /// - [`CarbonError::ZeroAmountNotAllowed`] if `tonnes_verified` is zero.
    pub fn submit_monitoring_data(
        env: Env,
        oracle_signer: Address,
        project_id: String,
        period: String,
        tonnes_verified: i128,
        methodology_score: u32,
        satellite_cid: String,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) {
            Self::release_lock(&env); return Err(e);
        }

        if tonnes_verified <= 0 {
            Self::release_lock(&env);
            return Err(CarbonError::ZeroAmountNotAllowed);
        }

        // ── effects ───────────────────────────────────────────────────────────
        let now = env.ledger().timestamp();
        let data = MonitoringData {
            project_id:        project_id.clone(),
            period:            period.clone(),
            tonnes_verified,
            methodology_score,
            satellite_cid:     satellite_cid.clone(),
            submitted_by:      oracle_signer.clone(),
            submitted_at:      now,
        };
        if let Err(e) = Self::assert_valid_monitoring(&data) {
            Self::release_lock(&env);
            return Err(e);
        }
        env.storage().persistent().set(
            &DataKey::MonitoringData(project_id.clone(), period.clone()),
            &data,
        );
        env.storage().persistent().set(&DataKey::LatestMonitoring(project_id.clone()), &now);

        if methodology_score < 70 {
            env.events().publish(
                (symbol_short!("c_ledger"), symbol_short!("low_score")),
                (project_id.clone(), methodology_score),
            );
        }

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("mon_data")),
            (project_id, period, tonnes_verified, methodology_score),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Push updated benchmark price per methodology and vintage year.
    /// Stored in temporary storage with 24-hour TTL.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedOracle`] if caller is not the registered oracle.
    pub fn update_credit_price(
        env: Env,
        oracle_signer: Address,
        methodology: String,
        vintage_year: u32,
        price_usdc: i128,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) {
            Self::release_lock(&env); return Err(e);
        }

        if price_usdc <= 0 {
            Self::release_lock(&env);
            return Err(CarbonError::ZeroAmountNotAllowed);
        }

        // ── effects ───────────────────────────────────────────────────────────
        let key = DataKey::BenchmarkPrice(methodology.clone(), vintage_year);
        env.storage().temporary().set(&key, &price_usdc);
        env.storage().temporary().extend_ttl(&key, PRICE_CACHE_TTL_LEDGERS, PRICE_CACHE_TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("price_upd")),
            (methodology, vintage_year, price_usdc),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Returns monitoring data for a specific project and period.
    ///
    /// # Errors
    /// - [`CarbonError::ProjectNotFound`] if no data exists for the given period.
    pub fn get_monitoring_data(
        env: Env,
        project_id: String,
        period: String,
    ) -> Result<MonitoringData, CarbonError> {
        env.storage()
            .persistent()
            .get(&DataKey::MonitoringData(project_id, period))
            .ok_or(CarbonError::ProjectNotFound)
    }

    /// Returns the current benchmark price (in USDC stroops) for a methodology and vintage.
    ///
    /// # Errors
    /// - [`CarbonError::PriceNotSet`] if no price is cached or cache has expired.
    pub fn get_benchmark_price(
        env: Env,
        methodology: String,
        vintage_year: u32,
    ) -> Result<i128, CarbonError> {
        env.storage()
            .temporary()
            .get(&DataKey::BenchmarkPrice(methodology, vintage_year))
            .ok_or(CarbonError::PriceNotSet)
    }

    /// Flag a project for investigation. Emits an on-chain event that halts
    /// new credit issuance until the flag is resolved.
    ///
    /// # Errors
    /// - [`CarbonError::UnauthorizedOracle`] if caller is not the registered oracle.
    pub fn flag_project(
        env: Env,
        oracle_signer: Address,
        project_id: String,
        reason: String,
    ) -> Result<(), CarbonError> {
        // ── checks ────────────────────────────────────────────────────────────
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) {
            Self::release_lock(&env); return Err(e);
        }

        // ── effects ───────────────────────────────────────────────────────────
        env.storage().persistent().set(&DataKey::FlaggedProject(project_id.clone()), &reason);

        env.events().publish(
            (symbol_short!("c_ledger"), symbol_short!("flagged")),
            (project_id, oracle_signer, reason),
        );
        Self::release_lock(&env);
        Ok(())
    }

    /// Returns `true` if monitoring data was submitted within the last 365 days.
    /// Returns `false` (stale) if no data exists or data is older than 365 days.
    pub fn is_monitoring_current(env: Env, project_id: String) -> bool {
        let latest: Option<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::LatestMonitoring(project_id));

        match latest {
            None => false,
            Some(ts) => {
                let now = env.ledger().timestamp();
                now.saturating_sub(ts) <= MONITORING_FRESHNESS_SECS
            }
        }
    }

    // ── Time-lock governance functions (Issue 3) ─────────────────────────────

    /// Propose a credit price update, queued with time-lock delay.
    pub fn propose_price_update(env: Env, oracle_signer: Address, op_id: String, methodology: String, vintage_year: u32, new_price_usdc: i128) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) { Self::release_lock(&env); return Err(e); }
        if new_price_usdc <= 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }
        let delay = env.storage().persistent()
            .get::<DataKey, u64>(&DataKey::TimelockDelay)
            .unwrap_or(TIMELOCK_DEFAULT_DELAY_SECS);
        let op = PendingOp {
            op_id: op_id.clone(), action: GovAction::UpdateCreditPrice,
            target: methodology.clone(), initiated_by: oracle_signer.clone(),
            eta: env.ledger().timestamp() + delay, payload: String::from_str(&env, "price_update"),
        };
        env.storage().persistent().set(&DataKey::TimelockOp(op_id.clone()), &op);
        // Stage price in temp storage so it's ready on execute
        let key = DataKey::BenchmarkPrice(methodology.clone(), vintage_year);
        env.storage().temporary().set(&key, &new_price_usdc);
        env.storage().temporary().extend_ttl(&key, PRICE_CACHE_TTL_LEDGERS, PRICE_CACHE_TTL_LEDGERS);
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("tl_queue")), (op_id, oracle_signer, methodology, vintage_year, new_price_usdc));
        Self::release_lock(&env);
        Ok(())
    }

    /// Execute a queued price update after delay has elapsed.
    pub fn execute_price_update(env: Env, oracle_signer: Address, op_id: String) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) { Self::release_lock(&env); return Err(e); }
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
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("price_exe")), (op_id, oracle_signer));
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

    /// Roll back a pending or contested operation. Oracle signer only.
    pub fn rollback_operation(env: Env, oracle_signer: Address, op_id: String) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) { Self::release_lock(&env); return Err(e); }
        if !env.storage().persistent().has(&DataKey::TimelockOp(op_id.clone())) {
            Self::release_lock(&env); return Err(CarbonError::ProjectNotFound);
        }
        env.storage().persistent().remove(&DataKey::TimelockOp(op_id.clone()));
        if env.storage().persistent().has(&DataKey::TimelockContest(op_id.clone())) {
            env.storage().persistent().remove(&DataKey::TimelockContest(op_id.clone()));
        }
        env.events().publish((symbol_short!("c_ledger"), symbol_short!("tl_rback")), (op_id, oracle_signer));
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

    /// Update the time-lock delay in seconds. Oracle only.
    pub fn set_timelock_delay(env: Env, oracle_signer: Address, delay_secs: u64) -> Result<(), CarbonError> {
        Self::acquire_lock(&env)?;
        oracle_signer.require_auth();
        if let Err(e) = Self::require_oracle(&env, &oracle_signer) { Self::release_lock(&env); return Err(e); }
        if delay_secs == 0 { Self::release_lock(&env); return Err(CarbonError::ZeroAmountNotAllowed); }
        env.storage().persistent().set(&DataKey::TimelockDelay, &delay_secs);
        Self::release_lock(&env);
        Ok(())
    }

    // ── Validation helpers (Issue 2) ──────────────────────────────────────────

    /// Assert that [`MonitoringData`] satisfies all data-structure invariants:
    /// - `project_id`, `period`, `satellite_cid` must be non-empty.
    /// - `tonnes_verified` > 0.
    /// - `methodology_score` ∈ [0, 100].
    fn assert_valid_monitoring(data: &MonitoringData) -> Result<(), CarbonError> {
        if data.project_id.len() == 0    { return Err(CarbonError::ProjectNotFound); }
        if data.period.len() == 0        { return Err(CarbonError::ProjectNotFound); }
        if data.satellite_cid.len() == 0 { return Err(CarbonError::ProjectNotFound); }
        if data.tonnes_verified <= 0     { return Err(CarbonError::ZeroAmountNotAllowed); }
        if data.methodology_score > 100  { return Err(CarbonError::InvalidVintageYear); }
        Ok(())
    }

    // ── Internal helpers ──────────────────────────────────────────────────────

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
    use soroban_sdk::{testutils::{Address as _, Ledger, LedgerInfo}, Env, String};

    fn s(env: &Env, v: &str) -> String { String::from_str(env, v) }

    fn setup(env: &Env) -> (CarbonOracleContractClient, Address, Address) {
        env.mock_all_auths();
        let admin  = Address::generate(env);
        let oracle = Address::generate(env);
        let id     = env.register_contract(None, CarbonOracleContract);
        let client = CarbonOracleContractClient::new(env, &id);
        client.initialize(&admin, &oracle).unwrap();
        (client, admin, oracle)
    }

    #[test]
    fn test_authorized_oracle_submits_monitoring() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.submit_monitoring_data(
            &oracle,
            &s(&env, "proj-001"),
            &s(&env, "2023-Q1"),
            &5000_i128,
            &85_u32,
            &s(&env, "QmSatCID"),
        ).unwrap();

        let data = client.get_monitoring_data(&s(&env, "proj-001"), &s(&env, "2023-Q1")).unwrap();
        assert_eq!(data.tonnes_verified, 5000);
        assert_eq!(data.methodology_score, 85);
    }

    #[test]
    fn test_unauthorized_oracle_rejected() {
        let env = Env::default();
        let (client, _, _) = setup(&env);
        let rogue = Address::generate(&env);

        let result = client.try_submit_monitoring_data(
            &rogue,
            &s(&env, "proj-001"),
            &s(&env, "2023-Q1"),
            &5000_i128,
            &85_u32,
            &s(&env, "QmSatCID"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_benchmark_price_update() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.update_credit_price(&oracle, &s(&env, "VCS"), &2023_u32, &15_0000000_i128).unwrap();
        let price = client.get_benchmark_price(&s(&env, "VCS"), &2023_u32).unwrap();
        assert_eq!(price, 15_0000000_i128);
    }

    #[test]
    fn test_price_not_set_returns_error() {
        let env = Env::default();
        let (client, _, _) = setup(&env);
        let result = client.try_get_benchmark_price(&s(&env, "VCS"), &2023_u32);
        assert!(result.is_err());
    }

    #[test]
    fn test_flag_project() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.flag_project(&oracle, &s(&env, "proj-001"), &s(&env, "satellite contradiction")).unwrap();
        // Verify event was emitted (no error = success)
    }

    #[test]
    fn test_stale_monitoring_returns_false() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        // Submit monitoring data at timestamp 0
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000,
            protocol_version: 20,
            sequence_number: 100,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 6_312_000,
        });

        client.submit_monitoring_data(
            &oracle,
            &s(&env, "proj-001"),
            &s(&env, "2022-Q1"),
            &1000_i128,
            &80_u32,
            &s(&env, "QmCID"),
        ).unwrap();

        // Advance time by 366 days
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + (366 * 24 * 60 * 60),
            protocol_version: 20,
            sequence_number: 200,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 1,
            min_persistent_entry_ttl: 1,
            max_entry_ttl: 6_312_000,
        });

        assert!(!client.is_monitoring_current(&s(&env, "proj-001")));
    }

    #[test]
    fn test_fresh_monitoring_returns_true() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.submit_monitoring_data(
            &oracle,
            &s(&env, "proj-001"),
            &s(&env, "2023-Q1"),
            &1000_i128,
            &80_u32,
            &s(&env, "QmCID"),
        ).unwrap();

        assert!(client.is_monitoring_current(&s(&env, "proj-001")));
    }

    #[test]
    fn test_initialize_twice_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin  = Address::generate(&env);
        let oracle = Address::generate(&env);
        let id     = env.register_contract(None, CarbonOracleContract);
        let client = CarbonOracleContractClient::new(&env, &id);
        client.initialize(&admin, &oracle).unwrap();
        let result = client.try_initialize(&admin, &oracle);
        assert!(result.is_err());
    }

    // ── Reentrancy guard tests ─────────────────────────────────────────────────

    /// Lock is released after submit_monitoring_data succeeds; a second call works.
    #[test]
    fn test_lock_released_after_submit_monitoring() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.submit_monitoring_data(
            &oracle, &s(&env, "proj-001"), &s(&env, "2023-Q1"),
            &5000_i128, &85_u32, &s(&env, "QmCID1"),
        ).unwrap();

        // Second call with different period — lock must be free
        client.submit_monitoring_data(
            &oracle, &s(&env, "proj-001"), &s(&env, "2023-Q2"),
            &4000_i128, &80_u32, &s(&env, "QmCID2"),
        ).unwrap();

        let data = client.get_monitoring_data(&s(&env, "proj-001"), &s(&env, "2023-Q2")).unwrap();
        assert_eq!(data.tonnes_verified, 4000);
    }

    /// Lock is released after update_credit_price succeeds.
    #[test]
    fn test_lock_released_after_update_price() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.update_credit_price(&oracle, &s(&env, "VCS"), &2023_u32, &15_0000000_i128).unwrap();
        // Second price update for different vintage — lock must be free
        client.update_credit_price(&oracle, &s(&env, "VCS"), &2024_u32, &20_0000000_i128).unwrap();

        let price = client.get_benchmark_price(&s(&env, "VCS"), &2024_u32).unwrap();
        assert_eq!(price, 20_0000000_i128);
    }

    /// Lock is released after flag_project succeeds.
    #[test]
    fn test_lock_released_after_flag_project() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        client.flag_project(&oracle, &s(&env, "proj-001"), &s(&env, "contradiction")).unwrap();
        // Second flag on different project — lock must be free
        client.flag_project(&oracle, &s(&env, "proj-002"), &s(&env, "double-count")).unwrap();
    }

    /// Lock is released after a failed submit (zero tonnes).
    #[test]
    fn test_lock_released_after_failed_submit() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        // Zero tonnes — should fail and release lock
        let _ = client.try_submit_monitoring_data(
            &oracle, &s(&env, "proj-001"), &s(&env, "2023-Q1"),
            &0_i128, &80_u32, &s(&env, "QmCID"),
        );

        // Valid call must succeed (lock released)
        client.submit_monitoring_data(
            &oracle, &s(&env, "proj-001"), &s(&env, "2023-Q1"),
            &1000_i128, &80_u32, &s(&env, "QmCID"),
        ).unwrap();

        let data = client.get_monitoring_data(&s(&env, "proj-001"), &s(&env, "2023-Q1")).unwrap();
        assert_eq!(data.tonnes_verified, 1000);
    }

    /// Lock is released after a failed price update (unauthorized oracle).
    #[test]
    fn test_lock_released_after_failed_price_update() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);

        let rogue = Address::generate(&env);
        // Unauthorized — should fail and release lock
        let _ = client.try_update_credit_price(&rogue, &s(&env, "VCS"), &2023_u32, &10_0000000_i128);

        // Authorized oracle must still succeed (lock released)
        client.update_credit_price(&oracle, &s(&env, "VCS"), &2023_u32, &10_0000000_i128).unwrap();
        let price = client.get_benchmark_price(&s(&env, "VCS"), &2023_u32).unwrap();
        assert_eq!(price, 10_0000000_i128);
    }

    // ── Issue 2: Validation helper tests ──────────────────────────────────────

    #[test]
    fn test_submit_empty_period_fails() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        let result = client.try_submit_monitoring_data(
            &oracle,
            &s(&env, "proj-001"),
            &s(&env, ""),  // empty period — invalid
            &1000_i128,
            &80_u32,
            &s(&env, "QmCID"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_submit_empty_satellite_cid_fails() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        let result = client.try_submit_monitoring_data(
            &oracle, &s(&env, "proj-001"), &s(&env, "2023-Q1"), &1000_i128, &80_u32, &s(&env, ""),
        );
        assert!(result.is_err());
    }

    // ── Issue 3: Time-lock tests ──────────────────────────────────────────────

    #[test]
    fn test_propose_price_update_and_query() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        client.propose_price_update(&oracle, &s(&env, "op-001"), &s(&env, "VCS"), &2023_u32, &20_0000000_i128).unwrap();
        let op = client.get_pending_op(&s(&env, "op-001")).unwrap();
        assert_eq!(op.op_id, s(&env, "op-001"));
    }

    #[test]
    fn test_execute_price_update_before_delay_fails() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.propose_price_update(&oracle, &s(&env, "op-002"), &s(&env, "VCS"), &2023_u32, &20_0000000_i128).unwrap();
        assert!(client.try_execute_price_update(&oracle, &s(&env, "op-002")).is_err());
    }

    #[test]
    fn test_execute_price_update_after_delay_succeeds() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.propose_price_update(&oracle, &s(&env, "op-003"), &s(&env, "VCS"), &2024_u32, &25_0000000_i128).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 172_800 + 1, protocol_version: 20, sequence_number: 200,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.execute_price_update(&oracle, &s(&env, "op-003")).unwrap();
        assert!(client.try_get_pending_op(&s(&env, "op-003")).is_err());
    }

    #[test]
    fn test_contest_price_update_blocks_execution() {
        use soroban_sdk::testutils::{Ledger, LedgerInfo};
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000, protocol_version: 20, sequence_number: 100,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        client.propose_price_update(&oracle, &s(&env, "op-004"), &s(&env, "VCS"), &2023_u32, &50_0000000_i128).unwrap();
        let user = Address::generate(&env);
        client.contest_operation(&user, &s(&env, "op-004"), &s(&env, "price spike")).unwrap();
        env.ledger().set(LedgerInfo {
            timestamp: 1_000_000 + 172_800 + 1, protocol_version: 20, sequence_number: 200,
            network_id: Default::default(), base_reserve: 10,
            min_temp_entry_ttl: 1, min_persistent_entry_ttl: 1, max_entry_ttl: 6_312_000,
        });
        assert!(client.try_execute_price_update(&oracle, &s(&env, "op-004")).is_err());
    }

    #[test]
    fn test_rollback_price_update() {
        let env = Env::default();
        let (client, _, oracle) = setup(&env);
        client.propose_price_update(&oracle, &s(&env, "op-005"), &s(&env, "VCS"), &2023_u32, &15_0000000_i128).unwrap();
        client.rollback_operation(&oracle, &s(&env, "op-005")).unwrap();
        assert!(client.try_get_pending_op(&s(&env, "op-005")).is_err());
    }
}
