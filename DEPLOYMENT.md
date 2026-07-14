# Contract Deployment Guide

Complete guide for deploying CarbonLedger smart contracts to Stellar testnet and mainnet.

## Prerequisites

### Installation

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env

# Add wasm32 target
rustup target add wasm32-unknown-unknown

# Install Soroban CLI
cargo install soroban-cli@latest

# Install Stellar CLI (optional, for additional utilities)
cargo install stellar-cli@latest
```

### Account Setup

You need three accounts for deployment:

1. **Admin Account**: Contract deployment & administration
2. **Verifier Account**: Project verification authority
3. **Oracle Account**: Price and monitoring data submission

Generate accounts:

```bash
# Generate new keypair
soroban contract invoke --help  # Verify soroban CLI is installed

# Using Stellar account generator
stellar account create --testnet
# This gives you: Public Key, Secret Key

# Keep secret keys secure (use environment variables or hardware wallets)
export ADMIN_SECRET_KEY="S..."
export VERIFIER_SECRET_KEY="S..."
export ORACLE_SECRET_KEY="S..."
```

### Funding Testnet Accounts

```bash
# Get free XLM from testnet friendbot
curl "https://friendbot.stellar.org?addr=GXXXXXX"

# Verify funding
soroban contract invoke \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  --source <PUBLIC_KEY> \
  -- get_balance
```

---

## Testnet Deployment

### Step 1: Build Contracts

```bash
# Navigate to contracts directory
cd carbonledger/contracts

# Build all contracts
cargo build --release --target wasm32-unknown-unknown

