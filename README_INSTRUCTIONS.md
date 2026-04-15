# 🥷 THE SOVEREIGN SHADOW: DEPLOYMENT MANUAL

Ye aapka exact command-line guide hai Base Mainnet par bot ko zero error ke saath start karne ke liye.

## Phase 1: Environment Setup

1. **Install Rust Compiler:**
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
   ```

2. **Install Foundry (For Contract Deployment):**
   ```bash
   curl -L https://foundry.paradigm.xyz | bash
   foundryup
   ```

3. **Configure Environment Variables:**
   `.env` file ko open karein aur ye values set karein:
   ```env
   # Use multiple keys to avoid 429 V2 Rate Limits
   SHADOW_RPC_URL_1=https://base-mainnet.g.alchemy.com/v2/KEY_1
   SHADOW_RPC_URL_2=https://base-mainnet.g.alchemy.com/v2/KEY_2
   SHADOW_WS_URL_1=wss://base-mainnet.g.alchemy.com/v2/KEY_1
   SHADOW_PRIVATE_KEY=0xYOUR_WALLET_PRIVATE_KEY
   CHAIN=base
   EXECUTOR_ADDRESS=0x0000000000000000000000000000000000000000 # Pehle deployment ke baad badlein
   TELEGRAM_BOT_TOKEN=YOUR_BOT_TOKEN
   TELEGRAM_CHAT_ID=YOUR_CHAT_ID
   ```

## Phase 2: Deploy The Ghost Executor

Aapka ShadowBot contract Base Mainnet par deploy karne ke liye ye command run karein:
```bash
forge create src/ShadowBot.sol:ShadowBot \
  --rpc-url https://mainnet.base.org \
  --private-key 0xYOUR_REAL_BASE_PRIVATE_KEY \
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
4. **Global gRPC:** Sniper hamesha empty RAM ke saath ready rahega, Scouts global data stream karenge.

---
**Lead Architect Note:** Aapka ₹200 ka budget sirf gas ke liye hai. Bot automatically Aave aur Balancer se Flash Loans lega, isliye liquidity ki tension na lein.