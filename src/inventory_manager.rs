// Pillar J: The Inventory Manager (Token Rebalancing)
use crate::models::MEVError;
use crate::utils::L1DataFeeCalculator;
use crate::constants::{self, TOKEN_WETH, DUST_THRESHOLD_WEI};
use ethers::prelude::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use ethers::types::transaction::eip2718::TypedTransaction;
use tracing::{info, warn, debug};

pub struct InventoryManager<M: Middleware> {
    provider: Arc<M>,
    wallet: LocalWallet,
    l1_fee_calc: Arc<L1DataFeeCalculator>,
    chain: Chain,
    lowest_gas_seen: Arc<AtomicU64>, // Pillar J: Gas floor tracking
}

impl<M: Middleware + 'static> InventoryManager<M> {
    pub fn new(
        provider: Arc<M>,
        wallet: LocalWallet,
        l1_fee_calc: Arc<L1DataFeeCalculator>,
        chain: Chain,
    ) -> Self {
        Self {
            provider,
            wallet,
            l1_fee_calc,
            chain,
            lowest_gas_seen: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Pillar J: Fully Autonomous Dust Management
    /// Scans a list of tokens and sweeps them to WETH if gas conditions are optimal.
    pub async fn auto_sweep(&self, tokens: Vec<Address>) -> Result<(), MEVError> {
        let gas_price = self.provider.get_gas_price().await.map_err(|e| MEVError::Other(e.to_string()))?;
        let current_gas = gas_price.as_u64();

        // Pillar J: Market Floor Tracking
        let prev_lowest = self.lowest_gas_seen.load(Ordering::Acquire);
        if prev_lowest == 0 || current_gas < prev_lowest {
            self.lowest_gas_seen.store(current_gas, Ordering::Release);
            info!("📉 [PILLAR J] New lowest gas floor detected: {} wei", current_gas);
        }

        // Golden Window Logic: Execute if gas is within 15% of floor or below 0.1 Gwei
        let hard_floor = 100_000_000u64; // 0.1 Gwei floor
        let window_threshold = if prev_lowest > 0 { (prev_lowest * 115) / 100 } else { hard_floor };
        
        let is_golden_window = current_gas <= window_threshold || current_gas <= hard_floor;

        // Pillar Q: Bootstrap Survival Mode
        // If balance < 0.0005 ETH (~120 INR), we ignore gas windows and sweep everything to stay alive.
        let eth_balance = self.provider.get_balance(self.wallet.address(), None).await.unwrap_or_default();
        let survival_threshold = U256::from(500_000_000_000_000u64); // 0.0005 ETH
        let is_emergency = eth_balance < survival_threshold;

        if !is_golden_window && !is_emergency {
            debug!("[PILLAR J] Waiting for Golden Window. Balance healthy.");
            return Ok(());
        }

        if is_emergency {
            warn!("🚨 [PILLAR Q] Survival Mode Active! Low ETH balance: {} wei. Sweeping all dust.", eth_balance);
        }

        for token in tokens {
            if token == TOKEN_WETH { continue; }
            
            let erc20 = IERC20::new(token, self.provider.clone());
            let balance = erc20.balance_of(self.wallet.address()).call().await
                .map_err(|e| MEVError::Other(e.to_string()))?;

            if balance.is_zero() { continue; }

            // Attempt sweep
            if let Err(e) = self.sweep_to_weth(token, balance, gas_price).await {
                warn!("[Pillar J] Auto-sweep failed for {:?}: {}", token, e);
            }
        }
        Ok(())
    }

    /// [PILLAR J] Unstoppable Compounding: Converts WETH profit to ETH gas reserve.
    pub async fn unwrap_weth_if_needed(&self) -> Result<(), MEVError> {
        let eth_balance = self.provider.get_balance(self.wallet.address(), None).await.unwrap_or_default();
        let min_eth = U256::from(500_000_000_000_000u64); // 0.0005 ETH floor

        if eth_balance < min_eth {
            let weth_contract = IWETH::new(TOKEN_WETH, self.provider.clone());
            let weth_bal = weth_contract.balance_of(self.wallet.address()).call().await.unwrap_or_default();

            if weth_bal > U256::from(10u128.pow(15)) { // At least 0.001 WETH to bother
                info!("🔄 [PILLAR J] ETH low. Unwrapping {} WETH for gas compounding.", weth_bal);
                let mut tx: TypedTransaction = weth_contract.withdraw(weth_bal).tx.into();
                
                self.provider.fill_transaction(&mut tx, None).await.map_err(|e| MEVError::Other(e.to_string()))?;
                let signature = self.wallet.sign_transaction(&tx).await.map_err(MEVError::WalletError)?;
                let raw_tx = tx.rlp_signed(&signature);

                match self.provider.send_raw_transaction(raw_tx).await {
                    Ok(pending) => {
                        info!("✅ [PILLAR J] WETH Unwrapped: {:?}", pending.tx_hash());
                    }
                    Err(e) => {
                        warn!("⚠️ [PILLAR J] WETH Unwrap failed: {}", e);
                    }
                }
            }
        }
        Ok(())
    }

    fn get_router(&self) -> Address {
        match self.chain {
            Chain::Base => constants::BASE_SUSHISWAP_ROUTER,
            Chain::Mainnet => constants::UNISWAP_V2_ROUTER,
            Chain::Arbitrum => constants::ARB_SUSHISWAP_ROUTER,
            _ => constants::UNISWAP_V2_ROUTER,
        }
    }

    /// Sweeps a given token balance to WETH if profitable.
    pub async fn sweep_to_weth(&self, token_address: Address, amount: U256, gas_price: U256) -> Result<(), MEVError> {
        if token_address == TOKEN_WETH || amount.is_zero() {
            return Ok(());
        }

        let router_addr = self.get_router();
        info!("[Pillar J] Evaluating sweep for {} of token {}", amount, token_address);

        // 1. Get expected WETH amount out
        let path = vec![token_address, TOKEN_WETH];
        let amounts_out = self.get_amounts_out(amount, &path).await?;
        let weth_out = amounts_out.last().cloned().unwrap_or_default();

        // Pillar J: Threshold Check (0.001 ETH by default)
        if weth_out < U256::from(DUST_THRESHOLD_WEI) {
            info!("[Pillar J] Balance too small to sweep ({} < {} wei)", weth_out, DUST_THRESHOLD_WEI);
            return Ok(());
        }

        // 2. Build transaction and estimate gas cost
        let calldata = self.build_swap_calldata(router_addr, amount, weth_out, &path)?;
        let tx_request = Eip1559TransactionRequest::new()
            .to(router_addr)
            .data(calldata.clone())
            .from(self.wallet.address());
        
        let gas_estimate = self.provider.estimate_gas(&tx_request.clone().into(), None).await.map_err(|e| MEVError::Other(e.to_string()))?;
        // Pillar J: Apply gas buffer to prevent marginal losses
        let mut total_gas_cost = (gas_estimate * gas_price) * (100 + constants::GAS_BUFFER_PERCENT) / 100;

        // 3. Add L1 fee for L2s
        if self.chain != Chain::Mainnet {
            let l1_fee = self.l1_fee_calc.estimate_l1_fee(self.chain, &calldata).await?;
            total_gas_cost += l1_fee;
        }

        // Pillar J: Include Approval cost if necessary
        let contract = IERC20::new(token_address, self.provider.clone());
        let allowance = contract.allowance(self.wallet.address(), router_addr).call().await.map_err(|e| MEVError::Other(e.to_string()))?;
        if allowance < amount {
            // Estimated gas for ERC20 Approve is ~45k
            total_gas_cost += U256::from(45_000u64) * gas_price;
        }

        // 4. Safety Check: Execute only if profitable
        if weth_out > total_gas_cost {
            info!("[Pillar J] Executing profitable sweep. WETH out: {}, Cost: {}", weth_out, total_gas_cost);
            
            // Approve router to spend token
            self.approve(token_address, router_addr, amount).await?;

            // Build transaction
            let mut tx: TypedTransaction = Eip1559TransactionRequest::new()
                .to(router_addr)
                .data(calldata.clone())
                .from(self.wallet.address())
                .into();
            
            // Fill, Sign, and Send Raw
            self.provider.fill_transaction(&mut tx, None).await.map_err(|e| MEVError::Other(e.to_string()))?;
            let signature = self.wallet.sign_transaction(&tx).await.map_err(MEVError::WalletError)?;
            let raw_tx = tx.rlp_signed(&signature);

            let pending_tx = self.provider.send_raw_transaction(raw_tx)
                .await
                .map_err(|e| MEVError::Other(e.to_string()))?;
            info!("[Pillar J] Sweep transaction sent: {:?}", pending_tx.tx_hash());
            // In a real scenario, you'd wait for confirmation.
        } else {
            warn!("[Pillar J] Sweep unprofitable. WETH out: {}, Cost: {}", weth_out, total_gas_cost);
        }

        Ok(())
    }

    /// Approves the router to spend a token.
    async fn approve(&self, token: Address, spender: Address, amount: U256) -> Result<(), MEVError> {
        let contract = IERC20::new(token, self.provider.clone());
        let allowance = contract.allowance(self.wallet.address(), spender).call().await.map_err(|e| MEVError::Other(e.to_string()))?;
        
        if allowance < amount {
            info!("[Pillar J] Approving {} to spend token {}", spender, token);
            
            let call = contract.approve(spender, amount);
            let mut tx: TypedTransaction = call.tx.into();
            
            // Fill, Sign, and Send Raw
            self.provider.fill_transaction(&mut tx, None).await.map_err(|e| MEVError::Other(e.to_string()))?;
            let signature = self.wallet.sign_transaction(&tx).await.map_err(MEVError::WalletError)?;
            let raw_tx = tx.rlp_signed(&signature);

            let pending_tx = self.provider.send_raw_transaction(raw_tx).await
                .map_err(|e| MEVError::Other(e.to_string()))?;
            info!("[Pillar J] Approval transaction sent: {:?}", pending_tx.tx_hash());
                
            let _ = pending_tx.await.map_err(|e| MEVError::Other(e.to_string()))?;
        }
        Ok(())
    }

    /// Builds calldata for `swapExactTokensForTokens`.
    fn build_swap_calldata(&self, router_addr: Address, amount_in: U256, amount_out_min: U256, path: &[Address]) -> Result<Bytes, MEVError> {
        let router = IUniswapV2Router02::new(router_addr, self.provider.clone());
        let deadline = U256::from(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() + 300); // 5 min deadline
        
        let call = router.swap_exact_tokens_for_tokens(
            amount_in,
            amount_out_min * 99 / 100, // 1% slippage
            path.to_vec(),
            self.wallet.address(),
            deadline,
        );
        Ok(call.calldata().unwrap())
    }

    /// Calls the router's `getAmountsOut` function.
    async fn get_amounts_out(&self, amount_in: U256, path: &[Address]) -> Result<Vec<U256>, MEVError> {
        let router_addr = self.get_router();
        let router = IUniswapV2Router02::new(router_addr, self.provider.clone());
        let amounts = router.get_amounts_out(amount_in, path.to_vec()).call().await
            .map_err(|e| MEVError::Other(format!("getAmountsOut failed: {}", e)))?;
        Ok(amounts)
    }
}

// We need a partial ABI for the router and ERC20
abigen!(
    IUniswapV2Router02,
    r#"[
        function getAmountsOut(uint amountIn, address[] calldata path) external view returns (uint[] memory amounts)
        function swapExactTokensForTokens(uint amountIn, uint amountOutMin, address[] calldata path, address to, uint deadline) external returns (uint[] memory amounts)
    ]"#
);

abigen!(
    IERC20,
    r#"[
        function approve(address spender, uint256 amount) external returns (bool)
        function allowance(address owner, address spender) external view returns (uint256)
        function balanceOf(address account) external view returns (uint256)
    ]"#
);

abigen!(
    IWETH,
    r#"[
        function withdraw(uint256 wad) external
        function balance_of(address account) external view returns (uint256)
    ]"#
);