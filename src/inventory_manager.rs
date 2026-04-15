use alloy::providers::{Provider, RootProvider};
use alloy::network::TransactionBuilder;
use alloy::signers::local::PrivateKeySigner;
use alloy::transports::BoxTransport;
use alloy_primitives::{Address, U256};
use alloy::sol;
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use tracing::{debug, info, error};

use crate::models::{Chain, MEVError, TOKEN_WETH};
use crate::gas_feed::GasPriceFeed;
use crate::utils::{CircuitBreaker, L1DataFeeCalculator};
use crate::nonce_manager::NonceManager;
use crate::constants::{MIN_SEARCHER_BALANCE_WEI, DUST_THRESHOLD_WEI, BASE_AERODROME_ROUTER, MAX_SWEEP_GAS_PRICE_WEI};

sol! {
    #[sol(rpc)]
    interface IMulticall3 {
        struct Call3 { address target; bool allowFailure; bytes callData; }
        struct Result { bool success; bytes returnData; }
        function aggregate3(Call3[] calldata calls) external payable returns (Result[] memory returnData);
    }

    #[sol(rpc)]
    interface IWETH {
        function withdraw(uint256 wad) external;
        function balanceOf(address account) external view returns (uint256);
    }

    #[sol(rpc)]
    interface IERC20 {
        function balanceOf(address account) external view returns (uint256);
        function approve(address spender, uint256 amount) external returns (bool);
    }

    #[sol(rpc)]
    interface IUniswapV2Router {
        function swapExactTokensForTokens(uint amountIn, uint amountOutMin, address[] calldata path, address to, uint deadline) external returns (uint[] memory amounts);
        function approve(address spender, uint256 amount) external returns (bool);
    }

    #[sol(rpc)]
    interface IShadowBot {
        function withdrawToken(address token) external;
    }
}

/// Pillar J: Inventory manager — sweeps dust tokens to WETH, unwraps WETH for gas.
pub struct InventoryManager {
    provider:        Arc<RootProvider<BoxTransport>>,
    wallet:          PrivateKeySigner,
    executor_address: Address,
    chain:           Chain,
    circuit_breaker: Arc<CircuitBreaker>,
    nonce_manager:   Arc<NonceManager>,
    gas_feed:        Arc<GasPriceFeed>,
    #[allow(dead_code)]
    l1_calc:         Arc<L1DataFeeCalculator>,
    running:         AtomicBool,
}

impl InventoryManager {
    pub fn new(
        provider:        Arc<RootProvider<BoxTransport>>,
        wallet:          PrivateKeySigner,
        executor_address: Address,
        chain:           Chain,
        circuit_breaker: Arc<CircuitBreaker>,
        nonce_manager:   Arc<NonceManager>,
        gas_feed:        Arc<GasPriceFeed>,
        l1_calc:         Arc<L1DataFeeCalculator>,
    ) -> Self {
        Self { provider, wallet, executor_address, chain, circuit_breaker, nonce_manager, gas_feed, l1_calc, running: AtomicBool::new(false) }
    }

    /// Pillar Q: Bootstrap Readiness Check.
    /// Ensures the bot has the minimum survival budget (₹200 / 0.001 ETH) before starting.
    pub async fn ensure_ready(&self) -> Result<(), MEVError> {
        let balance = self.provider.get_balance(self.wallet.address()).await
            .map_err(|e| MEVError::Other(format!("Bootstrap Balance Check Failed: {}", e)))?;
        
        let min_required = U256::from(crate::constants::MIN_SEARCHER_BALANCE_WEI);
        if balance < min_required {
            return Err(MEVError::Other(format!(
                "🛑 [BOOTSTRAP] Insufficient Gas! Have: {:.6} ETH, Need: {:.6} ETH (Survival Rule)",
                balance.to::<u128>() as f64 / 1e18, min_required.to::<u128>() as f64 / 1e18
            )));
        }
        Ok(())
    }

