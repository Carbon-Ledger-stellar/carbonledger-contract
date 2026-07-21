# Stateful fuzzing

`carbon_fuzz` drives **all four CarbonLedger contracts** through randomly
generated lifecycle sequences and checks, after every operation, that each
contract agrees with an independent shadow model and that a set of safety
invariants still hold. On failure it shrinks the sequence to a minimal
reproducer.

The campaign covers the full cross-contract lifecycle:

```
register → verify → (monitor / price / flag) → mint → transfer → list → purchase → retire
   registry        registry     oracle         credit   credit    market  market    credit
```

## Running

```bash
cargo test -p carbon_fuzz                     # default campaign, ~15s (debug)
FUZZ_SEQS=2000 cargo test -p carbon_fuzz      # a wider debug campaign
```

The toolchain is pinned in `rust-toolchain.toml`. It must be — `ethnum 1.5.0`,
pulled in transitively by `soroban-env-common 20.3.0`, fails to compile on
rustc ≳1.87 with `E0512`. Use the pinned toolchain rather than working around it.

### The 50k soak

The acceptance target — 50,000+ sequences in under 30 minutes — is the job of
the **release** soak. Release is not optional: the Soroban test host is roughly
two orders of magnitude slower unoptimized (a fresh `Env` alone is ~20 ms in
debug versus microseconds in release), and the whole budget is spent on host
invocations, so only a release build has any hope of the target. The design
holds per-operation cost to the single invocation the operation itself requires
(see [Throughput](#throughput)); tune `FUZZ_SEQS` to your runner.

```bash
FUZZ_SEQS=50000 FUZZ_DEEP=0 cargo test --release -p carbon_fuzz --test harness campaign
```

`FUZZ_DEEP=0` turns off per-world contract-state reconciliation, which costs a
getter invocation per entity. With it off, the only invocation per operation is
the operation itself, which is what keeps the soak inside the time budget (see
[Throughput](#throughput)). The narrower default campaign keeps `FUZZ_DEEP` on
so it reconciles contract state too.

## Reproducing a failure

Every run is a pure function of its seed: the harness uses a SplitMix64 PRNG
(`tests/harness/rng.rs`) and never touches the system clock or thread RNG.

**On failure the campaign prints a minimal reproducer.** It catches the failing
sequence, runs delta debugging (`tests/harness/shrink.rs`) to drop every
operation that is not needed to still trigger the failure, and prints the
handful that remain:

```
──────────────── fuzz failure ────────────────
seed=0  sequence=4137
panic: retire on b4137_2: model predicted Ok(()) but contract returned Err(InsufficientCredits)
raw sequence: 12 ops
minimal reproducer: 2 ops
  [ 0] Mint { seq: 4137, actor: 0, project: 0, batch: 2, amount: 50, ... }
  [ 1] Retire { seq: 4137, actor: 3, batch: 2, amount: 51, ... }

replay this campaign with:
  FUZZ_SEED=0 cargo test --release -p carbon_fuzz --test harness replay -- --nocapture
───────────────────────────────────────────────
```

The whole campaign replays deterministically from its seed:

```bash
FUZZ_SEED=0 cargo test -p carbon_fuzz --test harness replay -- --nocapture
```

## How it works

| Module | Role |
|---|---|
| `rng.rs` | Deterministic SplitMix64. Seeds are plain sequence indices. |
| `model.rs` | Shadow model — one plain-Rust sub-model per contract. |
| `ops.rs` | Precondition-aware operation generator. |
| `exec.rs` | Runs each op against contract *and* model, compares both. |
| `invariants.rs` | Safety properties + cross-contract gap reporting. |
| `shrink.rs` | Delta debugging to a minimal reproducer. |

Things worth knowing before extending it.

**The model is written from documented behaviour, not from the contract
source.** That independence is the entire point — a model derived by reading the
implementation would reproduce its bugs and agree with it perfectly. When model
and contract disagree, *either* may be wrong, and both are worth investigating.

**Check ordering is load-bearing.** Every mutating entry point validates in a
fixed sequence and returns the first failure. Each sub-model must reject in the
same order or it will predict the right *outcome* with the wrong *error code*.

**Generation is precondition-aware.** The generator tracks what it has already
created (registered projects, minted batches, live listings) and biases toward
operations whose preconditions hold — you can't retire before issuance — so the
deep lifecycle actually gets exercised. It still injects invalid operations at a
low rate (wrong caller, absent id, zero amount, overlapping serials) so every
rejection path is covered too. Ids are drawn from small pools so collisions —
duplicate ids, serial overlaps, over-retirement — are common rather than
astronomically rare.

### Throughput

Real Soroban host invocations are the wall: each is single-digit milliseconds in
release, and a single `Env` cannot absorb unbounded invocations (the test host
accumulates auth/event state and eventually aborts). Two design choices keep
50k sequences affordable:

- **One world is reused across `WORLD_BATCH` sequences**, with every id
  namespaced by sequence index so the sequences stay independent. This amortizes
  the ~14 ms cost of building a world (registering four contracts, generating
  addresses, funding the USDC token) while bounding both the per-`Env` invocation
  count and the growth of the contracts' global lists (`SerialRegistry`,
  `AllListings`), which are re-serialized in full on every write.
- **Per-operation checking reads only the model** (zero extra invocations); the
  operation's own call is the sole invocation, and its result is compared to the
  model's prediction. `retire` additionally validates the returned certificate
  for free. Contract-state reconciliation is reserved for `deep_reconcile`, run
  once per world in the default campaign and skipped in the soak.

### Budget

Each operation resets to the *default* host budget, because on-chain each
invocation is its own transaction with its own budget — accumulating across a
sequence is a harness artifact. It is reset to the default rather than to
unlimited so that a single operation exhausting its budget is still detectable.
This matters: `retire_credits` builds a `Vec<u64>` holding one entry per credit
retired, so a large enough retirement is an out-of-budget trap rather than a
returned error — which is why retirement amounts are bounded.

## Invariants

Model-only (checked after **every** operation, so a failure pins the exact op):

- No two registered serial ranges overlap.
- Retired never exceeds minted; active balance never negative.
- Retirement certificates carve a batch's serials into disjoint, contiguous
  spans; certificate amounts sum to exactly the retired balance.
- A listing's available amount stays in `[0, original]`, `Sold` iff drained.
- USDC settlement is conservative — the total never changes, no balance negative.

Deep, re-reading contract state (once per world, default campaign only):

- Every model project / batch / listing status matches the contract's.
- Every actor's USDC balance matches the model (fees and proceeds included).
- The contract reports every registered serial range as unavailable.

## Known gaps

The harness surfaces real defects rather than asserting them away. These are
reachable precisely because **the four contracts make no cross-contract calls to
one another** — they are independent state machines linked only by string ids
(the sole genuine cross-contract edge is marketplace → USDC token).
`invariants::report_gaps` detects them from model state.

**Closed: serial range not reconciled with amount.** An earlier version of this
harness documented an open gap — `mint_credits` did not check `amount` against
the declared serial range, so a batch minted with `amount = 100` over serials
`1..=5` let retirement walk serials past `serial_end` into numbers belonging to
other batches, defeating the double-counting defence. Upstream closed it by
requiring `serial_end - serial_start + 1 == amount` at mint. The harness caught
the behaviour change on the first run against the updated contracts;
`regression_mint_requires_serial_range_matches_amount` now pins the fix shut.

**Open — `known_gap_no_cross_contract_consistency`.** Nothing links the contracts:

- `mint_credits` never consults the registry, so credits can be minted for a
  project that was never registered, never verified, rejected, or suspended.
- `list_credits` never validates against `carbon_credit`, so a listing can offer
  more credits than were ever minted — or name a batch that does not exist.
- `purchase_credits` moves USDC but never moves credit ownership; a buyer pays
  and the credit batch is left untouched.
- `retire_credits` has no ownership check; any authenticated address may retire
  any batch.

When any of these is fixed, the corresponding test should be inverted into a
regression test proving the gap is closed.

## Coverage

Modelled: `carbon_registry` (register / verify / reject / suspend),
`carbon_credit` (mint / retire / transfer), `carbon_marketplace` (list / delist
/ purchase), `carbon_oracle` (monitoring / price / flag), and USDC settlement.

Out of scope (per the issue): fuzzing the external USDC token contract's
internals, and performance tuning of the fuzzer itself.
