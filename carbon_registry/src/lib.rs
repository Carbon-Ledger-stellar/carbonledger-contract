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

    /// Admin suspends a project under investigation, halting new credit issuance.
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
        project.total_credits_issued += amount;
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
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]).unwrap();

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
        client.initialize(&admin, &oracle, &vec![&env, verifier.clone()]).unwrap();

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
}
