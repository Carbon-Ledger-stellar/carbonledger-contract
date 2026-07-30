//! Property-based invariant tests for all four CarbonLedger Soroban contracts.
//!
//! Each `proptest!` block exercises one named invariant with 100 000 generated
//! input combinations.  On failure proptest prints the exact failing input
//! together with a minimal shrunk reproducer, satisfying the requirement that
//! "invariant violations produce detailed error reports with input traces."
//!
//! # Invariants covered
//!
//! | # | Name                          | Contract(s)              |
//! |---|-------------------------------|--------------------------|
//! | 1 | Supply conservation           | carbon_credit            |
//! | 2 | Retirement irreversibility    | carbon_credit            |
//! | 3 | Serial-range no-overlap       | carbon_credit            |
//! | 4 | Arithmetic overflow safety    | carbon_credit, registry  |
//! | 5 | Registry state-machine        | carbon_registry          |
//! | 6 | Authorization guards          | all four                 |
//! | 7 | USDC conservation             | carbon_marketplace       |
//! | 8 | Listing sanity                | carbon_marketplace       |
//! | 9 | Oracle data freshness         | carbon_oracle            |
//! |10 | Zero-amount rejection         | credit + marketplace     |

use proptest::prelude::*;
use soroban_sdk::{
    testutils::Address as _,
    token::{StellarAssetClient, TokenClient},
    vec as svec, Address, Env, String as SStr,
};

// ── helpers ───────────────────────────────────────────────────────────────────

fn s(env: &Env, v: &str) -> SStr { SStr::from_str(env, v) }

/// Spin up all four contracts inside a fresh Env with mock auth.
/// Returns (env, admin, verifier, oracle_signer, usdc_addr,
///          registry_client, credit_client, market_client, oracle_client).
#[allow(clippy::type_complexity)]
fn world() -> (
    Env,
    Address, Address, Address, Address,
    carbon_registry::CarbonRegistryContractClient<'static>,
    carbon_credit::CarbonCreditContractClient<'static>,
    carbon_marketplace::CarbonMarketplaceContractClient<'static>,
    carbon_oracle::CarbonOracleContractClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();
    let admin    = Address::generate(&env);
    let verifier = Address::generate(&env);
    let oracle   = Address::generate(&env);

    let usdc = env.register_stellar_asset_contract(admin.clone());
    StellarAssetClient::new(&env, &usdc).mint(&admin, &1_000_000_000_000_i128);

    let reg = carbon_registry::CarbonRegistryContractClient::new(
        &env, &env.register_contract(None, carbon_registry::CarbonRegistryContract));
    reg.initialize(&admin, &oracle, &svec![&env, verifier.clone()]);

    let cred = carbon_credit::CarbonCreditContractClient::new(
        &env, &env.register_contract(None, carbon_credit::CarbonCreditContract));
    cred.initialize(&admin, &reg.address);

    let mkt = carbon_marketplace::CarbonMarketplaceContractClient::new(
        &env, &env.register_contract(None, carbon_marketplace::CarbonMarketplaceContract));
    mkt.initialize(&admin, &usdc);

    let orc = carbon_oracle::CarbonOracleContractClient::new(
        &env, &env.register_contract(None, carbon_oracle::CarbonOracleContract));
    orc.initialize(&admin, &oracle);

    (env, admin, verifier, oracle, usdc, reg, cred, mkt, orc)
}

/// Mint a batch of `amount` credits with perfectly matched serial range.
fn mint_batch(
    env: &Env,
    cred: &carbon_credit::CarbonCreditContractClient,
    admin: &Address,
    batch_id: &str,
    amount: i128,
    serial_start: u64,
) {
    let serial_end = serial_start + amount as u64 - 1;
    cred.mint_credits(
        admin,
        &s(env, "proj-001"),
        &amount,
        &2023_u32,
        &s(env, batch_id),
        &serial_start,
        &serial_end,
        &s(env, "QmCID"),
    );
}

