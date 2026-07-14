# CarbonLedger Contracts - Quick Start

Get the Soroban contracts building and tested locally in 5 minutes.

## Prerequisites

```bash
# Check Rust is installed
rustc --version
# Expected: rustc 1.x.x

# Check Cargo is installed
cargo --version
# Expected: cargo 1.x.x
```

If not installed, install from https://rustup.rs/

## 1. Setup (2 minutes)

```bash
# Add wasm32 target for Soroban
rustup target add wasm32-unknown-unknown

# Verify target was added
rustup target list | grep wasm32
# Expected: wasm32-unknown-unknown (installed)
```

## 2. Build All Contracts (2 minutes)

```bash
# Navigate to contracts directory
cd contracts

# Build for WASM
cargo build --release --target wasm32-unknown-unknown

# Output should show:
# Compiling carbon_registry v0.1.0
# Compiling carbon_credit v0.1.0
# Compiling carbon_marketplace v0.1.0
# Compiling carbon_oracle v0.1.0
# Finished release
```

## 3. Verify Build Output (1 minute)

```bash
# Check WASM files exist
ls -lh target/wasm32-unknown-unknown/release/*.wasm

# Expected output:
# -rw-r--r--  carbon_credit.wasm         (~150 KB)
# -rw-r--r--  carbon_marketplace.wasm    (~140 KB)
# -rw-r--r--  carbon_oracle.wasm         (~130 KB)
# -rw-r--r--  carbon_registry.wasm       (~120 KB)
```

## 4. Run Tests (5 minutes)

```bash
# Run all tests
cargo test --release

# Run specific contract tests
cargo test --release -p carbon_registry
cargo test --release -p carbon_credit
cargo test --release -p carbon_marketplace
cargo test --release -p carbon_oracle

# Run with output
cargo test --release -- --nocapture
```

## 5. Code Quality Checks

```bash
# Format code
cargo fmt

# Check formatting
cargo fmt -- --check

# Run linter (clippy)
cargo clippy --target wasm32-unknown-unknown -- -D warnings
```

## Common Commands

### Build Individual Contract

```bash
cargo build --release --target wasm32-unknown-unknown -p carbon_credit
```

### Clean Build

```bash
cargo clean
cargo build --release --target wasm32-unknown-unknown
```

### Generate Documentation

```bash
cargo doc --no-deps --target wasm32-unknown-unknown --open
```

### Check for Security Issues

```bash
cargo install cargo-audit
cargo audit
```

### Optimize WASM Size

```bash
# Install wasm-opt
npm install -g wasm-opt

# Optimize a contract
wasm-opt -Oz -o carbon_credit_optimized.wasm \
  target/wasm32-unknown-unknown/release/carbon_credit.wasm
```

## Project Structure

```
contracts/
├── Cargo.toml                          # Workspace config
├── carbon_registry/
│   ├── Cargo.toml                      # Registry dependencies
│   └── src/lib.rs                      # Registry contract
├── carbon_credit/
│   ├── Cargo.toml                      # Credit dependencies
│   └── src/lib.rs                      # Credit contract
├── carbon_marketplace/
│   ├── Cargo.toml                      # Marketplace dependencies
│   └── src/lib.rs                      # Marketplace contract
├── carbon_oracle/
│   ├── Cargo.toml                      # Oracle dependencies
│   └── src/lib.rs                      # Oracle contract
├── README.md                           # Contract documentation
├── DEPLOYMENT.md                       # Deployment guide
└── QUICKSTART.md                       # This file
```

## Key Contract Functions

### Carbon Registry
- `register_project()` - Register new project
- `verify_project()` - Verify project authenticity
- `get_project()` - Retrieve project info
- `get_project_status()` - Check verification status

### Carbon Credit
- `mint_credits()` - Mint new credits
- `buy_credits()` - Atomic purchase with USDC
- `retire_credits()` - Permanently retire credits
- `get_balance()` - Check credit balance
- `get_retirement_record()` - Get retirement cert

