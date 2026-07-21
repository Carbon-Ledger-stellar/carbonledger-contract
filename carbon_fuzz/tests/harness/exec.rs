//! Executor: wires up all four contracts plus the USDC token and drives one
//! operation at a time against both the real contracts and the shadow model,
//! asserting they agree.
//!
//! Throughput note: constructing a fresh `Env` per logical sequence is far too
//! slow for a 50k-sequence campaign, and a single `Env` cannot absorb unbounded
//! invocations (the test host accumulates auth/event state and eventually
//! aborts). So one `World` is *reused across a batch of sequences*, with every
//! id namespaced by sequence index (see `ops`) to keep them independent. The
//! caller tears the world down and builds a fresh one every `WORLD_BATCH`
//! sequences to bound both effects.

use carbon_credit::{
    CarbonCreditContract, CarbonCreditContractClient, CreditStatus as OnCreditStatus,
};
use carbon_marketplace::{
    CarbonMarketplaceContract, CarbonMarketplaceContractClient, ListingStatus as OnListingStatus,
};
use carbon_oracle::{CarbonOracleContract, CarbonOracleContractClient};
use carbon_registry::{
    CarbonRegistryContract, CarbonRegistryContractClient, ProjectStatus as OnProjectStatus,
};
use soroban_sdk::{
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    vec as svec, Address, Env, String as SString,
};

use crate::model::{
    CreditStatus, Fault, LedgerModel, ListingStatus, Predicted, ProjectStatus,
};
use crate::ops::{
    batch_id, listing_id, methodology, period_id, project_id, retirement_id, Op, ADMIN,
    FIRST_TRADER, N_ACTORS, ORACLE, VERIFIER,
};

/// USDC seeded to each trader. Far larger than any sequence of purchases can
/// spend, so a `token::transfer` never traps on insufficient balance — keeping
/// purchase outcomes a pure function of the marketplace's own checks.
const FUND: i128 = 1_000_000_000_000_000;

/// Sequences per `World` before it is torn down and rebuilt. Bounds both the
/// per-`Env` invocation count and the growth of the contracts' global lists
/// (`SerialRegistry`, `AllListings`), which are re-serialized in full on writes.
pub const WORLD_BATCH: u32 = 32;

/// Translates a contract error (any of the four identical `CarbonError` enums)
/// into the model's vocabulary. A variant the generator cannot reach panics
/// loudly rather than silently widening the comparison. A leaked
/// `ReentrancyGuard` at top level means a prior op returned without releasing
/// its lock — exactly the latent bug the guard exists to prevent.
macro_rules! fault_from {
    ($err:expr, $ty:ty) => {{
        use $ty as E;
        match $err {
            E::ZeroAmountNotAllowed => Fault::ZeroAmountNotAllowed,
            E::InvalidSerialRange => Fault::InvalidSerialRange,
            E::InvalidVintageYear => Fault::InvalidVintageYear,
            E::SerialNumberConflict => Fault::SerialNumberConflict,
            E::DoubleCountingDetected => Fault::DoubleCountingDetected,
            E::AlreadyRetired => Fault::AlreadyRetired,
            E::InsufficientCredits => Fault::InsufficientCredits,
            E::InsufficientLiquidity => Fault::InsufficientLiquidity,
            E::ListingNotFound => Fault::ListingNotFound,
            E::ProjectNotFound => Fault::ProjectNotFound,
            E::ProjectAlreadyExists => Fault::ProjectAlreadyExists,
            E::UnauthorizedVerifier => Fault::UnauthorizedVerifier,
            E::UnauthorizedOracle => Fault::UnauthorizedOracle,
            E::ReentrancyGuard => {
                panic!("ReentrancyGuard observed at top level — a prior op leaked the lock")
            }
            other => panic!("unreachable contract error from generated ops: {:?}", other),
        }
    }};
}

/// Normalises a `try_*` result into `Result<(), Fault>`. `Err(Err(_))` is a host
/// trap (panic/overflow/budget), categorically different from a returned error.
macro_rules! outcome {
    ($call:expr, $ty:ty, $what:expr) => {
        match $call {
            Ok(_) => Ok(()),
            Err(Ok(e)) => Err(fault_from!(e, $ty)),
            Err(Err(trap)) => panic!("host trap during {}: {:?}", $what, trap),
        }
    };
}

pub struct World<'a> {
    pub env: Env,
    pub actors: Vec<Address>,
    pub usdc: Address,
    pub registry: CarbonRegistryContractClient<'a>,
    pub credit: CarbonCreditContractClient<'a>,
    pub market: CarbonMarketplaceContractClient<'a>,
    pub oracle: CarbonOracleContractClient<'a>,
    pub model: LedgerModel,
}

