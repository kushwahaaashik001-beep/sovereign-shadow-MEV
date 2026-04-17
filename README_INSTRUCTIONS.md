# 🥷 THE SOVEREIGN SHADOW: MISSION CONTROL v2.0 (Hybrid High-Efficiency)

Ye aapka core architectural blueprint hai. Humne P2P/Sentry logic ko delete karke **Private Relay (Flashbots/MEV-Blocker)** switch kar liya hai. 
Target: **$50-$100 Daily Micro-Profits** via Meme Token Cycles on Base Mainnet.

## 🚀 Tech Stack Breakdown
- **Language:** Rust (Stable/Nightly) for ultra-nanosecond math.
- **Provider:** Alloy (High-performance abstraction).
- **Simulation:** REVM 14.0 (In-process EVM for instant honeypot detection).
- **Execution:** Yul-optimized ShadowBot.sol + Private Bundles (Flashbots).
- **Infrastructure:** Hugging Face Space (Single Instance, 16GB RAM).

## 🛠️ Phase 1: Environment Setup (No Node Required)

1. **Install Rust Compiler:**
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Configure Secrets (.env):**
   `.env` file ko open karein aur ye values set karein:
   ```env
   # READ: Hybrid RPC (Alchemy/Quicknode)
   SHADOW_RPC_URL=https://base-mainnet.g.alchemy.com/v2/YOUR_KEY
   SHADOW_WS_URL=wss://base-mainnet.g.alchemy.com/v2/YOUR_KEY
   
   # WRITE: Private Relay Keys
   PRIVATE_KEY=0xYOUR_EXECUTION_WALLET_KEY
   RELAY_SIGNING_KEY=0xYOUR_FLASHBOTS_ID_KEY

   # CONTRACTS
   CHAIN=base
   EXECUTOR_ADDRESS=0x... # Deployed ShadowBot Address
   ```

## 🛡️ Phase 2: Deploy The Yul-Optimized Ghost

Aapka ShadowBot contract (Yul optimization ke saath) deploy karein:
```bash
forge create src/ShadowBot.sol:ShadowBot \
  --rpc-url https://mainnet.base.org \
  --private-key 0xYOUR_PRIVATE_KEY \
  --broadcast
```
*Deploy hone ke baad `Deployed to: 0x...` wala address copy karke `.env` ki `EXECUTOR_ADDRESS` field mein daal dein.*

## Phase 3: Bot Execution

1. **Compile & Check (Safety First):**
   ```bash
   cargo check
   ```

2. **Run Beast Mode (Production):**
   ```bash
   cargo run --release
   ```

## Phase 4: Monitoring

- **Telegram Dashboard:** Aapka phone har trade aur 24h profit harvest ka notification dikhayega.
- **Logs:** Terminal mein `[SIMULATION SUCCESS]` ka wait karein.

## Phase 5: Sovereign Survival Rules

1. **🛡️ 3x Gas Rule:** Sirf wahi trade fire karein jahan `Expected Profit > 3 * (L1_Data_Fee + L2_Execution_Fee)`.
2. **🥷 Private Bundles Only:** Kabhi भी trade public mempool mein mat bhejo. Hamesha Flashbots ya Base PBH (Private Bundle Handler) use karo.
3. **⚛️ Atomic Revert:** Zero-Loss Shield hamesha ON rakhein. Simulation mein `top_sim` profit से `MAX_BRANCH_LOSS_BPS` (10%) से ज़्यादा drop होते ही trade automatically kill हो जाएगी।
4. **Unified Intelligence:** Mempool scanning aur Execution ab ek hi process mein hain for ultra-low latency.

---
**Lead Architect Note:** Aapka ₹200 ka budget sirf gas ke liye hai. Bot automatically Aave aur Balancer se Flash Loans lega, isliye liquidity ki tension na lein.