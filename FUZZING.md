# Stateful fuzzing

`carbon_fuzz` drives the contracts through randomly generated operation
sequences and checks, after every operation, that the contract agrees with an
independent shadow model and that a set of safety invariants still hold.

The current campaign covers **`carbon_credit`**. The other three contracts are
not yet modelled — see [Coverage](#coverage).

## Running

```bash
cargo test -p carbon_fuzz                 # default campaign, ~30s
FUZZ_SEEDS=500 cargo test -p carbon_fuzz  # a wider campaign
```

The toolchain is pinned in `rust-toolchain.toml`. It must be — `ethnum 1.5.0`,
pulled in transitively by `soroban-env-common 20.3.0`, fails to compile on
rustc ≳1.87 with `E0512`. Use the pinned toolchain rather than working around
it locally.

## Reproducing a failure

Every run is a pure function of its seed: the harness uses a SplitMix64 PRNG
(`tests/harness/rng.rs`) and never touches the system clock or thread RNG. A
failure reports the seed, so:

```bash
FUZZ_SEED=137 cargo test -p carbon_fuzz --test harness replay -- --nocapture
```

Failures name the operation and the batch involved, e.g.

```
mint on batch-5: model predicted Err(DoubleCountingDetected)
                 but contract returned Err(InvalidVintageYear)
```

## How it works

| Module | Role |
|---|---|
| `rng.rs` | Deterministic SplitMix64. Seeds are plain sequence indices. |
| `model.rs` | Shadow model — a plain-Rust reimplementation of intended behaviour. |
| `ops.rs` | Operation generator. |
| `exec.rs` | Runs each op against contract *and* model, compares both. |
| `invariants.rs` | Safety properties checked against contract state. |

Two things are worth knowing before extending it.

**The model is written from documented behaviour, not from the contract
source.** That independence is the entire point — a model derived by reading
the implementation would reproduce its bugs and agree with it perfectly. When
model and contract disagree, *either* may be wrong, and both outcomes are
worth investigating.

**Check ordering is load-bearing.** `mint_credits` validates in a fixed
sequence and returns the first failure. The model must reject in the same
order or it will predict the right *outcome* with the wrong *error code*.

The generator draws ids from deliberately small pools (6 batches, 3 projects)
and cramped serial ranges. Uniformly random arguments would essentially never
collide, so duplicate-id and serial-overlap rejection would never be exercised.

### Budget

Each operation resets to the *default* host budget, because on-chain each
invocation is its own transaction with its own budget — accumulating across a
60-op sequence is a harness artifact. It is reset to the default rather than to
unlimited so that a single operation exhausting its budget is still detectable.
This matters: `retire_credits` builds a `Vec<u64>` holding one entry per credit
retired, so a large enough retirement is an out-of-budget trap rather than a
returned error.

## Invariants

Split by cost. Cheap model-only checks run after **every** operation, so a
failure pins the exact culprit. Checks that re-read contract state cost a full
invocation each and run once per sequence.

Per operation:
- No two registered serial ranges overlap.
- Retired never exceeds minted; active balance never negative.
- Retirement certificates carve a batch's serials into disjoint, contiguous spans.
- Certificate amounts sum to exactly the retired balance.

Per sequence:
- The contract reports every registered serial range as unavailable.
- `FullyRetired` is terminal, in both model and contract.
- Status is a pure function of the retired amount.
- Every minted batch is reachable via its project index.

## Known gap: serial ranges are not reconciled with amount

`known_gap_amount_not_reconciled_with_serial_range` documents a real defect
rather than asserting it away.

`mint_credits` never checks `amount` against the width of the declared serial
range, so a batch can be minted with `amount = 100` over serials `1..=5`.
Retirement then allocates serials sequentially *by count* from `serial_start`,
walking past `serial_end` into serials that were never registered in the global
registry — and which may already belong to another batch.

The global overlap check guards range *declarations*, not the serials
retirement actually hands out. Double-counting is therefore reachable despite
that defence, which is the property the registry exists to guarantee.

The fix is to require `amount == serial_end - serial_start + 1` at mint, or to
bound retirement at `serial_end`. When that lands, the test fails and should be
inverted into a regression test proving the gap is closed.

## Coverage

Modelled: `carbon_credit` (mint, retire, transfer).

Not yet modelled: `carbon_registry`, `carbon_marketplace`, `carbon_oracle`.

Worth knowing when extending: **the four contracts make no cross-contract calls
to each other.** They are independent state machines linked only by string ids.
`carbon_credit` stores a registry address but never reads it, so minting does
not check that a project exists or is verified; `carbon_marketplace` does not
know `carbon_credit` exists, so a listing is never validated against a real
batch. The only genuine cross-contract edge is marketplace → USDC token.

Cross-contract consistency is therefore *not* an invariant the code upholds
today. Asserting it in a future campaign will produce findings, not green
tests — which is a reason to do it, but plan for the results.
