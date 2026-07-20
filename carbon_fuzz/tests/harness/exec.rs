//! Executor: runs each operation against the real contract and the shadow
//! model, and asserts they agree on both the outcome and the resulting state.

use carbon_credit::{
    CarbonError, CarbonCreditContract, CarbonCreditContractClient, CreditStatus,
};
use soroban_sdk::{testutils::Address as _, Address, Env, String as SString};

use crate::model::{CreditModel, Fault, Predicted, Status};
use crate::ops::{batch_id, project_id, retirement_id, Op, N_ACTORS};

pub struct World<'a> {
    pub env: Env,
    pub client: CarbonCreditContractClient<'a>,
    pub actors: Vec<Address>,
    pub model: CreditModel,
}

/// Translates a contract error into the model's vocabulary. Any variant the
/// generator cannot reach is a genuine surprise, so it panics loudly rather
/// than silently widening the comparison.
fn to_fault(e: CarbonError) -> Fault {
    match e {
        CarbonError::ZeroAmountNotAllowed => Fault::ZeroAmountNotAllowed,
        CarbonError::InvalidSerialRange => Fault::InvalidSerialRange,
        CarbonError::InvalidVintageYear => Fault::InvalidVintageYear,
        CarbonError::SerialNumberConflict => Fault::SerialNumberConflict,
        CarbonError::DoubleCountingDetected => Fault::DoubleCountingDetected,
        CarbonError::AlreadyRetired => Fault::AlreadyRetired,
        CarbonError::InsufficientCredits => Fault::InsufficientCredits,
        CarbonError::ProjectNotFound => Fault::ProjectNotFound,
        CarbonError::UnauthorizedVerifier => Fault::UnauthorizedVerifier,
        // ReentrancyGuard leaking out to a top-level caller means a previous
        // operation returned without releasing the lock. That is exactly the
        // latent bug the guard review flagged, so surface it unmistakably.
        CarbonError::ReentrancyGuard => {
            panic!("ReentrancyGuard observed at top level — a prior op leaked the lock")
        }
        other => panic!("unreachable error from generated ops: {other:?}"),
    }
}

/// Normalises a `try_*` result into `Result<(), Fault>`.
///
/// The outer `Err(Err(_))` arm is a host trap (panic/overflow/budget), which is
/// categorically different from a returned error and must never be conflated
/// with one.
macro_rules! outcome {
    ($call:expr, $what:expr) => {
        match $call {
            Ok(_) => Ok(()),
            Err(Ok(e)) => Err(to_fault(e)),
            Err(Err(trap)) => panic!("host trap during {}: {:?}", $what, trap),
        }
    };
}

impl<'a> World<'a> {
    pub fn new() -> World<'a> {
        let env = Env::default();
        env.mock_all_auths();

        let actors: Vec<Address> = (0..N_ACTORS).map(|_| Address::generate(&env)).collect();
        let registry = Address::generate(&env);

        let id = env.register_contract(None, CarbonCreditContract);
        let client = CarbonCreditContractClient::new(&env, &id);
        // Actor 0 is the admin, and therefore the only address that may mint.
        client.initialize(&actors[0], &registry);