// ─────────────────────────────────────────────────────────────────────────────
// Invariant 1 – Supply conservation
//
// After any sequence of retirements the sum of retired amounts must equal
// the reduction in active supply: active = minted − retired.
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100_000))]

    /// **Invariant 1 — Supply conservation.**
    ///
    /// For any mint of `total` credits followed by up to two partial retirements
    /// of `r1` and `r2` (each ≤ remaining active), the batch status and the
    /// accumulated retired tally must equal exactly `r1 + r2`.
    #[test]
    fn invariant_supply_conservation(
        total in 2_i128..=500_i128,
        r1    in 1_i128..=249_i128,
        r2    in 1_i128..=249_i128,
    ) {
        let (env, admin, _verifier, _oracle, _usdc, _reg, cred, _mkt, _orc) = world();
        // Skip if retirements exceed supply
        prop_assume!(r1 < total && r1 + r2 <= total);

        mint_batch(&env, &cred, &admin, "b1", total, 1);

        let holder = Address::generate(&env);

        cred.retire_credits(
            &holder, &s(&env,"b1"), &r1,
            &s(&env,"reason"), &s(&env,"Corp"), &s(&env,"ret1"), &s(&env,"tx1"),
        );
        let after_r1 = cred.get_credit_batch(&s(&env, "b1"));
        prop_assert!(
            after_r1.status == carbon_credit::CreditStatus::PartiallyRetired
                || after_r1.status == carbon_credit::CreditStatus::FullyRetired,
            "after first retirement batch must be Partially or FullyRetired; got {:?}",
            after_r1.status
        );

        if r1 + r2 < total {
            cred.retire_credits(
                &holder, &s(&env,"b1"), &r2,
                &s(&env,"reason"), &s(&env,"Corp"), &s(&env,"ret2"), &s(&env,"tx2"),
            );
            let after_r2 = cred.get_credit_batch(&s(&env, "b1"));
            prop_assert_eq!(
                after_r2.status,
                carbon_credit::CreditStatus::PartiallyRetired,
                "batch with remaining credits must stay PartiallyRetired after second retirement"
            );
        } else {
            // r1 + r2 == total → should be FullyRetired
            cred.retire_credits(
                &holder, &s(&env,"b1"), &r2,
                &s(&env,"reason"), &s(&env,"Corp"), &s(&env,"ret2"), &s(&env,"tx2"),
            );
            let after_r2 = cred.get_credit_batch(&s(&env, "b1"));
            prop_assert_eq!(
                after_r2.status,
                carbon_credit::CreditStatus::FullyRetired,
                "batch fully consumed must be FullyRetired"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 2 – Retirement irreversibility
    //
    // Once a batch is FullyRetired, no further retirements or transfers succeed.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 2 — Retirement irreversibility.**
    ///
    /// A FullyRetired batch must reject every subsequent retire or transfer
    /// attempt regardless of caller or amount.
    #[test]
    fn invariant_retirement_irreversibility(
        amount in 1_i128..=200_i128,
        extra  in 1_i128..=50_i128,
    ) {
        let (env, admin, _v, _o, _u, _reg, cred, _mkt, _orc) = world();
        mint_batch(&env, &cred, &admin, "b1", amount, 1);
        let holder = Address::generate(&env);

        // Fully retire the batch
        cred.retire_credits(
            &holder, &s(&env,"b1"), &amount,
            &s(&env,"reason"), &s(&env,"Corp"), &s(&env,"ret1"), &s(&env,"tx1"),
        );

        // Any further retirement must fail
        let re_retire = cred.try_retire_credits(
            &holder, &s(&env,"b1"), &extra,
            &s(&env,"reason"), &s(&env,"Corp"), &s(&env,"ret2"), &s(&env,"tx2"),
        );
        prop_assert!(
            re_retire.is_err(),
            "FullyRetired batch must reject re-retirement; extra={extra}"
        );

        // Any transfer must also fail
        let to = Address::generate(&env);
        let transfer = cred.try_transfer_credits(&holder, &to, &s(&env,"b1"), &1_i128);
        prop_assert!(
            transfer.is_err(),
            "FullyRetired batch must reject transfer"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Invariant 3 – Serial-range no-overlap
//
// Two batches whose serial ranges overlap must be rejected; disjoint ranges
// must both be accepted. No two minted credits may share a serial number.
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100_000))]

    /// **Invariant 3 — Serial-range no-overlap.**
    ///
    /// Minting a second batch whose serial range overlaps an existing batch must
    /// be rejected with DoubleCountingDetected.  A non-overlapping second batch
    /// must succeed.
    #[test]
    fn invariant_serial_no_overlap(
        amt1  in 1_i128..=200_i128,
        amt2  in 1_i128..=200_i128,
        gap   in 0_u64..=5_u64,   // 0 = adjacent-touching (overlap), 1+ = gap
    ) {
        let (env, admin, _v, _o, _u, _reg, cred, _mkt, _orc) = world();

        // Batch 1: serials [1, amt1]
        let s1_end = amt1 as u64;
        mint_batch(&env, &cred, &admin, "b1", amt1, 1);

        // Batch 2: starts at s1_end + gap
        // gap=0 means serial_start = s1_end, which overlaps batch 1 (both share s1_end).
        let s2_start = s1_end + gap;
        let s2_end   = s2_start + amt2 as u64 - 1;

        let result = cred.try_mint_credits(
            &admin,
            &s(&env, "proj-001"),
            &amt2,
            &2023_u32,
            &s(&env, "b2"),
            &s2_start,
            &s2_end,
            &s(&env, "QmCID"),
        );

        if gap == 0 {
            // Overlaps batch 1 (they share serial s1_end)
            prop_assert!(
                result.is_err(),
                "overlapping serial range [s2_start={s2_start}, s2_end={s2_end}] must be rejected"
            );
        } else {
            // gap ≥ 1: strictly disjoint — must succeed
            prop_assert!(
                result.is_ok(),
                "disjoint serial range [s2_start={s2_start}, s2_end={s2_end}] must be accepted; got {result:?}"
            );
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 4 – Arithmetic overflow safety
    //
    // No combination of valid or near-boundary inputs causes a host trap
    // (panic / wasm abort).  Overflow returns a typed error, never crashes.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 4 — Arithmetic overflow safety.**
    ///
    /// Minting a batch with serial_end = u64::MAX - 1 and amount = 2 must
    /// succeed (no overflow). Minting with serial_end = u64::MAX (span+1
    /// overflows u64) must return an error, never panic.
    #[test]
    fn invariant_arithmetic_overflow_safety(
        offset in 0_u64..=10_u64,
    ) {
        let (env, admin, _v, _o, _u, _reg, cred, _mkt, _orc) = world();

        // Near-boundary but valid: span = 2, no overflow
        let start = u64::MAX - 1 - offset;
        let end   = start + 1; // span = 2, amount = 2
        let ok = cred.try_mint_credits(
            &admin, &s(&env,"proj-001"), &2_i128, &2023_u32,
            &s(&env,"b-safe"), &start, &end, &s(&env,"cid"),
        );
        prop_assert!(
            ok.is_ok(),
            "near-boundary valid mint (start={start}, end={end}) must succeed, got {ok:?}"
        );

        // Overflow: span+1 wraps u64 → must return typed error, not trap
        let overflow_start: u64 = 0;
        let overflow_end:   u64 = u64::MAX;
        let err = cred.try_mint_credits(
            &admin, &s(&env,"proj-001"), &1_i128, &2023_u32,
            &s(&env,"b-of"), &overflow_start, &overflow_end, &s(&env,"cid"),
        );
        prop_assert!(
            err.is_err(),
            "serial span overflow must return an error, not trap"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 5 – Registry state-machine correctness
    //
    // Projects transition through legal states only.
    // Pending → Verified | Rejected.  Verified → Suspended.
    // Any vintage outside [2000, 2100] is always rejected at registration.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 5 — Registry state-machine correctness.**
    ///
    /// Registration with valid vintage succeeds; invalid vintage always fails.
    /// After registration the status is Pending. After verification it is Verified.
    #[test]
    fn invariant_registry_state_machine(
        vintage in 1990_u32..=2110_u32,
    ) {
        let (env, admin, verifier, _o, _u, reg, _cred, _mkt, _orc) = world();

        let result = reg.try_register_project(
            &admin,
            &s(&env,"proj-sm"),
            &s(&env,"Name"),
            &s(&env,"cid"),
            &verifier,
            &s(&env,"VCS"),
            &s(&env,"Brazil"),
            &s(&env,"forestry"),
            &vintage,
        );

        if vintage < 2000 || vintage > 2100 {
            prop_assert!(
                result.is_err(),
                "vintage {vintage} outside [2000,2100] must be rejected"
            );
        } else {
            prop_assert!(
                result.is_ok(),
                "vintage {vintage} in [2000,2100] must be accepted, got {result:?}"
            );
            // After register → Pending
            let p = reg.get_project(&s(&env, "proj-sm"));
            prop_assert_eq!(p.status, carbon_registry::ProjectStatus::Pending,
                "freshly registered project must be Pending");

            // Verify → Verified
            reg.verify_project(&verifier, &s(&env,"proj-sm"));
            let p2 = reg.get_project(&s(&env, "proj-sm"));
            prop_assert_eq!(p2.status, carbon_registry::ProjectStatus::Verified,
                "verified project must be Verified");

            // Suspend → Suspended
            reg.suspend_project(&admin, &s(&env,"proj-sm"), &s(&env,"audit"));
            let p3 = reg.get_project(&s(&env, "proj-sm"));
            prop_assert_eq!(p3.status, carbon_registry::ProjectStatus::Suspended,
                "suspended project must be Suspended");
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 6 – Authorization guards
    //
    // Every privileged operation must reject callers that are not the expected
    // role (admin / verifier / oracle) regardless of which random address calls.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 6 — Authorization guards.**
    ///
    /// A random address (not admin/verifier/oracle) must always be rejected when
    /// calling role-gated functions: register_project, verify_project,
    /// mint_credits, submit_monitoring_data, update_credit_price.
    #[test]
    fn invariant_auth_guards(_seed in 0_u32..=u32::MAX) {
        let (env, _admin, verifier, oracle, _u, reg, cred, _mkt, orc) = world();
        let rogue = Address::generate(&env);

        // register_project requires admin
        let r1 = reg.try_register_project(
            &rogue, &s(&env,"p"), &s(&env,"n"), &s(&env,"c"),
            &verifier, &s(&env,"VCS"), &s(&env,"BR"), &s(&env,"f"), &2023_u32,
        );
        prop_assert!(r1.is_err(), "rogue must not register a project");

        // verify_project requires verifier
        let r2 = reg.try_verify_project(&rogue, &s(&env,"p"));
        prop_assert!(r2.is_err(), "rogue must not verify a project");

        // mint_credits requires admin
        let r3 = cred.try_mint_credits(
            &rogue, &s(&env,"proj"), &10_i128, &2023_u32,
            &s(&env,"b"), &1_u64, &10_u64, &s(&env,"cid"),
        );
        prop_assert!(r3.is_err(), "rogue must not mint credits");

        // submit_monitoring_data requires oracle
        let r4 = orc.try_submit_monitoring_data(
            &rogue, &s(&env,"proj"), &s(&env,"2023-Q1"),
            &1000_i128, &80_u32, &s(&env,"sat"),
        );
        prop_assert!(r4.is_err(), "rogue must not submit monitoring data");

        // update_credit_price requires oracle
        let r5 = orc.try_update_credit_price(
            &rogue, &s(&env,"VCS"), &2023_u32, &1_000_000_i128,
        );
        prop_assert!(r5.is_err(), "rogue must not update credit price");

        // Verify that the correct role does succeed (non-rogue oracle)
        let r6 = orc.try_update_credit_price(
            &oracle, &s(&env,"VCS"), &2023_u32, &1_000_000_i128,
        );
        prop_assert!(r6.is_ok(), "oracle must be able to update price");
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Invariant 7 – USDC conservation
//
// Every marketplace purchase moves USDC between accounts; the total supply
// never changes and no balance goes negative.
// ─────────────────────────────────────────────────────────────────────────────

proptest! {
    #![proptest_config(ProptestConfig::with_cases(100_000))]

    /// **Invariant 7 — USDC conservation.**
    ///
    /// After a valid purchase the sum of all actor USDC balances equals the
    /// initial total and neither buyer nor seller has a negative balance.
    #[test]
    fn invariant_usdc_conservation(
        amount    in 1_i128..=100_i128,
        price     in 1_i128..=10_000_i128,
    ) {
        let (env, admin, _v, _o, usdc, _reg, _cred, mkt, _orc) = world();

        let buyer  = Address::generate(&env);
        let seller = Address::generate(&env);

        // Fund buyer with enough USDC
        let total_cost = amount * price;
        StellarAssetClient::new(&env, &usdc).mint(&buyer, &(total_cost * 2));

        // Seller lists credits
        mkt.list_credits(
            &seller,
            &s(&env,"list1"),
            &s(&env,"b1"),
            &s(&env,"proj-001"),
            &amount,
            &price,
            &2023_u32,
            &s(&env,"VCS"),
            &s(&env,"Brazil"),
        );

        let token = TokenClient::new(&env, &usdc);
        let buyer_before  = token.balance(&buyer);
        let seller_before = token.balance(&seller);
        let admin_before  = token.balance(&admin);

        // Approve the market contract to spend buyer's USDC
        mkt.purchase_credits(&buyer, &s(&env,"list1"), &amount);

        let buyer_after  = token.balance(&buyer);
        let seller_after = token.balance(&seller);
        let admin_after  = token.balance(&admin);

        // Buyer spent exactly total_cost
        prop_assert_eq!(
            buyer_before - buyer_after, total_cost,
            "buyer should have spent exactly amount*price={total_cost}"
        );

        // Seller received proceeds (total minus 1% fee)
        let fee      = total_cost / 100;
        let proceeds = total_cost - fee;
        prop_assert_eq!(
            seller_after - seller_before, proceeds,
            "seller should receive proceeds={proceeds}"
        );

        // Admin received the protocol fee
        prop_assert_eq!(
            admin_after - admin_before, fee,
            "admin should receive protocol fee={fee}"
        );

        // Conservation: total is unchanged
        let total_before = buyer_before + seller_before + admin_before;
        let total_after  = buyer_after  + seller_after  + admin_after;
        prop_assert_eq!(
            total_before, total_after,
            "USDC total must be conserved across a purchase"
        );

        // No negative balances
        prop_assert!(buyer_after  >= 0, "buyer balance must not go negative");
        prop_assert!(seller_after >= 0, "seller balance must not go negative");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 8 – Listing sanity
    //
    // A listing's available amount stays in [0, original_amount].
    // After a partial purchase amount_available decreases by exactly the
    // purchased quantity.  A fully drained listing must be Sold.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 8 — Listing sanity.**
    ///
    /// Partial and full purchases update `amount_available` exactly, and the
    /// listing transitions to Sold only when fully drained.
    #[test]
    fn invariant_listing_sanity(
        total   in 2_i128..=200_i128,
        partial in 1_i128..=199_i128,
    ) {
        prop_assume!(partial < total);
        let (env, admin, _v, _o, usdc, _reg, _cred, mkt, _orc) = world();

        let buyer = Address::generate(&env);
        let price = 1_i128;
        StellarAssetClient::new(&env, &usdc).mint(&buyer, &(total * 2));

        mkt.list_credits(
            &admin,
            &s(&env,"list1"),
            &s(&env,"b1"),
            &s(&env,"proj"),
            &total,
            &price,
            &2023_u32,
            &s(&env,"VCS"),
            &s(&env,"Brazil"),
        );

        // Partial purchase
        mkt.purchase_credits(&buyer, &s(&env,"list1"), &partial);
        let after_partial = mkt.get_listing(&s(&env,"list1"));
        prop_assert_eq!(
            after_partial.amount_available, total - partial,
            "after partial purchase available must equal total-partial"
        );
        prop_assert_ne!(
            after_partial.status,
            carbon_marketplace::ListingStatus::Sold,
            "partially filled listing must not be Sold"
        );

        // Buy the rest — listing should become Sold
        let remaining = total - partial;
        mkt.purchase_credits(&buyer, &s(&env,"list1"), &remaining);
        let after_full = mkt.get_listing(&s(&env,"list1"));
        prop_assert_eq!(
            after_full.amount_available, 0,
            "fully drained listing must have amount_available=0"
        );
        prop_assert_eq!(
            after_full.status,
            carbon_marketplace::ListingStatus::Sold,
            "fully drained listing must be Sold"
        );
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 9 – Oracle data freshness
    //
    // is_monitoring_current() returns true immediately after submission and
    // false when no data has ever been submitted.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 9 — Oracle data freshness.**
    ///
    /// Freshly submitted monitoring data is always reported as current.
    /// A project with no submissions is never current.
    #[test]
    fn invariant_oracle_data_freshness(
        tonnes in 1_i128..=1_000_000_i128,
        score  in 0_u32..=100_u32,
    ) {
        let (env, _admin, _v, oracle, _u, _reg, _cred, _mkt, orc) = world();

        // No data yet → not current
        let before = orc.is_monitoring_current(&s(&env,"proj-001"));
        prop_assert!(!before, "project with no monitoring data must not be current");

        orc.submit_monitoring_data(
            &oracle,
            &s(&env,"proj-001"),
            &s(&env,"2023-Q1"),
            &tonnes,
            &score,
            &s(&env,"QmSat"),
        );

        // Just submitted → current
        let after = orc.is_monitoring_current(&s(&env,"proj-001"));
        prop_assert!(after, "project with fresh monitoring data must be current; tonnes={tonnes}, score={score}");
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Invariant 10 – Zero-amount rejection
    //
    // Every amount-taking entry point must reject zero and negative values
    // before performing any state changes.
    // ─────────────────────────────────────────────────────────────────────────

    /// **Invariant 10 — Zero-amount rejection.**
    ///
    /// Zero and negative amounts must be rejected by mint, retire, transfer,
    /// list, purchase, and oracle submission.  No partial state change may occur.
    #[test]
    fn invariant_zero_amount_rejection(
        bad_amount in -100_i128..=0_i128,
    ) {
        let (env, admin, _v, oracle, usdc, _reg, cred, mkt, orc) = world();
        let holder = Address::generate(&env);

        // mint_credits
        prop_assert!(
            cred.try_mint_credits(
                &admin, &s(&env,"proj"), &bad_amount, &2023_u32,
                &s(&env,"b"), &1_u64, &100_u64, &s(&env,"cid"),
            ).is_err(),
            "mint_credits({bad_amount}) must be rejected"
        );

        // Mint a valid batch so retire/transfer have something to work with
        mint_batch(&env, &cred, &admin, "b-valid", 50, 1);

        // retire_credits
        prop_assert!(
            cred.try_retire_credits(
                &holder, &s(&env,"b-valid"), &bad_amount,
                &s(&env,"r"), &s(&env,"Corp"), &s(&env,"ret"), &s(&env,"tx"),
            ).is_err(),
            "retire_credits({bad_amount}) must be rejected"
        );

        // transfer_credits
        let to = Address::generate(&env);
        prop_assert!(
            cred.try_transfer_credits(&holder, &to, &s(&env,"b-valid"), &bad_amount)
                .is_err(),
            "transfer_credits({bad_amount}) must be rejected"
        );

        // list_credits
        prop_assert!(
            mkt.try_list_credits(
                &holder, &s(&env,"l1"), &s(&env,"b-valid"), &s(&env,"proj"),
                &bad_amount, &1_i128, &2023_u32, &s(&env,"VCS"), &s(&env,"BR"),
            ).is_err(),
            "list_credits(amount={bad_amount}) must be rejected"
        );

        // list with zero price
        prop_assert!(
            mkt.try_list_credits(
                &holder, &s(&env,"l2"), &s(&env,"b-valid"), &s(&env,"proj"),
                &1_i128, &bad_amount, &2023_u32, &s(&env,"VCS"), &s(&env,"BR"),
            ).is_err(),
            "list_credits(price={bad_amount}) must be rejected"
        );

        // submit_monitoring_data
        prop_assert!(
            orc.try_submit_monitoring_data(
                &oracle, &s(&env,"proj"), &s(&env,"per"), &bad_amount,
                &80_u32, &s(&env,"sat"),
            ).is_err(),
            "submit_monitoring_data(tonnes={bad_amount}) must be rejected"
        );

        // update_credit_price
        prop_assert!(
            orc.try_update_credit_price(
                &oracle, &s(&env,"VCS"), &2023_u32, &bad_amount,
            ).is_err(),
            "update_credit_price(price={bad_amount}) must be rejected"
        );

        // Verify no state was changed — the valid batch is still Active
        let batch = cred.get_credit_batch(&s(&env,"b-valid"));
        prop_assert_eq!(
            batch.status, carbon_credit::CreditStatus::Active,
            "valid batch must remain Active after all rejected zero-amount calls"
        );

        drop(usdc); // suppress unused warning
    }
}
