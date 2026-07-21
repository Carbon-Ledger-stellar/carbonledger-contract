//! Stateful fuzzing harness for the CarbonLedger Soroban contracts.
//!
//! This crate is intentionally empty. The harness itself lives under `tests/`
//! so that the contract crates can be pulled in as *dev*-dependencies with
//! their `testutils` feature enabled. Declaring them as normal dependencies
//! would let Cargo's feature unification leak `soroban-sdk/testutils` into the
//! `wasm32-unknown-unknown` release build of the contracts.
//!
//! See `tests/harness/` for the generator, executor, invariants and shrinker,
//! and `FUZZING.md` at the repository root for the campaign documentation.