        World {
            env,
            client,
            actors,
            model: CreditModel::new(),
        }
    }

    fn s(&self, v: &str) -> SString {
        SString::from_str(&self.env, v)
    }

    pub fn step(&mut self, op: &Op) {
        // Each op models a separate transaction, and each transaction gets its
        // own budget on-chain. Without this the budget accumulates across the
        // whole sequence and the run dies of harness bookkeeping rather than a
        // real defect. Resetting to the *default* budget, not an unlimited one,
        // keeps single-operation exhaustion (e.g. a huge retirement building a
        // Vec<u64> per credit) detectable.
        self.env.budget().reset_default();

        match op {
            Op::Mint {
                actor,
                batch,
                project,
                amount,
                vintage_year,
                serial_start,
                serial_end,
            } => self.step_mint(
                *actor,
                *batch,
                *project,
                *amount,
                *vintage_year,
                *serial_start,
                *serial_end,
            ),
            Op::Retire {
                actor,
                batch,
                amount,
                retirement_seq,
            } => self.step_retire(*actor, *batch, *amount, *retirement_seq),
            Op::Transfer {
                from,
                to,
                batch,
                amount,
            } => self.step_transfer(*from, *to, *batch, *amount),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn step_mint(
        &mut self,
        actor: usize,
        batch: usize,
        project: usize,
        amount: i128,
        vintage_year: u32,
        serial_start: u64,
        serial_end: u64,
    ) {
        let bid = batch_id(batch);
        let pid = project_id(project);
        let predicted =
            self.model
                .predict_mint(actor == 0, &bid, amount, vintage_year, serial_start, serial_end);

        let actual = outcome!(
            self.client.try_mint_credits(
                &self.actors[actor],
                &self.s(&pid),
                &amount,
                &vintage_year,
                &self.s(&bid),
                &serial_start,
                &serial_end,
                &self.s("cid"),
            ),
            "mint_credits"
        );

        assert_outcome(&predicted, &actual, "mint", &bid);

        if predicted.is_ok() {
            self.model
                .apply_mint(&bid, &pid, amount, vintage_year, serial_start, serial_end);
            self.assert_batch_matches(&bid);
        }
    }

    fn step_retire(&mut self, actor: usize, batch: usize, amount: i128, seq: u32) {
        let bid = batch_id(batch);
        let rid = retirement_id(seq);
        let predicted = self.model.predict_retire(&bid, amount);

        let actual = outcome!(
            self.client.try_retire_credits(
                &self.actors[actor],
                &self.s(&bid),
                &amount,
                &self.s("reason"),
                &self.s("beneficiary"),
                &self.s(&rid),
                &self.s("txhash"),
            ),
            "retire_credits"
        );

        assert_outcome(&predicted, &actual, "retire", &bid);

        if predicted.is_ok() {
            let (lo, hi) = self.model.apply_retire(&bid, &rid, amount);

            // The certificate's serial span must match what the model computed.
            let cert = self.client.get_retirement_certificate(&self.s(&rid));
            assert_eq!(
                cert.amount, amount,
                "certificate {rid} recorded the wrong amount"
            );
            assert_eq!(
                cert.serial_numbers.len() as i128,
                amount,
                "certificate {rid} should hold exactly one serial per credit"
            );
            assert_eq!(
                cert.serial_numbers.first().unwrap(),
                lo,
                "certificate {rid} first serial diverged from the model"
            );
            assert_eq!(
                cert.serial_numbers.last().unwrap(),
                hi,
                "certificate {rid} last serial diverged from the model"
            );

            self.assert_batch_matches(&bid);
        }
    }

    fn step_transfer(&mut self, from: usize, to: usize, batch: usize, amount: i128) {
        let bid = batch_id(batch);
        let predicted = self.model.predict_transfer(&bid, amount);

        let actual = outcome!(
            self.client.try_transfer_credits(
                &self.actors[from],
                &self.actors[to],
                &self.s(&bid),
                &amount,
            ),
            "transfer_credits"
        );

        assert_outcome(&predicted, &actual, "transfer", &bid);

        // A successful transfer writes no storage, so the model is unchanged.
        // Assert that explicitly: if the contract ever gains a balance ledger,
        // this is the check that will notice the model went stale.
        if predicted.is_ok() {
            self.assert_batch_matches(&bid);
        }
    }

    // ── Directed helpers ──────────────────────────────────────────────────
    // The generator draws from a fixed pool; these let a targeted test drive
    // exact values. They keep the model in sync and assert success, so they are
    // only appropriate for operations the caller knows must succeed.

    pub fn mint_raw(
        &mut self,
        bid: &str,
        pid: &str,
        amount: i128,
        vintage_year: u32,
        serial_start: u64,
        serial_end: u64,
    ) {
        self.env.budget().reset_default();
        self.client.mint_credits(
            &self.actors[0],
            &self.s(pid),
            &amount,
            &vintage_year,
            &self.s(bid),
            &serial_start,
            &serial_end,
            &self.s("cid"),
        );
        self.model
            .apply_mint(bid, pid, amount, vintage_year, serial_start, serial_end);
    }

    pub fn retire_raw(&mut self, bid: &str, rid: &str, amount: i128) {
        self.env.budget().reset_default();
        self.client.retire_credits(
            &self.actors[0],
            &self.s(bid),
            &amount,
            &self.s("reason"),
            &self.s("beneficiary"),
            &self.s(rid),
            &self.s("txhash"),
        );
        self.model.apply_retire(bid, rid, amount);
    }

    pub fn certificate(&self, rid: &str) -> carbon_credit::RetirementCertificate {
        self.client.get_retirement_certificate(&self.s(rid))
    }

    /// Reconciles the contract's stored batch against the model's.
    fn assert_batch_matches(&self, bid: &str) {
        let expected = self
            .model
            .batches
            .get(bid)
            .expect("model lost a batch the contract accepted");
        let actual = self.client.get_credit_batch(&self.s(bid));

        assert_eq!(actual.amount, expected.amount, "batch {bid}: amount diverged");
        assert_eq!(
            actual.serial_start, expected.serial_start,
            "batch {bid}: serial_start diverged"
        );
        assert_eq!(
            actual.serial_end, expected.serial_end,
            "batch {bid}: serial_end diverged"
        );
        assert_eq!(
            actual.vintage_year, expected.vintage_year,
            "batch {bid}: vintage_year diverged"
        );

        let expected_status = match expected.status() {
            Status::Active => CreditStatus::Active,
            Status::PartiallyRetired => CreditStatus::PartiallyRetired,
            Status::FullyRetired => CreditStatus::FullyRetired,
        };
        assert_eq!(
            actual.status, expected_status,
            "batch {bid}: status diverged (model retired={}/{})",
            expected.retired, expected.amount
        );
    }
}

fn assert_outcome(predicted: &Predicted, actual: &Result<(), Fault>, op: &str, bid: &str) {
    assert_eq!(
        predicted, actual,
        "{op} on {bid}: model predicted {predicted:?} but contract returned {actual:?}"
    );
}
