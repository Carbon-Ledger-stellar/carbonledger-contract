# Proptest Invariant Testing — Coverage Reference

Property-based tests live in `carbon_fuzz/tests/proptest_invariants.rs`.
Each test block is configured with **100 000 cases** via
`ProptestConfig::with_cases(100_000)`.

## How to run

```bash
# Default (100 000 cases each invariant, ~2-5 min in release):
cargo test --release -p carbon_fuzz --test proptest_invariants

# With output visible:
cargo test --release -p carbon_fuzz --test proptest_invariants -- --nocapture

# List tests only (no execution):
cargo test --release -p carbon_fuzz --test proptest_invariants -- --list
```

On failure proptest prints the failing input and a minimal shrunk reproducer:

```
thread 'invariant_supply_conservation' panicked at 'Test failed: …
Minimal failing input:
  total = 3
  r1 = 2
  r2 = 2
```

Replay a shrunk seed with:
```bash
PROPTEST_CASES=1 cargo test --release -p carbon_fuzz \
  --test proptest_invariants invariant_supply_conservation
```

---

## Invariant catalogue

| # | Test name | Strategy | What it checks |
|---|-----------|----------|----------------|
| 1 | `invariant_supply_conservation` | `total ∈ [2,500]`, `r1,r2 ∈ [1,249]` | After any sequence of retirements the batch status matches the retired total; retired ≤ minted at all times |
| 2 | `invariant_retirement_irreversibility` | `amount ∈ [1,200]`, `extra ∈ [1,50]` | A FullyRetired batch rejects every subsequent retire and transfer |
| 3 | `invariant_serial_no_overlap` | `amt1,amt2 ∈ [1,200]`, `gap ∈ [0,5]` | Overlapping serial ranges are rejected; disjoint ranges are accepted |
| 4 | `invariant_arithmetic_overflow_safety` | `offset ∈ [0,10]` | Near-boundary mints succeed; span+1 overflow returns a typed error, never panics |
| 5 | `invariant_registry_state_machine` | `vintage ∈ [1990,2110]` | Vintage outside [2000,2100] always rejected; valid registration → Pending → Verified → Suspended |
| 6 | `invariant_auth_guards` | `seed ∈ [0,u32::MAX]` | A random non-privileged address is rejected by all role-gated functions; correct role succeeds |
| 7 | `invariant_usdc_conservation` | `amount ∈ [1,100]`, `price ∈ [1,10000]` | USDC total is conserved across purchases; buyer/seller/admin deltas match the 1% fee formula |
| 8 | `invariant_listing_sanity` | `total ∈ [2,200]`, `partial ∈ [1,199]` | Partial purchase reduces amount_available exactly; full drain → Sold status |
| 9 | `invariant_oracle_data_freshness` | `tonnes ∈ [1,1e6]`, `score ∈ [0,100]` | Freshly submitted data is always current; no-submission project is never current |
| 10 | `invariant_zero_amount_rejection` | `bad_amount ∈ [-100,0]` | All amount-taking entry points reject zero/negative values; no state change occurs |

---

## Code-path coverage notes

Each invariant exercises specific contract code paths:

- **Supply conservation & irreversibility** → `retire_credits` active-amount
  arithmetic, `FullyRetired` guard, `CreditStatus` transitions.
- **Serial no-overlap** → `verify_serial_range_internal`, overlap check in
  `mint_credits`, `DoubleCountingDetected` error path.
- **Arithmetic overflow** → `checked_add` on `span + 1` in `assert_valid_batch`,
  the `checked!` macro returning `ArithmeticOverflow`.
- **Registry state machine** → `InvalidVintageYear` guard, `ProjectStatus`
  transitions in `register_project` / `verify_project` / `suspend_project`.
- **Auth guards** → all four `require_admin` / `require_verifier` /
  `require_oracle` helper paths returning `UnauthorizedVerifier` /
  `UnauthorizedOracle`.
- **USDC conservation** → `purchase_credits` USDC transfer arithmetic,
  1% fee calculation, token balance reads.
- **Listing sanity** → `amount_available` decrement, `ListingStatus::Sold`
  transition in `purchase_credits`.
- **Oracle freshness** → `MONITORING_FRESHNESS_SECS` comparison,
  `LatestMonitoring` storage read/write.
- **Zero-amount rejection** → `ZeroAmountNotAllowed` guard in every mutating
  entry point across all four contracts.

---

## Adding new invariants

1. Add a new `proptest!` block in `carbon_fuzz/tests/proptest_invariants.rs`.
2. Use `ProptestConfig::with_cases(100_000)` (or higher for critical invariants).
3. Document the invariant in this file under the catalogue table.
4. The CI `proptest` job picks it up automatically on the next push.