impl<'a> World<'a> {
    pub fn new() -> World<'a> {
        let env = Env::default();
        env.mock_all_auths();

        let actors: Vec<Address> = (0..N_ACTORS).map(|_| Address::generate(&env)).collect();
        let admin = actors[ADMIN].clone();
        let verifier = actors[VERIFIER].clone();
        let oracle_signer = actors[ORACLE].clone();

        // USDC token, with the traders funded so purchases settle without trapping.
        let usdc = env.register_stellar_asset_contract(admin.clone());
        let minter = StellarAssetClient::new(&env, &usdc);
        let funded: Vec<usize> = (FIRST_TRADER..N_ACTORS).collect();
        for &t in &funded {
            minter.mint(&actors[t], &FUND);
        }

        let registry = CarbonRegistryContractClient::new(
            &env,
            &env.register_contract(None, CarbonRegistryContract),
        );
        registry.initialize(&admin, &oracle_signer, &svec![&env, verifier.clone()]);

        let credit = CarbonCreditContractClient::new(
            &env,
            &env.register_contract(None, CarbonCreditContract),
        );
        // carbon_credit stores a registry address but never reads it.
        credit.initialize(&admin, &registry.address);

        let market = CarbonMarketplaceContractClient::new(
            &env,
            &env.register_contract(None, CarbonMarketplaceContract),
        );
        market.initialize(&admin, &usdc);

        let oracle = CarbonOracleContractClient::new(
            &env,
            &env.register_contract(None, CarbonOracleContract),
        );
        oracle.initialize(&admin, &oracle_signer);

        let model = LedgerModel::new(&funded, FUND);

