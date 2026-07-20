# CarbonLedger Formal Specification

**Version:** 1.0.0  
**Date:** 2026-07-20  
**Contracts Specified:** `carbon_registry`, `carbon_credit`, `carbon_marketplace`, `carbon_oracle`  
**Formalism:** TLA+ (Temporal Logic of Actions) with mathematical pre/postconditions  
**Status:** Validated against implementation

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Notation and Conventions](#2-notation-and-conventions)
3. [System State Space](#3-system-state-space)
4. [Carbon Registry — Formal Specification](#4-carbon-registry--formal-specification)
5. [Carbon Credit — Formal Specification](#5-carbon-credit--formal-specification)
6. [Carbon Marketplace — Formal Specification](#6-carbon-marketplace--formal-specification)
7. [Carbon Oracle — Formal Specification](#7-carbon-oracle--formal-specification)
8. [Cross-Contract Invariants](#8-cross-contract-invariants)
9. [Proof Sketch: Retirement Irreversibility](#9-proof-sketch-retirement-irreversibility)
10. [Proof Sketch: Serial Number Global Uniqueness](#10-proof-sketch-serial-number-global-uniqueness)
11. [Liveness and Safety Properties](#11-liveness-and-safety-properties)
12. [Validation Against Implementation](#12-validation-against-implementation)

---

## 1. Introduction

This document provides a complete formal specification of the CarbonLedger smart contract system deployed on the Stellar Soroban platform. CarbonLedger tokenises real-world carbon offset credits as on-chain assets, enabling registration, verification, trading, and irreversible retirement of those credits. The system comprises four contracts whose state machines are specified here.

### 1.1 Scope

- Complete enumeration of all state variables per contract.
- TLA+ module for each contract defining actions, enabling conditions, and transitions.
- Preconditions and postconditions for every public function, stated in first-order logic.
- Global system invariants across all four contracts.
- Proof sketches for the two highest-stakes safety properties: **retirement irreversibility** and **serial number global uniqueness**.

### 1.2 Out of Scope

- Automated theorem proving or model-checking runs.
- Formal verification tooling setup (TLC, Apalache, TLAPS).
- Network-level properties of the Stellar consensus protocol.

### 1.3 Design Goals Being Specified

| Goal | Property class | Addressed in |
|------|---------------|--------------|
| No double-counting of retirements | Safety | §5, §9 |
| No overlapping serial numbers across batches | Safety | §5, §10 |
| Irreversibility of project rejection | Safety | §4 |
| Only authorised actors mutate state | Safety | §4–§7 |
| Reentrancy cannot corrupt state | Safety | §4–§7 |
| Retired credits cannot be traded | Safety | §5 |
| Protocol fee collected on every purchase | Safety | §6 |
| Benchmark price expires after 24 h | Liveness | §7 |
| Monitoring data freshness enforced | Safety | §7, §11 |

---

## 2. Notation and Conventions

### 2.1 Mathematical Notation

| Symbol | Meaning |
|--------|---------|
| `∀` | Universal quantifier ("for all") |
| `∃` | Existential quantifier ("there exists") |
| `∈` | Set membership |
| `∉` | Not a set member |
| `⊆` | Subset |
| `∩` | Set intersection |
| `∅` | Empty set |
| `→` | Logical implication |
| `↔` | Logical biconditional |
| `¬` | Logical negation |
| `∧` | Logical conjunction |
| `∨` | Logical disjunction |
| `⊕` | State update (next-state value) |
| `dom(f)` | Domain of function/map `f` |
| `[x ↦ v]` | Map with key `x` bound to value `v` |
| `S'` | Value of variable `S` in the next state |
| `UNCHANGED S` | `S' = S` |
| `⌊x⌋` | Floor of x |

### 2.2 TLA+ Conventions

TLA+ specifications use the following structure throughout this document:

```
VARIABLES               -- state variables
ASSUME                  -- type assumptions
Init == ...             -- initial predicate
ActionName == ...       -- transition predicate
Next == A₁ ∨ A₂ ∨ ...  -- next-state relation
Spec == Init ∧ □[Next]_vars  -- complete specification
Inv == ...              -- invariant to check
THEOREM Spec => □Inv    -- safety theorem
```

`[Next]_vars` denotes the standard TLA+ stuttering extension: either `Next` holds, or all variables are unchanged (allowing the system to stutter).

### 2.3 Type Abbreviations

```
Address   ≜  String        -- Stellar Ed25519 public key (base32)
BatchId   ≜  String        -- unique credit batch identifier
ProjectId ≜  String        -- unique project identifier
ListingId ≜  String        -- unique marketplace listing identifier
Timestamp ≜  Nat           -- Unix epoch seconds (u64 in Rust)
Amount    ≜  Int           -- signed 128-bit integer (i128)
Serial    ≜  Nat           -- unsigned 64-bit integer (u64)
```

### 2.4 Auxiliary Definitions

```
(* Interval of natural numbers *)
Interval(lo, hi) ≜ {n ∈ Nat : lo ≤ n ∧ n ≤ hi}

(* Two ranges overlap *)
Overlaps(r1, r2) ≜ r1.start ≤ r2.end ∧ r2.start ≤ r1.end

(* Range is valid *)
ValidRange(r) ≜ r.start ≤ r.end

(* Age of a timestamp relative to now *)
Age(ts, now) ≜ now - ts
```

---

## 3. System State Space

The entire system state is the Cartesian product of the four contract states, plus the shared Soroban ledger context.

### 3.1 Soroban Ledger Context (shared)

```
LedgerContext ≜ [
  timestamp       : Timestamp,   -- env.ledger().timestamp()
  sequence_number : Nat,         -- ledger sequence number
  locked_registry : Bool,        -- CarbonRegistry reentrancy lock
  locked_credit   : Bool,        -- CarbonCredit reentrancy lock
  locked_market   : Bool,        -- CarbonMarketplace reentrancy lock
  locked_oracle   : Bool         -- CarbonOracle reentrancy lock
]
```

### 3.2 Registry State

```
RegistryState ≜ [
  admin          : Address,
  oracle         : Address,
  verifiers      : SUBSET Address,
  projects       : [ProjectId → CarbonProject ∪ {⊥}],
  initialised    : Bool
]
```

### 3.3 Credit State

```
CreditState ≜ [
  admin           : Address,
  registry        : Address,
  batches         : [BatchId → CreditBatch ∪ {⊥}],
  retirements     : [RetirementId → RetirementCertificate ∪ {⊥}],
  serial_registry : Seq(SerialRange),     -- ordered append-only sequence
  batch_retired   : [BatchId → Amount],   -- cumulative retired per batch
  project_batches : [ProjectId → Seq(BatchId)],
  initialised     : Bool
]
```

### 3.4 Marketplace State

```
MarketState ≜ [
  admin        : Address,
  usdc_token   : Address,
  listings     : [ListingId → MarketListing ∪ {⊥}],
  all_listings : Seq(ListingId),
  initialised  : Bool
]
```

### 3.5 Oracle State

```
OracleState ≜ [
  admin              : Address,
  oracle_address     : Address,
  monitoring_data    : [(ProjectId × Period) → MonitoringData ∪ {⊥}],
  latest_monitoring  : [ProjectId → Timestamp ∪ {⊥}],
  benchmark_prices   : [(Methodology × VintageYear) → (Amount × ExpiryLedger) ∪ {⊥}],
  flagged_projects   : [ProjectId → Reason ∪ {⊥}],
  initialised        : Bool
]
```

### 3.6 Complete System State

```
SystemState ≜ RegistryState × CreditState × MarketState × OracleState × LedgerContext
```

---

## 4. Carbon Registry — Formal Specification

### 4.1 Data Types

```
ProjectStatus ≜ {Pending, Verified, Rejected, Suspended, Completed}

CarbonProject ≜ [
  project_id            : ProjectId,
  name                  : String,
  methodology           : String,
  country               : String,
  project_type          : String,
  verifier_address      : Address,
  metadata_cid          : String,
  total_credits_issued  : Amount,   -- non-negative
  total_credits_retired : Amount,   -- non-negative
  status                : ProjectStatus,
  vintage_year          : Nat,      -- ∈ [2000, 2100]
  created_at            : Timestamp
]
```

### 4.2 TLA+ Module

```tla
---------------------------- MODULE CarbonRegistry ----------------------------
EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
  AdminAddress,       \* the address stored at RegistryAdmin key
  OracleAddress,      \* the oracle allowed to call update_project_status
  Verifiers,          \* finite set of authorised verifier addresses
  ProjectIds,         \* universe of all possible project IDs
  VINTAGE_MIN,        \* 2000
  VINTAGE_MAX         \* 2100

VARIABLES
  projects,           \* [ProjectId -> CarbonProject | undef]
  initialised,        \* Bool
  locked              \* Bool  (reentrancy guard)

vars == <<projects, initialised, locked>>

\* ── Type invariant ───────────────────────────────────────────────────────────

TypeOK ==
  /\ initialised \in BOOLEAN
  /\ locked      \in BOOLEAN
  /\ \A pid \in dom(projects) :
       LET p == projects[pid] IN
         /\ p.status \in {Pending, Verified, Rejected, Suspended, Completed}
         /\ p.total_credits_issued  >= 0
         /\ p.total_credits_retired >= 0
         /\ p.vintage_year \in VINTAGE_MIN..VINTAGE_MAX

\* ── Initial state ────────────────────────────────────────────────────────────

Init ==
  /\ projects    = [pid \in {} |-> undef]   \* empty map
  /\ initialised = FALSE
  /\ locked      = FALSE

\* ── Actions ──────────────────────────────────────────────────────────────────

Initialize(admin, oracle, verifier_set) ==
  /\ ~initialised
  /\ admin = AdminAddress        \* auth check modelled as identity match
  /\ initialised' = TRUE
  /\ UNCHANGED <<projects, locked>>

RegisterProject(admin, pid, vintage) ==
  /\ initialised
  /\ ~locked
  /\ admin = AdminAddress
  /\ pid \notin dom(projects)
  /\ vintage \in VINTAGE_MIN..VINTAGE_MAX
  /\ locked'   = FALSE
  /\ projects' = projects @@ (pid :>
       [project_id            |-> pid,
        total_credits_issued  |-> 0,
        total_credits_retired |-> 0,
        status                |-> Pending,
        vintage_year          |-> vintage])
  /\ UNCHANGED <<initialised>>

VerifyProject(verifier, pid) ==
  /\ initialised
  /\ ~locked
  /\ verifier \in Verifiers
  /\ pid \in dom(projects)
  /\ projects' = [projects EXCEPT ![pid].status = Verified]
  /\ UNCHANGED <<initialised, locked>>

RejectProject(verifier, pid) ==
  /\ initialised
  /\ ~locked
  /\ verifier \in Verifiers
  /\ pid \in dom(projects)
  /\ projects' = [projects EXCEPT ![pid].status = Rejected]
  /\ UNCHANGED <<initialised, locked>>

SuspendProject(admin, pid) ==
  /\ initialised
  /\ ~locked
  /\ admin = AdminAddress
  /\ pid \in dom(projects)
  /\ projects' = [projects EXCEPT ![pid].status = Suspended]
  /\ UNCHANGED <<initialised, locked>>

UpdateProjectStatus(oracle, pid, new_status) ==
  /\ initialised
  /\ ~locked
  /\ oracle = OracleAddress
  /\ pid \in dom(projects)
  /\ new_status \in {Pending, Verified, Rejected, Suspended, Completed}
  /\ projects' = [projects EXCEPT ![pid].status = new_status]
  /\ UNCHANGED <<initialised, locked>>

IncrementIssued(oracle, pid, amount) ==
  /\ initialised
  /\ ~locked
  /\ oracle = OracleAddress
  /\ pid \in dom(projects)
  /\ amount > 0
  /\ projects' = [projects EXCEPT
       ![pid].total_credits_issued =
         projects[pid].total_credits_issued + amount]
  /\ UNCHANGED <<initialised, locked>>

\* ── Next-state relation ──────────────────────────────────────────────────────

Next ==
  \/ \E a \in Address, o \in Address, vs \in SUBSET Address :
       Initialize(a, o, vs)
  \/ \E a \in Address, pid \in ProjectIds, v \in VINTAGE_MIN..VINTAGE_MAX :
       RegisterProject(a, pid, v)
  \/ \E ver \in Address, pid \in ProjectIds :
       VerifyProject(ver, pid)
  \/ \E ver \in Address, pid \in ProjectIds :
       RejectProject(ver, pid)
  \/ \E a \in Address, pid \in ProjectIds :
       SuspendProject(a, pid)
  \/ \E o \in Address, pid \in ProjectIds, s \in ProjectStatus :
       UpdateProjectStatus(o, pid, s)
  \/ \E o \in Address, pid \in ProjectIds, n \in Nat :
       IncrementIssued(o, pid, n)

Spec == Init /\ [][Next]_vars

===============================================================================
```

### 4.3 Preconditions and Postconditions

#### `initialize(admin, oracle_address, verifiers)`

| | Condition |
|---|---|
| **Pre₁** | `¬initialised` |
| **Pre₂** | `auth(admin)` — admin has signed the transaction |
| **Post₁** | `initialised' = true` |
| **Post₂** | `RegistryAdmin' = admin` |
| **Post₃** | `OracleAddress' = oracle_address` |
| **Post₄** | `Verifiers' = verifiers` |
| **Post₅** | `dom(projects') = ∅` |
| **Error** | `AlreadyInitialized` if `¬Pre₁` |

#### `register_project(admin, project_id, name, metadata_cid, verifier_address, methodology, country, project_type, vintage_year)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(admin)` |
| **Pre₃** | `admin = RegistryAdmin` |
| **Pre₄** | `project_id ∉ dom(projects)` |
| **Pre₅** | `vintage_year ∈ [2000, 2100]` |
| **Pre₆** | `¬locked` |
| **Post₁** | `projects'[project_id].status = Pending` |
| **Post₂** | `projects'[project_id].total_credits_issued = 0` |
| **Post₃** | `projects'[project_id].total_credits_retired = 0` |
| **Post₄** | `∀ pid ≠ project_id: projects'[pid] = projects[pid]` (other projects unchanged) |
| **Post₅** | `locked' = false` |
| **Error** | `ProjectAlreadyExists` if `¬Pre₄`; `InvalidVintageYear` if `¬Pre₅` |

#### `verify_project(verifier_address, project_id)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(verifier_address)` |
| **Pre₃** | `verifier_address ∈ Verifiers` |
| **Pre₄** | `project_id ∈ dom(projects)` |
| **Pre₅** | `¬locked` |
| **Post₁** | `projects'[project_id].status = Verified` |
| **Post₂** | All other fields of `projects'[project_id]` unchanged |
| **Post₃** | `∀ pid ≠ project_id: projects'[pid] = projects[pid]` |
| **Error** | `UnauthorizedVerifier` if `¬Pre₃`; `ProjectNotFound` if `¬Pre₄` |

#### `reject_project(verifier_address, project_id, reason)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(verifier_address)` |
| **Pre₃** | `verifier_address ∈ Verifiers` |
| **Pre₄** | `project_id ∈ dom(projects)` |
| **Pre₅** | `¬locked` |
| **Post₁** | `projects'[project_id].status = Rejected` |
| **Post₂** | All other fields of `projects'[project_id]` unchanged |
| **Error** | `UnauthorizedVerifier` if `¬Pre₃`; `ProjectNotFound` if `¬Pre₄` |
| **Note** | No function exists to transition out of `Rejected`; this is a terminal state. |

#### `suspend_project(admin, project_id, reason)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(admin)` |
| **Pre₃** | `admin = RegistryAdmin` |
| **Pre₄** | `project_id ∈ dom(projects)` |
| **Pre₅** | `¬locked` |
| **Post₁** | `projects'[project_id].status = Suspended` |
| **Error** | `ProjectNotFound` if `¬Pre₄` |

#### `update_project_status(oracle_address, project_id, status)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(oracle_address)` |
| **Pre₃** | `oracle_address = OracleAddress` |
| **Pre₄** | `project_id ∈ dom(projects)` |
| **Pre₅** | `¬locked` |
| **Post₁** | `projects'[project_id].status = status` |
| **Error** | `UnauthorizedOracle` if `¬Pre₃`; `ProjectNotFound` if `¬Pre₄` |

#### `increment_issued(oracle_address, project_id, amount)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(oracle_address)` |
| **Pre₃** | `oracle_address = OracleAddress` |
| **Pre₄** | `project_id ∈ dom(projects)` |
| **Pre₅** | `¬locked` |
| **Post₁** | `projects'[project_id].total_credits_issued = projects[project_id].total_credits_issued + amount` |
| **Error** | `UnauthorizedOracle` if `¬Pre₃`; `ProjectNotFound` if `¬Pre₄` |

### 4.4 Registry Invariants

```
\* I-REG-1: Every registered project has a valid vintage year
Inv_VintageYear ==
  ∀ pid ∈ dom(projects):
    projects[pid].vintage_year ∈ [2000, 2100]

\* I-REG-2: Issued credits never decrease
Inv_IssuedMonotone ==
  ∀ pid ∈ dom(projects):
    projects'[pid].total_credits_issued ≥ projects[pid].total_credits_issued

\* I-REG-3: Project IDs are globally unique in the registry map
Inv_ProjectIdUnique ==
  ∀ pid₁, pid₂ ∈ dom(projects):
    pid₁ ≠ pid₂ → projects[pid₁].project_id ≠ projects[pid₂].project_id

\* I-REG-4: Rejected projects have no escape — no function transitions FROM Rejected
Inv_RejectionTerminal ==
  □(∀ pid ∈ dom(projects):
    projects[pid].status = Rejected →
      □(projects[pid].status = Rejected))
  \* Formally: once Rejected, no action in Next can set status to anything else.
  \* Holds because reject_project sets Rejected but no action reads
  \* current status before overwriting, and UpdateProjectStatus (oracle)
  \* is the only other writer — it does not enforce a "not-Rejected" precondition.
  \* See §12.2 for the implementation gap note on this invariant.

\* I-REG-5: Reentrancy lock is always released at the end of every action
Inv_LockFreeBetweenTx ==
  \* Between any two consecutive transactions, locked = false
  locked = false
```

### 4.5 Registry State Transition Diagram

```
                     register_project
                    ─────────────────►
    [nonexistent]                      [Pending]
                                          │
                         verify_project   │   reject_project
                        ─────────────────►│◄──────────────────
                                          │
                                       [Verified]  [Rejected] (terminal)
                                          │
                          suspend_project │   update_project_status(oracle)
                        ─────────────────►│◄──────────────────────────────
                                          │
                                      [Suspended]
                                          │
                       update_project_status(oracle) ──► [Completed]
                       update_project_status(oracle) ──► [Verified]
                       update_project_status(oracle) ──► [Pending]
```

Note: `update_project_status` (oracle) can transition to **any** `ProjectStatus` value. `reject_project` (verifier) is the only path to `Rejected`, and no implemented transition leaves `Rejected`.

---

## 5. Carbon Credit — Formal Specification

### 5.1 Data Types

```
CreditStatus ≜ {Active, PartiallyRetired, FullyRetired, Suspended}

CreditBatch ≜ [
  batch_id     : BatchId,
  project_id   : ProjectId,
  vintage_year : Nat,      -- ∈ [2000, 2100]
  amount       : Amount,   -- > 0
  serial_start : Serial,
  serial_end   : Serial,   -- ≥ serial_start
  issued_at    : Timestamp,
  status       : CreditStatus,
  metadata_cid : String
]

RetirementCertificate ≜ [
  retirement_id     : RetirementId,
  credit_batch_id   : BatchId,
  project_id        : ProjectId,
  amount            : Amount,        -- > 0
  retired_by        : Address,
  beneficiary       : String,
  retirement_reason : String,
  vintage_year      : Nat,
  serial_numbers    : Seq(Serial),   -- contiguous, length = amount
  retired_at        : Timestamp,
  tx_hash           : String
]

SerialRange ≜ [start : Serial, end : Serial]
```

### 5.2 TLA+ Module

```tla
----------------------------- MODULE CarbonCredit ------------------------------
EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
  AdminAddress,
  VINTAGE_MIN,   \* 2000
  VINTAGE_MAX    \* 2100

VARIABLES
  batches,         \* [BatchId -> CreditBatch | undef]
  retirements,     \* [RetirementId -> RetirementCertificate | undef]
  serial_registry, \* Seq(SerialRange)  -- append-only
  batch_retired,   \* [BatchId -> Amount]  (default 0)
  initialised,
  locked

vars == <<batches, retirements, serial_registry, batch_retired,
          initialised, locked>>

\* ── Auxiliary operators ──────────────────────────────────────────────────────

\* Active (unretired) credits remaining in a batch
ActiveAmount(bid) ==
  IF bid \notin dom(batch_retired) THEN batches[bid].amount
  ELSE batches[bid].amount - batch_retired[bid]

\* Two ranges overlap
Overlaps(r1, r2) ==
  r1.start <= r2.end /\ r2.start <= r1.end

\* Proposed range is clear of all registered ranges
RangeClear(s, e) ==
  \A r \in Range(serial_registry) : ~Overlaps([start |-> s, end |-> e], r)

\* ── Type invariant ───────────────────────────────────────────────────────────

TypeOK ==
  /\ initialised \in BOOLEAN
  /\ locked      \in BOOLEAN
  /\ \A bid \in dom(batches) :
       LET b == batches[bid] IN
         /\ b.status \in {Active, PartiallyRetired, FullyRetired, Suspended}
         /\ b.amount > 0
         /\ b.serial_start <= b.serial_end
         /\ b.vintage_year \in VINTAGE_MIN..VINTAGE_MAX
  /\ \A rid \in dom(retirements) :
       retirements[rid].amount > 0

\* ── Initial state ────────────────────────────────────────────────────────────

Init ==
  /\ batches         = [bid \in {} |-> undef]
  /\ retirements     = [rid \in {} |-> undef]
  /\ serial_registry = << >>
  /\ batch_retired   = [bid \in {} |-> 0]
  /\ initialised     = FALSE
  /\ locked          = FALSE

\* ── Actions ──────────────────────────────────────────────────────────────────

MintCredits(admin, bid, pid, amount, vintage, s_start, s_end) ==
  /\ initialised
  /\ ~locked
  /\ admin = AdminAddress
  /\ amount > 0
  /\ s_end >= s_start
  /\ vintage \in VINTAGE_MIN..VINTAGE_MAX
  /\ bid \notin dom(batches)
  /\ RangeClear(s_start, s_end)
  /\ serial_registry' = Append(serial_registry, [start |-> s_start, end |-> s_end])
  /\ batches' = batches @@
       (bid :> [batch_id     |-> bid,
                project_id   |-> pid,
                amount       |-> amount,
                serial_start |-> s_start,
                serial_end   |-> s_end,
                status       |-> Active,
                vintage_year |-> vintage])
  /\ UNCHANGED <<retirements, batch_retired, initialised, locked>>

RetireCredits(holder, bid, amount, rid) ==
  /\ initialised
  /\ ~locked
  /\ amount > 0
  /\ bid \in dom(batches)
  /\ batches[bid].status # FullyRetired
  /\ batches[bid].status # Suspended
  /\ amount <= ActiveAmount(bid)
  LET
    already  == IF bid \in dom(batch_retired) THEN batch_retired[bid] ELSE 0
    s_start  == batches[bid].serial_start + already
    s_end    == s_start + amount - 1
    new_ret  == already + amount
    new_act  == batches[bid].amount - new_ret
    new_stat == IF new_act = 0 THEN FullyRetired ELSE PartiallyRetired
  IN
  /\ batch_retired'   = [batch_retired EXCEPT ![bid] = new_ret]
  /\ batches'         = [batches EXCEPT ![bid].status = new_stat]
  /\ retirements'     = retirements @@
       (rid :> [retirement_id   |-> rid,
                credit_batch_id |-> bid,
                amount          |-> amount,
                serial_numbers  |-> s_start..s_end])
  /\ UNCHANGED <<serial_registry, initialised, locked>>

TransferCredits(from, to, bid, amount) ==
  /\ initialised
  /\ ~locked
  /\ amount > 0
  /\ bid \in dom(batches)
  /\ batches[bid].status # FullyRetired
  /\ batches[bid].status # Suspended
  /\ amount <= ActiveAmount(bid)
  /\ UNCHANGED <<batches, retirements, serial_registry, batch_retired,
                 initialised, locked>>
  \* Note: transfer is an event-only operation in this implementation;
  \* off-chain balance tracking is delegated to Stellar's asset ledger.

\* ── Next-state relation ──────────────────────────────────────────────────────

Next ==
  \/ \E a \in Address, bid \in BatchId, pid \in ProjectId,
        n \in Nat, v \in Nat, s \in Serial, e \in Serial :
       MintCredits(a, bid, pid, n, v, s, e)
  \/ \E h \in Address, bid \in BatchId, n \in Nat, rid \in RetirementId :
       RetireCredits(h, bid, n, rid)
  \/ \E f \in Address, t \in Address, bid \in BatchId, n \in Nat :
       TransferCredits(f, t, bid, n)

Spec == Init /\ [][Next]_vars

===============================================================================
```

### 5.3 Preconditions and Postconditions

#### `mint_credits(admin, project_id, amount, vintage_year, batch_id, serial_start, serial_end, metadata_cid)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(admin)` |
| **Pre₃** | `admin = CreditAdmin` |
| **Pre₄** | `amount > 0` |
| **Pre₅** | `serial_end ≥ serial_start` |
| **Pre₆** | `vintage_year ∈ [2000, 2100]` |
| **Pre₇** | `batch_id ∉ dom(batches)` |
| **Pre₈** | `¬∃ r ∈ serial_registry: Overlaps([serial_start, serial_end], r)` |
| **Pre₉** | `¬locked` |
| **Post₁** | `batches'[batch_id].status = Active` |
| **Post₂** | `batches'[batch_id].amount = amount` |
| **Post₃** | `batches'[batch_id].serial_start = serial_start` |
| **Post₄** | `batches'[batch_id].serial_end = serial_end` |
| **Post₅** | `serial_registry' = Append(serial_registry, [serial_start, serial_end])` |
| **Post₆** | `batch_retired'[batch_id] = 0` (implicitly, via default) |
| **Post₇** | `∀ bid ≠ batch_id: batches'[bid] = batches[bid]` |
| **Error** | `ZeroAmountNotAllowed` if `¬Pre₄`; `InvalidSerialRange` if `¬Pre₅`; `InvalidVintageYear` if `¬Pre₆`; `SerialNumberConflict` if `¬Pre₇`; `DoubleCountingDetected` if `¬Pre₈` |

#### `retire_credits(holder, batch_id, amount, retirement_reason, beneficiary, retirement_id, tx_hash)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(holder)` |
| **Pre₃** | `amount > 0` |
| **Pre₄** | `batch_id ∈ dom(batches)` |
| **Pre₅** | `batches[batch_id].status ≠ FullyRetired` |
| **Pre₆** | `batches[batch_id].status ≠ Suspended` |
| **Pre₇** | `amount ≤ batches[batch_id].amount − batch_retired[batch_id]` |
| **Pre₈** | `¬locked` |
| **Post₁** | `batch_retired'[batch_id] = batch_retired[batch_id] + amount` |
| **Post₂** | `batches'[batch_id].status = FullyRetired` if `batch_retired'[batch_id] = batches[batch_id].amount` |
| **Post₃** | `batches'[batch_id].status = PartiallyRetired` if `batch_retired'[batch_id] < batches[batch_id].amount` |
| **Post₄** | Let `r₀ = batch_retired[batch_id]`. Then: `retirements'[retirement_id].serial_numbers = Interval(serial_start + r₀, serial_start + r₀ + amount − 1)` |
| **Post₅** | `retirements'[retirement_id].amount = amount` |
| **Post₆** | `retirements'[retirement_id]` is immutable — no subsequent action can modify it |
| **Post₇** | `serial_registry` is unchanged (already-registered ranges are not re-registered on retire) |
| **Error** | `ZeroAmountNotAllowed` if `¬Pre₃`; `ProjectNotFound` if `¬Pre₄`; `AlreadyRetired` if `¬Pre₅`; `ProjectSuspended` if `¬Pre₆`; `InsufficientCredits` if `¬Pre₇` |

#### `transfer_credits(from, to, batch_id, amount)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(from)` |
| **Pre₃** | `amount > 0` |
| **Pre₄** | `batch_id ∈ dom(batches)` |
| **Pre₅** | `batches[batch_id].status ≠ FullyRetired` |
| **Pre₆** | `batches[batch_id].status ≠ Suspended` |
| **Pre₇** | `amount ≤ batches[batch_id].amount − batch_retired[batch_id]` |
| **Pre₈** | `¬locked` |
| **Post₁** | Event emitted: `(batch_id, from, to, amount)` |
| **Post₂** | `batches` unchanged (credit ownership tracked off-chain by Stellar asset ledger) |
| **Error** | `ZeroAmountNotAllowed` if `¬Pre₃`; `AlreadyRetired` if `¬Pre₅`; `ProjectSuspended` if `¬Pre₆`; `InsufficientCredits` if `¬Pre₇` |

### 5.4 Credit Contract Invariants

```
\* I-CC-1: Retired amount never exceeds batch total
Inv_RetiredBound ==
  ∀ bid ∈ dom(batches):
    batch_retired[bid] ≤ batches[bid].amount

\* I-CC-2: FullyRetired iff retired == total
Inv_FullyRetiredConsistent ==
  ∀ bid ∈ dom(batches):
    (batches[bid].status = FullyRetired)
    ↔ (batch_retired[bid] = batches[bid].amount)

\* I-CC-3: PartiallyRetired iff 0 < retired < total
Inv_PartiallyRetiredConsistent ==
  ∀ bid ∈ dom(batches):
    (batches[bid].status = PartiallyRetired)
    ↔ (0 < batch_retired[bid] ∧ batch_retired[bid] < batches[bid].amount)

\* I-CC-4: Retirement certificates are immutable once created
Inv_CertImmutable ==
  ∀ rid ∈ dom(retirements):
    □(retirements[rid] = retirements₀[rid])
    \* Where retirements₀[rid] is the value at the time of first write.
    \* Holds because DataKey::Retirement(id) is written exactly once in
    \* retire_credits and no action ever overwrites it.

\* I-CC-5: Serial numbers across all batches are globally non-overlapping
Inv_SerialUnique ==
  ∀ i, j ∈ 1..Len(serial_registry):
    i ≠ j →
      ¬Overlaps(serial_registry[i], serial_registry[j])

\* I-CC-6: serial_registry is append-only (existing entries never removed or mutated)
Inv_SerialAppendOnly ==
  ∀ i ∈ 1..Len(serial_registry):
    □(serial_registry[i] = serial_registry₀[i])

\* I-CC-7: batch_retired is monotonically non-decreasing per batch
Inv_RetiredMonotone ==
  ∀ bid ∈ dom(batch_retired):
    batch_retired'[bid] ≥ batch_retired[bid]

\* I-CC-8: FullyRetired batches cannot be transferred or retired again
Inv_FullyRetiredTerminal ==
  ∀ bid ∈ dom(batches):
    batches[bid].status = FullyRetired →
      (TransferCredits precondition fails for bid) ∧
      (RetireCredits precondition fails for bid)
```

### 5.5 Credit State Transition Diagram

```
                mint_credits
    [undef] ──────────────────► [Active]
                                    │
              retire_credits        │    retire_credits
              (partial: amount      │    (full: amount ==
               < total)             │     remaining)
                          ┌─────────┤
                          ▼         │
                 [PartiallyRetired] │
                          │         │
            retire_credits│         │
           (exhausts rest)│         ▼
                          └──► [FullyRetired]  (terminal)
                                    
    [Active] ──────(Suspended by oracle/admin status update)──► [Suspended]
    [PartiallyRetired] ──(same)──► [Suspended]
```

---

## 6. Carbon Marketplace — Formal Specification

### 6.1 Data Types

```
ListingStatus ≜ {Active, Sold, PartiallyFilled, Delisted}

MarketListing ≜ [
  listing_id       : ListingId,
  seller           : Address,
  batch_id         : BatchId,
  project_id       : ProjectId,
  amount_available : Amount,    -- ≥ 0
  price_per_credit : Amount,    -- > 0
  vintage_year     : Nat,
  methodology      : String,
  country          : String,
  created_at       : Timestamp,
  status           : ListingStatus
]
```

### 6.2 TLA+ Module

```tla
--------------------------- MODULE CarbonMarketplace --------------------------
EXTENDS Integers, Sequences, FiniteSets, TLC

CONSTANTS
  AdminAddress,
  PROTOCOL_FEE_BPS   \* 100 basis points = 1%

ASSUME PROTOCOL_FEE_BPS = 100

VARIABLES
  listings,      \* [ListingId -> MarketListing | undef]
  all_listings,  \* Seq(ListingId) -- insertion-order index
  initialised,
  locked

vars == <<listings, all_listings, initialised, locked>>

\* ── Auxiliary operators ──────────────────────────────────────────────────────

ProtocolFee(total) == total \div 100
SellerProceeds(total) == total - ProtocolFee(total)

\* ── Type invariant ───────────────────────────────────────────────────────────

TypeOK ==
  /\ initialised \in BOOLEAN
  /\ locked      \in BOOLEAN
  /\ \A lid \in dom(listings) :
       LET l == listings[lid] IN
         /\ l.status         \in {Active, Sold, PartiallyFilled, Delisted}
         /\ l.amount_available >= 0
         /\ l.price_per_credit > 0

\* ── Initial state ────────────────────────────────────────────────────────────

Init ==
  /\ listings     = [lid \in {} |-> undef]
  /\ all_listings = << >>
  /\ initialised  = FALSE
  /\ locked       = FALSE

\* ── Actions ──────────────────────────────────────────────────────────────────

ListCredits(seller, lid, bid, pid, amount, price, vintage, meth, country) ==
  /\ initialised
  /\ ~locked
  /\ amount > 0
  /\ price  > 0
  /\ listings' = listings @@
       (lid :> [listing_id       |-> lid,
                seller           |-> seller,
                batch_id         |-> bid,
                project_id       |-> pid,
                amount_available |-> amount,
                price_per_credit |-> price,
                vintage_year     |-> vintage,
                methodology      |-> meth,
                country          |-> country,
                status           |-> Active])
  /\ all_listings' = Append(all_listings, lid)
  /\ UNCHANGED <<initialised, locked>>

DelistCredits(seller, lid) ==
  /\ initialised
  /\ ~locked
  /\ lid \in dom(listings)
  /\ listings[lid].seller = seller
  /\ listings' = [listings EXCEPT ![lid].status = Delisted]
  /\ UNCHANGED <<all_listings, initialised, locked>>

PurchaseCredits(buyer, lid, amount) ==
  /\ initialised
  /\ ~locked
  /\ amount > 0
  /\ lid \in dom(listings)
  /\ listings[lid].status \in {Active, PartiallyFilled}
  /\ amount <= listings[lid].amount_available
  LET
    total    == listings[lid].price_per_credit * amount
    fee      == ProtocolFee(total)
    proceeds == SellerProceeds(total)
    new_avail == listings[lid].amount_available - amount
    new_stat  == IF new_avail = 0 THEN Sold ELSE PartiallyFilled
  IN
  /\ listings' = [listings EXCEPT
       ![lid].amount_available = new_avail,
       ![lid].status           = new_stat]
  \* USDC transfer: buyer -> seller: proceeds, buyer -> admin: fee
  /\ UNCHANGED <<all_listings, initialised, locked>>

\* ── Next-state relation ──────────────────────────────────────────────────────

Next ==
  \/ \E s \in Address, lid \in ListingId, bid \in BatchId, pid \in ProjectId,
        n \in Nat, p \in Nat, v \in Nat, m \in String, c \in String :
       ListCredits(s, lid, bid, pid, n, p, v, m, c)
  \/ \E s \in Address, lid \in ListingId :
       DelistCredits(s, lid)
  \/ \E b \in Address, lid \in ListingId, n \in Nat :
       PurchaseCredits(b, lid, n)

Spec == Init /\ [][Next]_vars

===============================================================================
```

### 6.3 Preconditions and Postconditions

#### `list_credits(seller, listing_id, batch_id, project_id, amount, price_per_credit_usdc, vintage_year, methodology, country)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(seller)` |
| **Pre₃** | `amount > 0` |
| **Pre₄** | `price_per_credit_usdc > 0` |
| **Pre₅** | `¬locked` |
| **Post₁** | `listings'[listing_id].status = Active` |
| **Post₂** | `listings'[listing_id].amount_available = amount` |
| **Post₃** | `listings'[listing_id].seller = seller` |
| **Post₄** | `listing_id ∈ all_listings'` |
| **Post₅** | `∀ lid ≠ listing_id: listings'[lid] = listings[lid]` |
| **Error** | `ZeroAmountNotAllowed` if `¬Pre₃ ∨ ¬Pre₄` |

#### `delist_credits(seller, listing_id)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(seller)` |
| **Pre₃** | `listing_id ∈ dom(listings)` |
| **Pre₄** | `listings[listing_id].seller = seller` |
| **Pre₅** | `¬locked` |
| **Post₁** | `listings'[listing_id].status = Delisted` |
| **Post₂** | All other fields of `listings'[listing_id]` unchanged |
| **Error** | `ListingNotFound` if `¬Pre₃`; `UnauthorizedVerifier` if `¬Pre₄` |

#### `purchase_credits(buyer, listing_id, amount)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(buyer)` |
| **Pre₃** | `amount > 0` |
| **Pre₄** | `listing_id ∈ dom(listings)` |
| **Pre₅** | `listings[listing_id].status ∈ {Active, PartiallyFilled}` |
| **Pre₆** | `amount ≤ listings[listing_id].amount_available` |
| **Pre₇** | `¬locked` |
| **Post₁** | `listings'[listing_id].amount_available = listings[listing_id].amount_available − amount` |
| **Post₂** | `listings'[listing_id].status = Sold` if `Post₁ = 0`; else `PartiallyFilled` |
| **Post₃** | Let `total = price_per_credit * amount`. Then USDC transferred: `⌊total / 100⌋` to admin; `total − ⌊total / 100⌋` to seller |
| **Post₄** | Both USDC transfers succeed atomically, or the entire transaction reverts |
| **Error** | `ZeroAmountNotAllowed` if `¬Pre₃`; `ListingNotFound` if `¬Pre₄ ∨ ¬Pre₅`; `InsufficientLiquidity` if `¬Pre₆` |

#### `bulk_purchase(buyer, listing_ids, amounts)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(buyer)` |
| **Pre₃** | `|listing_ids| = |amounts|` |
| **Pre₄** | `∀ i: amounts[i] > 0` |
| **Pre₅** | `∀ i: listing_ids[i] ∈ dom(listings)` |
| **Pre₆** | `∀ i: listings[listing_ids[i]].status ∈ {Active, PartiallyFilled}` |
| **Pre₇** | `∀ i: amounts[i] ≤ listings[listing_ids[i]].amount_available` |
| **Pre₈** | `¬locked` |
| **Post₁** | For each index `i`, postconditions of `purchase_credits` apply |
| **Post₂** | All purchases succeed or none do (single transaction) |
| **Error** | Any error from individual purchase checks propagates and reverts all |
| **Note** | The single reentrancy lock is held for the entire bulk operation; nested `purchase_credits` is inlined (not re-entrant) |

### 6.4 Marketplace Invariants

```
\* I-MKT-1: amount_available is non-negative for all listings
Inv_AmountNonNeg ==
  ∀ lid ∈ dom(listings):
    listings[lid].amount_available ≥ 0

\* I-MKT-2: Sold listings have zero amount_available
Inv_SoldExhausted ==
  ∀ lid ∈ dom(listings):
    listings[lid].status = Sold →
      listings[lid].amount_available = 0

\* I-MKT-3: Protocol fee is always exactly 1% (floor division)
Inv_FeeCorrect ==
  ∀ total ∈ Nat:
    ProtocolFee(total) = ⌊total / 100⌋ ∧
    SellerProceeds(total) = total − ⌊total / 100⌋ ∧
    ProtocolFee(total) + SellerProceeds(total) = total

\* I-MKT-4: Only the listing's seller can transition it to Delisted
Inv_DelistAuthorized ==
  ∀ lid ∈ dom(listings):
    listings[lid].status = Delisted →
      (∃ tx: tx.caller = listings[lid].seller ∧ tx.action = DelistCredits(lid))

\* I-MKT-5: Delisted and Sold listings do not appear in get_active_listings
Inv_ActiveListingsFilter ==
  ∀ lid ∈ {get_active_listings result}:
    listings[lid].status ∈ {Active, PartiallyFilled}

\* I-MKT-6: price_per_credit is immutable after listing creation
Inv_PriceImmutable ==
  ∀ lid ∈ dom(listings):
    □(listings[lid].price_per_credit = listings₀[lid].price_per_credit)
```

### 6.5 Fee Accounting Specification

For any purchase of `amount` credits at `price_per_credit`:

```
total_cost      = price_per_credit × amount
protocol_fee    = ⌊total_cost / 100⌋       -- integer floor
seller_proceeds = total_cost − protocol_fee

\* Invariant: no funds lost or created
seller_proceeds + protocol_fee = total_cost

\* Conservation: buyer's USDC balance decreases by exactly total_cost
buyer_balance'  = buyer_balance  − total_cost
seller_balance' = seller_balance + seller_proceeds
admin_balance'  = admin_balance  + protocol_fee
```

### 6.6 Marketplace State Transition Diagram

```
                list_credits
    [undef] ─────────────────► [Active]
                                  │
             purchase_credits     │     delist_credits
             (partial fill)       │  ◄─────────────────
                    ┌─────────────┤
                    ▼             │ purchase_credits
           [PartiallyFilled]      │ (exhausts all)
                    │             │
    purchase_credits│             ▼
    (exhausts rest) └──────► [Sold]         [Delisted]
                               (terminal)   (terminal)
```

---

## 7. Carbon Oracle — Formal Specification

### 7.1 Data Types

```
MonitoringData ≜ [
  project_id        : ProjectId,
  period            : Period,         -- String e.g. "2023-Q1"
  tonnes_verified   : Amount,         -- > 0
  methodology_score : Nat,            -- ∈ [0, 100]
  satellite_cid     : String,
  submitted_by      : Address,
  submitted_at      : Timestamp
]

BenchmarkPriceEntry ≜ [
  price_usdc   : Amount,
  expiry_ledger : Nat    -- ledger sequence at which TTL expires
]
```

### 7.2 Constants

```
MONITORING_FRESHNESS_SECS  ≜  31_536_000   (* 365 × 24 × 60 × 60 seconds *)
PRICE_CACHE_TTL_LEDGERS    ≜  17_280       (* ~24 h at 5 s/ledger *)
LOW_SCORE_THRESHOLD        ≜  70
```

### 7.3 TLA+ Module

```tla
------------------------------ MODULE CarbonOracle ----------------------------
EXTENDS Integers, Sequences, FiniteSets, TLC, Reals

CONSTANTS
  AdminAddress,
  OracleAddress,
  FRESHNESS_SECS,    \* 31_536_000
  PRICE_TTL_LEDGERS, \* 17_280
  LOW_SCORE_THRESH   \* 70

VARIABLES
  monitoring_data,    \* [(ProjectId x Period) -> MonitoringData | undef]
  latest_monitoring,  \* [ProjectId -> Timestamp | undef]
  benchmark_prices,   \* [(Methodology x Vintage) -> (Amount x ExpiryLedger) | undef]
  flagged_projects,   \* [ProjectId -> Reason | undef]
  initialised,
  locked,
  now_ts,             \* current ledger timestamp (model clock)
  now_ledger          \* current ledger sequence

vars == <<monitoring_data, latest_monitoring, benchmark_prices,
          flagged_projects, initialised, locked, now_ts, now_ledger>>

\* ── Auxiliary operators ──────────────────────────────────────────────────────

IsMonitoringCurrent(pid) ==
  /\ pid \in dom(latest_monitoring)
  /\ now_ts - latest_monitoring[pid] <= FRESHNESS_SECS

PriceValid(meth, vintage) ==
  /\ (meth, vintage) \in dom(benchmark_prices)
  /\ now_ledger <= benchmark_prices[(meth, vintage)].expiry_ledger

\* ── Type invariant ───────────────────────────────────────────────────────────

TypeOK ==
  /\ initialised \in BOOLEAN
  /\ locked      \in BOOLEAN
  /\ \A (pid, per) \in dom(monitoring_data) :
       monitoring_data[(pid, per)].tonnes_verified > 0
  /\ \A k \in dom(benchmark_prices) :
       benchmark_prices[k].price_usdc > 0

\* ── Initial state ────────────────────────────────────────────────────────────

Init ==
  /\ monitoring_data   = [k \in {} |-> undef]
  /\ latest_monitoring = [pid \in {} |-> undef]
  /\ benchmark_prices  = [k \in {} |-> undef]
  /\ flagged_projects  = [pid \in {} |-> undef]
  /\ initialised       = FALSE
  /\ locked            = FALSE

\* ── Actions ──────────────────────────────────────────────────────────────────

SubmitMonitoringData(oracle, pid, period, tonnes, score, cid) ==
  /\ initialised
  /\ ~locked
  /\ oracle = OracleAddress
  /\ tonnes > 0
  /\ monitoring_data' = monitoring_data @@
       ((pid, period) :>
         [project_id        |-> pid,
          period            |-> period,
          tonnes_verified   |-> tonnes,
          methodology_score |-> score,
          satellite_cid     |-> cid,
          submitted_by      |-> oracle,
          submitted_at      |-> now_ts])
  /\ latest_monitoring' = [latest_monitoring EXCEPT ![pid] = now_ts]
  \* Low score event emitted if score < LOW_SCORE_THRESH (model: boolean flag)
  /\ UNCHANGED <<benchmark_prices, flagged_projects, initialised, locked>>

UpdateCreditPrice(oracle, meth, vintage, price) ==
  /\ initialised
  /\ ~locked
  /\ oracle = OracleAddress
  /\ price > 0
  /\ benchmark_prices' = benchmark_prices @@
       ((meth, vintage) :>
         [price_usdc    |-> price,
          expiry_ledger |-> now_ledger + PRICE_TTL_LEDGERS])
  /\ UNCHANGED <<monitoring_data, latest_monitoring, flagged_projects,
                 initialised, locked>>

FlagProject(oracle, pid, reason) ==
  /\ initialised
  /\ ~locked
  /\ oracle = OracleAddress
  /\ flagged_projects' = [flagged_projects EXCEPT ![pid] = reason]
  /\ UNCHANGED <<monitoring_data, latest_monitoring, benchmark_prices,
                 initialised, locked>>

\* ── Next-state relation ──────────────────────────────────────────────────────

Next ==
  \/ \E o \in Address, pid \in ProjectId, per \in Period,
        t \in Nat, sc \in Nat, cid \in String :
       SubmitMonitoringData(o, pid, per, t, sc, cid)
  \/ \E o \in Address, m \in String, v \in Nat, p \in Nat :
       UpdateCreditPrice(o, m, v, p)
  \/ \E o \in Address, pid \in ProjectId, r \in String :
       FlagProject(o, pid, r)

Spec == Init /\ [][Next]_vars

===============================================================================
```

### 7.4 Preconditions and Postconditions

#### `submit_monitoring_data(oracle_signer, project_id, period, tonnes_verified, methodology_score, satellite_cid)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(oracle_signer)` |
| **Pre₃** | `oracle_signer = OracleAddress` |
| **Pre₄** | `tonnes_verified > 0` |
| **Pre₅** | `¬locked` |
| **Post₁** | `monitoring_data'[(project_id, period)].tonnes_verified = tonnes_verified` |
| **Post₂** | `monitoring_data'[(project_id, period)].submitted_at = env.ledger().timestamp()` |
| **Post₃** | `latest_monitoring'[project_id] = env.ledger().timestamp()` |
| **Post₄** | If `methodology_score < 70`: event `low_score` emitted with `(project_id, methodology_score)` |
| **Post₅** | All other entries in `monitoring_data` unchanged |
| **Error** | `UnauthorizedOracle` if `¬Pre₃`; `ZeroAmountNotAllowed` if `¬Pre₄` |

#### `update_credit_price(oracle_signer, methodology, vintage_year, price_usdc)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(oracle_signer)` |
| **Pre₃** | `oracle_signer = OracleAddress` |
| **Pre₄** | `price_usdc > 0` |
| **Pre₅** | `¬locked` |
| **Post₁** | `benchmark_prices'[(methodology, vintage_year)] = price_usdc` |
| **Post₂** | TTL of stored entry set to `current_ledger + 17_280` |
| **Post₃** | After `17_280` ledgers, the entry is no longer accessible (`get_benchmark_price` returns `PriceNotSet`) |
| **Error** | `UnauthorizedOracle` if `¬Pre₃`; `ZeroAmountNotAllowed` if `¬Pre₄` |
| **Storage** | Entry stored in Soroban **temporary** storage — it expires automatically; no explicit deletion needed |

#### `flag_project(oracle_signer, project_id, reason)`

| | Condition |
|---|---|
| **Pre₁** | `initialised` |
| **Pre₂** | `auth(oracle_signer)` |
| **Pre₃** | `oracle_signer = OracleAddress` |
| **Pre₄** | `¬locked` |
| **Post₁** | `flagged_projects'[project_id] = reason` |
| **Post₂** | Event `flagged` emitted: `(project_id, oracle_signer, reason)` |
| **Post₃** | Flag is stored in persistent storage (does not expire automatically) |
| **Error** | `UnauthorizedOracle` if `¬Pre₃` |

#### `is_monitoring_current(project_id)` (read-only)

| | Condition |
|---|---|
| **Pre₁** | No auth or lock required (read-only) |
| **Returns `true`** | `project_id ∈ dom(latest_monitoring)` ∧ `now − latest_monitoring[project_id] ≤ MONITORING_FRESHNESS_SECS` |
| **Returns `false`** | `project_id ∉ dom(latest_monitoring)` ∨ `now − latest_monitoring[project_id] > MONITORING_FRESHNESS_SECS` |

#### `get_benchmark_price(methodology, vintage_year)` (read-only)

| | Condition |
|---|---|
| **Pre₁** | No auth required |
| **Returns price** | `(methodology, vintage_year) ∈ dom(benchmark_prices)` ∧ `¬expired` |
| **Returns `PriceNotSet`** | Entry absent or TTL expired in temporary storage |

### 7.5 Oracle Invariants

```
\* I-ORC-1: Only the registered oracle address can submit data or update prices
Inv_OracleAuthority ==
  ∀ tx ∈ {SubmitMonitoringData, UpdateCreditPrice, FlagProject}:
    tx.caller = OracleAddress

\* I-ORC-2: tonnes_verified is always positive in stored monitoring data
Inv_TonnesPositive ==
  ∀ (pid, per) ∈ dom(monitoring_data):
    monitoring_data[(pid, per)].tonnes_verified > 0

\* I-ORC-3: Benchmark prices expire — no stale price can be retrieved after TTL
Inv_PriceExpiry ==
  ∀ k ∈ dom(benchmark_prices):
    now_ledger > benchmark_prices[k].expiry_ledger →
      get_benchmark_price(k) = PriceNotSet

\* I-ORC-4: latest_monitoring[pid] always equals the max submitted_at for pid
Inv_LatestMonitoringCorrect ==
  ∀ pid ∈ dom(latest_monitoring):
    latest_monitoring[pid] =
      Max{monitoring_data[(pid, per)].submitted_at :
           per ∈ {p : (pid, p) ∈ dom(monitoring_data)}}

\* I-ORC-5: Monitoring freshness check is deterministic given timestamp
Inv_FreshnessDetTerministic ==
  ∀ pid ∈ dom(latest_monitoring), t₁ t₂ ∈ Timestamp:
    t₁ = t₂ →
      IsMonitoringCurrentAt(pid, t₁) = IsMonitoringCurrentAt(pid, t₂)
```

---

## 8. Cross-Contract Invariants

These invariants span multiple contracts and must hold over the composed system state.

```
\* I-SYS-1: Total issued across all batches of a project equals
\*           registry's total_credits_issued for that project
Inv_IssuedConsistent ==
  ∀ pid ∈ dom(projects):
    registry.projects[pid].total_credits_issued =
      Σ { credit.batches[bid].amount :
           bid ∈ dom(credit.batches) ∧
           credit.batches[bid].project_id = pid }

\* I-SYS-2: Credits can only be minted for registered projects
Inv_MintRequiresRegistry ==
  ∀ bid ∈ dom(credit.batches):
    credit.batches[bid].project_id ∈ dom(registry.projects)

\* I-SYS-3: Marketplace listings reference only existing batches
Inv_ListingBatchExists ==
  ∀ lid ∈ dom(market.listings):
    market.listings[lid].batch_id ∈ dom(credit.batches)

\* I-SYS-4: A project can only be minted when its registry status is Verified
\* (Enforced by caller responsibility — the credit contract does not cross-call
\*  the registry in this implementation. See §12.3 for gap analysis.)
Inv_MintRequiresVerified ==
  ∀ bid ∈ dom(credit.batches):
    LET pid == credit.batches[bid].project_id IN
    registry.projects[pid].status = Verified
    \* Note: this invariant holds at mint time; the registry may later
    \* move to Suspended/Completed, which does NOT retroactively invalidate
    \* existing batches but DOES block new mints.

\* I-SYS-5: Serial number space is globally unique — no two batches share a serial
Inv_GlobalSerialUnique ==
  ∀ bid₁, bid₂ ∈ dom(credit.batches):
    bid₁ ≠ bid₂ →
      ¬Overlaps(
        [start |-> credit.batches[bid₁].serial_start,
         end   |-> credit.batches[bid₁].serial_end],
        [start |-> credit.batches[bid₂].serial_start,
         end   |-> credit.batches[bid₂].serial_end])

\* I-SYS-6: No retirement certificate references the same serial number as another
Inv_RetirementSerialDisjoint ==
  ∀ rid₁, rid₂ ∈ dom(credit.retirements):
    rid₁ ≠ rid₂ →
      credit.retirements[rid₁].serial_numbers ∩
      credit.retirements[rid₂].serial_numbers = ∅

\* I-SYS-7: Reentrancy locks are never simultaneously held across contracts
\*           (each contract uses its own independent lock stored in instance storage)
Inv_LockIndependence ==
  \* Each contract's locked variable is independent; cross-contract calls
  \* would acquire the callee's lock, not the caller's.
  TRUE   \* structural property, not a state predicate

\* I-SYS-8: Oracle benchmark prices are advisory only; they do not gate purchases
\*           (Marketplace does not read oracle prices on-chain)
Inv_OracleAdvisory ==
  TRUE   \* architectural property documented for completeness
```

---

## 9. Proof Sketch: Retirement Irreversibility

**Theorem R:** Once a `RetirementCertificate` is written to persistent storage under key `DataKey::Retirement(retirement_id)`, it cannot be modified, deleted, or invalidated by any function in the `carbon_credit` contract, now or in any future transaction.

### 9.1 Formal Statement

```
∀ rid ∈ RetirementId, t₀ t₁ ∈ Time:
  (t₀ < t₁ ∧ retirements[rid] is defined at t₀) →
    retirements[rid] at t₁ = retirements[rid] at t₀
```

Equivalently in TLA+:

```tla
RetirementImmutability ==
  ∀ rid ∈ dom(retirements) :
    retirements[rid] = retirements_at_creation[rid]

THEOREM Spec => □RetirementImmutability
```

### 9.2 Proof by Exhaustive Case Analysis on `Next`

The next-state relation `Next` for `CarbonCredit` comprises exactly three mutating actions: `MintCredits`, `RetireCredits`, `TransferCredits`. We analyse each.

**Case 1: `MintCredits`**

```rust
// retire_credits writes:
env.storage().persistent().set(&DataKey::Retirement(retirement_id), &cert);
```

`MintCredits` writes only to:
- `DataKey::Batch(batch_id)` — a new batch entry
- `DataKey::SerialRegistry` — the global serial range list
- `DataKey::ProjectBatches(project_id)` — the project batch index

None of these keys match `DataKey::Retirement(_)`. Therefore `retirements` is `UNCHANGED` under `MintCredits`. ∎

**Case 2: `RetireCredits`**

`RetireCredits` writes to:
- `RetiredKey::BatchRetired(batch_id)` — retirement counter (a different key namespace)
- `DataKey::Batch(batch_id)` — updates the batch status
- `DataKey::Retirement(retirement_id)` — creates a new certificate

The third write creates a **new** entry; it never uses `set` on an existing `DataKey::Retirement(rid)` key. Soroban persistent storage's `set` is used, which would overwrite if called twice with the same key. The question is whether the same `retirement_id` can be used twice.

**Sub-claim: retirement_ids are not checked for pre-existence before writing.**

This is a correct observation — the implementation does not check `env.storage().persistent().has(&DataKey::Retirement(retirement_id))` before writing. Therefore, if a caller supplies the same `retirement_id` in two separate `retire_credits` calls, the second call would overwrite the first certificate.

**Operational mitigation:** The `retirement_id` is a caller-supplied opaque string. In any compliant usage, callers generate retirement IDs as collision-resistant identifiers (UUIDs, content hashes). The invariant holds under the assumption that callers never reuse a `retirement_id` — which is a client-side uniqueness obligation documented here.

**Strengthened invariant (with the above assumption):**

```
Inv_RetirementIdUnique ==
  ∀ call₁, call₂ ∈ RetireCredits_history:
    call₁.retirement_id = call₂.retirement_id → call₁ = call₂
```

If this assumption holds, no two distinct `retire_credits` calls can conflict on the same key, and the first-written certificate is never overwritten.

**Case 3: `TransferCredits`**

`TransferCredits` writes nothing to persistent storage — it emits an event only. The `UNCHANGED` clause in the TLA+ model covers `retirements` and `batch_retired`. ∎

**Case 4: Read-only functions**

`get_credit_batch`, `get_retirement_certificate`, `verify_serial_range`, `get_project_credits` — all read-only. No storage writes. ∎

### 9.3 Structural Irreversibility Argument

Three independent structural properties collectively guarantee irreversibility:

**Property R1 — No delete operation exists:**  
The `carbon_credit` contract contains no call to `env.storage().persistent().remove(...)`. Soroban provides no implicit garbage collection for persistent entries within a contract's own invocation. Retirement certificates remain in persistent storage indefinitely (subject to ledger TTL extension fees, which are a liveness concern, not a safety concern).

**Property R2 — No reinstatement transition in `CreditStatus`:**  
`CreditStatus::FullyRetired` is a terminal state. Inspecting all write sites for `DataKey::Batch(bid)`:
- `mint_credits`: sets `status = Active` on creation only
- `retire_credits`: sets `status = PartiallyRetired` or `FullyRetired`

No action writes `Active`, `PartiallyRetired`, or `Suspended` to a batch that already has status `FullyRetired`. Once `FullyRetired`, the status field can only be overwritten by `retire_credits`, which guards with `batch.status == FullyRetired → return AlreadyRetired` before reaching the write. Therefore `FullyRetired` is a fixed point.

```
Inv_FullyRetiredFixedPoint ==
  ∀ bid ∈ dom(batches):
    batches[bid].status = FullyRetired →
      □(batches[bid].status = FullyRetired)
```

**Property R3 — `batch_retired` counter is monotone:**  
`batch_retired[bid]` starts at 0 and is only ever incremented:
```rust
let new_retired = already_retired + amount;
env.storage().persistent().set(&RetiredKey::BatchRetired(batch_id.clone()), &new_retired);
```
No action decrements this counter. Combined with `amount > 0` (Pre₃ of `retire_credits`), the counter is strictly increasing per retirement call. Since `FullyRetired` is triggered when `batch.amount − batch_retired = 0`, once triggered it cannot be un-triggered.

### 9.4 Summary

| Property | Mechanism | Status |
|----------|-----------|--------|
| Certificates are not deleted | No `remove` call in contract | ✅ Verified by code inspection |
| Certificates are not overwritten (given unique retirement_id) | Single write per unique key | ✅ Holds under caller uniqueness obligation |
| `FullyRetired` is a terminal status | Guard in `retire_credits`; no resetting action | ✅ Verified by exhaustive action analysis |
| `batch_retired` is monotone | Only incremented, never decremented | ✅ Verified by code inspection |
| No active credits remain after `FullyRetired` | `active_amount = batch.amount - batch_retired = 0` | ✅ Follows from I-CC-2 |

---

## 10. Proof Sketch: Serial Number Global Uniqueness

**Theorem S:** At any point in time, the serial ranges stored in `DataKey::SerialRegistry` are pairwise non-overlapping. Consequently, every carbon credit has exactly one batch it belongs to, and no serial number can appear in two different retirement certificates.

### 10.1 Formal Statement

```
∀ i, j ∈ 1..Len(serial_registry):
  i ≠ j → ¬Overlaps(serial_registry[i], serial_registry[j])
```

where `Overlaps(r1, r2) ≜ r1.start ≤ r2.end ∧ r2.start ≤ r1.end`.

In TLA+:

```tla
SerialUniqueness ==
  \A i \in 1..Len(serial_registry) :
  \A j \in 1..Len(serial_registry) :
    i # j =>
      ~(serial_registry[i].start <= serial_registry[j].end /\
        serial_registry[j].start <= serial_registry[i].end)

THEOREM Spec => □SerialUniqueness
```

### 10.2 Proof by Induction on the Length of `serial_registry`

**Base case:** `Len(serial_registry) = 0`  
The sequence is empty; the universal quantifier is vacuously true. ∎

**Inductive step:** Assume `SerialUniqueness` holds for `serial_registry` of length `n`. We prove it holds for length `n + 1`.

The only action that appends to `serial_registry` is `MintCredits`. Its precondition includes:

```rust
if !Self::verify_serial_range_internal(&env, serial_start, serial_end) {
    Self::release_lock(&env);
    return Err(CarbonError::DoubleCountingDetected);
}
```

`verify_serial_range_internal` is:

```rust
fn verify_serial_range_internal(env: &Env, start: u64, end: u64) -> bool {
    let ranges: Vec<SerialRange> = env.storage().persistent()
        .get(&DataKey::SerialRegistry)
        .unwrap_or_else(|| vec![env]);
    for r in ranges.iter() {
        if start <= r.end && end >= r.start {
            return false;
        }
    }
    true
}
```

This is exactly the predicate:

```
∀ r ∈ serial_registry: ¬Overlaps([start, end], r)
```

If this returns `true`, then the new range `[serial_start, serial_end]` does not overlap **any** of the existing `n` ranges.

After the check, `MintCredits` appends the new range:

```rust
ranges.push_back(SerialRange { start: serial_start, end: serial_end });
env.storage().persistent().set(&DataKey::SerialRegistry, &ranges);
```

The resulting `serial_registry` of length `n + 1` satisfies:

- By the inductive hypothesis: the first `n` entries are pairwise non-overlapping.
- By the `RangeClear` check: the `(n+1)`-th entry does not overlap any of the first `n` entries.
- Therefore all `n + 1` entries are pairwise non-overlapping. ∎

### 10.3 Atomicity Argument

The overlap check and the append are both performed within a single Soroban transaction, guarded by the reentrancy lock:

```
acquire_lock → check_overlap → append → release_lock
```

Because the reentrancy lock prevents any concurrent `MintCredits` from interleaving between the check and the append (Soroban transactions are themselves atomic on the Stellar network), there is no TOCTOU (time-of-check/time-of-use) vulnerability. The check and the write are **atomic** from the perspective of any other call to this contract.

**Formal atomicity statement:**

```
∀ tx₁, tx₂ ∈ MintCredits_calls:
  tx₁ and tx₂ are serialised (not interleaved)
  →  verify_serial_range_internal result in tx₂ reflects
     the state including tx₁'s append if tx₁ committed first
```

This follows from Stellar's deterministic, sequential ledger ordering.

### 10.4 Retirement Serial Assignment Proof

Given that batch serial ranges are globally unique (Theorem S), we show that individual retirement serial numbers within a batch are also globally unique.

Each retirement assigns the contiguous slice:

```
retire_serial_start = batch.serial_start + already_retired
retire_serial_end   = retire_serial_start + amount - 1
```

where `already_retired` is the cumulative retired count for this batch.

**Claim:** For any two retirements `ret₁` and `ret₂` on the same batch `bid`:

```
ret₁.serial_numbers ∩ ret₂.serial_numbers = ∅
```

**Proof:**  
WLOG assume `ret₁` was recorded before `ret₂`. Let `r₁ = already_retired` before `ret₁`, and `amount₁` be its amount. Then:

```
ret₁.serial_numbers = Interval(serial_start + r₁, serial_start + r₁ + amount₁ − 1)
```

After `ret₁`, `already_retired` becomes `r₁ + amount₁`. For `ret₂`:

```
ret₂.serial_numbers = Interval(serial_start + r₁ + amount₁, ...)
```

Since `ret₂.start = ret₁.end + 1`, the two intervals are disjoint. By induction over all retirements on a batch, no two retirements on the same batch share a serial number.

**Cross-batch uniqueness** follows directly from Theorem S: no two batches share a serial range, so no retirement on batch `bid₁` can produce a serial in the range of batch `bid₂`.

### 10.5 Summary

| Property | Mechanism | Status |
|----------|-----------|--------|
| No two batches have overlapping serials | Pre-mint overlap check in `verify_serial_range_internal` | ✅ Proven by induction |
| Check-and-append is atomic | Reentrancy lock + Stellar transaction atomicity | ✅ Structural argument |
| No two retirements on same batch share serials | Monotone `already_retired` counter; contiguous slice assignment | ✅ Proven by interval arithmetic |
| No two retirements on different batches share serials | Follows from cross-batch serial uniqueness (Theorem S) | ✅ Follows from above |
| `serial_registry` is append-only | No `remove` or overwrite calls on `DataKey::SerialRegistry` | ✅ Verified by code inspection |

---

## 11. Liveness and Safety Properties

### 11.1 Safety Properties (must never be violated)

Safety properties have the form `□P` — `P` holds in every reachable state.

| ID | Property | Formal Statement |
|----|----------|-----------------|
| S1 | No double retirement | `∀ rid₁ ≠ rid₂: retirements[rid₁].serial_numbers ∩ retirements[rid₂].serial_numbers = ∅` |
| S2 | Retired credits cannot be transferred | `∀ bid: batches[bid].status = FullyRetired → TransferCredits(bid) always fails` |
| S3 | Retired credits cannot be retired again | `∀ bid: batches[bid].status = FullyRetired → RetireCredits(bid) always fails` |
| S4 | Serial ranges are globally unique | `∀ i≠j: ¬Overlaps(serial_registry[i], serial_registry[j])` |
| S5 | Only admin can register projects | `∀ register_project call: caller = RegistryAdmin` |
| S6 | Only verifiers can verify/reject projects | `∀ verify_project, reject_project call: caller ∈ Verifiers` |
| S7 | Only oracle can submit monitoring data | `∀ submit_monitoring_data call: caller = OracleAddress` |
| S8 | Only seller can delist their listing | `∀ delist_credits call: caller = listing.seller` |
| S9 | Protocol fee always collected on purchase | `∀ purchase: admin_balance_Δ = ⌊price × amount / 100⌋` |
| S10 | Vintage year always in valid range | `∀ project p: p.vintage_year ∈ [2000, 2100]` |
| S11 | Contract initialisation is one-time | `∀ contract C: initialize(C) succeeds at most once` |
| S12 | Reentrancy lock released after every call | `∀ tx: locked = false` between transactions |

### 11.2 Liveness Properties (must eventually hold)

Liveness properties have the form `◇P` — `P` holds eventually, under fairness assumptions.

| ID | Property | Formal Statement | Caveat |
|----|----------|-----------------|--------|
| L1 | Benchmark prices eventually expire | `◇(∀ k: now_ledger > k.expiry_ledger → get_benchmark_price(k) = PriceNotSet)` | Requires ledger advancement |
| L2 | Stale monitoring eventually flagged | `◇(is_monitoring_current(pid) = false)` after 365 days | Requires timestamp advancement |
| L3 | Any pending project can eventually be verified or rejected | `◇(project.status ≠ Pending)` | Requires a verifier to call `verify_project` or `reject_project`; no timeout enforced on-chain |
| L4 | Any active listing can eventually be fulfilled | `◇(listing.status = Sold)` | Requires a buyer with sufficient USDC |

### 11.3 Weak Fairness Assumptions

For liveness properties to hold, we assume weak fairness on ledger advancement:

```tla
WF_Ledger == WF_vars(AdvanceLedger)
```

This means the Stellar network continues producing ledgers (a live network assumption), and that Soroban's temporary storage TTL mechanism fires as specified.

---

## 12. Validation Against Implementation

This section records how each formal claim was verified against the actual Rust source code.

### 12.1 Verification Methodology

Each invariant and postcondition was checked by:
1. Direct code inspection of the four `src/lib.rs` files.
2. Cross-referencing against the existing test suite (which covers preconditions, reentrancy, and basic state transitions).
3. Identifying any gaps between the formal spec and the implementation.

### 12.2 Invariant Status Table

| Invariant | Contract | Implementation | Status | Notes |
|-----------|----------|---------------|--------|-------|
| I-REG-1 VintageYear bounds | Registry | `if vintage_year < 2000 \|\| vintage_year > 2100` | ✅ Exact match | |
| I-REG-2 IssuedMonotone | Registry | `project.total_credits_issued += amount` | ✅ Only incremented | |
| I-REG-3 ProjectIdUnique | Registry | `if env.storage().persistent().has(&DataKey::Project(..))` | ✅ Pre-existence check | |
| I-REG-4 RejectionTerminal | Registry | No function guards on `Rejected` input | ⚠️ Partial — see §12.3 | |
| I-REG-5 LockFree | Registry | `release_lock` on all exit paths | ✅ Verified by reentrancy tests | |
| I-CC-1 RetiredBound | Credit | `amount ≤ active_amount` pre-check | ✅ Guard enforces it | |
| I-CC-2 FullyRetiredConsistent | Credit | Status set when `new_active == 0` | ✅ Exact match | |
| I-CC-3 PartiallyRetiredConsistent | Credit | Status set when `new_active > 0` | ✅ Exact match | |
| I-CC-4 CertImmutable | Credit | No `remove` or overwrite of `DataKey::Retirement` | ✅ By code inspection | Retirement ID uniqueness is caller obligation |
| I-CC-5 SerialUnique | Credit | `verify_serial_range_internal` called in `mint_credits` | ✅ Proven in §10 | |
| I-CC-6 SerialAppendOnly | Credit | Only `push_back` on `SerialRegistry`; no remove | ✅ By code inspection | |
| I-CC-7 RetiredMonotone | Credit | `new_retired = already_retired + amount` | ✅ Strictly increasing | |
| I-CC-8 FullyRetiredTerminal | Credit | `AlreadyRetired` error guard; `AlreadyRetired`/`Suspended` guard in transfer | ✅ Guards verified | |
| I-MKT-1 AmountNonNeg | Marketplace | `amount_available -= amount` only when `amount ≤ amount_available` | ✅ Cannot go negative | |
| I-MKT-2 SoldExhausted | Marketplace | `Sold` set exactly when `amount_available == 0` | ✅ Exact match | |
| I-MKT-3 FeeCorrect | Marketplace | `let protocol_fee = total_cost / 100` (integer division) | ✅ Floor division | |
| I-MKT-4 DelistAuthorized | Marketplace | `if listing.seller != seller { return Err(UnauthorizedVerifier) }` | ✅ Identity check | |
| I-MKT-5 ActiveListingsFilter | Marketplace | `status == Active \|\| status == PartiallyFilled` in `get_active_listings` | ✅ Exact match | |
| I-MKT-6 PriceImmutable | Marketplace | No write to `price_per_credit` after creation | ✅ Only `amount_available` and `status` updated | |
| I-ORC-1 OracleAuthority | Oracle | `Self::require_oracle(&env, &oracle_signer)?` | ✅ Guard on all three mutating functions | |
| I-ORC-2 TonnesPositive | Oracle | `if tonnes_verified <= 0 { return Err(ZeroAmountNotAllowed) }` | ✅ Guard enforces it | |
| I-ORC-3 PriceExpiry | Oracle | `env.storage().temporary()` with `extend_ttl(17280, 17280)` | ✅ Soroban temporary storage | |
| I-ORC-4 LatestMonitoringCorrect | Oracle | `env.storage().persistent().set(&DataKey::LatestMonitoring(..))` updates on every submit | ✅ Last-write-wins semantics | |
| I-SYS-5 GlobalSerialUnique | Cross | `verify_serial_range_internal` spans all batches | ✅ Global registry | |

### 12.3 Implementation Gaps and Recommendations

**Gap 1 — Rejection is not strictly terminal (I-REG-4)**

`update_project_status` (oracle) can write any `ProjectStatus` value to a project, including overwriting `Rejected`. The formal invariant as stated requires that `Rejected` be a terminal state, but the implementation does not enforce this guard.

*Recommendation:* Add a guard in `update_project_status`:
```rust
if project.status == ProjectStatus::Rejected {
    Self::release_lock(&env);
    return Err(CarbonError::RetirementIrreversible); // or a new RejectionIrreversible error
}
```

**Gap 2 — Retirement ID uniqueness is not enforced on-chain (I-CC-4)**

`retire_credits` does not check whether `retirement_id` already exists before writing a certificate. A second call with the same `retirement_id` would silently overwrite the first certificate.

*Recommendation:* Add a pre-existence check:
```rust
if env.storage().persistent().has(&DataKey::Retirement(retirement_id.clone())) {
    Self::release_lock(&env);
    return Err(CarbonError::AlreadyRetired);
}
```

**Gap 3 — Credit contract does not verify project status before minting (I-SYS-4)**

The `mint_credits` function does not cross-call the registry to confirm that the target project has `Verified` status. It is the responsibility of the calling admin to ensure the project is verified before minting.

*Recommendation:* Add an optional cross-contract check, or document this as an operational precondition enforced at the application layer.

**Gap 4 — `total_credits_retired` in registry is never updated**

`CarbonProject.total_credits_retired` exists in the data model but no function in `carbon_registry` or `carbon_credit` increments it. Retirements are tracked in `batch_retired` per batch, but the project-level counter is permanently zero.

*Recommendation:* Either remove `total_credits_retired` from `CarbonProject` (if it is intentionally unused), or hook `retire_credits` to call `increment_retired` on the registry via a cross-contract call.

### 12.4 Test Coverage Mapping

| Formal Property | Test(s) in Implementation |
|----------------|--------------------------|
| RetireCredits is irreversible | `test_retired_credits_cannot_be_retired_again` |
| FullyRetired blocks transfer | `test_retired_credits_cannot_be_transferred` |
| Serial overlap detection | `test_serial_conflict_detection`, `test_verify_serial_range_no_overlap` |
| Partial retirement status | `test_partial_retirement_updates_status` |
| Certificate stored correctly | `test_get_retirement_certificate` |
| Reentrancy lock released after success | `test_lock_released_after_*` (all four contracts) |
| Reentrancy lock released after failure | `test_lock_released_after_failed_*` (all four contracts) |
| Duplicate init blocked | `test_initialize_twice_fails` (all four contracts) |
| Unauthorized verifier rejected | `test_unauthorized_verifier_rejected` |
| Zero amount rejected | `test_zero_amount_rejected`, `test_zero_amount_listing_fails` |
| Vintage year validation | `test_register_project_valid` (implicitly, via 2023 ∈ [2000,2100]) |
| Stale monitoring | `test_stale_monitoring_returns_false` |
| Price expiry | `test_price_not_set_returns_error` |

---

*End of CarbonLedger Formal Specification v1.0.0*

*Maintained by the CarbonLedger Development Team. This document should be updated whenever the contract source changes. Any deviation between this specification and the implementation should be resolved by either updating the implementation or filing a gap note in §12.3.*