### Carbon Marketplace
- `create_listing()` - List credits for sale
- `cancel_listing()` - Cancel active listing
- `buy_listing()` - Execute marketplace purchase
- `get_listing()` - View active listings
- `get_listing_history()` - View past trades

### Carbon Oracle
- `submit_monitoring_data()` - Submit monitoring data
- `submit_price()` - Submit benchmark pricing
- `flag_project()` - Flag suspicious projects
- `get_monitoring_data()` - Retrieve monitoring records
- `get_current_price()` - Get latest price

## Error Types

All contracts use typed error handling:

```rust
// Example error matching
match registry.verify_project(project_id) {
    Ok(()) => println!("Project verified"),
    Err(CarbonError::ProjectNotFound) => println!("Project doesn't exist"),
    Err(CarbonError::Unauthorized) => println!("No permission"),
    Err(e) => println!("Error: {:?}", e),
}
```

## Testing Patterns

### Unit Test

```rust
#[test]
fn test_register_project() {
    let env = Env::default();
    let registry = CarbonRegistryContract::new(&env);
    
    // Register project
    let project_id = registry.register_project(
        "Test Project",
        "Location",
        "Verra",
        "TICKER",
        1000,
        25000000,
    );
    
    assert_eq!(project_id, 1);
}
```

### Integration Test

```rust
#[test]
fn test_mint_and_buy() {
    let env = Env::default();
    let registry = CarbonRegistryContract::new(&env);
    let credit = CarbonCreditContract::new(&env);
    
    // Register and verify project
    let project_id = registry.register_project(...);
    registry.verify_project(project_id);
    
    // Mint credits
    credit.mint_credits(project_id, 1000);
    
    // Buy credits
    credit.buy_credits(buyer, project_id, 100);
    
    // Assert balance
    assert_eq!(credit.get_balance(buyer, project_id), 100);
}
```

## Debugging

### Enable Logging

```bash
# Run tests with debug output
RUST_LOG=debug cargo test --release -- --nocapture
```

### Check Contract State

```bash
# After deployment to testnet/mainnet
soroban contract invoke \
  --id <CONTRACT_ID> \
  -- get_admin
```

### Verify Arithmetic

```rust
// All operations use checked arithmetic
let result = amount.checked_add(quantity)?;  // Returns Result
```

## Development Tips

1. **Always use checked arithmetic** - Use `checked_add()`, `checked_mul()`, etc.
2. **Require authorization** - Call `.require_auth()` on all state-changing functions
3. **Use typed errors** - Define specific error types for each contract
4. **Test edge cases** - Zero amounts, max values, unauthorized actors
5. **Document functions** - Use `///` comments for public functions
6. **Log important events** - Log all state transitions for auditability

## Next Steps

1. **Read the full documentation**: See `README.md` for comprehensive guide
2. **Deploy to testnet**: Follow `DEPLOYMENT.md` for step-by-step instructions
3. **Explore Soroban**: Visit https://developers.stellar.org/
4. **Join the community**: Discord: https://discord.gg/stellardev

## Troubleshooting

### "Failed to compile"
- Check Rust version: `rustup update`
- Verify wasm32 target: `rustup target add wasm32-unknown-unknown`
- Clean build: `cargo clean && cargo build --release --target wasm32-unknown-unknown`

### "Tests failing"
- Run with output: `cargo test --release -- --nocapture`
- Check environment: `RUST_LOG=debug cargo test --release -- --nocapture`

### "WASM file too large"
- Use `wasm-opt` for optimization
- Review contract for unnecessary code
- Check dependencies for bloat

### "Contract already initialized"
- Verify initialize is called only once during deployment
- Check initialization guards in contract

## Support

- **Documentation**: See `README.md` and `DEPLOYMENT.md`
- **Soroban Docs**: https://developers.stellar.org/
- **Discord**: https://discord.gg/stellardev
- **GitHub Issues**: Report bugs to the repository

---

**Last Updated**: July 14, 2026

Ready to deploy? Head to `DEPLOYMENT.md` for testnet and mainnet instructions.