        World {
            env,
            actors,
            usdc,
            registry,
            credit,
            market,
            oracle,
            model,
        }
    }

    fn s(&self, v: &str) -> SString {
        SString::from_str(&self.env, v)
    }

    /// Executes one operation against contract and model, asserting agreement.
    /// This is the whole per-op cost in the high-volume campaign: exactly the
    /// one invocation the operation itself requires, plus model bookkeeping.
    pub fn step(&mut self, op: &Op) {
        // Each op is its own transaction on-chain, so reset to the default
        // budget — reset to *default*, not unlimited, so a single op exhausting
        // its budget (e.g. a huge retirement building a Vec<u64> per credit) is
        // still detectable as a trap.
        self.env.budget().reset_default();

        match op {
            Op::RegisterProject { seq, actor, project, vintage_year } => {
                self.step_register(*seq, *actor, *project, *vintage_year)
            }
            Op::VerifyProject { seq, actor, project } => {
                self.step_verify(*seq, *actor, *project)
            }
            Op::RejectProject { seq, actor, project } => {
                self.step_reject(*seq, *actor, *project)
            }
            Op::SuspendProject { seq, actor, project } => {
                self.step_suspend(*seq, *actor, *project)
            }
            Op::SubmitMonitoring { seq, actor, project, period, tonnes, score } => {
                self.step_monitor(*seq, *actor, *project, *period, *tonnes, *score)
            }
            Op::UpdatePrice { actor, methodology: m, vintage_year, price } => {
                self.step_price(*actor, *m, *vintage_year, *price)
            }
            Op::FlagProject { seq, actor, project } => self.step_flag(*seq, *actor, *project),
            Op::Mint { seq, actor, project, batch, amount, vintage_year, serial_start, serial_end } => {
                self.step_mint(*seq, *actor, *project, *batch, *amount, *vintage_year, *serial_start, *serial_end)
            }
            Op::Retire { seq, actor, batch, amount, retirement } => {
                self.step_retire(*seq, *actor, *batch, *amount, *retirement)
            }
            Op::Transfer { seq, from, to, batch, amount } => {
                self.step_transfer(*seq, *from, *to, *batch, *amount)
            }
            Op::ListCredits { seq, seller, listing, batch, project, amount, price, vintage_year, methodology: m } => {
                self.step_list(*seq, *seller, *listing, *batch, *project, *amount, *price, *vintage_year, *m)
            }
            Op::DelistCredits { actor, listing } => self.step_delist(*actor, *listing),
            Op::Purchase { buyer, listing, amount } => self.step_purchase(*buyer, *listing, *amount),
        }
    }

    // ── Registry ──────────────────────────────────────────────────────────────

    fn step_register(&mut self, seq: u32, actor: usize, project: usize, vintage: u32) {
        let pid = project_id(seq, project);
        let predicted = self
            .model
            .registry
            .predict_register(actor == ADMIN, &pid, vintage);
        let actual = outcome!(
            self.registry.try_register_project(
                &self.actors[actor],
                &self.s(&pid),
                &self.s("name"),
                &self.s("cid"),
                &self.actors[VERIFIER],
                &self.s("VCS"),
                &self.s("Brazil"),
                &self.s("forestry"),
                &vintage,
            ),
            carbon_registry::CarbonError,
            "register_project"
        );
        assert_outcome(&predicted, &actual, "register", &pid);
        if predicted.is_ok() {
            self.model.registry.apply_register(&pid);
        }
    }

    fn step_verify(&mut self, seq: u32, actor: usize, project: usize) {
        let pid = project_id(seq, project);
        let predicted = self
            .model
            .registry
            .predict_verifier_action(actor == VERIFIER, &pid);
        let actual = outcome!(
            self.registry
                .try_verify_project(&self.actors[actor], &self.s(&pid)),
            carbon_registry::CarbonError,
            "verify_project"
        );
        assert_outcome(&predicted, &actual, "verify", &pid);
        if predicted.is_ok() {
            self.model.registry.set_status(&pid, ProjectStatus::Verified);
        }
    }

    fn step_reject(&mut self, seq: u32, actor: usize, project: usize) {
        let pid = project_id(seq, project);
        let predicted = self
            .model
            .registry
            .predict_verifier_action(actor == VERIFIER, &pid);
        let actual = outcome!(
            self.registry.try_reject_project(
                &self.actors[actor],
                &self.s(&pid),
                &self.s("fraud"),
            ),
            carbon_registry::CarbonError,
            "reject_project"
        );
        assert_outcome(&predicted, &actual, "reject", &pid);
        if predicted.is_ok() {
            self.model.registry.set_status(&pid, ProjectStatus::Rejected);
        }
    }

    fn step_suspend(&mut self, seq: u32, actor: usize, project: usize) {
        let pid = project_id(seq, project);
        let predicted = self
            .model
            .registry
            .predict_admin_action(actor == ADMIN, &pid);
        let actual = outcome!(
            self.registry.try_suspend_project(
                &self.actors[actor],
                &self.s(&pid),
                &self.s("investigation"),
            ),
            carbon_registry::CarbonError,
            "suspend_project"
        );
        assert_outcome(&predicted, &actual, "suspend", &pid);
        if predicted.is_ok() {
            self.model.registry.set_status(&pid, ProjectStatus::Suspended);
        }
    }

    // ── Oracle ────────────────────────────────────────────────────────────────

    fn step_monitor(&mut self, seq: u32, actor: usize, project: usize, period: u32, tonnes: i128, score: u32) {
        let pid = project_id(seq, project);
        let predicted = self.model.oracle.predict_submit(actor == ORACLE, tonnes);
        let actual = outcome!(
            self.oracle.try_submit_monitoring_data(
                &self.actors[actor],
                &self.s(&pid),
                &self.s(&period_id(seq, period)),
                &tonnes,
                &score,
                &self.s("sat-cid"),
            ),
            carbon_oracle::CarbonError,
            "submit_monitoring_data"
        );
        assert_outcome(&predicted, &actual, "monitoring", &pid);
        if predicted.is_ok() {
            self.model.oracle.monitored.insert(pid);
        }
    }

    fn step_price(&mut self, actor: usize, m: usize, vintage: u32, price: i128) {
        let predicted = self.model.oracle.predict_price(actor == ORACLE, price);
        let actual = outcome!(
            self.oracle.try_update_credit_price(
                &self.actors[actor],
                &self.s(methodology(m)),
                &vintage,
                &price,
            ),
            carbon_oracle::CarbonError,
            "update_credit_price"
        );
        assert_outcome(&predicted, &actual, "price", methodology(m));
        if predicted.is_ok() {
            self.model
                .oracle
                .priced
                .insert((methodology(m).to_string(), vintage));
        }
    }

    fn step_flag(&mut self, seq: u32, actor: usize, project: usize) {
        let pid = project_id(seq, project);
        let predicted = self.model.oracle.predict_flag(actor == ORACLE);
        let actual = outcome!(
            self.oracle.try_flag_project(
                &self.actors[actor],
                &self.s(&pid),
                &self.s("contradiction"),
            ),
            carbon_oracle::CarbonError,
            "flag_project"
        );
        assert_outcome(&predicted, &actual, "flag", &pid);
    }

    // ── Credit ────────────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn step_mint(&mut self, seq: u32, actor: usize, project: usize, batch: usize, amount: i128, vintage: u32, serial_start: u64, serial_end: u64) {
        let pid = project_id(seq, project);
        let bid = batch_id(seq, batch);
        let predicted = self
            .model
            .credit
            .predict_mint(actor == ADMIN, &bid, amount, vintage, serial_start, serial_end);
        let actual = outcome!(
            self.credit.try_mint_credits(
                &self.actors[actor],
                &self.s(&pid),
                &amount,
                &vintage,
                &self.s(&bid),
                &serial_start,
                &serial_end,
                &self.s("cid"),
            ),
            carbon_credit::CarbonError,
            "mint_credits"
        );
        assert_outcome(&predicted, &actual, "mint", &bid);
        if predicted.is_ok() {
            self.model
                .credit
                .apply_mint(&bid, &pid, amount, vintage, serial_start, serial_end);
        }
    }

    fn step_retire(&mut self, seq: u32, actor: usize, batch: usize, amount: i128, retirement: u32) {
        let bid = batch_id(seq, batch);
        let rid = retirement_id(retirement);
        let predicted = self.model.credit.predict_retire(&bid, amount);

        // Retire returns the certificate on success, so we validate its serial
        // span here for free — no extra getter invocation.
        let result = self.credit.try_retire_credits(
            &self.actors[actor],
            &self.s(&bid),
            &amount,
            &self.s("reason"),
            &self.s("beneficiary"),
            &self.s(&rid),
            &self.s("txhash"),
        );
        let actual: Result<(), Fault> = match &result {
            Ok(_) => Ok(()),
            Err(Ok(e)) => Err(fault_from!(*e, carbon_credit::CarbonError)),
            Err(Err(trap)) => panic!("host trap during retire_credits: {:?}", trap),
        };
        assert_outcome(&predicted, &actual, "retire", &bid);

        if predicted.is_ok() {
            let (lo, hi) = self.model.credit.apply_retire(&bid, &rid, amount);
            let cert = result.unwrap().unwrap();
            assert_eq!(cert.amount, amount, "cert {rid}: wrong amount");
            assert_eq!(
                cert.serial_numbers.len() as i128,
                amount,
                "cert {rid}: expected one serial per credit"
            );
            assert_eq!(
                cert.serial_numbers.first().unwrap(),
                lo,
                "cert {rid}: first serial diverged from model"
            );
            assert_eq!(
                cert.serial_numbers.last().unwrap(),
                hi,
                "cert {rid}: last serial diverged from model"
            );
        }
    }

    fn step_transfer(&mut self, seq: u32, from: usize, to: usize, batch: usize, amount: i128) {
        let bid = batch_id(seq, batch);
        let predicted = self.model.credit.predict_transfer(&bid, amount);
        let actual = outcome!(
            self.credit
                .try_transfer_credits(&self.actors[from], &self.actors[to], &self.s(&bid), &amount),
            carbon_credit::CarbonError,
            "transfer_credits"
        );
        assert_outcome(&predicted, &actual, "transfer", &bid);
        // A successful transfer writes no storage, so the model is unchanged.
    }

    // ── Marketplace ──────────────────────────────────────────────────────────

    #[allow(clippy::too_many_arguments)]
    fn step_list(&mut self, seq: u32, seller: usize, listing: u32, batch: usize, project: usize, amount: i128, price: i128, vintage: u32, m: usize) {
        let lid = listing_id(listing);
        let predicted = self.model.market.predict_list(amount, price);
        let actual = outcome!(
            self.market.try_list_credits(
                &self.actors[seller],
                &self.s(&lid),
                &self.s(&batch_id(seq, batch)),
                &self.s(&project_id(seq, project)),
                &amount,
                &price,
                &vintage,
                &self.s(methodology(m)),
                &self.s("Brazil"),
            ),
            carbon_marketplace::CarbonError,
            "list_credits"
        );
        assert_outcome(&predicted, &actual, "list", &lid);
        if predicted.is_ok() {
            self.model
                .market
                .apply_list(&lid, seller, &batch_id(seq, batch), amount, price);
        }
    }

    fn step_delist(&mut self, actor: usize, listing: u32) {
        let lid = listing_id(listing);
        let predicted = self.model.market.predict_delist(&lid, actor);
        let actual = outcome!(
            self.market
                .try_delist_credits(&self.actors[actor], &self.s(&lid)),
            carbon_marketplace::CarbonError,
            "delist_credits"
        );
        assert_outcome(&predicted, &actual, "delist", &lid);
        if predicted.is_ok() {
            self.model.market.apply_delist(&lid);
        }
    }

    fn step_purchase(&mut self, buyer: usize, listing: u32, amount: i128) {
        let lid = listing_id(listing);
        let predicted = self.model.market.predict_purchase(&lid, amount);
        let actual = outcome!(
            self.market
                .try_purchase_credits(&self.actors[buyer], &self.s(&lid), &amount),
            carbon_marketplace::CarbonError,
            "purchase_credits"
        );
        assert_outcome(&predicted, &actual, "purchase", &lid);
        if predicted.is_ok() {
            let (seller, proceeds, fee) = self.model.market.apply_purchase(&lid, amount);
            self.model.credit_usdc(buyer, -(proceeds + fee));
            self.model.credit_usdc(seller, proceeds);
            self.model.credit_usdc(ADMIN, fee);
        }
    }

    // ── Deep reconciliation ────────────────────────────────────────────────────
    // Re-reads contract state and compares it to the model. Every getter is a
    // full invocation, so this is *not* run in the high-volume campaign; the
    // thorough campaign calls it once per world, which amortizes over WORLD_BATCH
    // sequences. It samples rather than scanning everything so its cost stays
    // bounded no matter how much state the world has accumulated.

    pub fn deep_reconcile(&self) {
        self.reconcile_projects();
        self.reconcile_batches();
        self.reconcile_listings();
        self.reconcile_usdc();
        self.reconcile_serial_registry();
    }

    fn reconcile_projects(&self) {
        for (pid, status) in &self.model.registry.projects {
            let on = self.registry.get_project(&self.s(pid));
            let expected = match status {
                ProjectStatus::Pending => OnProjectStatus::Pending,
                ProjectStatus::Verified => OnProjectStatus::Verified,
                ProjectStatus::Rejected => OnProjectStatus::Rejected,
                ProjectStatus::Suspended => OnProjectStatus::Suspended,
                ProjectStatus::Completed => OnProjectStatus::Completed,
            };
            assert_eq!(on.status, expected, "project {pid}: status diverged");
        }
    }

    fn reconcile_batches(&self) {
        for (bid, batch) in &self.model.credit.batches {
            let on = self.credit.get_credit_batch(&self.s(bid));
            assert_eq!(on.amount, batch.amount, "batch {bid}: amount diverged");
            assert_eq!(on.vintage_year, batch.vintage_year, "batch {bid}: vintage_year diverged");
            assert_eq!(on.serial_start, batch.serial_start, "batch {bid}: serial_start diverged");
            assert_eq!(on.serial_end, batch.serial_end, "batch {bid}: serial_end diverged");
            let expected = match batch.status() {
                CreditStatus::Active => OnCreditStatus::Active,
                CreditStatus::PartiallyRetired => OnCreditStatus::PartiallyRetired,
                CreditStatus::FullyRetired => OnCreditStatus::FullyRetired,
            };
            assert_eq!(on.status, expected, "batch {bid}: status diverged");
        }
    }

    fn reconcile_listings(&self) {
        for (lid, listing) in &self.model.market.listings {
            let on = self.market.get_listing(&self.s(lid));
            assert_eq!(
                on.amount_available, listing.amount_available,
                "listing {lid}: amount_available diverged"
            );
            let expected = match listing.status {
                ListingStatus::Active => OnListingStatus::Active,
                ListingStatus::Sold => OnListingStatus::Sold,
                ListingStatus::PartiallyFilled => OnListingStatus::PartiallyFilled,
                ListingStatus::Delisted => OnListingStatus::Delisted,
            };
            assert_eq!(on.status, expected, "listing {lid}: status diverged");
        }
    }

    fn reconcile_usdc(&self) {
        let token = TokenClient::new(&self.env, &self.usdc);
        for (&actor, &expected) in &self.model.usdc {
            let on = token.balance(&self.actors[actor]);
            assert_eq!(on, expected, "actor {actor}: USDC balance diverged");
        }
        // The admin accrues protocol fees but is not in the funded set.
        let on_admin = token.balance(&self.actors[ADMIN]);
        assert_eq!(on_admin, self.model.usdc_of(ADMIN), "admin USDC balance diverged");
    }

    fn reconcile_serial_registry(&self) {
        for &(lo, hi) in &self.model.credit.ranges {
            assert!(
                !self.credit.verify_serial_range(&lo, &hi),
                "contract claims registered range [{lo},{hi}] is still free"
            );
        }
    }
}

fn assert_outcome(predicted: &Predicted, actual: &Result<(), Fault>, op: &str, id: &str) {
    assert_eq!(
        predicted, actual,
        "{op} on {id}: model predicted {predicted:?} but contract returned {actual:?}"
    );
}
