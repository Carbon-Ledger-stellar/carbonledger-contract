<!--
Thanks for contributing to CarbonLedger! Please fill in the sections below and
tick the checklists that apply. Delete sections that are not relevant.
-->

## Summary

<!-- What does this PR do and why? Link the issue it addresses. -->

Closes #

## Type of change

- [ ] Bug fix
- [ ] New feature
- [ ] Security / hardening
- [ ] Refactor / cleanup
- [ ] Documentation
- [ ] Tests

## Arithmetic safety checklist

<!--
Required for any change that touches numeric logic in `carbon_*/src/lib.rs`
(amounts, prices, fees, serial numbers, timestamps, counters). Overflow/underflow
in a smart contract is a security issue: unchecked math either wraps silently or
traps the whole transaction with no diagnostic. If this PR contains no arithmetic
changes, tick "N/A" and skip the rest.
-->

- [ ] N/A — this PR changes no arithmetic

If arithmetic is touched:

- [ ] Every `+`, `-`, `*` uses a `checked_add` / `checked_sub` / `checked_mul`
      (or a documented `saturating_*`) — no bare operators on untrusted values
- [ ] Overflow/underflow returns `CarbonError::ArithmeticOverflow` (a typed error),
      never a wrapped value and never an unhandled trap
- [ ] The reentrancy lock is released on every new error path (use the `checked!`
      macro inside lock-held sections)
- [ ] Intermediate expressions cannot overflow even when the final result fits
      (e.g. compute `start + (n - 1)`, not `start + n - 1`)
- [ ] Input-range assumptions are documented (see the module-level "Input-range
      assumptions" comment in each contract; amounts are assumed `< 1e15`)
- [ ] Boundary tests added: zero, negative (rejected by type/guard), typical range,
      and the relevant `MAX` boundary (`i128::MAX` / `u64::MAX`)
- [ ] Panic/overflow behavior is documented for each new overflow scenario

## Testing

<!-- How was this verified? Paste relevant command output. -->

- [ ] `cargo test -p carbon_credit -p carbon_marketplace -p carbon_oracle -p carbon_registry` passes
- [ ] `cargo build --release --target wasm32-unknown-unknown` succeeds for all contracts
      (release profile has `overflow-checks = true`)

## Notes for reviewers

<!-- Anything else the reviewer should know: design tradeoffs, follow-ups, etc. -->
