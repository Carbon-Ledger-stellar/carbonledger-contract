# CarbonLedger Permission Matrix

## Overview

Every state-mutating function in CarbonLedger enforces caller permissions at runtime using
Soroban's `require_auth()` combined with contract-stored role checks. Read-only functions
(getters) have no permission requirements.

Permission checks follow the pattern:

1. `<caller>.require_auth()` — Soroban validates the transaction signature.
2. `Self::require_<role>(&env, &caller)` — contract validates the caller matches
   the stored role address.

---

## Permission Matrix

### `carbon_registry` Contract

| Function                    | Admin | Verifier | Oracle | Any Address |
|-----------------------------|:-----:|:--------:|:------:|:-----------:|
| `initialize`                |  ✅   |          |        |             |
| `register_project`          |  ✅   |          |        |             |
| `verify_project`            |       |    ✅    |        |             |
| `reject_project`            |       |    ✅    |        |             |
| `update_project_status`     |       |          |   ✅   |             |
| `suspend_project`           |  ✅   |          |        |             |
| `increment_issued`          |       |          |   ✅   |             |
| `propose_suspend_project`   |  ✅   |          |        |             |
| `execute_suspend_project`   |  ✅   |          |        |             |
| `contest_operation`         |       |          |        |     ✅      |
| `rollback_operation`        |  ✅   |          |        |             |
| `set_timelock_delay`        |  ✅   |          |        |             |
| `get_project` (read)        |       |          |        |     ✅      |
| `get_pending_op` (read)     |       |          |        |     ✅      |
| `get_contest` (read)        |       |          |        |     ✅      |

**Role definitions (registry):**
- **Admin** — address stored at `DataKey::RegistryAdmin` (set during `initialize`).
- **Verifier** — any address in the `Vec<Address>` stored at `DataKey::Verifiers`.
- **Oracle** — address stored at `DataKey::OracleAddress` (set during `initialize`).

---

### `carbon_credit` Contract

| Function                    | Admin | Any Holder | Any Address |
|-----------------------------|:-----:|:----------:|:-----------:|
| `initialize`                |  ✅   |            |             |
| `mint_credits`              |  ✅   |            |             |
| `retire_credits`            |       |     ✅     |             |
| `transfer_credits`          |       |     ✅     |             |
| `propose_pause`             |  ✅   |            |             |
| `execute_pause`             |  ✅   |            |             |
| `contest_operation`         |       |            |     ✅      |
| `rollback_operation`        |  ✅   |            |             |
| `set_timelock_delay`        |  ✅   |            |             |
| `get_credit_batch` (read)   |       |            |     ✅      |
| `get_retirement_certificate`|       |            |     ✅      |
| `verify_serial_range` (read)|       |            |     ✅      |
| `get_project_credits` (read)|       |            |     ✅      |
| `get_pending_op` (read)     |       |            |     ✅      |
| `get_contest` (read)        |       |            |     ✅      |

**Role definitions (credit):**
- **Admin** — address stored at `DataKey::Admin` (set during `initialize`).
- **Any Holder** — the `holder` / `from` address passed to the function; only
  `require_auth()` is checked (no stored role), meaning the caller must be the
  transaction signer matching that address.

---

### `carbon_marketplace` Contract

| Function                    | Admin | Seller | Buyer | Any Address |
|-----------------------------|:-----:|:------:|:-----:|:-----------:|
| `initialize`                |  ✅   |        |       |             |
| `list_credits`              |       |   ✅   |       |             |
| `delist_credits`            |       |   ✅†  |       |             |
| `purchase_credits`          |       |        |  ✅   |             |
| `bulk_purchase`             |       |        |  ✅   |             |
| `propose_update_fee`        |  ✅   |        |       |             |
| `execute_update_fee`        |  ✅   |        |       |             |
| `contest_operation`         |       |        |       |     ✅      |
| `rollback_operation`        |  ✅   |        |       |             |
| `set_timelock_delay`        |  ✅   |        |       |             |
| `get_listing` (read)        |       |        |       |     ✅      |
| `get_active_listings` (read)|       |        |       |     ✅      |
| `get_listings_by_project`   |       |        |       |     ✅      |
| `get_listings_by_vintage`   |       |        |       |     ✅      |
| `get_pending_op` (read)     |       |        |       |     ✅      |
| `get_contest` (read)        |       |        |       |     ✅      |

**† `delist_credits`**: `seller.require_auth()` + `listing.seller == seller` ownership check.

**Role definitions (marketplace):**
- **Admin** — address stored at `DataKey::Admin`.
- **Seller** — address passed as `seller` parameter; `require_auth()` enforced.
- **Buyer** — address passed as `buyer` parameter; `require_auth()` enforced.

---

### `carbon_oracle` Contract

| Function                          | Oracle | Any Address |
|-----------------------------------|:------:|:-----------:|
| `initialize`                      |   ✅†  |             |
| `submit_monitoring_data`          |   ✅   |             |
| `update_credit_price`             |   ✅   |             |
| `flag_project`                    |   ✅   |             |
| `propose_price_update`            |   ✅   |             |
| `execute_price_update`            |   ✅   |             |
| `contest_operation`               |        |     ✅      |
| `rollback_operation`              |   ✅   |             |
| `set_timelock_delay`              |   ✅   |             |
| `get_monitoring_data` (read)      |        |     ✅      |
| `get_benchmark_price` (read)      |        |     ✅      |
| `is_monitoring_current` (read)    |        |     ✅      |
| `get_pending_op` (read)           |        |     ✅      |
| `get_contest` (read)              |        |     ✅      |