    /// Pillar J: Sweep dust tokens → WETH when gas is cheap.
    pub async fn auto_sweep(&self, tokens: Vec<Address>) -> Result<(), MEVError> {
        if self.running.swap(true, Ordering::SeqCst) { return Ok(()); }

        // Gas-Aware Check: Only sweep when gas is ultra-cheap ( < 0.05 gwei)
        let (base_fee, _, _) = self.gas_feed.current().await;
        if base_fee > U256::from(MAX_SWEEP_GAS_PRICE_WEI) {
            debug!("[PILLAR J] Skipping sweep: Gas price too high ({:.4} gwei)", base_fee.to::<u128>() as f64 / 1e9);
            self.running.store(false, Ordering::SeqCst);
            return Ok(());
        }

        if self.circuit_breaker.is_open() {
            self.running.store(false, Ordering::SeqCst);
            return Ok(());
        }

        let router_address = if self.chain == Chain::Base { BASE_AERODROME_ROUTER } else { crate::constants::UNISWAP_V2_ROUTER };
        debug!("[PILLAR J] auto_sweep using router {:?} on {:?}", router_address, self.chain);

        for token in tokens {
            if token == TOKEN_WETH { continue; }
            
            let token_contract = IERC20::IERC20Instance::new(token, self.provider.clone());
            
            // 1. Check & Pull from ShadowBot Contract first
            let contract_bal = token_contract.balanceOf(self.executor_address).call().await
                .map(|r| r._0).unwrap_or(U256::ZERO);
            
            if contract_bal > U256::from(DUST_THRESHOLD_WEI) {
                info!("[PILLAR J] Pulling dust from ShadowBot: {:?}", token);
                let bot_contract = IShadowBot::IShadowBotInstance::new(self.executor_address, self.provider.clone());
                let call = bot_contract.withdrawToken(token);
                let tx = alloy::rpc::types::TransactionRequest::default()
                    .with_to(self.executor_address)
                    .with_from(self.wallet.address())
                    .with_input(call.calldata().clone())
                    .with_nonce(self.nonce_manager.next());
                
                if let Ok(pending) = self.provider.send_transaction(tx).await {
                    let _ = pending.watch().await;
                }
            }

            // 2. Sweep from EOA
            let balance = token_contract.balanceOf(self.wallet.address()).call().await
                .map(|r| r._0).unwrap_or(U256::ZERO);

            if balance >= U256::from(DUST_THRESHOLD_WEI) {
                info!("[PILLAR J] Sweeping dust for token {:?}", token);

                // 2.1 Approve router
                let approve_call = token_contract.approve(router_address, balance);
                let nonce = self.nonce_manager.next();
                let approve_tx = alloy::rpc::types::TransactionRequest::default()
                    .with_to(token)
                    .with_from(self.wallet.address())
                    .with_input(approve_call.calldata().to_vec())
                    .with_nonce(nonce);
                
                if let Err(e) = self.provider.send_transaction(approve_tx).await {
                    error!("❌ [PILLAR J] Approval failed for {:?}: {}", token, e);
                    self.running.store(false, Ordering::SeqCst);
                    continue;
                }

                // 2.2 Swap to WETH
                let router = IUniswapV2Router::IUniswapV2RouterInstance::new(router_address, self.provider.clone());
                let path = vec![token, TOKEN_WETH];
                let deadline = U256::from(SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() + 60);
                
                let swap_call = router.swapExactTokensForTokens(balance, U256::ZERO, path, self.wallet.address(), deadline);
                let nonce = self.nonce_manager.next();
                let p = self.provider.clone();
                let swap_tx = alloy::rpc::types::TransactionRequest::default()
                    .with_to(router_address)
                    .with_from(self.wallet.address())
                    .with_input(swap_call.calldata().clone())
                    .with_nonce(nonce);
                
                tokio::spawn(async move {
                    match p.send_transaction(swap_tx).await {
                        Ok(pending) => {
                            let _ = pending.watch().await;
                            info!("✅ [PILLAR J] Swept token {:?} to WETH", token);
                        },
                        Err(e) => error!("❌ [PILLAR J] Failed to sweep token {:?}: {}", token, e),
                    }
                });
            }
        }

        self.running.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// Pillar J: Unwrap WETH → ETH when native gas balance is low.
    pub async fn unwrap_weth_if_needed(&self) -> Result<(), MEVError> {
        let address = self.wallet.address();
        let eth_balance: U256 = self.provider.get_balance(address).await
            .map_err(|e| MEVError::Other(e.to_string()))?;

        let threshold = U256::from(MIN_SEARCHER_BALANCE_WEI);

        if eth_balance < threshold {
            let weth_contract = IWETH::IWETHInstance::new(TOKEN_WETH, self.provider.clone());
            let weth_balance = weth_contract.balanceOf(address).call().await
                .map(|r| r._0).unwrap_or(U256::ZERO);

            if weth_balance > U256::ZERO {
                // Withdraw enough to reach threshold * 2 (safety buffer)
                let deficit = threshold.saturating_mul(U256::from(2)).saturating_sub(eth_balance);
                let amount_to_withdraw = deficit.min(weth_balance);

                info!("[PILLAR J] ETH balance low ({:?}). Unwrapping {:?} WETH", eth_balance, amount_to_withdraw);

                let call = weth_contract.withdraw(amount_to_withdraw);
                let nonce = self.nonce_manager.next();
                let tx = alloy::rpc::types::TransactionRequest::default()
                    .with_to(TOKEN_WETH)
                    .with_from(address)
                    .with_input(call.calldata().clone())
                    .with_nonce(nonce);

                match self.provider.send_transaction(tx).await {
                    Ok(_) => info!("✅ [PILLAR J] WETH unwrapped successfully"),
                    Err(e) => error!("❌ [PILLAR J] Failed to unwrap WETH: {}", e),
                }
            }
        }

        Ok(())
    }
}