# Verify WASM files exist
ls -lh target/wasm32-unknown-unknown/release/*.wasm

# Expected output:
# carbon_registry.wasm  (~120 KB)
# carbon_credit.wasm    (~150 KB)
# carbon_marketplace.wasm (~140 KB)
# carbon_oracle.wasm    (~130 KB)
```

### Step 2: Deploy Carbon Registry (Foundation)

```bash
# Set environment
export SOROBAN_RPC_HOST=https://soroban-testnet.stellar.org
export SOROBAN_NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
export ADMIN_KEY="S..."  # Your admin account secret

# Deploy contract
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/carbon_registry.wasm \
  --source <ADMIN_PUBLIC_KEY>

# Output will show contract ID
# Save this as REGISTRY_CONTRACT_ID
export REGISTRY_CONTRACT_ID="CXXXXXX"
```

### Step 3: Initialize Carbon Registry

```bash
# Get account public keys
ADMIN_PUB="G..."
VERIFIER_PUB="G..."

# Initialize the registry
soroban contract invoke \
  --id $REGISTRY_CONTRACT_ID \
  --source $ADMIN_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- initialize \
  --admin $ADMIN_PUB \
  --verifier $VERIFIER_PUB

echo "Registry initialized: $REGISTRY_CONTRACT_ID"
```

### Step 4: Deploy Carbon Credit

```bash
# Deploy credit contract
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/carbon_credit.wasm \
  --source <ADMIN_PUBLIC_KEY>

export CREDIT_CONTRACT_ID="CXXXXXX"
```

### Step 5: Initialize Carbon Credit

```bash
# Get USDC token contract ID on testnet (example)
# Testnet USDC: CBBD47AB2EB00C3D666CB8F88B6253F4E6A0D693OP
export USDC_CONTRACT_ID="CBBD47AB2EB00C3D666CB8F88B6253F4E6A0D693OP"

soroban contract invoke \
  --id $CREDIT_CONTRACT_ID \
  --source $ADMIN_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- initialize \
  --admin $ADMIN_PUB \
  --usdc_token $USDC_CONTRACT_ID

echo "Credit contract initialized: $CREDIT_CONTRACT_ID"
```

### Step 6: Deploy Carbon Marketplace

```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/carbon_marketplace.wasm \
  --source <ADMIN_PUBLIC_KEY>

export MARKETPLACE_CONTRACT_ID="CXXXXXX"

# Initialize marketplace
soroban contract invoke \
  --id $MARKETPLACE_CONTRACT_ID \
  --source $ADMIN_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- initialize \
  --admin $ADMIN_PUB \
  --usdc_token $USDC_CONTRACT_ID

echo "Marketplace initialized: $MARKETPLACE_CONTRACT_ID"
```

### Step 7: Deploy Carbon Oracle

```bash
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/carbon_oracle.wasm \
  --source <ADMIN_PUBLIC_KEY>

export ORACLE_CONTRACT_ID="CXXXXXX"

# Initialize oracle
soroban contract invoke \
  --id $ORACLE_CONTRACT_ID \
  --source $ADMIN_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- initialize \
  --admin $ADMIN_PUB \
  --oracle_address $ORACLE_PUB

echo "Oracle initialized: $ORACLE_CONTRACT_ID"
```

### Step 8: Save Deployment Configuration

Create `.env.testnet`:

```bash
# Testnet Contract IDs
REGISTRY_CONTRACT_ID=CXXXXXX
CREDIT_CONTRACT_ID=CXXXXXX
MARKETPLACE_CONTRACT_ID=CXXXXXX
ORACLE_CONTRACT_ID=CXXXXXX

# Testnet Accounts
ADMIN_PUBLIC_KEY=GXXXXXX
VERIFIER_PUBLIC_KEY=GXXXXXX
ORACLE_PUBLIC_KEY=GXXXXXX

# USDC on Testnet
USDC_CONTRACT_ID=CBBD47AB2EB00C3D666CB8F88B6253F4E6A0D693OP

# Network
SOROBAN_RPC_HOST=https://soroban-testnet.stellar.org
NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
```

---

## Testing on Testnet

### Register a Test Project

```bash
DEVELOPER_KEY="S..."  # Developer account secret
DEVELOPER_PUB="G..."  # Developer public key

soroban contract invoke \
  --id $REGISTRY_CONTRACT_ID \
  --source $DEVELOPER_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- register_project \
  --developer $DEVELOPER_PUB \
  --name "Test Reforestation Project" \
  --location "Amazonas, Brazil" \
  --standard "Verra" \
  --credit_ticker "CO2-TEST-2026" \
  --total_supply 1000 \
  --price_per_ton 25000000

# Get project ID from response
export PROJECT_ID=1
```

### Verify the Project

```bash
VERIFIER_KEY="S..."  # Verifier account secret

soroban contract invoke \
  --id $REGISTRY_CONTRACT_ID \
  --source $VERIFIER_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- verify_project \
  --project_id $PROJECT_ID
```

### Mint and Buy Credits

```bash
# Mint 1000 credits
soroban contract invoke \
  --id $CREDIT_CONTRACT_ID \
  --source $DEVELOPER_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- mint_credits \
  --project_id $PROJECT_ID \
  --amount 1000

# Buyer purchases 100 credits
BUYER_KEY="S..."
BUYER_PUB="G..."

soroban contract invoke \
  --id $CREDIT_CONTRACT_ID \
  --source $BUYER_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- buy_credits \
  --buyer $BUYER_PUB \
  --project_id $PROJECT_ID \
  --amount 100
```

### Retire Credits

```bash
soroban contract invoke \
  --id $CREDIT_CONTRACT_ID \
  --source $BUYER_KEY \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- retire_credits \
  --buyer $BUYER_PUB \
  --benefactor "Acme Corp" \
  --project_id $PROJECT_ID \
  --amount 50
```

---

## Mainnet Deployment

### Pre-Deployment Checklist

- [ ] All tests pass on testnet
- [ ] Contracts have been audited by security firm
- [ ] Governance approval obtained
- [ ] Deployment parameters reviewed
- [ ] Multi-sig wallet configured (if using)
- [ ] Emergency procedures documented
- [ ] Monitoring and alerts configured
- [ ] Rollback plan prepared

### Mainnet Deployment Steps

```bash
# Set mainnet environment
export SOROBAN_RPC_HOST=https://soroban-mainnet.stellar.org
export NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"

# 1. Deploy Carbon Registry (with multi-sig for production)
soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/carbon_registry.wasm \
  --source <MULTISIG_ACCOUNT>

export REGISTRY_MAINNET="CXXXXXX"

# 2. Initialize on mainnet (with governance approval)
soroban contract invoke \
  --id $REGISTRY_MAINNET \
  --source <MULTISIG_SECRET> \
  --rpc-url https://soroban-mainnet.stellar.org \
  --network-passphrase "Public Global Stellar Network ; September 2015" \
  -- initialize \
  --admin <GOVERNANCE_MULTISIG> \
  --verifier <TRUSTED_VERIFIER>

# 3. Deploy and initialize remaining contracts similarly
# Credit Contract
# Marketplace Contract  
# Oracle Contract
```

### Mainnet Environment File

Create `.env.mainnet`:

```bash
# Mainnet Contract IDs
REGISTRY_CONTRACT_ID=CXXXXXX
CREDIT_CONTRACT_ID=CXXXXXX
MARKETPLACE_CONTRACT_ID=CXXXXXX
ORACLE_CONTRACT_ID=CXXXXXX

# Mainnet Accounts (Multi-Sig)
GOVERNANCE_MULTISIG=GXXXXXX
ADMIN_MULTISIG=GXXXXXX
VERIFIER_ADDRESS=GXXXXXX

# Mainnet USDC
USDC_CONTRACT_ID=CBBD47AB2EB00C3D666CB8F88B6253F4E6A0D693OP

# Network
SOROBAN_RPC_HOST=https://soroban-mainnet.stellar.org
NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
```

---

## Deployment Automation Script

Create `scripts/deploy.sh`:

```bash
#!/bin/bash
set -e

# Configuration
NETWORK=${1:-testnet}
WASM_DIR="target/wasm32-unknown-unknown/release"

echo "Deploying CarbonLedger contracts to $NETWORK..."

# Load environment
if [ "$NETWORK" = "testnet" ]; then
    export SOROBAN_RPC_HOST=https://soroban-testnet.stellar.org
    export NETWORK_PASSPHRASE="Test SDF Network ; September 2015"
    source .env.testnet
else
    export SOROBAN_RPC_HOST=https://soroban-mainnet.stellar.org
    export NETWORK_PASSPHRASE="Public Global Stellar Network ; September 2015"
    source .env.mainnet
fi

# Deploy Registry
echo "Deploying Carbon Registry..."
REGISTRY_ID=$(soroban contract deploy \
  --wasm $WASM_DIR/carbon_registry.wasm \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" | grep -o 'C[A-Za-z0-9]\{55\}')

echo "Registry deployed: $REGISTRY_ID"

# Initialize Registry
soroban contract invoke \
  --id $REGISTRY_ID \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  -- initialize \
  --admin $ADMIN_PUBLIC_KEY \
  --verifier $VERIFIER_PUBLIC_KEY

# Deploy Credit Contract
echo "Deploying Carbon Credit..."
CREDIT_ID=$(soroban contract deploy \
  --wasm $WASM_DIR/carbon_credit.wasm \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" | grep -o 'C[A-Za-z0-9]\{55\}')

echo "Credit deployed: $CREDIT_ID"

# Initialize Credit
soroban contract invoke \
  --id $CREDIT_ID \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  -- initialize \
  --admin $ADMIN_PUBLIC_KEY \
  --usdc_token $USDC_CONTRACT_ID

# Deploy Marketplace
echo "Deploying Carbon Marketplace..."
MARKETPLACE_ID=$(soroban contract deploy \
  --wasm $WASM_DIR/carbon_marketplace.wasm \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" | grep -o 'C[A-Za-z0-9]\{55\}')

echo "Marketplace deployed: $MARKETPLACE_ID"

# Initialize Marketplace
soroban contract invoke \
  --id $MARKETPLACE_ID \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  -- initialize \
  --admin $ADMIN_PUBLIC_KEY \
  --usdc_token $USDC_CONTRACT_ID

# Deploy Oracle
echo "Deploying Carbon Oracle..."
ORACLE_ID=$(soroban contract deploy \
  --wasm $WASM_DIR/carbon_oracle.wasm \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" | grep -o 'C[A-Za-z0-9]\{55\}')

echo "Oracle deployed: $ORACLE_ID"

# Initialize Oracle
soroban contract invoke \
  --id $ORACLE_ID \
  --source $ADMIN_KEY \
  --rpc-url $SOROBAN_RPC_HOST \
  --network-passphrase "$NETWORK_PASSPHRASE" \
  -- initialize \
  --admin $ADMIN_PUBLIC_KEY \
  --oracle_address $ORACLE_PUBLIC_KEY

# Save deployment results
cat > .env.$NETWORK.deployed << EOF
REGISTRY_CONTRACT_ID=$REGISTRY_ID
CREDIT_CONTRACT_ID=$CREDIT_ID
MARKETPLACE_CONTRACT_ID=$MARKETPLACE_ID
ORACLE_CONTRACT_ID=$ORACLE_ID
DEPLOYMENT_TIMESTAMP=$(date -u +"%Y-%m-%dT%H:%M:%SZ")
NETWORK=$NETWORK
EOF

echo "Deployment complete!"
echo "Contract IDs saved to .env.$NETWORK.deployed"
```

### Run Deployment

```bash
chmod +x scripts/deploy.sh

# Deploy to testnet
./scripts/deploy.sh testnet

# Deploy to mainnet (requires governance approval)
./scripts/deploy.sh mainnet
```

---

## Post-Deployment Verification

### Verify Contracts Are Initialized

```bash
# Check registry
soroban contract invoke \
  --id $REGISTRY_CONTRACT_ID \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- get_admin

# Check credit contract
soroban contract invoke \
  --id $CREDIT_CONTRACT_ID \
  --rpc-url https://soroban-testnet.stellar.org \
  --network-passphrase "Test SDF Network ; September 2015" \
  -- get_usdc_token
```

### Health Check

```bash
# Test basic operations
soroban contract invoke \
  --id $REGISTRY_CONTRACT_ID \
  --source $DEVELOPER_KEY \
  -- register_project \
  --developer $DEVELOPER_PUB \
  --name "Health Check Project" \
  --location "Test Location" \
  --standard "Verra" \
  --credit_ticker "HC-TEST" \
  --total_supply 100 \
  --price_per_ton 10000000

echo "✅ Registry functional"
```

### Monitor Contract Calls

```bash
# View recent contract invocations
curl -s "https://soroban-testnet.stellar.org/events?contract=$REGISTRY_CONTRACT_ID" | jq

# Monitor USDC transfers related to marketplace
curl -s "https://soroban-testnet.stellar.org/events?contract=$USDC_CONTRACT_ID" | jq
```

---

## Rollback Procedure

If critical issues are discovered post-deployment:

### Step 1: Pause Operations

```bash
# Suspend all projects to halt trading
soroban contract invoke \
  --id $REGISTRY_CONTRACT_ID \
  --source $ADMIN_KEY \
  -- pause_all_projects
```

### Step 2: Investigate

- Review transaction history on Stellar Expert
- Check contract logs
- Analyze failure patterns

### Step 3: Redeploy (if necessary)

- Deploy new contract code
- Migrate state from old contract (if possible)
- Resume operations

---

## Monitoring & Alerting

### Key Metrics to Monitor

- **Contract Invocation Rate**: Spike indicates unusual activity
- **USDC Transfer Volume**: Track marketplace activity
- **Error Rates**: Monitor failed transactions
- **Gas Costs**: Track fee trends

### Set Up Alerts

```bash
# Alert on high error rate (>5% in 1 hour)
# Alert on unusual USDC volume
# Alert on contract upgrade proposals
```

---

## Support & Troubleshooting

### Common Issues

**"Contract not found"**
- Verify contract ID is correct
- Check network RPC URL
- Ensure contract is deployed to that network

**"Unauthorized"**
- Verify source account has authorization
- Check account funding
- Confirm account signature

**"Overflow Error"**
- Amount exceeds safe arithmetic bounds
- Check input values
- Implement input validation on frontend

### Get Help

- **Stellar Developers**: https://discord.gg/stellardev
- **Soroban Docs**: https://developers.stellar.org/
- **GitHub Issues**: https://github.com/Carbon-Ledger-stellar/

---

**Last Updated**: July 14, 2026
