# The Sovereign Shadow — MEV Engine
---
title: The Sovereign Shadow
emoji: ⚡
colorFrom: black
colorTo: gray
sdk: docker
pinned: false
---

## Tech Status: ✅ 100% READY

---

## Architecture (All Pillars Active)

| Pillar | Name | Status |
|--------|------|--------|
| A | Mempool Surveillance | ✅ txpool_content, multi-worker |
| B | State Mirror + REVM | ✅ multicall sync, bytecode cache |
| C | Pathfinding (HotGraph) | ✅ DFS cycle finder, DashMap |
| D | GSS Math Engine | ✅ Golden Section Search |
| E | Bundle Builder | ✅ Flashbots + multi-relay |
| F | Neural Memory | ✅ DashMap O(1), prune stale |
| G | Ghost Protocol | ✅ private bundles, stealth jitter |
| H | Predator Detection | ✅ SELFDESTRUCT scan |
| I | Adaptive Bidding | ✅ network heat aware |
| J | Inventory Manager | ✅ dust sweep to WETH |
| K | Simulation Branching | ✅ REVM local sim |
| L | Poison Token Filter | ✅ bytecode analysis |
| M | Flash Loans | ✅ Aave V3 + Balancer V2 |
| N | Zero-Loss Shield | ✅ on-chain require() |
| O | L2 Specialist | ✅ Base, Arb, Sepolia |
| P | Auto-Compounding | ✅ 70% excess to vault |
| Q | Bootstrap Protocol | ✅ zero capital start |
| R | Shadow Simulation | ✅ gas price guard |
| S | Intent Solver | ✅ UniswapX ready |
| T | Anti-Drift Guardian | ✅ stale block check |
| U | Zero-Cost Infra | ✅ 1GB RAM optimized |
| V | Veto Protocol | ✅ GLOBAL_PAUSE atomic |
| W | Wash-Trap Radar | ✅ same token filter |
| X | X-Ray Scanner | ✅ opcode analysis |
| Y | Yield Scavenger | ✅ micro-profit sweep |
| Z | Factory Scanner | ✅ auto new pool detect |

---

## Step 1: Install Dependencies

```bash
# Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Foundry (for Solidity testing)
curl -L https://foundry.paradigm.xyz | bash
foundryup
```

---

## Step 2: Setup Environment

```bash
cp .env.example .env
# Edit .env — add your RPC URL and private key
```

`.env` minimum required:
```env
SHADOW_WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY
SHADOW_RPC_URL=https://base-mainnet.g.alchemy.com/v2/YOUR_KEY
SHADOW_PRIVATE_KEY=0x...your_key...
CHAIN=base
EXECUTOR_ADDRESS=0x0000000000000000000000000000000000000000
```

---

## Step 3: Local Solidity Testing (Foundry Fork Test)

```bash
# Install OpenZeppelin + forge-std
forge install OpenZeppelin/openzeppelin-contracts --no-commit
forge install foundry-rs/forge-std --no-commit

# Run fork tests against mainnet
forge test --fork-url $SHADOW_RPC_URL -vvv

# Expected output:
# [PASS] test_onlyOwner()
# [PASS] test_zeroLossShield_reverts()
# [PASS] test_directArbitrage_noLoan()
# [PASS] test_withdraw()
# [PASS] test_withdrawETH()
# [PASS] test_pathEncoding()
```

---

## Step 4: Deploy Executor Contract

### Base Mainnet
```bash
forge create contracts/Executor.sol:Executor \
  --rpc-url $SHADOW_RPC_URL \
  --private-key $SHADOW_PRIVATE_KEY \
  --constructor-args \
    0xA238Dd80C259a72e81d7e4664a9801593F98d1c5 \
    0xBA12222222228d8Ba445958a75a0704d566BF2C8
```

### Ethereum Mainnet
```bash
forge create contracts/Executor.sol:Executor \
  --rpc-url $SHADOW_RPC_URL \
  --private-key $SHADOW_PRIVATE_KEY \
  --constructor-args \
    0x87870Bca3F3fD6335C3F4ce8392D69350B4fA4E2 \
    0xBA12222222228d8Ba445958a75a0704d566BF2C8
```

After deploy, copy the address and set in `.env`:
```env
EXECUTOR_ADDRESS=0xYOUR_DEPLOYED_ADDRESS
```

---

## Step 5: Run the Bot

```bash
# Development (with logs)
RUST_LOG=info cargo run

# Production (optimized)
cargo build --release
RUST_LOG=warn ./target/release/the-sovereign-shadow
```

---

## Zero-Capital Flow

```
Bot starts
  → Detects swap in mempool (Pillar A)
  → Finds arbitrage cycle (Pillar C)
  → Calculates optimal input via GSS (Pillar D)
  → Simulates locally via REVM (Pillar B/K)
  → If profitable:
      → Borrows from Aave/Balancer (0 capital needed)
      → Executes arbitrage atomically
      → Repays loan + premium
      → Zero-Loss Shield: if not profitable → REVERT (0 gas lost)
      → Profit stays in Executor contract
  → Withdraw profits via withdraw() function
```

---

## Security

- Only owner (deployer) can call `executeArbitrage` and `withdraw`
- Zero-Loss Shield: `require(currentAmount >= amount + premium, "ZLS: not profitable")`
- Circuit Breaker: auto-pause after 5 consecutive failures
- GLOBAL_PAUSE: instant kill-switch via atomic bool