**† `initialize`**: `admin.require_auth()` but admin is not stored as a role in oracle
(oracle address is stored instead). The admin address from `initialize` is only used to
bootstrap the contract.

**Role definitions (oracle):**
- **Oracle** — address stored at `DataKey::OracleAddress` (set during `initialize`).

---

## Cross-Contract Call Graph

```
┌───────────────────────────────────────────────────────────────────┐
│                     Off-Chain / External                          │
│  Admin · Verifier · Oracle · Holder · Seller · Buyer             │
└───────┬────────────┬───────────┬──────────────────────────────────┘
        │            │           │
        ▼            ▼           ▼
┌───────────────┐  ┌───────────────┐  ┌──────────────────────┐
│carbon_registry│  │carbon_credit  │  │carbon_marketplace    │
│               │  │               │  │                      │
│register_proj  │  │mint_credits   │  │list_credits          │
│verify_project │  │retire_credits │  │delist_credits        │
│reject_project │  │transfer_cred  │  │purchase_credits      │
│suspend_proj   │  │               │  │bulk_purchase         │
│update_status  │  │               │  │propose_update_fee    │
│propose_suspend│  │propose_pause  │  │execute_update_fee    │
│execute_suspend│  │execute_pause  │  │contest_operation     │
│contest_op     │  │contest_op     │  │rollback_operation    │
│rollback_op    │  │rollback_op    │  │                      │
│               │  │               │  │                      │
└───────────────┘  └───────────────┘  └──────────────────────┘
        ▲                                       ▲
        │  increment_issued(oracle, proj, amt)  │
        │  (oracle_address required)            │
        │                                       │
┌───────────────────────────────────────────────────────────┐
│                    carbon_oracle                          │
│                                                           │
│  submit_monitoring_data  ──→  events (c_ledger/mon_data) │
│  update_credit_price     ──→  events (c_ledger/price_upd)│
│  flag_project            ──→  events (c_ledger/flagged)  │
│  propose_price_update    ──→  time-lock queue             │
│  execute_price_update    ──→  activates staged price      │
└───────────────────────────────────────────────────────────┘
```

### Cross-Contract Call Details

| Caller Contract   | Callee Contract    | Function Called        | Required Caller Role |
|-------------------|--------------------|------------------------|----------------------|
| `carbon_credit`   | `carbon_registry`  | `increment_issued`     | Oracle address       |
| External (oracle) | `carbon_registry`  | `update_project_status`| Oracle address       |
| External (oracle) | `carbon_oracle`    | `submit_monitoring_data`| Oracle address      |
| External (oracle) | `carbon_oracle`    | `update_credit_price`  | Oracle address       |
| External (oracle) | `carbon_oracle`    | `flag_project`         | Oracle address       |

> **Note:** `carbon_credit` stores the registry contract address (`DataKey::RegistryContract`)
> for future cross-contract calls. Current implementation emits events consumed off-chain;
> direct cross-contract invocation of `increment_issued` is a planned integration point.

---

## Runtime Permission Enforcement

All state-mutating functions implement the following invariant:

```
acquire_lock()                      // reentrancy guard
<caller>.require_auth()             // Soroban signature check
Self::require_<role>(&env, &caller) // stored role membership check
... state mutation ...
release_lock()
```

Failing any check returns an error immediately, and `release_lock()` is called on every
error path to prevent the reentrancy guard from becoming permanently set.

### Error Codes

| Error                  | Code | Meaning                                    |
|------------------------|------|--------------------------------------------|
| `UnauthorizedVerifier` | 7    | Caller not admin or not in verifier set    |
| `UnauthorizedOracle`   | 8    | Caller does not match stored oracle address|
| `RetirementIrreversible`| 15  | Time-lock delay not elapsed / contested    |
| `ReentrancyGuard`      | 20   | Concurrent state mutation detected         |

---

## Bypass Analysis

The following attack vectors were reviewed and mitigated:

| Bypass Attempt                           | Status    | Mitigation                                      |
|------------------------------------------|-----------|-------------------------------------------------|
| Call `mint_credits` without admin sig    | Blocked   | `admin.require_auth()` + `require_admin` check  |
| Pass a different admin address           | Blocked   | `require_admin` compares against stored address |
| Call `verify_project` as non-verifier    | Blocked   | `require_verifier` checks Vec membership        |
| Submit oracle data as non-oracle         | Blocked   | `require_oracle` compares stored address        |
| Delist another seller's listing          | Blocked   | `listing.seller == seller` ownership check      |
| Execute time-locked op before delay      | Blocked   | `timestamp < op.eta` guard                      |
| Execute contested op                     | Blocked   | Contest record presence blocks execution        |
| Re-enter via cross-contract call         | Blocked   | Per-contract reentrancy lock (`DataKey::Locked`)|
| Double-initialize a contract             | Blocked   | `AlreadyInitialized` guard on all `initialize`  |
